//! 2. The gong: a bronze gong swings down into the middle of everyone's screen and a
//! mallet winds up and strikes it. Another gong while it hangs there hits it again,
//! harder each time.
//!
//! The wooden rig, the cords and the mallet turn as they swing, so they're polygons, their
//! CSS gradients laid down in bands (see [`gradient`]); the round things (the disc, the
//! caps, the mallet's head) are pictures that move with them without turning.

use std::{collections::HashMap, f32::consts::SQRT_2, sync::Arc};

use super::Effect;
use crate::protocol::Reaction;
use crate::ui::reactions::{
    looks::{self, FONT, Look},
    particles::{Emit, Shape},
    scene::{Canvas, place},
    stage::{Clock, Stage, Who, bezier, clamp01, ease, ease_in_out, ease_out, lerp, mix, rgb},
    synth::{Filter, Graph, Noise, Tone, Wave},
};

const WOOD_V: [(f32, [u8; 3]); 5] = [
    (0., rgb(0x2e1406)),
    (0.22, rgb(0x6e3514)),
    (0.5, rgb(0xa25a2a)),
    (0.78, rgb(0x6e3514)),
    (1., rgb(0x2e1406)),
];
pub(super) const WOOD_H: [(f32, [u8; 3]); 3] = [
    (0., rgb(0xb5683a)),
    (0.45, rgb(0x7a3d18)),
    (1., rgb(0x3b1a09)),
];
const CORD: [(f32, [u8; 3]); 3] = [
    (0., rgb(0x1b0d04)),
    (0.5, rgb(0x5a3a1c)),
    (1., rgb(0x1b0d04)),
];
/// The disc's bronze, round from 25° past the top.
const BRONZE: [&str; 9] = [
    "#8a5418", "#f6d27a", "#a8681f", "#e9b85a", "#7a4612", "#f1c66b", "#9c5f1c", "#ffe39a",
    "#8a5418",
];
/// The sheen sweeping round the disc: (where round it, opacity).
const SHEEN: [(f32, f32); 11] = [
    (0., 0.),
    (0.08, 0.),
    (0.13, 0.75),
    (0.19, 0.),
    (0.44, 0.),
    (0.5, 0.6),
    (0.56, 0.),
    (0.8, 0.),
    (0.86, 0.45),
    (0.92, 0.),
    (1., 0.),
];
const SPARKS: [[u8; 3]; 4] = [rgb(0xfff3c4), rgb(0xffd27a), rgb(0xffb347), rgb(0xffe9a0)];
const HINT: &str = "hit it again for a bigger gong";
/// When the mallet first winds up, and how long that swing and a quick one take.
const INTRO_SWING: f32 = 0.43;
const INTRO_SWING_SECONDS: f32 = 0.82;
const SWING_SECONDS: f32 = 0.15;
const LEAVE_SECONDS: f32 = 0.82;

/// The rig's measures for a gong `d` across, as the prototype's `build` lays them out:
/// from the rig's top left unless they say otherwise.
struct Rig {
    d: f32,
    r: f32,
    width: f32,
    height: f32,
    bar: f32,
    post: f32,
    cord: f32,
    hang_top: f32,
    cx: f32,
    cy: f32,
    /// The rig's top left on screen.
    left: f32,
    top: f32,
    /// The mallet's pivot and where its head rests, on the boss.
    pivot: [f32; 2],
    tip: [f32; 2],
    length: f32,
    /// The mallet's turn at rest, in degrees.
    rest: f32,
    handle: f32,
    head: f32,
    cord_x: f32,
    cord_length: f32,
    cord_turn: f32,
}

impl Rig {
    fn new(w: f32, h: f32) -> Self {
        let d = (h * 0.5).min(w * 0.36).min(480.).round();
        let r = d / 2.;
        let (width, bar, post, cord, height) = (d * 1.55, d * 0.075, d * 0.075, d * 0.16, d * 1.6);
        let hang_top = bar * 0.6;
        let (cx, cy) = (width / 2., hang_top + cord + r);
        let pivot = [cx + 1.05 * r, cy + 0.95 * r];
        let tip = [cx + 0.2 * r, cy + 0.14 * r];
        let (cord_x, cord_end) = (d * 0.2, d * 0.13);
        Self {
            d,
            r,
            width,
            height,
            bar,
            post,
            cord,
            hang_top,
            cx,
            cy,
            left: w / 2. - width / 2.,
            top: h / 2. - cy + 8.,
            pivot,
            tip,
            length: (pivot[0] - tip[0]).hypot(pivot[1] - tip[1]),
            rest: (pivot[1] - tip[1]).atan2(pivot[0] - tip[0]).to_degrees(),
            handle: d * 0.045,
            head: d * 0.17,
            cord_x,
            cord_length: (cord_x - cord_end).hypot(cord + d * 0.02),
            cord_turn: (cord_x - cord_end).atan2(cord).to_degrees(),
        }
    }
}

/// An animation of the gong swinging on its cords: degrees at even steps, eased out.
#[derive(Clone, Copy)]
struct Swing {
    at: f32,
    angles: [f32; 7],
    seconds: f32,
}

#[derive(Clone, Copy)]
enum Mallet {
    /// Not yet shown.
    Waiting,
    /// Winding up and coming down onto the gong.
    Swing { at: f32, intro: bool },
    /// Bouncing back off it, then resting.
    Rebound { at: f32 },
}

/// A ring of light spreading out over the whole screen from a strike.
struct Ring {
    at: f32,
    centre: [f32; 2],
    seconds: f32,
    /// How many times its size it grows to.
    grow: f32,
    border: f32,
    first: bool,
}

struct Pictures {
    disc: Arc<Look>,
    cap: Arc<Look>,
    head: Arc<Look>,
    glow: Arc<Look>,
    dim: Arc<Look>,
    hint: Arc<Look>,
}

