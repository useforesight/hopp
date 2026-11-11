//! 3. Table flip: (╯°□°)╯︵ ┻━┻ from the call window, a wooden table spins across
//! everyone's screen and smashes at the edge, then a calm little figure puts it back
//! ┬─┬ノ( º _ ºノ). Another flip while it plays is an extra angry rattle.
//!
//! As on the Mac, the sender's rage is a badge over the middle of the call window.

use std::{f32::consts::TAU, sync::Arc};

use super::{
    Effect,
    gong::{Easing, frames, gradient, linear, rounded, shadow, slab},
};
use crate::protocol::Reaction;
use crate::ui::reactions::{
    looks::{self, FONT, Look},
    particles::{Emit, Shape},
    scene::{Canvas, place},
    stage::{Clock, Stage, Who, bezier, ease, ease_in_out, ease_out, lerp, rgb},
    synth::{Filter, Graph, Noise, Tone, Wave},
};

/// The table's box at scale 1.
const TW: f32 = 230.;
const TH: f32 = 150.;
const WOOD: [(f32, [u8; 3]); 3] = [
    (0., rgb(0xd08a4c)),
    (0.55, rgb(0xa8622c)),
    (1., rgb(0x7a4119)),
];
const LEG: [(f32, [u8; 3]); 4] = [
    (0., rgb(0x5a2c10)),
    (0.4, rgb(0x9a5526)),
    (0.75, rgb(0x7a3f18)),
    (1., rgb(0x4a230c)),
];
/// The legs at the back, in shadow (`brightness(.6)`).
const LEG_BACK: [(f32, [u8; 3]); 4] = [
    (0., rgb(0x361a0a)),
    (0.4, rgb(0x5c3317)),
    (0.75, rgb(0x49260e)),
    (1., rgb(0x2c1507)),
];
const APRON: [(f32, [u8; 3]); 2] = [(0., rgb(0x7a4119)), (1., rgb(0x5a2c10))];
const SPLINTERS: [[u8; 3]; 5] = [
    rgb(0xc9803f),
    rgb(0xa8622c),
    rgb(0x7a4119),
    rgb(0xe3a56a),
    rgb(0x5a2c10),
];
const CHIPS: [[u8; 3]; 3] = [rgb(0xe3a56a), rgb(0xf0c08a), rgb(0x8a4a1c)];
const RAGE: &str = "(ಠ益ಠ)";
const THROW: &str = "(╯°□°)╯︵";
const CALM: &str = "ノ( º _ ºノ)";
const PLEASE: &str = "please respect the furniture";
const FACE_SIZE: f32 = 30.;
const FIG_SIZE: f32 = 46.;

/// When the table's thrown, how long it flies, and when it's put back.
const THROW_AT: f32 = 0.48;
const FLIGHT: f32 = 0.95;
const SMASH_AT: f32 = THROW_AT + FLIGHT;
const PUT_BACK: f32 = SMASH_AT + 1.2;
const END: f32 = PUT_BACK + 3.25;
/// When the rage badge fades.
const CALM_DOWN: f32 = 2.5;

/// The table's flight: a curve from the call window, over the top, to the screen's edge.
struct Flight {
    from: [f32; 2],
    over: [f32; 2],
    to: [f32; 2],
    dir: f32,
}

/// What the smash leaves: a crack in the screen, the table in pieces and dust.
struct Smash {
    at: [f32; 2],
    crack: Arc<Look>,
    pieces: Vec<Piece>,
    dust: Vec<Dust>,
}

#[derive(Clone, Copy)]
enum Fill {
    Top,
    Leg,
    Flat([u8; 3]),
}

struct Piece {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Degrees, and degrees a frame.
    turn: f32,
    spin: f32,
    size: [f32; 2],
    radius: f32,
    fill: Fill,
}

struct Dust {
    size: f32,
    at: [f32; 2],
    drift: [f32; 2],
}

pub struct Flip {
    now: f32,
    /// The rage badge's faces' widths.
    rage: f32,
    throw: f32,
    flight: Option<Flight>,
    smash: Option<Smash>,
    /// When the last extra rattle began.
    rattle: Option<f32>,
    dust: Arc<Look>,
    bubble: Arc<Look>,
    calm: f32,
}

