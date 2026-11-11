//! 5. Soundboard: eight pads (the tray's), each with its own sound and full-screen moment
//! for everyone in the call.

use std::sync::Arc;

use super::Effect;
use crate::ui::reactions::{
    looks::{self, EMOJI, Look},
    particles::{Emit, Shape},
    scene::{Canvas, step_alpha},
    stage::{Clock, Stage, Who, bezier, ease_in_out, ease_out, keyframes, lerp, rgb},
    synth::{Filter, Graph, Id, Noise, Tone, Wave, saturation},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Airhorn,
    Badum,
    Trombone,
    Crickets,
    Drumroll,
    Chaching,
    Wooow,
    Applause,
}

/// The pads, in the tray's order: the choice on the wire, what the banner says, its emoji,
/// and how long its moment shows.
const PADS: [(&str, Kind, &str, &str, f32); 8] = [
    ("airhorn", Kind::Airhorn, "blew the airhorn", "📯", 2.5),
    ("badumtss", Kind::Badum, "played a ba-dum-tss", "🥁", 1.95),
    (
        "sad_trombone",
        Kind::Trombone,
        "played the sad trombone",
        "🎺",
        4.3,
    ),
    (
        "crickets",
        Kind::Crickets,
        "summoned the crickets",
        "🦗",
        4.7,
    ),
    ("drumroll", Kind::Drumroll, "started a drumroll", "🥁", 3.85),
    ("chaching", Kind::Chaching, "hit cha-ching", "💰", 1.7),
    ("wooow", Kind::Wooow, "went wooow", "😮", 2.4),
    (
        "applause",
        Kind::Applause,
        "started the applause",
        "👏",
        2.8,
    ),
];

const SPARKS: [[u8; 3]; 3] = [rgb(0x7df9ff), rgb(0xffffff), rgb(0xffd166)];
const TADA: [[u8; 3]; 2] = [rgb(0xffffff), rgb(0xffd166)];
const GLINTS: [[u8; 3]; 2] = [rgb(0xfff6c8), rgb(0xffd166)];
const WOW: [[u8; 3]; 4] = [rgb(0xffffff), rgb(0xffd166), rgb(0x7df9ff), rgb(0xff7ab6)];
/// The prototype's `.p5-word` type: heavy, italic, outlined.
const WORD_FONT: &str = "Impact, 'Arial Black', Inter, 'Adwaita Sans', sans-serif";
const WOW_FONT: &str = "'Arial Rounded MT Bold', Inter, 'Adwaita Sans', sans-serif";
/// Big words are drawn this much larger than they show, so their pop past full size
/// isn't drawn again finer.
const BIG: f32 = 1.2;
const INK: &str = "#140a28";

fn spring(t: f32) -> f32 {
    bezier(0.2, 1.5, 0.4, 1., t)
}

pub struct Pad {
    kind: Kind,
    born: f32,
    seconds: f32,
    tile: [f32; 2],
    words: Vec<Word>,
    rings: Vec<Ring>,
    /// The tumbleweed's strokes, and a name for its pictures.
    weed: (String, u64),
    /// WOOOOW's letters: each picture and its middle from the word's.
    wow: Vec<(Arc<Look>, f32)>,
    wow_size: f32,
}

impl Pad {
    pub fn new(stage: &mut Stage, who: &Who, choice: &str) -> Option<Self> {
        let &(_, kind, verb, emoji, seconds) = PADS.iter().find(|pad| pad.0 == choice)?;
        stage.banner(who, verb, emoji, 2600.);
        {
            let graph = stage.sound();
            let mut outs = Outs::default();
            match kind {
                Kind::Airhorn => airhorn_sound(graph, &mut outs),
                Kind::Badum => badum_sound(graph, &mut outs),
                Kind::Trombone => trombone_sound(graph, &mut outs),
                Kind::Crickets => crickets_sound(graph, &mut outs),
                Kind::Drumroll => drumroll_sound(graph, &mut outs),
                Kind::Chaching => chaching_sound(graph, &mut outs),
                Kind::Wooow => wooow_sound(graph, &mut outs),
                Kind::Applause => applause_sound(graph, &mut outs),
            }
        }
        let mut pad = Self {
            kind,
            born: stage.now,
            seconds,
            tile: stage.tile(),
            words: Vec::new(),
            rings: Vec::new(),
            weed: (String::new(), 0),
            wow: Vec::new(),
            wow_size: 0.,
        };
        let (w, h) = (stage.w, stage.h);
        let [x, y] = pad.tile;
        match kind {
            Kind::Badum => pad.words.push(Word::new(
                "BA",
                [w * 0.3, h * 0.46],
                100.,
                rgb(0xffd166),
                -8.,
                0.,
                1.3,
            )),
            Kind::Crickets => pad.weed = weed(stage),
            Kind::Applause => clapping_hands(stage, 0),
            Kind::Chaching => stage.emit(Emit {
                x,
                y,
                count: 34,
                angle: -90.,
                spread: 70.,
                speed: [10., 19.],
                gravity: 0.45,
                drag: 0.99,
                life: [80., 130.],
                size: [24., 34.],
                shape: Shape::Emoji,
                emoji: &["🪙"],
                wobble: false,
                ..Emit::default()
            }),
            Kind::Wooow => {
                pad.wow_letters(w);
                stage.emit(Emit {
                    x: w / 2.,
                    y: h / 2.,
                    count: 60,
                    angle: 0.,
                    spread: 360.,
                    speed: [4., 14.],
                    gravity: 0.05,
                    life: [50., 90.],
                    size: [10., 16.],
                    shape: Shape::Star,
                    colours: &WOW,
                    glow: true,
                    ..Emit::default()
                });
            }
            _ => {}
        }
        Some(pad)
    }