pub struct Gong {
    rig: Rig,
    pictures: Pictures,
    /// The screen's diagonal, for how far the rings spread.
    diagonal: f32,
    now: f32,
    power: u8,
    counts: HashMap<String, u32>,
    mallet: Mallet,
    /// When the mallet lands, and how hard.
    strike: Option<(f32, u8)>,
    hang: Option<Swing>,
    /// When the disc was last struck and how far it wobbles, in degrees.
    wobble: Option<(f32, f32)>,
    glow: Option<(f32, f32)>,
    /// The sheen's turn: since when, from and to (degrees).
    sheen: (f32, f32, f32),
    rings: Vec<Ring>,
    hint: Option<f32>,
    leave_at: Option<f32>,
    /// When it began to fly away.
    left: Option<f32>,
}

impl Gong {
    pub fn new(stage: &mut Stage, who: &Who) -> Self {
        let rig = Rig::new(stage.w, stage.h);
        let pictures = Pictures {
            disc: disc_look(rig.d),
            cap: cap_look(rig.bar * 1.3),
            head: head_look(rig.head),
            glow: glow_look(),
            dim: dim_look(),
            hint: hint_look(),
        };
        let mut gong = Self {
            pictures,
            diagonal: stage.w.hypot(stage.h),
            now: 0.,
            power: 1,
            counts: HashMap::new(),
            mallet: Mallet::Waiting,
            strike: None,
            hang: None,
            wobble: None,
            glow: None,
            sheen: (0., 0., 0.),
            rings: Vec::new(),
            hint: None,
            leave_at: None,
            left: None,
            rig,
        };
        whoosh(stage.sound(), false);
        gong.announce(stage, who);
        gong
    }

    fn announce(&mut self, stage: &mut Stage, who: &Who) {
        let n = self.counts.entry(who.key.clone()).or_insert(0);
        *n += 1;
        let text = if *n > 1 {
            format!("hit the gong ×{n}")
        } else {
            "hit the gong".into()
        };
        stage.banner(who, &text, "🔔", 3400.);
    }

    /// The mallet winds up from `at`; it lands a swing's time later.
    fn swing(&mut self, stage: &mut Stage, at: f32, intro: bool) {
        let seconds = if intro {
            INTRO_SWING_SECONDS
        } else {
            SWING_SECONDS
        };
        self.mallet = Mallet::Swing { at, intro };
        self.strike = Some((at + seconds, if intro { 1 } else { self.power }));
        stage.sound().noise(Noise {
            when: seconds * if intro { 0.84 } else { 0.5 },
            filter: Filter::Bandpass,
            freq: 500.,
            sweep_to: Some(1900.),
            q: 1.,
            dur: 0.16,
            gain: 0.13,
            attack: 0.12,
            ..Noise::default()
        });
    }

    fn strike(&mut self, stage: &mut Stage, at: f32, power: u8) {
        let p = f32::from(power);
        let centre = self.disc_centre(at);
        self.wobble = Some((at, (5. + p * 3.).min(22.)));
        let sw = (1.6 + p).min(7.);
        self.hang = Some(Swing {
            at,
            angles: [0., sw, -sw * 0.75, sw * 0.5, -sw * 0.3, sw * 0.12, 0.],
            seconds: 2.3,
        });
        let turned = self.sheen.2;
        self.sheen = (at, turned, turned + 100. + p * 45.);
        self.glow = Some((at, p));
        let grow = self.diagonal / self.rig.d * 1.2;
        for i in 0..(2 + power).min(7) {
            let i = f32::from(i);
            self.rings.push(Ring {
                at: at + i * 0.115,
                centre,
                seconds: 1.4 + i * 0.14 + p * 0.06,
                grow,
                border: 3. + p + if i == 0. { 3. } else { 0. },
                first: i == 0.,
            });
        }
        stage.shake(9. + p * 4., 650. + p * 110.);
        stage.flash(rgb(0xffcf70), 420., (0.1 + p * 0.05).min(0.4));
        let rig = &self.rig;
        stage.emit(Emit {
            x: rig.left + rig.tip[0],
            y: rig.top + rig.tip[1],
            count: 18 + usize::from(power) * 12,
            angle: 0.,
            spread: 360.,
            speed: [3., 8. + p * 2.],
            gravity: 0.18,
            drag: 0.96,
            life: [30., 70.],
            size: [3., 6.],
            shape: Shape::Circle,
            glow: true,
            colours: &SPARKS,
            ..Emit::default()
        });
        gong_sound(stage.sound(), p);
        if self.hint.is_none() {
            self.hint = Some(at + 0.5);
        }
        self.leave_at = Some(at + if power == 1 { 3.3 } else { 2.7 });
    }

    fn leave(&mut self, stage: &mut Stage, at: f32) {
        self.left = Some(at);
        self.leave_at = None;
        whoosh(stage.sound(), true);
    }

    /// Where the rig is, down from its place, and its turn in degrees; None once it's gone.
    fn rig_pose(&self, t: f32) -> Option<[f32; 2]> {
        let rig = &self.rig;
        if let Some(left) = self.left {
            let u = (t - left) / LEAVE_SECONDS;
            return (u < 1.).then(|| {
                frames(
                    &[
                        (0., [0., 0.], ease_out),
                        (0.24, [18., -1.], |t| bezier(0.55, 0., 0.85, 0.35, t)),
                        (1., [-(rig.top + rig.height + 80.), 5.], linear),
                    ],
                    u,
                )
            });
        }
        Some(frames(
            &[
                (0., [-(rig.top + rig.height + 40.), -7.], |t| {
                    bezier(0.45, 0., 0.9, 0.55, t)
                }),
                (0.68, [16., 1.], |t| bezier(0.2, 0.7, 0.4, 1., t)),
                (0.85, [-5., -0.3], ease_in_out),
                (1., [0., 0.], linear),
            ],
            t / 0.76,
        ))
    }