impl Flip {
    pub fn new(stage: &mut Stage, who: &Who) -> Self {
        growl(stage.sound());
        stage.banner(who, "flipped the table", "┻━┻", 3200.);
        let (bubble, _) = bubble_look();
        Self {
            now: 0.,
            rage: looks::text_width(RAGE, FACE_SIZE, 800, FONT),
            throw: looks::text_width(THROW, FACE_SIZE, 800, FONT),
            flight: None,
            smash: None,
            rattle: None,
            dust: looks::blob(rgb(0xebd7be), &[(0., 0.7), (0.99, 0.)]),
            bubble,
            calm: looks::text_width(CALM, FIG_SIZE, 700, FONT),
        }
    }

    fn throw(&mut self, stage: &mut Stage) {
        heave(stage.sound());
        spin_whoosh(stage.sound(), 5, FLIGHT);
        let c = stage.tile();
        let dir = if c[0] > stage.w / 2. { -1. } else { 1. };
        let x = if dir < 0. { 0. } else { stage.w };
        let y = stage.h * stage.rand(0.42, 0.58);
        let from = [c[0], c[1] - 20.];
        let to = [x - dir * TW * 0.42 * 1.35, y];
        self.flight = Some(Flight {
            from,
            over: [(from[0] + to[0]) / 2. + dir * 40., -stage.h * 0.32],
            to,
            dir,
        });
    }

    fn smash(&mut self, stage: &mut Stage) {
        let Some(flight) = &self.flight else { return };
        let (x, away) = (flight.to[0] + flight.dir * TW * 0.42 * 1.35, -flight.dir);
        let y = flight.to[1];
        crash(stage.sound());
        stage.shake(26., 700.);
        stage.flash([255, 255, 255], 220., 0.45);
        let crack = crack_look(stage);
        // The table in pieces, flung back the way it came.
        let bits = [
            (TW * 0.52, 24., Fill::Top, 5.),
            (TW * 0.48, 24., Fill::Top, 5.),
            (17., TH - 30., Fill::Leg, 3.),
            (17., TH - 30., Fill::Leg, 3.),
            (15., TH - 50., Fill::Leg, 3.),
            (15., 70., Fill::Leg, 3.),
            (60., 14., Fill::Flat(rgb(0x6a3514)), 2.),
            (44., 12., Fill::Flat(rgb(0x7a4119)), 2.),
        ];
        let pieces = bits
            .into_iter()
            .map(|(w, h, fill, radius)| Piece {
                x: x + away * 70. + stage.rand(-40., 40.),
                y: y + stage.rand(-50., 50.),
                vx: away * stage.rand(3., 14.),
                vy: stage.rand(-17., -6.),
                turn: stage.rand(0., 360.),
                spin: stage.rand(-22., 22.),
                size: [w, h],
                radius,
                fill,
            })
            .collect();
        let angle = if away > 0. { 0. } else { 180. };
        stage.emit(Emit {
            x,
            y,
            count: 110,
            angle,
            spread: 150.,
            speed: [5., 19.],
            gravity: 0.45,
            drag: 0.975,
            life: [50., 110.],
            size: [5., 13.],
            shape: Shape::Rect,
            colours: &SPLINTERS,
            ..Emit::default()
        });
        stage.emit(Emit {
            x,
            y,
            count: 40,
            angle,
            spread: 110.,
            speed: [8., 22.],
            gravity: 0.5,
            life: [40., 80.],
            size: [4., 8.],
            shape: Shape::Tri,
            colours: &CHIPS,
            ..Emit::default()
        });
        let dust = (0..5)
            .map(|_| Dust {
                size: stage.rand(90., 180.),
                at: [x, y + stage.rand(-60., 60.)],
                drift: [away * stage.rand(60., 180.), stage.rand(-60., 20.)],
            })
            .collect();
        self.smash = Some(Smash {
            at: [x, y],
            crack,
            pieces,
            dust,
        });
    }