    /// WOOOOW, a letter at a time, each its own colour.
    fn wow_letters(&mut self, screen_w: f32) {
        let size = (screen_w * 0.16).min(190.);
        let big = 1.6;
        let font = size * big;
        let advances: Vec<f32> = "WOOOOW"
            .chars()
            .map(|ch| advance(&ch.to_string(), font, 900, WOW_FONT) / big)
            .collect();
        let total: f32 = advances.iter().sum();
        let mut left = -total / 2.;
        for (i, (ch, advance)) in "WOOOOW".chars().zip(advances).enumerate() {
            let c = looks::hex(hsl(190. - i as f32 * 34., 1., 0.68 + (i % 2) as f32 * 0.06));
            let pad = 30. * big * 2.5;
            let (lw, lh) = (advance * big + pad * 2., font * 1.3 + pad * 2.);
            let body = format!(
                r##"<defs><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="{dy}" stdDeviation="{blur}" flood-color="#4cc9f0" flood-opacity=".5"/></filter></defs><text x="{x}" y="{y}" text-anchor="middle" font-family="{WOW_FONT}" font-size="{font}" font-weight="900" fill="{c}" stroke="{INK}" stroke-opacity=".8" stroke-width="{stroke}" stroke-linejoin="round" paint-order="stroke" filter="url(#s)">{ch}</text>"##,
                x = lw / 2.,
                y = lh / 2. + font * 0.36,
                dy = 10. * big,
                blur = 15. * big,
                stroke = 8. * big,
            );
            let look = Look::new(format!("p5-wow:{ch}:{c}:{font}"), [lw, lh], body);
            self.wow.push((look, left + advance / 2.));
            left += advance;
        }
        self.wow_size = size;
    }

    fn ring(&mut self, stage: &Stage, from: f32, rgb: [u8; 3], alpha: f32, seconds: f32) {
        let [x, y] = self.tile;
        let r = (x.max(stage.w - x)).hypot(y.max(stage.h - y));
        self.rings.push(Ring {
            at: self.tile,
            r,
            rgb,
            alpha,
            from,
            seconds,
        });
    }
}

impl Effect for Pad {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        let (w, h) = (stage.w, stage.h);
        let [x, y] = self.tile;
        match self.kind {
            Kind::Airhorn => {
                for (i, at) in [0.03, 0.3, 0.57].into_iter().enumerate() {
                    if !clock.crossed(at) {
                        continue;
                    }
                    if i == 2 {
                        self.ring(stage, at, rgb(0xffd166), 0.6, 1.6);
                        stage.shake(13., 900.);
                    } else {
                        self.ring(stage, at, rgb(0xff6e8c), 0.5, 1.);
                        stage.shake(6., 220.);
                    }
                }
                if clock.crossed(0.74) {
                    self.ring(stage, 0.74, rgb(0xff4d6d), 0.5, 1.5);
                }
                if clock.crossed(0.91) {
                    self.ring(stage, 0.91, rgb(0xffffff), 0.45, 1.5);
                }
            }
            Kind::Badum => {
                if clock.crossed(0.17) {
                    self.words.push(Word::new(
                        "DUM",
                        [w * 0.5, h * 0.52],
                        110.,
                        rgb(0xff7ab6),
                        4.,
                        0.17,
                        1.3,
                    ));
                }
                if clock.crossed(0.4) {
                    let at = [w * 0.72, h * 0.42];
                    self.words
                        .push(Word::new("TSS!", at, 140., rgb(0x7df9ff), -6., 0.4, 1.5));
                    stage.emit(Emit {
                        x: at[0],
                        y: at[1],
                        count: 50,
                        angle: 0.,
                        spread: 360.,
                        speed: [4., 13.],
                        gravity: 0.15,
                        life: [40., 70.],
                        size: [8., 14.],
                        shape: Shape::Star,
                        colours: &SPARKS,
                        glow: true,
                        ..Emit::default()
                    });
                    stage.flash(rgb(0xe6fbff), 300., 0.35);
                    stage.shake(7., 320.);
                }
            }
            Kind::Trombone => {
                let wahs = [
                    ("wah", 0.34, 0.3, 84.),
                    ("wah", 0.47, 0.38, 74.),
                    ("wah", 0.6, 0.47, 66.),
                    ("waaaah", 0.74, 0.6, 104.),
                ];
                for (i, (text, fx, fy, size)) in wahs.into_iter().enumerate() {
                    let at = 0.03 + i as f32 * 0.48;
                    if clock.crossed(at) {
                        let last = i == 3;
                        let mut word = Word::new(
                            text,
                            [w * fx, h * fy],
                            size,
                            rgb(0xa9c1e6),
                            4. + i as f32 * 5.,
                            at,
                            if last { 2.2 } else { 1.3 },
                        );
                        word.droop = 18.;
                        word.drop = 70.;
                        word.wobble = last;
                        self.words.push(word);
                    }
                }
            }
            Kind::Drumroll => {
                if clock.crossed(2.2) {
                    stage.flash(rgb(0xffffff), 550., 0.85);
                    stage.shake(14., 700.);
                    stage.emit(Emit {
                        x,
                        y,
                        count: 170,
                        angle: 0.,
                        spread: 360.,
                        speed: [8., 22.],
                        gravity: 0.3,
                        life: [90., 150.],
                        size: [7., 13.],
                        ..Emit::default()
                    });
                    stage.emit(Emit {
                        x,
                        y,
                        count: 40,
                        angle: 0.,
                        spread: 360.,
                        speed: [5., 16.],
                        gravity: 0.12,
                        life: [40., 70.],
                        size: [10., 16.],
                        shape: Shape::Star,
                        colours: &TADA,
                        glow: true,
                        ..Emit::default()
                    });
                    self.words.push(Word::new(
                        "TA-DA!",
                        [w * 0.5, h * 0.42],
                        150.,
                        rgb(0xffd166),
                        -4.,
                        2.2,
                        1.6,
                    ));
                }
            }
            Kind::Chaching => {
                if clock.crossed(0.14) {
                    stage.flash(rgb(0xffd166), 600., 0.28);
                    self.words.push(Word::new(
                        "CHA-CHING!",
                        [w * 0.5, h * 0.4],
                        120.,
                        rgb(0xffd166),
                        -5.,
                        0.14,
                        1.5,
                    ));
                }
                for i in 0..20 {
                    if !clock.crossed(0.1 + i as f32 * 0.08) {
                        continue;
                    }
                    for _ in 0..6 {
                        let x = stage.rand(0., w);
                        let emoji: &'static [&'static str] = if stage.rng.chance(0.85) {
                            &["🪙"]
                        } else {
                            &["💰"]
                        };
                        stage.emit(Emit {
                            x,
                            y: -40.,
                            count: 1,
                            angle: 90.,
                            spread: 20.,
                            speed: [3., 8.],
                            gravity: 0.32,
                            life: [150., 220.],
                            size: [22., 38.],
                            shape: Shape::Emoji,
                            emoji,
                            wobble: false,
                            ..Emit::default()
                        });
                    }
                    let (x, y) = (stage.rand(0., w), stage.rand(0., h * 0.7));
                    stage.emit(Emit {
                        x,
                        y,
                        count: 3,
                        angle: 0.,
                        spread: 360.,
                        speed: [0.5, 2.],
                        gravity: 0.,
                        life: [20., 35.],
                        size: [8., 14.],
                        shape: Shape::Star,
                        colours: &GLINTS,
                        glow: true,
                        ..Emit::default()
                    });
                }
            }
            Kind::Applause => {
                for i in 1..30 {
                    if clock.crossed(i as f32 * 0.095) {
                        clapping_hands(stage, i);
                    }
                }
            }
            Kind::Crickets | Kind::Wooow => {}
        }
        clock.now < self.seconds
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = stage.now - self.born;
        match self.kind {
            Kind::Trombone => self.draw_trombone(stage, canvas, t),
            Kind::Crickets => self.draw_crickets(stage, canvas, t),
            Kind::Drumroll => self.draw_spot(stage, canvas, t),
            Kind::Wooow => self.draw_wow(stage, canvas, t),
            _ => {}
        }
        for ring in &self.rings {
            ring.draw(canvas, t);
        }
        for word in &self.words {
            word.draw(canvas, t);
        }
    }
}