    /// The gong's swing on its cords, in degrees.
    fn hang_turn(&self, t: f32) -> f32 {
        let Some(swing) = self.hang else { return 0. };
        let u = (t - swing.at) / swing.seconds;
        if !(0. ..1.).contains(&u) {
            return 0.;
        }
        let steps: Vec<_> = (0..7)
            .map(|i| (i as f32 / 6., [swing.angles[i]], linear as Easing))
            .collect();
        frames(&steps, ease_out(u))[0]
    }

    /// The disc's wobble after a strike: its turns about the upright and the level (as
    /// squashes), and its scale.
    fn wobble(&self, t: f32) -> [f32; 3] {
        let Some((at, a)) = self.wobble else {
            return [1., 1., 1.];
        };
        let u = (t - at) / 1.7;
        if !(0. ..1.).contains(&u) {
            return [1., 1., 1.];
        }
        let steps: Vec<_> = (0..10)
            .map(|i| {
                let k = i as f32 / 9.;
                let amp = a * (1. - k).powf(1.6);
                let sign = if i % 2 == 1 { 1. } else { -1. };
                match i {
                    0 => (0., [0., 0., 1.], linear as Easing),
                    1 => (
                        0.04,
                        [amp * sign, -amp * 0.4 * sign, 0.95],
                        linear as Easing,
                    ),
                    _ => (k, [amp * sign, -amp * 0.4 * sign, 1.], linear as Easing),
                }
            })
            .collect();
        let [y, x, scale] = frames(&steps, ease_out(u));
        [
            y.to_radians().cos() * scale,
            x.to_radians().cos() * scale,
            scale,
        ]
    }

    /// The mallet's turn (degrees), scale and opacity.
    fn mallet_pose(&self, t: f32) -> [f32; 3] {
        let r = self.rig.rest;
        match self.mallet {
            Mallet::Waiting => [r + 30., 1., 0.],
            Mallet::Swing { at, intro: true } => frames(
                &[
                    (0., [r + 35., 1., 0.], ease_out),
                    (0.2, [r + 18., 1., 1.], |t| bezier(0.2, 0.8, 0.3, 1., t)),
                    (0.72, [r + 74., 1.2, 1.], linear),
                    (0.84, [r + 77., 1.22, 1.], |t| bezier(0.7, 0., 1., 0.6, t)),
                    (1., [r - 3., 0.95, 1.], linear),
                ],
                (t - at) / INTRO_SWING_SECONDS,
            ),
            Mallet::Swing { at, intro: false } => frames(
                &[
                    (0., [r + 14., 1., 1.], |t| bezier(0.2, 0.8, 0.3, 1., t)),
                    (0.55, [r + 52., 1.12, 1.], |t| bezier(0.7, 0., 1., 0.6, t)),
                    (1., [r - 3., 0.95, 1.], linear),
                ],
                (t - at) / SWING_SECONDS,
            ),
            Mallet::Rebound { at } => frames(
                &[
                    (0., [r - 3., 0.95, 1.], linear),
                    (0.35, [r + 22., 1.04, 1.], linear),
                    (1., [r + 14., 1., 1.], linear),
                ],
                ease_out((t - at) / 0.52),
            ),
        }
    }