    /// The call window's own rattle: a turn in degrees and a shift, from the throw's jolt
    /// and any extra rattles, the latest on top as the prototype's animations are.
    fn call_pose(&self, t: f32) -> (f32, [f32; 2]) {
        let mut turn = (f32::NEG_INFINITY, 0.);
        let mut shift = [0., 0.];
        let jolt = (t - THROW_AT) / 0.9;
        if (0. ..1.).contains(&jolt) {
            let [r, x, y] = frames(
                &[
                    (0., [0., 0., 0.], linear),
                    (0.15, [-6., -18., -14.], linear),
                    (0.4, [3.5, 8., 6.], linear),
                    (0.65, [-1.5, -3., -2.], linear),
                    (1., [0., 0., 0.], linear),
                ],
                ease_out(jolt),
            );
            turn = (THROW_AT, r);
            shift = [x, y];
        }
        if let Some(at) = self.rattle {
            let u = (t - at) / 0.3;
            if (0. ..1.).contains(&u) && at >= turn.0 {
                let steps: [(f32, [f32; 1], Easing); 4] = [
                    (0., [0.], linear),
                    (1. / 3., [-3.], linear),
                    (2. / 3., [2.], linear),
                    (1., [0.], linear),
                ];
                turn = (at, frames(&steps, u)[0]);
            }
        }
        // The tile shaking with rage until the throw.
        if t < THROW_AT {
            let steps: [(f32, [f32; 2], Easing); 5] = [
                (0., [0., 0.], linear),
                (0.25, [-3., 1.], linear),
                (0.5, [3., -1.], linear),
                (0.75, [-2., 0.], linear),
                (1., [0., 0.], linear),
            ];
            let [x, y] = frames(&steps, (t / 0.12).fract());
            shift = [shift[0] + x, shift[1] + y];
        }
        (turn.1, shift)
    }

    fn draw_badge(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = self.now;
        let shown = if t < CALM_DOWN {
            ease(t / 0.2)
        } else {
            1. - ease((t - CALM_DOWN) / 0.2)
        };
        if shown <= 0. {
            return;
        }
        let (turn, shift) = self.call_pose(t);
        let tile = stage.tile();
        let centre = [tile[0] + shift[0], tile[1] + shift[1]];
        let thrown = t >= THROW_AT;
        let (face, width) = if thrown {
            (THROW, self.throw)
        } else {
            (RAGE, self.rage)
        };
        let pill = [width + 52., FACE_SIZE + 32.];
        canvas.picture(&pill_look(pill, turn.round()), centre, 1., shown);
        // The face trembles, and pops as the table goes.
        let u = t / 0.07;
        let k = if u.floor() as i32 % 2 == 0 {
            u.fract()
        } else {
            1. - u.fract()
        };
        let tremble = [lerp(-1.5, 1.5, k), lerp(1., -1., k)];
        let pop = if thrown {
            let steps: [(f32, [f32; 1], Easing); 3] = [
                (0., [0.7], linear),
                (0.5, [1.25], linear),
                (1., [1.], linear),
            ];
            frames(&steps, bezier(0.2, 1.6, 0.4, 1., (t - THROW_AT) / 0.32))[0]
        } else {
            1.
        };
        let look = face_look(face, width, (turn + lerp(-1.5, 1.5, k)).round());
        let at = place(tremble, centre, turn.to_radians(), 1.);
        canvas.picture(&look, at, pop, shown);
        // Steam puffing off its top right corner.
        let puff = ease_out((t / 0.5).fract());
        let steam = place(
            [
                pill[0] / 2. + 6. - 11.,
                -pill[1] / 2. - 8. + 13. + lerp(4., -14., puff),
            ],
            centre,
            turn.to_radians(),
            1.,
        );
        canvas.picture(
            &looks::emoji("💢"),
            steam,
            22. / 72. * lerp(0.7, 1.2, puff),
            shown * (1. - puff),
        );
    }