impl Pad {
    /// The room goes grey, and a trombone droops in the corner and falls.
    fn draw_trombone(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let grey = keyframes(
            &[(0., 0.), (0.2, 1.), (0.8, 1.), (1., 0.)],
            t / 4.2,
            ease_in_out,
        );
        canvas.rect(
            [-40., -40.],
            [stage.w + 80., stage.h + 80.],
            0.,
            rgb(0x1c2840),
            0.4 * grey,
        );
        if t >= 4.3 {
            return;
        }
        // Where the prototype's 200px horn turns about (30% across, 60% down it).
        let pivot = [stage.w * 0.06 + 60., stage.h * 0.92 - 80.];
        let pose = |turn: f32, x: f32, y: f32| [turn, x, y];
        let between = |a: [f32; 3], b: [f32; 3], k: f32| {
            [
                lerp(a[0], b[0], k),
                lerp(a[1], b[1], k),
                lerp(a[2], b[2], k),
            ]
        };
        let [turn, dx, dy] = if t >= 2.95 {
            let k = bezier(0.5, 0., 0.8, 0.3, (t - 2.95) / 0.7);
            between(pose(30., 0., 18.), pose(55., -40., stage.h), k)
        } else if t >= 1.48 {
            // Eight droops back and forth, ending where they began.
            let p = (t - 1.48) / 0.18;
            let k = if p >= 8. {
                0.
            } else if (p as u32) % 2 == 1 {
                ease_in_out(1. - p.fract())
            } else {
                ease_in_out(p.fract())
            };
            between(pose(24., 0., 10.), pose(30., 0., 18.), k)
        } else if t >= 0.04 {
            // Each note tips it a little further.
            let i = [0.04, 0.52, 1.].iter().filter(|&&at| t >= at).count() - 1;
            let start = [0.04, 0.52, 1.][i];
            let k = spring((t - start) / 0.38);
            let base = -12. + i as f32 * 9.;
            pose(lerp(base, base + 9., k), 0., 0.)
        } else {
            between(pose(-30., -260., 60.), pose(-12., 0., 0.), spring(t / 0.45))
        };
        let look = turned_emoji("🎺", 200., 380., [60., 120.], turn, 3.);
        let [w, h] = look.size;
        canvas.picture_in(
            &look,
            [pivot[0] + dx - w / 2., pivot[1] + dy - h / 2.],
            [w, h],
            1.,
        );
    }

    /// Lights down, a cricket chirping in the corner and a tumbleweed bouncing past.
    fn draw_crickets(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let dim = keyframes(
            &[(0., 0.), (0.14, 1.), (0.85, 1.), (1., 0.)],
            t / 4.6,
            ease_in_out,
        );
        canvas.picture_in(
            &dim_look(),
            [-40., -40.],
            [stage.w + 80., stage.h + 80.],
            dim,
        );
        let k = t / 4.5;
        if k < 1. {
            let alpha = keyframes(&[(0., 0.), (0.1, 1.), (0.85, 1.), (1., 0.)], k, ease_out);
            let scale = keyframes(&[(0., 0.5), (0.1, 1.), (1., 1.)], k, ease_out);
            // It chirps: a hop and a tilt every quarter second.
            let up = (t / 0.25) as u32 % 2 == 1;
            let look = turned_emoji("🦗", 44., 72., [22., 22.], if up { 3. } else { 0. }, 3.);
            let centre = [
                stage.w * 0.92 - 25.,
                stage.h * 0.93 - 28. - if up { 2. } else { 0. },
            ];
            canvas.picture(&look, centre, scale, alpha);
        }
        let k = (t - 0.45) / 3.5;
        if (0. ..1.).contains(&k) {
            let hops = [
                (0., 0.),
                (0.12, -90.),
                (0.26, 0.),
                (0.38, -52.),
                (0.5, 0.),
                (0.61, -30.),
                (0.72, 0.),
                (0.82, -12.),
                (0.9, 0.),
                (1., 0.),
            ];
            let i = hops.windows(2).position(|pair| k <= pair[1].0).unwrap_or(8);
            let ((a, ya), (b, yb)) = (hops[i], hops[i + 1]);
            let s = (k - a) / (b - a);
            let eased = if i % 2 == 1 {
                bezier(0.55, 0., 0.75, 0.4, s)
            } else {
                bezier(0.25, 0.6, 0.45, 1., s)
            };
            let x = lerp(-150., stage.w + 150., k) + 60.;
            let y = stage.h * 0.74 + lerp(ya, yb, eased);
            let look = self.weed_look(1000. * k);
            let [w, h] = look.size;
            canvas.picture_in(&look, [x - w / 2., y - 80.], [w, h], 1.);
        }
    }