    /// From the rig's own place to the screen, posed at `pose`.
    fn on_rig(&self, pose: [f32; 2]) -> impl Fn([f32; 2]) -> [f32; 2] + '_ {
        let rig = &self.rig;
        let centre = [
            rig.left + rig.width / 2.,
            rig.top + rig.height / 2. + pose[0],
        ];
        let turn = pose[1].to_radians();
        move |p| {
            place(
                [p[0] - rig.width / 2., p[1] - rig.height / 2.],
                centre,
                turn,
                1.,
            )
        }
    }

    /// From the hanging gong's own place (its top left, above the cords) to the rig's.
    fn on_hang(&self, t: f32) -> impl Fn([f32; 2]) -> [f32; 2] + '_ {
        let rig = &self.rig;
        let turn = self.hang_turn(t).to_radians();
        move |p| place([p[0] - rig.r, p[1]], [rig.cx, rig.hang_top], turn, 1.)
    }

    fn disc_centre(&self, t: f32) -> [f32; 2] {
        let pose = self.rig_pose(t).unwrap_or([0., 0.]);
        self.on_rig(pose)(self.on_hang(t)([self.rig.r, self.rig.cord + self.rig.r]))
    }

    fn draw_rig(&self, canvas: &mut Canvas, t: f32, pose: [f32; 2]) {
        let rig = &self.rig;
        let on_rig = self.on_rig(pose);
        let (d, pw, bh) = (rig.d, rig.post, rig.bar);
        let post_height = rig.height - pw * 0.6;
        for x in [0., rig.width - pw] {
            let shape = rounded([x, 0.], [pw, post_height], [6., 6., 3., 3.]);
            shadow(canvas, &shape, [0., 10.], 25., 0.45, &on_rig);
            gradient(canvas, &shape, 0, [x, x + pw], &WOOD_V, 1., &on_rig);
            grain(canvas, &shape, 0, [x, x + pw], 13., &on_rig);
            // Its inset shade on the right.
            let shade = slab(&shape, 0, x + pw - 5., x + pw);
            canvas.polygon(shade.into_iter().map(&on_rig), [0, 0, 0], 0.18);
        }
        for x in [-pw, rig.width - pw * 2.] {
            let y = rig.height - pw * 0.8;
            let shape = rounded([x, y], [pw * 3., pw * 0.8], [8., 8., 4., 4.]);
            shadow(canvas, &shape, [0., 10.], 18., 0.5, &on_rig);
            gradient(canvas, &shape, 1, [y, y + pw * 0.8], &WOOD_H, 1., &on_rig);
            grain(canvas, &shape, 1, [y, y + pw * 0.8], 12., &on_rig);
        }
        let y = -bh * 0.3;
        let bar = rounded([-pw * 0.8, y], [rig.width + pw * 1.6, bh], [10.; 4]);
        shadow(canvas, &bar, [0., 8.], 20., 0.45, &on_rig);
        gradient(canvas, &bar, 1, [y, y + bh], &WOOD_H, 1., &on_rig);
        grain(canvas, &bar, 1, [y, y + bh], 12., &on_rig);
        let lit = slab(&bar, 1, y, y + 2.);
        canvas.polygon(lit.into_iter().map(&on_rig), rgb(0xffd2a0), 0.35);
        let cap = bh * 1.3;
        for x in [-pw * 1.1, rig.width + pw * 1.1 - cap] {
            let centre = on_rig([x + cap / 2., -bh * 0.45 + cap / 2.]);
            canvas.picture(&self.pictures.cap, centre, 1., 1.);
        }
        if let Some((at, p)) = self.glow {
            let u = (t - at) / (1.4 + p * 0.12);
            if (0. ..1.).contains(&u) {
                let [alpha, scale] = frames(
                    &[
                        (0., [0., 0.8], linear),
                        (0.08, [(0.6 + p * 0.1).min(1.), 1.1], linear),
                        (1., [0., 1.5], linear),
                    ],
                    ease_out(u),
                );
                let size = d * 2. * scale;
                let centre = on_rig([rig.cx, rig.cy]);
                canvas.picture_in(
                    &self.pictures.glow,
                    [centre[0] - size / 2., centre[1] - size / 2.],
                    [size, size],
                    alpha,
                );
            }
        }
        // The gong on its cords.
        let on_hang = self.on_hang(t);
        let hung = |p: [f32; 2]| on_rig(on_hang(p));
        for (x, turn) in [
            (rig.r - rig.cord_x, -rig.cord_turn),
            (rig.r + rig.cord_x, rig.cord_turn),
        ] {
            let cord = rounded([-2., 0.], [4., rig.cord_length], [2.; 4]);
            let turn = turn.to_radians();
            let on_cord = |p: [f32; 2]| hung(place(p, [x, 0.], turn, 1.));
            gradient(canvas, &cord, 0, [-2., 2.], &CORD, 1., &on_cord);
        }
        let [wide, tall, _] = self.wobble(t);
        let centre = hung([rig.r, rig.cord + rig.r]);
        let [w, h] = self.pictures.disc.size;
        canvas.picture_in(
            &self.pictures.disc,
            [centre[0] - w * wide / 2., centre[1] - h * tall / 2.],
            [w * wide, h * tall],
            1.,
        );
        // The sheen, laid over the bronze (in the prototype it's an overlay blend).
        let (at, from, to) = self.sheen;
        let turned = lerp(from, to, bezier(0.1, 0.8, 0.2, 1., (t - at) / 2.8));
        let squash = |p: [f32; 2]| hung([rig.r + p[0] * wide, rig.cord + rig.r + p[1] * tall]);
        for pair in SHEEN.windows(2) {
            let ((a, alpha_a), (b, alpha_b)) = (pair[0], pair[1]);
            if alpha_a == 0. && alpha_b == 0. {
                continue;
            }
            let slices = 6;
            for i in 0..slices {
                let (k0, k1) = (i as f32 / slices as f32, (i + 1) as f32 / slices as f32);
                let alpha = lerp(alpha_a, alpha_b, (k0 + k1) / 2.);
                let from = (lerp(a, b, k0) * 360. + turned).to_radians();
                let to = (lerp(a, b, k1) * 360. + turned).to_radians();
                let r = rig.r - 1.;
                let mut points = vec![squash([0., 0.])];
                for s in 0..=3 {
                    let angle = lerp(from, to, s as f32 / 3.);
                    points.push(squash([angle.sin() * r, -angle.cos() * r]));
                }
                canvas.polygon(points, rgb(0xfffae1), alpha * 0.4);
            }
        }
        // The mallet.
        let [turn, k, alpha] = self.mallet_pose(t);
        let pivot = rig.pivot;
        let turn = turn.to_radians();
        let on_mallet = |p: [f32; 2]| {
            on_rig(place(
                [(p[0] - pivot[0]) * k, (p[1] - pivot[1]) * k],
                pivot,
                turn,
                1.,
            ))
        };
        let (mh, hs) = (rig.handle, rig.head);
        let top = pivot[1] - mh / 2.;
        let handle = rounded([pivot[0] - rig.length, top], [rig.length, mh], [6.; 4]);
        shadow(canvas, &handle, [0., 6.], 14., 0.45 * alpha, &on_mallet);
        gradient(
            canvas,
            &handle,
            1,
            [top, top + mh],
            &WOOD_H,
            alpha,
            &on_mallet,
        );
        let head = on_mallet([pivot[0] - rig.length + hs * 0.05, pivot[1]]);
        canvas.picture(&self.pictures.head, head, k, alpha);
    }

    fn draw_ring(&self, canvas: &mut Canvas, ring: &Ring) {
        let u = (self.now - ring.at) / ring.seconds;
        if !(0. ..1.).contains(&u) {
            return;
        }
        let k = bezier(0.12, 0.7, 0.3, 1., u);
        let scale = lerp(0.96, ring.grow, k);
        let alpha = lerp(0.95, 0., k);
        let (half, b, c) = (self.rig.d / 2., ring.border, ring.centre);
        // Its glow, outside and in.
        let glow = rgb(0xffb43c);
        canvas.ring(c, (half + 8.) * scale, 18. * scale, glow, 0.22 * alpha);
        canvas.ring(c, (half - b - 8.) * scale, 18. * scale, glow, 0.14 * alpha);
        if ring.first {
            canvas.ring(
                c,
                half * 0.9 * scale,
                half * 0.16 * scale,
                rgb(0xffdc8c),
                0.14 * alpha,
            );
        }
        canvas.ring(
            c,
            (half - b / 2.) * scale,
            b * scale,
            rgb(0xffde96),
            0.9 * alpha,
        );
    }
}