    fn draw_smash(&self, smash: &Smash, canvas: &mut Canvas) {
        let age = self.now - SMASH_AT;
        if age < 2.3 {
            let [alpha, scale] = frames(
                &[
                    (0., [0., 0.6], linear),
                    (0.05, [1., 1.], linear),
                    (0.7, [1., 1.], linear),
                    (1., [0., 1.], linear),
                ],
                ease_out(age / 2.3),
            );
            canvas.picture(&smash.crack, smash.at, scale, alpha);
        }
        let alpha = (1. - (age - 1.1) / 0.5).clamp(0., 1.);
        if age < 1.65 {
            for piece in &smash.pieces {
                draw_piece(canvas, piece, alpha);
            }
        }
        let u = age / 1.4;
        if u < 1. {
            let k = bezier(0.1, 0.7, 0.3, 1., u);
            for dust in &smash.dust {
                let centre = [
                    dust.at[0] + dust.drift[0] * k,
                    dust.at[1] + dust.drift[1] * k,
                ];
                let scale = dust.size * lerp(0.3, 1.6, k) / self.dust.size[0];
                canvas.picture(&self.dust, centre, scale, lerp(0.9, 0., k));
            }
        }
    }

    fn draw_put_back(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = self.now - PUT_BACK;
        let (cx, cy) = (stage.w / 2., stage.h * 0.52);
        let gone = if t > 2.7 {
            (1. - (t - 2.7) / 0.5).max(0.)
        } else {
            1.
        };
        // The table, upside down, then turned upright.
        let [turn, dy, scale, alpha] = if t < 0.42 {
            let k = bezier(0.2, 1.4, 0.4, 1., t / 0.42);
            [180., lerp(-30., 0., k), lerp(0.9, 1., k), k.clamp(0., 1.)]
        } else if t < 1. {
            [180., 0., 1., 1.]
        } else {
            let [turn, dy, scale] = frames(
                &[
                    (0., [180., 0., 1.], |t| bezier(0.4, 0., 0.6, 1., t)),
                    (0.5, [90., -70., 1.04], |t| bezier(0.3, 0., 0.5, 1., t)),
                    (0.85, [-4., 4., 1.], ease_out),
                    (1., [0., 0., 1.], linear),
                ],
                (t - 1.) / 0.9,
            );
            [turn, dy, scale, 1.]
        };
        table(canvas, [cx - 70., cy + dy], turn, scale, alpha * gone);
        // The figure walks in from the right, bobbing, and helps it over.
        if t < 0.25 {
            return;
        }
        let k = bezier(0.25, 1., 0.4, 1., (t - 0.25) / 0.7);
        let slide = 260. * (1. - k);
        let bob = if t < 0.95 {
            let u = ((t - 0.25) / 0.175).fract();
            -6. * (1. - (u * 2. - 1.).abs())
        } else {
            0.
        };
        let [lean, lift] = if (1. ..1.9).contains(&t) {
            let steps: [(f32, [f32; 2], Easing); 3] = [
                (0., [0., 0.], linear),
                (0.5, [-8., -14.], linear),
                (1., [0., 0.], linear),
            ];
            frames(&steps, ease_in_out((t - 1.) / 0.9))
        } else {
            [0., 0.]
        };
        let shown = k.clamp(0., 1.) * gone;
        let (left, top) = (cx + 52. + slide, cy - 26. + bob + lift);
        let look = fig_look(self.calm, lean.round());
        let m = FIG_MARGIN;
        canvas.picture_in(&look, [left - m, top - m], look.size, shown);
        // "please respect the furniture", popping up once it's done.
        if t >= 1.78 {
            let u = t - 1.78;
            let scale = lerp(0.6, 1., bezier(0.2, 1.6, 0.4, 1., u / 0.4));
            let [w, h] = self.bubble.size;
            let corner = [left + self.calm * 0.6, top - 14.];
            canvas.picture_in(
                &self.bubble,
                [corner[0], corner[1] - h * scale],
                [w * scale, h * scale],
                ease(u / 0.25) * gone,
            );
        }
    }
}