    fn weed_look(&self, turn: f32) -> Arc<Look> {
        let turn = ((turn / 10.).round() * 10.).rem_euclid(360.);
        let (strokes, id) = &self.weed;
        Look::new(
            format!("p5-weed:{id}:{turn}"),
            [160., 180.],
            format!(
                r##"<defs><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="18" stdDeviation="5" flood-color="#000" flood-opacity=".35"/></filter></defs><g filter="url(#s)"><g transform="translate(20 20) rotate({turn} 60 60) scale(1.2)" fill="none" stroke-linecap="round">{strokes}</g></g>"##
            ),
        )
    }

    /// A spotlight closes in on the call window, then TA-DA.
    fn draw_spot(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let (scale, alpha) = if t < 2.2 {
            let ease = |t| bezier(0.3, 0.6, 0.4, 1., t);
            let k = t / 2.2;
            (
                keyframes(&[(0., 3.5), (0.25, 1.35), (1., 0.95)], k, ease),
                keyframes(&[(0., 0.), (0.25, 1.), (1., 1.)], k, ease),
            )
        } else {
            (0.95, 1. - (t - 2.2) / 0.16)
        };
        let alpha = step_alpha(alpha);
        if alpha <= 0. {
            return;
        }
        let [x, y] = self.tile;
        let size = (300. * scale).round();
        let (x0, y0) = ((x - size / 2.).round(), (y - size / 2.).round());
        let (x1, y1) = (x0 + size, y0 + size);
        canvas.picture_in(&spot_look(), [x0, y0], [size, size], alpha);
        // The dark beyond it.
        let dark = rgb(0x04040c);
        let a = 0.76 * alpha;
        let (w, h) = (stage.w + 40., stage.h + 40.);
        canvas.rect([-40., -40.], [w + 40., y0 + 40.], 0., dark, a);
        canvas.rect([-40., y1], [w + 40., h - y1], 0., dark, a);
        canvas.rect([-40., y0], [x0 + 40., size], 0., dark, a);
        canvas.rect([x1, y0], [w - x1, size], 0., dark, a);
    }

    /// WOOOOW, its letters bouncing in a wave, and a 😮 rising from the call.
    fn draw_wow(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let k = t / 2.4;
        if k < 1. {
            let ease = |t| bezier(0.25, 1.3, 0.4, 1., t);
            let scale = keyframes(&[(0., 0.2), (0.2, 1.), (0.8, 1.06), (1., 1.25)], k, ease);
            let alpha = keyframes(&[(0., 0.), (0.2, 1.), (0.8, 1.), (1., 0.)], k, ease);
            let centre = [stage.w / 2., stage.h * 0.45];
            for (i, (look, x)) in self.wow.iter().enumerate() {
                let local = t - (0.25 + i as f32 * 0.09);
                let (grow, lift) = if (0. ..1.04).contains(&local) {
                    let p = (local / 0.52).fract();
                    (
                        keyframes(&[(0., 1.), (0.5, 1.45), (1., 1.)], p, ease_in_out),
                        keyframes(&[(0., 0.), (0.5, -0.06), (1., 0.)], p, ease_in_out),
                    )
                } else {
                    (1., 0.)
                };
                let at = [
                    centre[0] + x * scale,
                    centre[1] + grow * lift * self.wow_size * scale,
                ];
                canvas.picture(look, at, scale * grow / 1.6, alpha);
            }
        }
        let k = (t - 0.08) / 1.8;
        if (0. ..1.).contains(&k) {
            let rise = keyframes(&[(0., 0.5), (0.25, 0.9), (0.75, 1.2), (1., 1.6)], k, spring);
            let scale = keyframes(&[(0., 0.), (0.25, 1.15), (0.75, 1.), (1., 0.9)], k, spring);
            let alpha = keyframes(&[(0., 1.), (0.75, 1.), (1., 0.)], k, spring);
            let [x, y] = self.tile;
            let look = looks::emoji("😮");
            canvas.picture(
                &look,
                [x, y + (0.5 - rise) * 54.],
                scale * 54. / 58.5,
                alpha,
            );
        }
    }
}

/// A big word that pops in, holds, and drops away (the prototype's `word`).
struct Word {
    text: &'static str,
    at: [f32; 2],
    size: f32,
    rgb: [u8; 3],
    turn: f32,
    droop: f32,
    drop: f32,
    wobble: bool,
    from: f32,
    seconds: f32,
    /// Its width as drawn, [`BIG`] times its size.
    width: f32,
}

impl Word {
    fn new(
        text: &'static str,
        at: [f32; 2],
        size: f32,
        rgb: [u8; 3],
        turn: f32,
        from: f32,
        seconds: f32,
    ) -> Self {
        // Measured upright; the slant takes a little more.
        let width = looks::text_width(text, size * BIG, 900, WORD_FONT) + size * BIG * 0.15;
        Self {
            text,
            at,
            size,
            rgb,
            turn,
            droop: 0.,
            drop: 30.,
            wobble: false,
            from,
            seconds,
            width,
        }
    }