impl Effect for Gong {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        self.now = clock.now;
        if clock.crossed(INTRO_SWING) && self.left.is_none() {
            self.swing(stage, INTRO_SWING, true);
        }
        if clock.crossed(0.52) {
            clunk(stage.sound());
            self.hang = Some(Swing {
                at: 0.52,
                angles: [0., 8., -5.5, 3.2, -1.6, 0.6, 0.],
                seconds: 1.5,
            });
        }
        if let Some((at, power)) = self.strike
            && clock.crossed(at)
        {
            self.strike = None;
            self.mallet = Mallet::Rebound { at };
            self.strike(stage, at, power);
        }
        if let Some(at) = self.leave_at
            && clock.crossed(at)
        {
            self.leave(stage, at);
        }
        self.rings.retain(|ring| clock.now < ring.at + ring.seconds);
        self.left.is_none_or(|at| clock.now < at + 0.9) || !self.rings.is_empty()
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = self.now;
        let dim = match self.left {
            Some(at) => 1. - ease((t - at) / 0.5),
            None => ease(t / 0.5),
        };
        canvas.picture_in(
            &self.pictures.dim,
            [-40., -40.],
            [stage.w + 80., stage.h + 80.],
            dim,
        );
        if let Some(pose) = self.rig_pose(t) {
            self.draw_rig(canvas, t, pose);
        }
        for ring in &self.rings {
            self.draw_ring(canvas, ring);
        }
        if let Some(on) = self.hint
            && t >= on
        {
            // Fading in and rising a little, springily; back as it goes.
            let (shown, rise) = match self.left {
                Some(at) => {
                    let u = (t - at) / 0.4;
                    (1. - ease(u), 1. - bezier(0.2, 1.5, 0.4, 1., u))
                }
                None => {
                    let u = (t - on) / 0.4;
                    (ease(u), bezier(0.2, 1.5, 0.4, 1., u))
                }
            };
            let rig = &self.rig;
            let [w, _] = self.pictures.hint.size;
            canvas.picture_in(
                &self.pictures.hint,
                [stage.w / 2. - w / 2., rig.top + rig.height + 4. - 6. * rise],
                self.pictures.hint.size,
                shown,
            );
        }
    }

    fn again(&mut self, stage: &mut Stage, who: &Who, _: &Reaction) -> bool {
        if self.left.is_some() {
            return false;
        }
        self.power = (self.power + 1).min(6);
        self.announce(stage, who);
        self.leave_at = None;
        let now = self.now;
        self.swing(stage, now, false);
        true
    }
}

// ---- Sound ----------------------------------------------------------------------------

/// The gong's long shimmering note: inharmonic partials beating against each other, upper
/// ones blooming in after the hit, over a felt thump.
fn gong_sound(sound: &mut Graph, p: f32) {
    let v = (0.6 + p * 0.09).min(1.);
    let bus = |sound: &mut Graph, wet: f32| {
        let g = sound.gain(1.);
        let to = sound.dest(wet);
        sound.connect(g, to);
        g
    };
    let (wet, dry) = (bus(sound, 0.85), bus(sound, 0.3));
    // A slow tremolo on the wet bus gives the shimmering wash.
    let rate = sound.rng.range(2.6, 3.6);
    let lfo = sound.osc(Wave::Sine, rate);
    let depth = sound.gain(0.22);
    sound.connect(lfo, depth);
    sound.modulate(depth, wet, |sound, id| sound.gain_of(id));
    sound.play(lfo, 0., 7.);
    let f0 = sound.rng.range(70., 76.);
    // (ratio, gain, decay seconds): from about 72 Hz up to about 560 Hz.
    let partials = [
        (1., 1., 5.6),
        (1.47, 0.72, 5.2),
        (2.09, 0.58, 4.6),
        (2.56, 0.5, 4.2),
        (3.14, 0.38, 3.8),
        (3.71, 0.32, 3.4),
        (4.42, 0.24, 3.1),
        (5.21, 0.18, 2.7),
        (6.33, 0.13, 2.4),
        (7.6, 0.1, 2.1),
    ];
    for (i, (ratio, gain, decay)) in partials.into_iter().enumerate() {
        let f = f0 * ratio;
        sound.tone(Tone {
            freq: f,
            glide_to: Some(f * 0.985),
            dur: decay,
            gain: gain * 0.17 * v,
            attack: 0.004,
            detune: -4.,
            to: Some(if i < 3 { dry } else { wet }),
            ..Tone::default()
        });
        // Beating against the first.
        sound.tone(Tone {
            freq: f + 0.35 + i as f32 * 0.17,
            dur: decay * 0.95,
            gain: gain * 0.12 * v,
            attack: 0.01,
            detune: 5.,
            to: Some(wet),
            ..Tone::default()
        });
    }
    // The tam-tam's bloom: upper partials swelling in after the hit.
    for _ in 0..6 {
        let tone = Tone {
            freq: sound.rng.range(620., 1500.),
            dur: sound.rng.range(2.8, 4.),
            attack: sound.rng.range(0.35, 0.9),
            gain: 0.018 * v * (1. + p * 0.2),
            to: Some(wet),
            ..Tone::default()
        };
        sound.tone(tone);
    }
    // The wash, from looped noise.
    let wash = sound.noise_source(true);
    let filter = sound.filter(Filter::Bandpass, 3200., 2.5);
    sound.frequency(filter).set(3200., 0.).exp(1100., 3.6);
    let envelope = sound.gain(1.);
    sound
        .gain_of(envelope)
        .set(0.0001, 0.)
        .exp(0.05 * v, 0.45)
        .exp(0.0001, 3.6);
    sound.connect(wash, filter);
    sound.connect(filter, envelope);
    sound.connect(envelope, wet);
    sound.play(wash, 0., 3.65);
    // The strike: a short broadband smack and the felt's thump.
    sound.noise(Noise {
        filter: Filter::Lowpass,
        freq: 1800.,
        dur: 0.12,
        gain: 0.5 * v,
        to: Some(dry),
        ..Noise::default()
    });
    sound.tone(Tone {
        freq: 115.,
        glide_to: Some(42.),
        dur: 0.38,
        gain: 0.6 * v,
        to: Some(dry),
        ..Tone::default()
    });
}