impl Effect for Flip {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        self.now = clock.now;
        if clock.crossed(THROW_AT) {
            self.throw(stage);
        }
        if clock.crossed(SMASH_AT) {
            self.smash(stage);
        }
        if let Some(smash) = &mut self.smash {
            let floor = stage.h - 20.;
            for _ in 0..stage.ticks {
                for b in &mut smash.pieces {
                    b.vy += 0.75;
                    b.vx *= 0.99;
                    b.x += b.vx;
                    b.y += b.vy;
                    b.turn += b.spin;
                    if b.y > floor && b.vy > 0. {
                        b.y = floor;
                        b.vy *= -0.35;
                        b.vx *= 0.7;
                        b.spin *= 0.6;
                    }
                }
            }
        }
        if clock.crossed(PUT_BACK) {
            tok(stage.sound(), 0.3, 0.22);
        }
        if clock.crossed(PUT_BACK + 1.) {
            stage.sound().noise(Noise {
                filter: Filter::Bandpass,
                freq: 600.,
                sweep_to: Some(1200.),
                q: 1.,
                dur: 0.45,
                attack: 0.25,
                gain: 0.06,
                wet: 0.2,
                ..Noise::default()
            });
        }
        if clock.crossed(PUT_BACK + 1.78) {
            let sound = stage.sound();
            tok(sound, 0., 0.34);
            tok(sound, 0.11, 0.18);
            stage.shake(2., 160.);
        }
        clock.now < END
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = self.now;
        self.draw_badge(stage, canvas);
        if let Some(flight) = &self.flight {
            let u = (t - THROW_AT) / FLIGHT;
            if (0. ..1.).contains(&u) {
                let k = bezier(0.3, 0.1, 0.7, 1., u);
                let v = 1. - k;
                let at = [0, 1].map(|i| {
                    v * v * flight.from[i] + 2. * v * k * flight.over[i] + k * k * flight.to[i]
                });
                let turn = flight.dir * 360. * 2.25 * k;
                table(canvas, at, turn, 0.3 + 1.05 * k.sqrt(), 1.);
            }
        }
        if let Some(smash) = &self.smash {
            self.draw_smash(smash, canvas);
        }
        if t >= PUT_BACK {
            self.draw_put_back(stage, canvas);
        }
    }

    fn again(&mut self, stage: &mut Stage, _: &Who, _: &Reaction) -> bool {
        growl(stage.sound());
        self.rattle = Some(self.now);
        true
    }
}

/// The table, its middle at `centre`, turned `turn` degrees and scaled by `scale`.
fn table(canvas: &mut Canvas, centre: [f32; 2], turn: f32, scale: f32, alpha: f32) {
    if alpha <= 0. {
        return;
    }
    let turn = turn.to_radians();
    let to = |p: [f32; 2]| {
        place(
            [(p[0] - TW / 2.) * scale, (p[1] - TH / 2.) * scale],
            centre,
            turn,
            1.,
        )
    };
    let leg = |x: f32, top: f32, height: f32| {
        vec![
            [x, top],
            [x + 17., top],
            [x + 17. * 0.82, top + height],
            [x + 17. * 0.18, top + height],
        ]
    };
    for x in [42., TW - 42. - 17.] {
        gradient(
            canvas,
            &leg(x, 26., TH - 42.),
            0,
            [x, x + 17.],
            &LEG_BACK,
            alpha,
            &to,
        );
    }
    let apron = rounded([12., 24.], [TW - 24., 14.], [0., 0., 3., 3.]);
    gradient(canvas, &apron, 1, [24., 38.], &APRON, alpha, &to);
    for x in [14., TW - 14. - 17.] {
        gradient(
            canvas,
            &leg(x, 30., TH - 30.),
            0,
            [x, x + 17.],
            &LEG,
            alpha,
            &to,
        );
    }
    let top = rounded([0., 0.], [TW, 24.], [5.; 4]);
    let under = top.iter().map(|p| to([p[0], p[1] + 6.]));
    canvas.polygon(under, [0, 0, 0], 0.12 * alpha);
    gradient(canvas, &top, 1, [0., 24.], &WOOD, alpha, &to);
    for (from, until, colour, opacity) in [
        (7., 9.5, rgb(0x3c1905), 0.18),
        (0., 2., rgb(0xffdcaa), 0.5),
        (19., 24., rgb(0x461e08), 0.5),
    ] {
        let band = slab(&top, 1, from, until);
        canvas.polygon(band.into_iter().map(to), colour, opacity * alpha);
    }
}