    fn draw(&self, canvas: &mut Canvas, t: f32) {
        let k = (t - self.from) / self.seconds;
        if !(0. ..=1.).contains(&k) {
            return;
        }
        let ease = |t| bezier(0.3, 0.7, 0.4, 1., t);
        let frames = |values: [f32; 5]| {
            let at = [0., 0.14, 0.26, 0.75, 1.];
            keyframes(
                &std::array::from_fn::<_, 5, _>(|i| (at[i], values[i])),
                k,
                ease,
            )
        };
        let scale = frames([0., 1.18, 1., 1., 0.92]);
        let alpha = frames([0., 1., 1., 1., 0.]);
        let dy = frames([0., 0., 0., 0., self.drop]);
        let mut turn = self.turn + frames([0., 0., 0., 0., self.droop]);
        let mut squash = 1.;
        if self.wobble {
            // Back and forth every 180 ms, for as long as it shows.
            let p = (t - self.from) / 0.18;
            if p < (self.seconds / 0.18).ceil() {
                let f = if (p as u32) % 2 == 1 {
                    1. - p.fract()
                } else {
                    p.fract()
                };
                let e = ease_in_out(f);
                turn += -7. + 14. * e;
                squash = 1. - 0.1 * e;
            }
        }
        let look = self.look((turn / 2.).round() * 2.);
        let [w, h] = look.size;
        let s = scale / BIG;
        let [x, y] = self.at;
        canvas.picture_in(
            &look,
            [x - w * s / 2., y + dy - h * s * squash / 2.],
            [w * s, h * s * squash],
            alpha,
        );
    }

    /// The word turned `turn` degrees, outlined, with its hard shadow and soft glow.
    fn look(&self, turn: f32) -> Arc<Look> {
        let size = self.size * BIG;
        let pad = 30. * BIG;
        let (w, h) = (self.width + pad * 2., size + pad * 2.);
        let (sin, cos) = turn.to_radians().sin_cos();
        let (bw, bh) = (
            (w * cos.abs() + h * sin.abs()).ceil(),
            (w * sin.abs() + h * cos.abs()).ceil(),
        );
        let (cx, cy) = (bw / 2., bh / 2.);
        let base = cy + size * 0.36;
        let text = looks::escape(self.text);
        let c = looks::hex(self.rgb);
        let stroke = 6. * BIG;
        Look::new(
            format!("p5-word:{}:{size}:{c}:{turn}", self.text),
            [bw, bh],
            format!(
                r##"<defs><filter id="g" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="{blur}"/></filter></defs><g transform="rotate({turn} {cx} {cy})" font-family="{WORD_FONT}" font-size="{size}" font-weight="900" font-style="italic" text-anchor="middle" stroke-linejoin="round"><text x="{cx}" y="{base}" fill="#fff" fill-opacity=".3" filter="url(#g)">{text}</text><text x="{cx}" y="{drop}" fill="{INK}" fill-opacity=".45" stroke="{INK}" stroke-opacity=".45" stroke-width="{stroke}">{text}</text><text x="{cx}" y="{base}" fill="{c}" stroke="{INK}" stroke-opacity=".85" stroke-width="{stroke}" paint-order="stroke">{text}</text></g>"##,
                blur = 20. * BIG,
                drop = base + 7. * BIG,
            ),
        )
    }
}

/// A shock wave out from the call window to past the screen's far corner.
struct Ring {
    at: [f32; 2],
    r: f32,
    rgb: [u8; 3],
    alpha: f32,
    from: f32,
    seconds: f32,
}

impl Ring {
    fn draw(&self, canvas: &mut Canvas, t: f32) {
        let k = (t - self.from) / self.seconds;
        if !(0. ..1.).contains(&k) {
            return;
        }
        let e = bezier(0.1, 0.8, 0.3, 1., k);
        let r = self.r * lerp(0.02, 1., e);
        let alpha = self.alpha * (1. - e);
        // The prototype's soft band, rising from 80% of its radius to 92% and gone by
        // the edge, as three rings.
        for (at, width, a) in [(0.83, 0.06, 0.25), (0.91, 0.1, 0.75), (0.98, 0.04, 0.25)] {
            canvas.ring(self.at, r * at, r * width, self.rgb, alpha * a);
        }
    }
}

/// The darkness for the crickets, drawn small and stretched over the screen.
fn dim_look() -> Arc<Look> {
    Look::new(
        "p5-dim",
        [64., 40.],
        r##"<defs><radialGradient id="d" cx=".5" cy=".6" r="1" gradientTransform="matrix(1.2 0 0 1 -.1 0)"><stop offset="0" stop-color="#0a0c19" stop-opacity=".5"/><stop offset="1" stop-color="#04050c" stop-opacity=".78"/></radialGradient></defs><rect width="64" height="40" fill="url(#d)"/>"##,
    )
}

/// The spotlight's circle: a faint warm light, darkening towards its edge, dark at it.
fn spot_look() -> Arc<Look> {
    Look::new(
        "p5-spot",
        [300., 300.],
        r##"<defs><radialGradient id="l" cx="150" cy="150" r="148" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff0c8" stop-opacity=".14"/><stop offset="1" stop-color="#fff0c8" stop-opacity="0"/></radialGradient><radialGradient id="d" cx="150" cy="150" r="150" gradientUnits="userSpaceOnUse"><stop offset=".45" stop-color="#04040c" stop-opacity="0"/><stop offset=".6" stop-color="#04040c" stop-opacity=".1"/><stop offset=".81" stop-color="#04040c" stop-opacity=".38"/><stop offset=".93" stop-color="#04040c" stop-opacity=".66"/><stop offset="1" stop-color="#04040c" stop-opacity=".76"/></radialGradient></defs><circle cx="150" cy="150" r="150" fill="url(#l)"/><rect width="300" height="300" fill="url(#d)"/>"##,
    )
}

/// Emoji `ch` set `font` big (its box `font` square) in a square `side` across, turned
/// `turn` degrees (in steps of `step`) about (`ox`, `oy`) in its box, which is put in the
/// middle.
fn turned_emoji(
    ch: &str,
    font: f32,
    side: f32,
    [ox, oy]: [f32; 2],
    turn: f32,
    step: f32,
) -> Arc<Look> {
    let turn = (turn / step).round() * step;
    let c = side / 2.;
    Look::new(
        format!("p5-emoji:{ch}:{font}:{side}:{ox}:{oy}:{turn}"),
        [side, side],
        format!(
            r#"<g transform="rotate({turn} {c} {c}) translate({dx} {dy})"><text x="{x}" y="{y}" text-anchor="middle" font-family="{EMOJI}" font-size="{font}">{}</text></g>"#,
            looks::escape(ch),
            dx = c - ox,
            dy = c - oy,
            x = font / 2.,
            y = font * 0.88,
        ),
    )
}