fn whoosh(sound: &mut Graph, up: bool) {
    let (freq, to) = if up { (1300., 260.) } else { (280., 1300.) };
    sound.noise(Noise {
        filter: Filter::Bandpass,
        freq,
        sweep_to: Some(to),
        q: 1.1,
        dur: 0.6,
        attack: 0.3,
        gain: 0.2,
        wet: 0.3,
        ..Noise::default()
    });
}

/// The rig landing.
fn clunk(sound: &mut Graph) {
    sound.tone(Tone {
        freq: 95.,
        glide_to: Some(50.),
        dur: 0.22,
        gain: 0.35,
        wet: 0.2,
        ..Tone::default()
    });
    sound.tone(Tone {
        freq: 420.,
        glide_to: Some(360.),
        dur: 0.07,
        gain: 0.12,
        wave: Wave::Triangle,
        wet: 0.2,
        ..Tone::default()
    });
}

// ---- Pictures -------------------------------------------------------------------------

/// The disc, `d` across, in room for its shadow: brushed bronze round a raised boss.
fn disc_look(d: f32) -> Arc<Look> {
    let r = d / 2.;
    let m = 140.;
    let c = m + r;
    let mut body = format!(
        r##"<defs><clipPath id="c"><circle cx="{c}" cy="{c}" r="{r}"/></clipPath><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="30" stdDeviation="35" flood-color="#000" flood-opacity=".55"/></filter>
<radialGradient id="rim" gradientUnits="userSpaceOnUse" cx="{c}" cy="{c}" r="{far}"><stop offset=".55" stop-color="#3c1e05" stop-opacity="0"/><stop offset=".8" stop-color="#3c1e05" stop-opacity=".35"/><stop offset=".94" stop-color="#1e0e02" stop-opacity=".75"/></radialGradient>
<radialGradient id="boss" gradientUnits="userSpaceOnUse" cx="{c}" cy="{c}" r="{far}"><stop offset=".06" stop-color="#f9dd8a"/><stop offset=".11" stop-color="#d79b3c"/><stop offset=".145" stop-color="#9a5f1d"/><stop offset=".155" stop-color="#3c1e05" stop-opacity=".55"/><stop offset=".17" stop-color="#ffdc8c" stop-opacity=".35"/><stop offset=".19" stop-color="#ffdc8c" stop-opacity="0"/></radialGradient>
<radialGradient id="hi" gradientUnits="userSpaceOnUse" cx="{hx}" cy="{hy}" r="{hr}"><stop offset=".015" stop-color="#fffadc" stop-opacity=".95"/><stop offset=".04" stop-color="#ffecaa" stop-opacity=".6"/><stop offset=".1" stop-color="#ffecaa" stop-opacity="0"/></radialGradient>
<radialGradient id="light"><stop offset="0" stop-color="#fff" stop-opacity=".4"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient></defs>
<circle cx="{c}" cy="{c}" r="{edge}" fill="#4a2a08" filter="url(#s)"/><g clip-path="url(#c)">"##,
        far = r * SQRT_2,
        hx = m + d * 0.44,
        hy = m + d * 0.41,
        hr = d * 0.8134,
        edge = r + 3.,
    );
    // The bronze, round from 25° past the top in slices (SVG has no conic gradients).
    let slices = 120;
    for i in 0..slices {
        let (a, b) = (i as f32 / slices as f32, (i as f32 + 1.4) / slices as f32);
        let k = (a + b) / 2. * (BRONZE.len() - 1) as f32;
        let (from, to) = (
            k.floor() as usize,
            (k.ceil() as usize).min(BRONZE.len() - 1),
        );
        let colour = mix(parse(BRONZE[from]), parse(BRONZE[to]), k.fract());
        let point = |k: f32| {
            let angle = (k * 360. + 25.).to_radians();
            (c + angle.sin() * (r + 4.), c - angle.cos() * (r + 4.))
        };
        let ((x0, y0), (x1, y1)) = (point(a), point(b));
        body.push_str(&format!(
            r#"<path d="M{c} {c}L{x0} {y0}L{x1} {y1}Z" fill="{}"/>"#,
            looks::hex(colour)
        ));
    }
    // Fine turned rings.
    let mut ring = 0.;
    while ring < r {
        body.push_str(&format!(
            r##"<circle cx="{c}" cy="{c}" r="{a}" stroke="#fff0c8" stroke-opacity=".07" stroke-width="2"/><circle cx="{c}" cy="{c}" r="{b}" stroke="#000" stroke-opacity=".07" stroke-width="2"/>"##,
            a = ring + 1.,
            b = ring + 4.,
        ));
        ring += 5.;
    }
    body.push_str(&format!(
        r#"<rect width="{s}" height="{s}" fill="url(#rim)"/><rect width="{s}" height="{s}" fill="url(#boss)"/><rect width="{s}" height="{s}" fill="url(#hi)"/><ellipse cx="{lx}" cy="{ly}" rx="{lrx}" ry="{lry}" fill="url(#light)"/></g>"#,
        s = d + m * 2.,
        lx = m + d * 0.32,
        ly = m + d * 0.24,
        lrx = d * 0.4328,
        lry = d * 0.4837,
    ));
    Look::new(format!("gong:disc:{d}"), [d + m * 2., d + m * 2.], body)
}