fn draw_piece(canvas: &mut Canvas, piece: &Piece, alpha: f32) {
    if alpha <= 0. {
        return;
    }
    let [w, h] = piece.size;
    let shape = rounded([-w / 2., -h / 2.], [w, h], [piece.radius; 4]);
    let turn = piece.turn.to_radians();
    let to = |p: [f32; 2]| place(p, [piece.x, piece.y], turn, 1.);
    shadow(canvas, &shape, [0., 4.], 10., 0.3 * alpha, &to);
    match piece.fill {
        Fill::Top => gradient(canvas, &shape, 1, [-h / 2., h / 2.], &WOOD, alpha, &to),
        Fill::Leg => gradient(canvas, &shape, 0, [-w / 2., w / 2.], &LEG, alpha, &to),
        Fill::Flat(colour) => canvas.polygon(shape.into_iter().map(to), colour, alpha),
    }
}

// ---- Pictures -------------------------------------------------------------------------

/// The rage badge's pill for a face `size` big, turned `turn` degrees.
fn pill_look(size: [f32; 2], turn: f32) -> Arc<Look> {
    let m = 80.;
    let ([w, h], c) = (size, [size[0] / 2. + m, size[1] / 2. + m]);
    Look::new(
        format!("flip:pill:{w}:{turn}"),
        [w + m * 2., h + m * 2.],
        format!(
            r##"<defs><radialGradient id="g" gradientUnits="userSpaceOnUse" cx="{c0}" cy="{gy}" r="{gr}"><stop offset="0" stop-color="#ff4628" stop-opacity=".92"/><stop offset=".75" stop-color="#aa0a19" stop-opacity=".95"/></radialGradient><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="16" stdDeviation="22" flood-color="#780014" flood-opacity=".45"/></filter></defs><g transform="rotate({turn} {c0} {c1})"><rect x="{m}" y="{m}" width="{w}" height="{h}" rx="{r}" fill="url(#g)" filter="url(#s)"/></g>"##,
            c0 = c[0],
            c1 = c[1],
            gy = m + h * 0.55,
            gr = (w / 2.).hypot(h * 0.55),
            r = h / 2.,
        ),
    )
}

/// The badge's face, `width` wide, turned `turn` degrees.
fn face_look(face: &str, width: f32, turn: f32) -> Arc<Look> {
    let m = 16.;
    let (w, h) = (width + m * 2., FACE_SIZE + m * 2.);
    let y = m + FACE_SIZE * 0.85;
    Look::new(
        format!("flip:face:{face}:{turn}"),
        [w, h],
        format!(
            r##"<g transform="rotate({turn} {cx} {cy})" font-family="{FONT}" font-size="{FACE_SIZE}" font-weight="800" text-anchor="middle"><text x="{cx}" y="{shadow}" fill="#5a000a" fill-opacity=".55">{face}</text><text x="{cx}" y="{y}" fill="#fff">{face}</text></g>"##,
            face = looks::escape(face),
            cx = w / 2.,
            cy = h / 2.,
            shadow = y + 3.,
        ),
    )
}

/// Room round the figure for its shadow and its lean.
const FIG_MARGIN: f32 = 30.;

/// The calm little figure, `width` wide, leaning `turn` degrees.
fn fig_look(width: f32, turn: f32) -> Arc<Look> {
    let m = FIG_MARGIN;
    let (w, h) = (width + m * 2., FIG_SIZE + m * 2.);
    Look::new(
        format!("flip:figure:{turn}"),
        [w, h],
        format!(
            r##"<defs><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="3" stdDeviation="6" flood-color="#000" flood-opacity=".45"/></filter></defs><g transform="rotate({turn} {cx} {cy})"><text x="{m}" y="{y}" font-family="{FONT}" font-size="{FIG_SIZE}" font-weight="700" fill="#fff" filter="url(#s)">{}</text></g>"##,
            looks::escape(CALM),
            cx = w / 2.,
            cy = h / 2.,
            y = m + FIG_SIZE * 0.85,
        ),
    )
}