/// The `i`th of thirty handfuls of 👏 thrown up from the bottom, most in the middle.
fn clapping_hands(stage: &mut Stage, i: usize) {
    let swell = (std::f32::consts::PI * i as f32 / 30.).sin();
    for _ in 0..1 + (swell * 4.).round() as usize {
        let x = stage.rand(30., stage.w - 30.);
        stage.emit(Emit {
            x,
            y: stage.h + 20.,
            count: 1,
            angle: -90.,
            spread: 24.,
            speed: [10., 16.],
            gravity: 0.25,
            life: [70., 110.],
            size: [26., 42.],
            shape: Shape::Emoji,
            emoji: &["👏"],
            wobble: false,
            ..Emit::default()
        });
    }
}

/// A tumbleweed: thirty random curls, in two browns.
fn weed(stage: &mut Stage) -> (String, u64) {
    let mut point = || {
        let a = stage.rng.range(0., std::f32::consts::TAU);
        let r = stage.rng.unit().sqrt() * 42.;
        format!("{:.1} {:.1}", 50. + a.cos() * r, 50. + a.sin() * r)
    };
    let (mut dark, mut light) = (String::new(), String::new());
    for i in 0..30 {
        let path = format!(
            r#"<path d="M{} C{} {} {}"/>"#,
            point(),
            point(),
            point(),
            point()
        );
        if i % 2 == 1 {
            dark.push_str(&path);
        } else {
            light.push_str(&path);
        }
    }
    let id = stage.rng.next_u64();
    (
        format!(
            r##"<g stroke="#a47a45" stroke-width="2.4">{dark}</g><g stroke="#d9b27a" stroke-width="1.6">{light}</g>"##
        ),
        id,
    )
}

/// The width `text` takes up in a line, from its start to the next one's.
fn advance(text: &str, size: f32, weight: u16, family: &str) -> f32 {
    looks::text_width(&format!("|{text}|"), size, weight, family)
        - looks::text_width("||", size, weight, family)
}

/// CSS's `hsl()`, with `s` and `l` from 0 to 1.
fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let hp = h.rem_euclid(360.) / 60.;
    let x = c * (1. - (hp % 2. - 1.).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    let m = l - c / 2.;
    [r, g, b].map(|v| ((v + m) * 255.).round().clamp(0., 255.) as u8)
}

// ---- Sounds ---------------------------------------------------------------------------

/// The prototype's `out(wet)`: one way out for each amount of reverb, shared.
#[derive(Default)]
struct Outs(Vec<(f32, Id)>);

impl Outs {
    fn to(&mut self, graph: &mut Graph, wet: f32) -> Id {
        if let Some(&(_, id)) = self.0.iter().find(|(w, _)| *w == wet) {
            return id;
        }
        let id = graph.dest(wet);
        self.0.push((wet, id));
        id
    }
}

/// The prototype's `noiseLoop`: looping noise, so long sounds never run out of it.
struct Loop {
    when: f32,
    dur: f32,
    gain: f32,
    filter: Filter,
    freq: f32,
    q: f32,
    sweep_to: Option<f32>,
    attack: f32,
    hold: Option<f32>,
}

impl Default for Loop {
    fn default() -> Self {
        Self {
            when: 0.,
            dur: 1.,
            gain: 0.1,
            filter: Filter::Bandpass,
            freq: 1000.,
            q: 1.,
            sweep_to: None,
            attack: 0.01,
            hold: None,
        }
    }
}

fn noise_loop(graph: &mut Graph, o: Loop, to: Id) {
    let t = o.when;
    let src = graph.noise_source(true);
    let f = graph.filter(o.filter, o.freq, o.q);
    graph.frequency(f).set(o.freq, t);
    if let Some(sweep) = o.sweep_to {
        graph.frequency(f).exp(sweep, t + o.dur);
    }
    let g = graph.gain(1.);
    graph.gain_of(g).set(0.0001, t).exp(o.gain, t + o.attack);
    if let Some(hold) = o.hold {
        graph.gain_of(g).set(o.gain, t + hold);
    }
    graph.gain_of(g).exp(0.0001, t + o.dur);
    graph.connect(src, f);
    graph.connect(f, g);
    graph.connect(g, to);
    graph.play(src, t, t + o.dur + 0.05);
}

/// Three blasts of five detuned saws, saturated, the last one long and sagging.
fn airhorn_sound(graph: &mut Graph, outs: &mut Outs) {
    let t0 = 0.03;
    let sh = graph.shaper(saturation(2.2));
    let lp = graph.filter(Filter::Lowpass, 3800., 1.2);
    let pk = graph.filter(Filter::Peaking, 1250., 1.4);
    graph.gain_of(pk).set(7., 0.);
    let out = outs.to(graph, 0.22);
    graph.connect(sh, lp);
    graph.connect(lp, pk);
    graph.connect(pk, out);
    for (i, (w, d)) in [(0., 0.2), (0.27, 0.2), (0.54, 1.05)]
        .into_iter()
        .enumerate()
    {
        let (t, last) = (t0 + w, i == 2);
        let g = graph.gain(0.);
        graph
            .gain_of(g)
            .set(0., t)
            .linear(0.13, t + 0.015)
            .set(0.13, t + d)
            .linear(0., t + d + 0.08);
        graph.connect(g, sh);
        for (f, detune, level) in [
            (233., 0., 0.8),
            (466., -16., 0.6),
            (468., 11., 0.6),
            (698., 6., 0.35),
            (935., -9., 0.25),
        ] {
            let o = graph.osc(Wave::Saw, f);
            graph.detune(o).set(detune, 0.);
            graph.frequency(o).set(f * 0.9, t).exp(f, t + 0.06);
            if last {
                graph
                    .frequency(o)
                    .set(f, t + d - 0.15)
                    .exp(f * 0.96, t + d + 0.08);
            }
            let og = graph.gain(level);
            graph.connect(o, og);
            graph.connect(og, g);
            graph.play(o, t, t + d + 0.12);
        }
    }
}