/// A brass ball, `size` across, as the rig's bar ends in.
fn cap_look(size: f32) -> Arc<Look> {
    let m = 10.;
    let c = m + size / 2.;
    Look::new(
        format!("gong:cap:{size}"),
        [size + m * 2., size + m * 2.],
        format!(
            r##"<defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="{gx}" cy="{gy}" r="{gr}"><stop offset="0" stop-color="#fff1b0"/><stop offset=".45" stop-color="#d49a35"/><stop offset="1" stop-color="#6b4210"/></radialGradient><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="2" stdDeviation="3" flood-color="#000" flood-opacity=".5"/></filter></defs><circle cx="{c}" cy="{c}" r="{r}" fill="url(#g)" filter="url(#s)"/>"##,
            gx = m + size * 0.35,
            gy = m + size * 0.3,
            gr = size * 0.955,
            r = size / 2.,
        ),
    )
}

/// The mallet's red felt head, `size` across.
fn head_look(size: f32) -> Arc<Look> {
    let m = 30.;
    let (c, r) = (m + size / 2., size / 2.);
    let all = size + m * 2.;
    Look::new(
        format!("gong:head:{size}"),
        [all, all],
        format!(
            r##"<defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="{gx}" cy="{gy}" r="{gr}"><stop offset="0" stop-color="#ff9b8a"/><stop offset=".42" stop-color="#c4162f"/><stop offset="1" stop-color="#5a0716"/></radialGradient><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="8" stdDeviation="8" flood-color="#000" flood-opacity=".45"/></filter><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="5"/></filter><clipPath id="c"><circle cx="{c}" cy="{c}" r="{r}"/></clipPath></defs><circle cx="{c}" cy="{c}" r="{r}" fill="url(#g)" filter="url(#s)"/><g clip-path="url(#c)"><path d="M0 0H{all}V{all}H0ZM{hx} {hy}a{r} {r} 0 1 0 {size} 0a{r} {r} 0 1 0 -{size} 0Z" fill-rule="evenodd" fill="#000" fill-opacity=".4" filter="url(#b)"/></g>"##,
            gx = m + size * 0.35,
            gy = m + size * 0.3,
            gr = size * 0.955,
            hx = c - r - 4.,
            hy = c - 6.,
        ),
    )
}

/// The flash of light round the gong as it's struck.
fn glow_look() -> Arc<Look> {
    Look::new(
        "gong:glow",
        [64., 64.],
        r##"<defs><radialGradient id="g"><stop offset="0" stop-color="#ffd678" stop-opacity=".95"/><stop offset=".495" stop-color="#ffaa3c" stop-opacity=".5"/><stop offset=".99" stop-color="#ff8c28" stop-opacity="0"/></radialGradient></defs><rect width="64" height="64" fill="url(#g)"/>"##,
    )
}

/// The screen darkening towards its edges while the gong is up.
fn dim_look() -> Arc<Look> {
    Look::new(
        "gong:dim",
        [64., 64.],
        r##"<defs><radialGradient id="g" r=".7071"><stop offset=".2" stop-color="#190a00" stop-opacity="0"/><stop offset="1" stop-color="#0c0500" stop-opacity=".62"/></radialGradient></defs><rect width="64" height="64" fill="url(#g)"/>"##,
    )
}

fn hint_look() -> Arc<Look> {
    let spacing = 13. * 0.02;
    let width = looks::text_width(HINT, 13., 700, FONT) + spacing * HINT.len() as f32 + 28.;
    let height = 30.;
    Look::new(
        "gong:hint",
        [width, height],
        format!(
            r##"<rect x=".5" y=".5" width="{w}" height="{h}" rx="{rr}" fill="#140a00" fill-opacity=".65" stroke="#ffd682" stroke-opacity=".4"/><text x="{x}" y="{y}" text-anchor="middle" font-family="{FONT}" font-size="13" font-weight="700" letter-spacing="{spacing}" fill="#ffe6a8">{HINT}</text>"##,
            w = width - 1.,
            h = height - 1.,
            rr = (height - 1.) / 2.,
            x = width / 2.,
            y = height / 2. + 4.6,
        ),
    )
}

fn parse(hex: &str) -> [u8; 3] {
    rgb(u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0))
}

// ---- Shapes ---------------------------------------------------------------------------
// What CSS draws with rounded boxes, gradients and soft shadows, as polygons that turn
// with whatever they're part of. Shared with the table flip.

/// An easing, from 0 to 1.
pub(super) type Easing = fn(f32) -> f32;

pub(super) fn linear(t: f32) -> f32 {
    t
}

/// Values through keyframes `(offset, values, easing on to the next)` at `t` (0 to 1), as
/// `element.animate` plays keyframes that each have their own easing.
pub(super) fn frames<const N: usize>(frames: &[(f32, [f32; N], Easing)], t: f32) -> [f32; N] {
    let t = clamp01(t);
    for pair in frames.windows(2) {
        let ((a, from, easing), (b, to, _)) = (pair[0], pair[1]);
        if t <= b {
            let k = if b > a { easing((t - a) / (b - a)) } else { 1. };
            return std::array::from_fn(|i| lerp(from[i], to[i], k));
        }
    }
    frames.last().map_or([0.; N], |last| last.1)
}

/// A rounded box's outline from its top left `at`, its corners' radii from the top left
/// round, each corner in a few straight steps.
pub(super) fn rounded(at: [f32; 2], size: [f32; 2], radii: [f32; 4]) -> Vec<[f32; 2]> {
    let ([x, y], [w, h]) = (at, size);
    let most = (w / 2.).min(h / 2.);
    let mut points = Vec::with_capacity(20);
    for (i, (cx, cy)) in [(0., 0.), (1., 0.), (1., 1.), (0., 1.)]
        .into_iter()
        .enumerate()
    {
        let r = radii[i].min(most).max(0.);
        let centre = [
            x + cx * w + if cx == 0. { r } else { -r },
            y + cy * h + if cy == 0. { r } else { -r },
        ];
        let start = std::f32::consts::PI * (1. + i as f32 * 0.5);
        let steps = if r > 0. { 4 } else { 0 };
        for s in 0..=steps {
            let angle = start + std::f32::consts::FRAC_PI_2 * s as f32 / 4.;
            points.push([centre[0] + angle.cos() * r, centre[1] + angle.sin() * r]);
        }
    }
    points
}