/// The speech bubble, its tail at the bottom left, and its size.
fn bubble_look() -> (Arc<Look>, [f32; 2]) {
    let width = looks::text_width(PLEASE, 15., 650, FONT) + 24.;
    let height = 15. * 1.2 + 14.;
    let outline: String = rounded([0., 0.], [width, height], [14., 14., 14., 4.])
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, p[0], p[1]))
        .collect();
    (
        Look::new(
            "flip:bubble",
            [width, height],
            format!(
                r##"<path d="{outline}Z" fill="#fff"/><text x="12" y="{y}" font-family="{FONT}" font-size="15" font-weight="650" fill="#2a1640">{PLEASE}</text>"##,
                y = height / 2. + 5.2,
            ),
        ),
        [width, height],
    )
}

/// Cracks in the screen where the table hit: rays out and two jagged rings, drawn anew
/// each time.
fn crack_look(stage: &mut Stage) -> Arc<Look> {
    let mut d = String::new();
    for i in 0..11 {
        let a = i as f32 / 11. * TAU + stage.rand(-0.2, 0.2);
        let length = stage.rand(70., 190.);
        d.push_str("M0 0");
        for s in 1..=3 {
            let s = s as f32 / 3.;
            let x = (a + stage.rand(-0.25, 0.25)).cos() * length * s;
            let y = (a + stage.rand(-0.25, 0.25)).sin() * length * s;
            d.push_str(&format!(" L{x:.1} {y:.1}"));
        }
    }
    for r in [34., 78.] {
        d.push_str(&format!(" M{r} 0"));
        for i in 1..=12 {
            let a = i as f32 / 12. * TAU;
            let k = r * stage.rand(0.85, 1.15);
            d.push_str(&format!(" L{:.1} {:.1}", a.cos() * k, a.sin() * k));
        }
    }
    let key = stage.rng.next_u64();
    Look::new(
        format!("flip:crack:{key}"),
        [400., 400.],
        format!(
            r##"<g transform="translate(200 200)"><path d="{d}" stroke="#fff" stroke-opacity=".85" stroke-width="2" stroke-linejoin="round"/><path d="{d}" stroke="#000" stroke-opacity=".25" stroke-width="1" transform="translate(1.5 1.5)"/></g>"##
        ),
    )
}

// ---- Sound ----------------------------------------------------------------------------

/// A rising, raging growl.
fn growl(sound: &mut Graph) {
    let filter = sound.filter(Filter::Bandpass, 650., 1.4);
    let to = sound.dest(0.12);
    sound.connect(filter, to);
    sound.frequency(filter).set(500., 0.).linear(1100., 0.45);
    for (freq, glide_to, gain, detune) in [(105., 170., 0.45, 0.), (108., 176., 0.3, 25.)] {
        sound.tone(Tone {
            freq,
            glide_to: Some(glide_to),
            dur: 0.48,
            gain,
            attack: 0.06,
            wave: Wave::Saw,
            detune,
            to: Some(filter),
            ..Tone::default()
        });
    }
    sound.noise(Noise {
        filter: Filter::Bandpass,
        freq: 1400.,
        q: 0.8,
        dur: 0.4,
        attack: 0.08,
        gain: 0.12,
        ..Noise::default()
    });
}

/// The "HUP!" of the throw.
fn heave(sound: &mut Graph) {
    sound.tone(Tone {
        freq: 190.,
        glide_to: Some(120.),
        dur: 0.18,
        gain: 0.35,
        wave: Wave::Saw,
        wet: 0.1,
        ..Tone::default()
    });
    sound.noise(Noise {
        filter: Filter::Lowpass,
        freq: 900.,
        dur: 0.15,
        gain: 0.35,
        ..Noise::default()
    });
}

/// Whooshes rising as the table spins, `n` of them over `total` seconds.
fn spin_whoosh(sound: &mut Graph, n: usize, total: f32) {
    let each = total / n as f32;
    for i in 0..n {
        let i = i as f32;
        sound.noise(Noise {
            when: i * each,
            filter: Filter::Bandpass,
            freq: 350. + i * 180.,
            sweep_to: Some(1600. + i * 300.),
            q: 1.6,
            dur: each,
            attack: each * 0.6,
            gain: 0.12 + i * 0.05,
            wet: 0.2,
            ..Noise::default()
        });
    }
}