/// Two toms, then a crash: sizzling noise and a cymbal's inharmonic ring.
fn badum_sound(graph: &mut Graph, outs: &mut Outs) {
    for (when, freq) in [(0., 200.), (0.17, 140.)] {
        let to = Some(outs.to(graph, 0.12));
        graph.tone(Tone {
            when,
            freq,
            glide_to: Some(freq * 0.5),
            dur: 0.3,
            gain: 0.6,
            to,
            ..Tone::default()
        });
        let to = Some(outs.to(graph, 0.1));
        graph.noise(Noise {
            when,
            dur: 0.04,
            gain: 0.25,
            filter: Filter::Lowpass,
            freq: 1800.,
            to,
            ..Noise::default()
        });
    }
    let to = outs.to(graph, 0.3);
    noise_loop(
        graph,
        Loop {
            when: 0.4,
            dur: 1.4,
            gain: 0.38,
            filter: Filter::Highpass,
            freq: 6500.,
            q: 0.5,
            attack: 0.003,
            ..Loop::default()
        },
        to,
    );
    let to = Some(outs.to(graph, 0.2));
    graph.noise(Noise {
        when: 0.4,
        dur: 0.14,
        gain: 0.35,
        filter: Filter::Bandpass,
        freq: 2500.,
        q: 0.9,
        to,
        ..Noise::default()
    });
    for freq in [3150., 4370., 5210., 6880.] {
        let to = Some(outs.to(graph, 0.3));
        graph.tone(Tone {
            when: 0.4,
            freq,
            wave: Wave::Square,
            dur: 0.9,
            gain: 0.014,
            to,
            ..Tone::default()
        });
    }
}

/// Four falling notes on a muted brass, the last one long, wobbling and sagging.
fn trombone_sound(graph: &mut Graph, outs: &mut Outs) {
    let t0 = 0.03;
    for (i, (w, f, d)) in [
        (0., 293.7, 0.42),
        (0.48, 277.2, 0.42),
        (0.96, 261.6, 0.42),
        (1.44, 246.9, 1.5),
    ]
    .into_iter()
    .enumerate()
    {
        let (t, last) = (t0 + w, i == 3);
        let g = graph.gain(0.);
        let lp = graph.filter(Filter::Lowpass, 320., 7.);
        graph
            .frequency(lp)
            .set(320., t)
            .exp(1500., t + 0.14)
            .exp(if last { 900. } else { 480. }, t + d);
        graph
            .gain_of(g)
            .set(0., t)
            .linear(0.2, t + 0.05)
            .set(0.2, t + d - 0.12)
            .linear(0., t + d);
        let out = outs.to(graph, 0.25);
        graph.connect(lp, g);
        graph.connect(g, out);
        for detune in [0., 9.] {
            let o = graph.osc(Wave::Saw, f);
            graph.detune(o).set(detune, 0.);
            graph.frequency(o).set(f * 1.03, t).exp(f, t + 0.08);
            if last {
                graph.frequency(o).set(f, t + 0.3).exp(f * 0.93, t + d);
                let lfo = graph.osc(Wave::Sine, 5.5);
                let pitch = graph.gain(7.);
                graph.connect(lfo, pitch);
                graph.modulate(pitch, o, Graph::frequency);
                if detune == 0. {
                    let mute = graph.gain(550.);
                    graph.connect(lfo, mute);
                    graph.modulate(mute, lp, Graph::frequency);
                }
                graph.play(lfo, t + 0.25, t + d);
            }
            graph.connect(o, lp);
            graph.play(o, t, t + d + 0.05);
        }
    }
}

/// Chirps in threes at random, over a night breeze.
fn crickets_sound(graph: &mut Graph, outs: &mut Outs) {
    let mut chirp = |graph: &mut Graph, when: f32, freq: f32| {
        for k in 0..3 {
            let to = Some(outs.to(graph, 0.35));
            graph.tone(Tone {
                when: when + k as f32 * 0.045,
                freq,
                dur: 0.03,
                gain: 0.07,
                attack: 0.004,
                to,
                ..Tone::default()
            });
        }
    };
    let mut t = 0.5;
    while t < 3.7 {
        chirp(graph, t, 4400.);
        if graph.rng.chance(0.6) {
            chirp(graph, t + 0.19, 4750.);
        }
        t += graph.rng.range(0.42, 0.8);
    }
    let to = outs.to(graph, 0.3);
    noise_loop(
        graph,
        Loop {
            dur: 3.8,
            gain: 0.05,
            filter: Filter::Bandpass,
            freq: 450.,
            sweep_to: Some(900.),
            q: 0.6,
            attack: 1.2,
            ..Loop::default()
        },
        to,
    );
}

/// A snare roll, faster and louder, then a boom and a crash.
fn drumroll_sound(graph: &mut Graph, outs: &mut Outs) {
    let mut t = 0f32;
    while t < 2.15 {
        let k = t / 2.15;
        let gain = 0.05 + 0.3 * k * k;
        let freq = graph.rng.range(2600., 3600.);
        let to = Some(outs.to(graph, 0.15));
        graph.noise(Noise {
            when: t,
            dur: 0.06,
            gain,
            filter: Filter::Bandpass,
            freq,
            q: 0.8,
            to,
            ..Noise::default()
        });
        let to = Some(outs.to(graph, 0.1));
        graph.tone(Tone {
            when: t,
            freq: 185.,
            glide_to: Some(160.),
            dur: 0.05,
            wave: Wave::Triangle,
            gain: gain * 0.5,
            to,
            ..Tone::default()
        });
        t += 0.075 - 0.043 * k;
    }
    let to = Some(outs.to(graph, 0.2));
    graph.tone(Tone {
        when: 2.2,
        freq: 120.,
        glide_to: Some(40.),
        dur: 0.55,
        gain: 0.9,
        to,
        ..Tone::default()
    });
    let to = outs.to(graph, 0.4);
    noise_loop(
        graph,
        Loop {
            when: 2.2,
            dur: 2.3,
            gain: 0.45,
            filter: Filter::Highpass,
            freq: 5000.,
            q: 0.5,
            attack: 0.003,
            ..Loop::default()
        },
        to,
    );
    let to = Some(outs.to(graph, 0.3));
    graph.noise(Noise {
        when: 2.2,
        dur: 0.3,
        gain: 0.4,
        filter: Filter::Bandpass,
        freq: 2200.,
        q: 0.7,
        to,
        ..Noise::default()
    });
}