/// The part of convex `shape` whose `axis` (0: x, 1: y) runs from `low` to `high`.
pub(super) fn slab(shape: &[[f32; 2]], axis: usize, low: f32, high: f32) -> Vec<[f32; 2]> {
    let clip = |shape: &[[f32; 2]], edge: f32, above: bool| {
        let inside = |p: [f32; 2]| (p[axis] >= edge) == above;
        let mut out = Vec::with_capacity(shape.len() + 2);
        for (i, &a) in shape.iter().enumerate() {
            let b = shape[(i + 1) % shape.len()];
            if inside(a) {
                out.push(a);
            }
            if inside(a) != inside(b) {
                let k = (edge - a[axis]) / (b[axis] - a[axis]);
                out.push([lerp(a[0], b[0], k), lerp(a[1], b[1], k)]);
            }
        }
        out
    };
    clip(&clip(shape, low, true), high, false)
}

/// The colour a CSS gradient through `stops` has at `k`.
pub(super) fn shade(stops: &[(f32, [u8; 3])], k: f32) -> [u8; 3] {
    for pair in stops.windows(2) {
        let ((a, from), (b, to)) = (pair[0], pair[1]);
        if k <= b {
            return mix(from, to, if b > a { (k - a) / (b - a) } else { 1. });
        }
    }
    stops.last().map_or([0; 3], |last| last.1)
}

/// Convex `shape` filled with a gradient through `stops` running along `axis` across
/// `span`, in bands, each put on screen by `to`.
pub(super) fn gradient(
    canvas: &mut Canvas,
    shape: &[[f32; 2]],
    axis: usize,
    span: [f32; 2],
    stops: &[(f32, [u8; 3])],
    alpha: f32,
    to: &impl Fn([f32; 2]) -> [f32; 2],
) {
    let bands = ((span[1] - span[0]).abs() / 4.).ceil().clamp(3., 12.) as usize;
    for i in 0..bands {
        let (a, b) = (i as f32 / bands as f32, (i + 1) as f32 / bands as f32);
        // Each a little over the next, so no seam shows between them.
        let high = if i + 1 < bands {
            b + 0.4 / bands as f32
        } else {
            b
        };
        let band = slab(
            shape,
            axis,
            lerp(span[0], span[1], a),
            lerp(span[0], span[1], high),
        );
        canvas.polygon(band.into_iter().map(to), shade(stops, (a + b) / 2.), alpha);
    }
}

/// Wood grain: a faint dark line every `period` across `span`.
pub(super) fn grain(
    canvas: &mut Canvas,
    shape: &[[f32; 2]],
    axis: usize,
    span: [f32; 2],
    period: f32,
    to: &impl Fn([f32; 2]) -> [f32; 2],
) {
    let mut at = span[0] + period / 2.;
    while at + 3. < span[1] {
        let line = slab(shape, axis, at, at + 2.5);
        canvas.polygon(line.into_iter().map(to), [0, 0, 0], 0.12);
        at += period;
    }
}

/// A soft `box-shadow` under `shape`: three copies, each a little bigger and fainter.
pub(super) fn shadow(
    canvas: &mut Canvas,
    shape: &[[f32; 2]],
    offset: [f32; 2],
    blur: f32,
    alpha: f32,
    to: &impl Fn([f32; 2]) -> [f32; 2],
) {
    let n = shape.len() as f32;
    let middle = shape
        .iter()
        .fold([0., 0.], |sum, p| [sum[0] + p[0] / n, sum[1] + p[1] / n]);
    for i in 0..3 {
        let grow = blur * (i as f32 - 0.5) / 3.;
        let points = shape.iter().map(|p| {
            to([
                p[0] + offset[0] + (p[0] - middle[0]).signum() * grow,
                p[1] + offset[1] + (p[1] - middle[1]).signum() * grow,
            ])
        });
        canvas.polygon(points, [0, 0, 0], alpha / 4.);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframes_ease_each_step_their_own_way() {
        let steps: [(f32, [f32; 1], Easing); 3] = [
            (0., [0.], linear),
            (0.5, [10.], |t| t * t),
            (1., [20.], linear),
        ];
        assert_eq!(frames(&steps, 0.25), [5.]);
        assert_eq!(frames(&steps, 0.75), [12.5]);
        assert_eq!(frames(&steps, 2.), [20.]);
    }

    #[test]
    fn a_slab_of_a_box_is_the_box_cut_across() {
        let shape = rounded([0., 0.], [10., 20.], [0.; 4]);
        let part = slab(&shape, 1, 5., 8.);
        assert!(part.iter().all(|p| (5. ..=8.).contains(&p[1])));
        assert!(part.iter().any(|p| p[0] == 10.));
    }

    #[test]
    fn the_gong_takes_more_strikes_until_it_leaves() {
        let mut stage = Stage::new(1440., 900., 1);
        let who = Who::new("a", "Sam Rivers", [1, 2, 3], false);
        let mut gong = Gong::new(&mut stage, &who);
        let reaction = Reaction::new("gong");
        let mut before = 0.;
        for frame in 1..=120 {
            let now = frame as f32 / 60.;
            gong.step(&mut stage, Clock { before, now });
            before = now;
        }
        assert!(gong.again(&mut stage, &who, &reaction));
        assert_eq!(gong.power, 2);
        let mut playing = true;
        for frame in 121..=60 * 10 {
            let now = frame as f32 / 60.;
            playing = gong.step(&mut stage, Clock { before, now });
            before = now;
            if gong.left.is_some() {
                break;
            }
        }
        assert!(playing);
        assert!(!gong.again(&mut stage, &who, &reaction));
    }
}