/// A bit of wood clattering down.
fn knock(sound: &mut Graph, when: f32, f: f32, g: f32) {
    sound.tone(Tone {
        when,
        freq: f,
        glide_to: Some(f * 0.92),
        dur: 0.07,
        gain: g,
        wave: Wave::Triangle,
        wet: 0.25,
        ..Tone::default()
    });
    sound.tone(Tone {
        when,
        freq: f * 2.7,
        dur: 0.03,
        gain: g * 0.4,
        wet: 0.25,
        ..Tone::default()
    });
    sound.noise(Noise {
        when,
        filter: Filter::Bandpass,
        freq: 2200.,
        q: 2.,
        dur: 0.025,
        gain: g * 0.6,
        ..Noise::default()
    });
}

fn crash(sound: &mut Graph) {
    sound.noise(Noise {
        filter: Filter::Lowpass,
        freq: 7000.,
        sweep_to: Some(600.),
        dur: 0.6,
        gain: 0.9,
        wet: 0.35,
        ..Noise::default()
    });
    sound.noise(Noise {
        filter: Filter::Highpass,
        freq: 2500.,
        dur: 0.3,
        gain: 0.45,
        wet: 0.2,
        ..Noise::default()
    });
    sound.noise(Noise {
        when: 0.03,
        filter: Filter::Bandpass,
        freq: 900.,
        q: 0.7,
        dur: 0.45,
        gain: 0.5,
        wet: 0.3,
        ..Noise::default()
    });
    sound.tone(Tone {
        freq: 92.,
        glide_to: Some(32.),
        dur: 0.5,
        gain: 0.95,
        wet: 0.15,
        ..Tone::default()
    });
    sound.tone(Tone {
        freq: 160.,
        glide_to: Some(60.),
        dur: 0.18,
        gain: 0.5,
        wave: Wave::Triangle,
        wet: 0.1,
        ..Tone::default()
    });
    // The pieces clattering down.
    let mut when = 0.08;
    for i in 0..14 {
        let i = i as f32;
        when += sound.rng.range(0.035, 0.11) * (1. + i * 0.08);
        let f = sound.rng.range(260., 950.);
        knock(sound, when, f, 0.32 * (1. - i / 16.));
    }
}

/// The table set down.
fn tok(sound: &mut Graph, when: f32, g: f32) {
    sound.tone(Tone {
        when,
        freq: 880.,
        glide_to: Some(840.),
        dur: 0.1,
        gain: g,
        wave: Wave::Triangle,
        wet: 0.3,
        ..Tone::default()
    });
    sound.tone(Tone {
        when,
        freq: 440.,
        dur: 0.16,
        gain: g * 0.6,
        wet: 0.3,
        ..Tone::default()
    });
    sound.tone(Tone {
        when,
        freq: 140.,
        glide_to: Some(90.),
        dur: 0.12,
        gain: g * 0.7,
        wet: 0.1,
        ..Tone::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_flies_away_from_the_call_and_is_put_back() {
        let mut stage = Stage::new(1440., 900., 4);
        stage.call = [1052., 137., 300., 618.];
        let who = Who::new("a", "Sam Rivers", [1, 2, 3], false);
        let mut flip = Flip::new(&mut stage, &who);
        let mut before = 0.;
        let mut frame = 0;
        while flip.step(
            &mut stage,
            Clock {
                before,
                now: frame as f32 / 60.,
            },
        ) {
            before = frame as f32 / 60.;
            frame += 1;
            stage.ticks = 1;
            if frame == 30 {
                assert!(flip.again(&mut stage, &who, &Reaction::new("table_flip")));
            }
        }
        // To the left edge, away from the call window on the right.
        assert_eq!(flip.smash.as_ref().map(|smash| smash.at[0]), Some(0.));
        assert!((END - before).abs() < 0.05);
    }
}