/// The till's clunk, two bells, and coins tinkling.
fn chaching_sound(graph: &mut Graph, outs: &mut Outs) {
    let to = Some(outs.to(graph, 0.1));
    graph.noise(Noise {
        dur: 0.05,
        gain: 0.4,
        filter: Filter::Lowpass,
        freq: 2500.,
        to,
        ..Noise::default()
    });
    graph.tone(Tone {
        freq: 620.,
        wave: Wave::Square,
        dur: 0.04,
        gain: 0.08,
        to,
        ..Tone::default()
    });
    graph.noise(Noise {
        when: 0.05,
        dur: 0.18,
        gain: 0.25,
        filter: Filter::Bandpass,
        freq: 900.,
        q: 2.,
        sweep_to: Some(400.),
        to,
        ..Noise::default()
    });
    let bell_to = Some(outs.to(graph, 0.3));
    for (freq, gain) in [(1568., 0.22), (2093., 0.16)] {
        for (m, level) in [(1., 1.), (2.76, 0.5), (5.4, 0.25), (8.93, 0.12)] {
            graph.tone(Tone {
                when: 0.14,
                freq: freq * m,
                dur: 1.7 / f32::powf(m, 0.3),
                gain: gain * level,
                attack: 0.002,
                to: bell_to,
                ..Tone::default()
            });
        }
    }
    let to = Some(outs.to(graph, 0.4));
    for _ in 0..45 {
        let when = 0.2 + graph.rng.unit() * 1.5;
        let freq = graph.rng.range(3200., 7200.);
        let dur = graph.rng.range(0.06, 0.18);
        let gain = graph.rng.range(0.015, 0.05);
        graph.tone(Tone {
            when,
            freq,
            dur,
            gain,
            to,
            ..Tone::default()
        });
    }
}

/// A crowd going "wooow": nine voices swooping up and down through vowel formants.
fn wooow_sound(graph: &mut Graph, outs: &mut Outs) {
    let (t0, long) = (0.02, 2.3);
    let out = outs.to(graph, 0.4);
    for _ in 0..9 {
        let f = *graph.rng.pick(&[110., 130., 150., 180., 210., 240., 260.])
            * graph.rng.range(0.97, 1.03);
        let t = t0 + graph.rng.range(0., 0.18);
        let osc = graph.osc(Wave::Saw, f);
        graph
            .frequency(osc)
            .set(f * 0.8, t)
            .exp(f * 1.3, t + 0.75)
            .exp(f * 0.78, t + long);
        let rate = graph.rng.range(4.5, 6.);
        let vib = graph.osc(Wave::Sine, rate);
        let depth = graph.gain(f * 0.015);
        graph.connect(vib, depth);
        graph.modulate(depth, osc, Graph::frequency);
        let g = graph.gain(1.);
        graph
            .gain_of(g)
            .set(0.0001, t)
            .exp(0.12, t + 0.35)
            .set(0.12, t + 1.2)
            .exp(0.0001, t + long);
        let f1 = graph.filter(Filter::Bandpass, 320., 5.);
        let f2 = graph.filter(Filter::Bandpass, 800., 6.);
        let g2 = graph.gain(0.6);
        graph
            .frequency(f1)
            .set(320., t)
            .exp(650., t + 0.75)
            .exp(380., t + long);
        graph
            .frequency(f2)
            .set(800., t)
            .exp(1080., t + 0.75)
            .exp(780., t + long);
        graph.connect(osc, f1);
        graph.connect(f1, g);
        graph.connect(osc, f2);
        graph.connect(f2, g2);
        graph.connect(g2, g);
        graph.connect(g, out);
        graph.play(osc, t, t + long + 0.05);
        graph.play(vib, t, t + long + 0.05);
    }
}

/// A room's applause: a bed of noise and two hundred claps, swelling and fading.
fn applause_sound(graph: &mut Graph, outs: &mut Outs) {
    let long = 3.2;
    let to = outs.to(graph, 0.3);
    noise_loop(
        graph,
        Loop {
            dur: long,
            gain: 0.07,
            filter: Filter::Bandpass,
            freq: 1500.,
            q: 0.4,
            attack: 1.2,
            hold: Some(1.8),
            ..Loop::default()
        },
        to,
    );
    let to = Some(outs.to(graph, 0.25));
    for _ in 0..200 {
        let rng = &mut graph.rng;
        let when = long * (rng.unit() + rng.unit() + rng.unit()) / 3.;
        let swell = (std::f32::consts::PI * when / long).sin();
        let dur = rng.range(0.03, 0.07);
        let gain = 0.04 + 0.2 * swell * rng.unit();
        let freq = rng.range(900., 2600.);
        let q = rng.range(0.8, 1.6);
        graph.noise(Noise {
            when,
            dur,
            gain,
            filter: Filter::Bandpass,
            freq,
            q,
            to,
            ..Noise::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_eight_pads_play() {
        let mut stage = Stage::new(1440., 900., 1);
        let who = Who::new("a", "Sam Rivera", [1, 2, 3], false);
        for (choice, ..) in PADS {
            assert!(Pad::new(&mut stage, &who, choice).is_some(), "{choice}");
        }
        assert!(Pad::new(&mut stage, &who, "kazoo").is_none());
    }

    #[test]
    fn css_colours_come_out_right() {
        assert_eq!(hsl(190., 1., 0.68), [0x5c, 0xe4, 0xff]);
        assert_eq!(hsl(0., 1., 0.5), [255, 0, 0]);
    }
}
