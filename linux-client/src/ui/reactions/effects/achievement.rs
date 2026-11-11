//! 8. Achievement unlocked: a console-style toast for everyone, for one person or the
//! whole room, with the two-note "bloop-bling" chime. A newer one sends it on its way.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use super::Effect;
use crate::ui::reactions::{
    looks::{self, EMOJI, FONT, Look},
    particles::{Emit, Shape},
    scene::Canvas,
    stage::{Clock, Stage, Who, avatar_svg, bezier, clamp01, ease, ease_in_out, lerp, rgb},
    synth::{Filter, Graph, Noise, Tone, Wave},
};

const GOLD: [[u8; 3]; 5] = [
    rgb(0xffd34d),
    rgb(0xffe58a),
    rgb(0xfff3c4),
    rgb(0xffb627),
    rgb(0xffffff),
];
/// The pill's height, and its width closed round the trophy.
const HIGH: f32 = 72.;
/// The toast's top while it's up.
const TOP: f32 = 100.;
/// The trophy is drawn this big and scaled down.
const TROPHY: f32 = 48.;
/// How far the icon's glow reaches into the pill.
const GLOW: f32 = 112.;

/// The toast dropping in.
fn drop_in(t: f32) -> f32 {
    bezier(0.2, 1.6, 0.4, 1., t)
}

/// The toast flying off.
fn fly_off(t: f32) -> f32 {
    bezier(0.55, -0.5, 0.8, 0.4, t)
}

/// The pill opening and closing.
fn open(t: f32) -> f32 {
    bezier(0.2, 1.12, 0.3, 1., t)
}

/// Its words sliding in.
fn slide(t: f32) -> f32 {
    bezier(0.2, 1.5, 0.4, 1., t)
}

/// A value easing to another, as a CSS transition does.
#[derive(Clone, Copy)]
struct Glide {
    from: f32,
    to: f32,
    start: f32,
    seconds: f32,
    curve: fn(f32) -> f32,
}

impl Glide {
    fn still(value: f32) -> Self {
        Self::new(value, value, 0., 0., ease)
    }

    fn new(from: f32, to: f32, start: f32, seconds: f32, curve: fn(f32) -> f32) -> Self {
        Self {
            from,
            to,
            start,
            seconds,
            curve,
        }
    }

    fn at(&self, now: f32) -> f32 {
        let k = if self.seconds > 0. {
            (now - self.start) / self.seconds
        } else {
            1.
        };
        if k >= 1. {
            self.to
        } else {
            lerp(self.from, self.to, (self.curve)(k.max(0.)))
        }
    }

    /// Off to `to` from `start` (perhaps a moment from now, as a delayed transition).
    fn go(&mut self, start: f32, to: f32, seconds: f32, curve: fn(f32) -> f32) {
        if to != self.to {
            *self = Self::new(self.at(start), to, start, seconds, curve);
        }
    }
}

/// Some of the pill's insides: its words, or its points and who they're for.
struct Part {
    svg: String,
    id: u64,
    width: f32,
}

impl Part {
    fn new(svg: String, width: f32) -> Self {
        let mut hasher = DefaultHasher::new();
        svg.hash(&mut hasher);
        Self {
            svg,
            id: hasher.finish(),
            width,
        }
    }

    /// As much of it as fits in `room` (rounded down a little), the pill hiding the rest;
    /// none if none does.
    fn look(&self, room: f32) -> Option<Arc<Look>> {
        let width = if room >= self.width {
            self.width
        } else {
            (room / 6.).floor() * 6.
        };
        (width >= 1.).then(|| {
            Look::new(
                format!("p8-part:{:x}:{width}", self.id),
                [width, HIGH],
                self.svg.clone(),
            )
        })
    }
}

pub struct Achievement {
    born: f32,
    /// Where the stars come out for whoever it's for.
    tile: Option<[f32; 2]>,
    body: Part,
    end: Part,
    /// The pill's width open.
    wide: f32,
    width: Glide,
    rise: Glide,
    scale: Glide,
    shown: Glide,
    body_shown: Glide,
    body_slide: Glide,
    end_shown: Glide,
    end_slide: Glide,
    glint: Option<f32>,
    /// Along the pill's foot, for the sparkles: its left, width and bottom.
    sparkles: Option<[f32; 3]>,
    /// When it began to leave.
    out: Option<f32>,
}

impl Achievement {
    pub fn new(stage: &mut Stage, who: &Who, text: &str, to: Option<Who>) -> Self {
        let text: String = if text.is_empty() { "Nailed it" } else { text }
            .chars()
            .take(60)
            .collect();
        let rec = to.as_ref();
        stage.banner(who, &says(who, rec), "🏆", 3000.);
        chime(stage.sound());
        let body = body(rec.map_or("Everyone", Who::called), &text);
        let end = end(rec, &stage.people);
        let now = stage.now;
        Self {
            born: now,
            tile: rec.map(|_| stage.tile()),
            wide: HIGH + body.width + end.width,
            body,
            end,
            width: Glide::still(HIGH),
            rise: Glide::new(-40., 0., now, 0.5, drop_in),
            scale: Glide::new(0.6, 1., now, 0.5, drop_in),
            shown: Glide::new(0., 1., now, 0.2, ease),
            body_shown: Glide::still(0.),
            body_slide: Glide::still(-14.),
            end_shown: Glide::still(0.),
            end_slide: Glide::still(-14.),
            glint: None,
            sparkles: None,
            out: None,
        }
    }

    /// The toast's top left on screen now, and its scale.
    fn place(&self, stage: &Stage, now: f32) -> ([f32; 2], f32) {
        let width = self.width.at(now).max(HIGH);
        let s = self.scale.at(now);
        (
            [
                stage.w / 2. - width * s / 2.,
                TOP + self.rise.at(now) + HIGH / 2. - HIGH * s / 2.,
            ],
            s,
        )
    }

    /// Open the pill to show its insides, or close it round the trophy.
    fn widen(&mut self, now: f32, open_up: bool) {
        let (to, shown, slid) = if open_up {
            (self.wide, 1., 0.)
        } else {
            (HIGH, 0., -14.)
        };
        self.width.go(now, to, 0.62, open);
        self.body_shown.go(now + 0.18, shown, 0.3, ease);
        self.body_slide.go(now + 0.18, slid, 0.55, slide);
        self.end_shown.go(now + 0.28, shown, 0.3, ease);
        self.end_slide.go(now + 0.28, slid, 0.55, slide);
    }

    fn leave(&mut self, now: f32) {
        self.out = Some(now);
        self.rise.go(now, -70., 0.45, fly_off);
        self.scale.go(now, 0.85, 0.45, fly_off);
        self.shown.go(now + 0.05, 0., 0.4, ease);
    }
}

impl Effect for Achievement {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        let now = stage.now;
        if self.out.is_none() {
            // The trophy pops: gold stars burst out of the circle.
            if clock.crossed(0.17) {
                let (at, s) = self.place(stage, now);
                let [x, y] = [at[0] + HIGH / 2. * s, at[1] + HIGH / 2. * s];
                stage.emit(Emit {
                    x,
                    y,
                    count: 34,
                    angle: 0.,
                    spread: 360.,
                    speed: [3., 10.],
                    gravity: 0.12,
                    drag: 0.94,
                    life: [40., 80.],
                    size: [8., 15.],
                    shape: Shape::Star,
                    colours: &GOLD,
                    glow: true,
                    spin: true,
                    ..Emit::default()
                });
                stage.emit(Emit {
                    x,
                    y,
                    count: 24,
                    angle: 0.,
                    spread: 360.,
                    speed: [1., 6.],
                    gravity: 0.05,
                    drag: 0.95,
                    life: [30., 60.],
                    size: [2., 4.],
                    shape: Shape::Circle,
                    colours: &GOLD,
                    glow: true,
                    ..Emit::default()
                });
            }
            // The circle opens into the wide bar.
            if clock.crossed(0.42) {
                self.widen(now, true);
                stage.sound().noise(Noise {
                    dur: 0.35,
                    gain: 0.05,
                    filter: Filter::Bandpass,
                    freq: 700.,
                    sweep_to: Some(3500.),
                    q: 0.8,
                    attack: 0.08,
                    wet: 0.2,
                    ..Noise::default()
                });
                if let Some([x, y]) = self.tile {
                    stage.emit(Emit {
                        x,
                        y,
                        count: 40,
                        angle: -90.,
                        spread: 200.,
                        speed: [4., 11.],
                        gravity: 0.2,
                        drag: 0.95,
                        life: [40., 90.],
                        size: [7., 13.],
                        shape: Shape::Star,
                        colours: &GOLD,
                        glow: true,
                        ..Emit::default()
                    });
                }
            }
            if clock.crossed(1.05) {
                self.glint = Some(now);
                let (at, s) = self.place(stage, now);
                let width = self.width.at(now) * s;
                self.sparkles = Some([at[0], width, at[1] + HIGH * s]);
            }
            if clock.crossed(4.3) {
                self.widen(now, false);
            }
            if clock.crossed(4.7) {
                self.leave(now);
            }
        }
        // Stars trickle off the pill's foot as the glint crosses it (even once a newer
        // toast has sent this one off, as on the Mac).
        if let Some([left, width, bottom]) = self.sparkles {
            for i in 0..5 {
                if clock.crossed(1.05 + i as f32 * 0.11) {
                    stage.emit(Emit {
                        x: left + width * (i as f32 + 0.5) / 5.,
                        y: bottom - 6.,
                        count: 5,
                        angle: 90.,
                        spread: 70.,
                        speed: [1., 3.],
                        gravity: 0.08,
                        life: [30., 55.],
                        size: [5., 9.],
                        shape: Shape::Star,
                        colours: &GOLD,
                        glow: true,
                        ..Emit::default()
                    });
                }
            }
        }
        self.out.is_none_or(|out| now < out + 0.5)
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let now = stage.now;
        let alpha = self.shown.at(now);
        if alpha <= 0.004 {
            return;
        }
        let width = self.width.at(now).max(HIGH);
        let (origin, s) = self.place(stage, now);
        let at = |x: f32, y: f32| [origin[0] + x * s, origin[1] + y * s];

        // Its shadow, and a warm glow round it.
        let mut halo = |grow: f32, dy: f32, colour: [u8; 3], a: f32| {
            canvas.rect(
                at(-grow, dy - grow),
                [(width + grow * 2.) * s, (HIGH + grow * 2.) * s],
                (HIGH / 2. + grow) * s,
                colour,
                a * alpha,
            );
        };
        for i in 0..24 {
            halo(-20. + i as f32 * 3., 20., [0, 0, 0], 0.018);
        }
        for i in 0..17 {
            halo(-10. + i as f32 * 3., 0., rgb(0xffbe28), 0.01);
        }

        // The pill: its ends, and the middle stretched between them.
        let [left, middle, right] = pill();
        canvas.picture_in(&left, at(-2., -2.), [38. * s, 76. * s], alpha);
        if width > HIGH + 0.5 {
            canvas.picture_in(
                &middle,
                at(HIGH / 2. - 0.5, -2.),
                [(width - HIGH + 1.) * s, 76. * s],
                alpha,
            );
        }
        canvas.picture_in(
            &right,
            at(width - HIGH / 2., -2.),
            [38. * s, 76. * s],
            alpha,
        );

        // The gold circle, glowing as it breathes, and the trophy popping in.
        let breath = (now - self.born) / 1.1;
        let breath = ease_in_out(if (breath as u32) % 2 == 1 {
            1. - breath.fract()
        } else {
            breath.fract()
        });
        let reach = ((width.min(GLOW) / 8.).ceil() * 8.).min(GLOW);
        let glow_size = [GLOW * s, HIGH * s];
        canvas.picture_in(&icon_glow(false, reach), at(0., 0.), glow_size, alpha);
        canvas.picture_in(
            &icon_glow(true, reach),
            at(0., 0.),
            glow_size,
            alpha * breath,
        );
        canvas.picture_in(&coin(), at(0., 0.), [HIGH * s, HIGH * s], alpha);
        let pop = bezier(0.2, 1.9, 0.4, 1., clamp01((now - self.born) / 0.75));
        canvas.picture(
            &trophy(lerp(-40., 0., pop)),
            at(HIGH / 2., HIGH / 2.),
            pop * s * 28. / TROPHY,
            alpha,
        );

        // Its insides, as much of them as the pill shows.
        let parts = [
            (&self.body, HIGH, &self.body_shown, &self.body_slide),
            (
                &self.end,
                HIGH + self.body.width,
                &self.end_shown,
                &self.end_slide,
            ),
        ];
        for (part, x, shown, slid) in parts {
            let a = shown.at(now) * alpha;
            let x = x + slid.at(now);
            if a > 0.004
                && let Some(look) = part.look(width - x)
            {
                canvas.picture_in(&look, at(x, 0.), [look.size[0] * s, HIGH * s], a);
            }
        }

        // The glint sweeping across it.
        if let Some(start) = self.glint {
            let k = (now - start) / 1.;
            if k < 1. {
                glint(
                    canvas,
                    width,
                    bezier(0.45, 0., 0.2, 1., k),
                    alpha,
                    |[x, y]| at(x, y),
                );
            }
        }
    }

    fn again(&mut self, stage: &mut Stage, _: &Who, _: &crate::protocol::Reaction) -> bool {
        // A newer one sends this one off, and takes its place.
        if self.out.is_none() {
            self.leave(stage.now);
        }
        false
    }
}

/// What the banner says `who` did.
fn says(who: &Who, rec: Option<&Who>) -> String {
    match rec {
        None => "unlocked an achievement for everyone".into(),
        Some(rec) if crate::ink::person(&rec.key) == crate::ink::person(&who.key) => {
            "unlocked an achievement".into()
        }
        Some(rec) => format!(
            "awarded {} an achievement",
            if rec.mine { "you" } else { &rec.name }
        ),
    }
}

/// The glint `k` of the way across a pill `width` wide, as a band leaning into its sweep
/// and fading at both edges, cut to the pill's shape.
fn glint(canvas: &mut Canvas, width: f32, k: f32, alpha: f32, at: impl Fn([f32; 2]) -> [f32; 2]) {
    let r = HIGH / 2.;
    let band = width * 0.34;
    let left = lerp(-0.45, 1.25, k) * width;
    let lean = -(-22_f32).to_radians().tan();
    // The pill's sides, at a height.
    let side = |y: f32| r - (r * r - (y - r).powi(2)).max(0.).sqrt();
    let rows = 12;
    let bands = 16;
    for row in 0..rows {
        let ys = [row, row + 1].map(|row| row as f32 * HIGH / rows as f32);
        for b in 0..bands {
            let (u0, u1) = (b as f32 / bands as f32, (b + 1) as f32 / bands as f32);
            let a = 0.65 * (1. - ((u0 + u1) - 1.).abs()) * alpha;
            let [top, bottom] = ys.map(|y| {
                let shift = lean * (y - r);
                let (from, to) = (side(y), width - side(y));
                let x0 = (left + u0 * band + shift).clamp(from, to);
                let x1 = (left + u1 * band + shift).clamp(from, to);
                ([x0, y], [x1, y])
            });
            if top.0[0] < top.1[0] || bottom.0[0] < bottom.1[0] {
                canvas.polygon(
                    [top.0, top.1, bottom.1, bottom.0].map(&at),
                    rgb(0xffecaa),
                    a,
                );
            }
        }
    }
}

/// "ACHIEVEMENT UNLOCKED · NAME" over what it was for.
fn body(name: &str, text: &str) -> Part {
    let head = "ACHIEVEMENT UNLOCKED ";
    let tail = format!("· {}", name.to_uppercase());
    let kicker = advance(&format!("{head}{tail}"), 11.5, 750)
        + (head.chars().count() + tail.chars().count()) as f32 * 11.5 * 0.09;
    let title = advance(text, 19., 750) - text.chars().count() as f32 * 0.19;
    Part::new(
        format!(
            r##"<g font-family="{FONT}" font-weight="750"><text x="6" y="27.2" font-size="11.5" letter-spacing="1.035" fill="#ffd34d">{}<tspan fill="#fff" fill-opacity=".55">{}</tspan></text><text x="6" y="51.4" font-size="19" letter-spacing="-.19" fill="#fff">{}</text></g>"##,
            looks::escape(head),
            looks::escape(&tail),
            looks::escape(text),
        ),
        6. + kicker.max(title) + 22.,
    )
}

/// "+100 G", and whoever it's for: them in a ring of gold, or everyone stacked up.
fn end(rec: Option<&Who>, people: &[Who]) -> Part {
    let chip = [advance("+100 G", 15., 800) + 24., 15. * 1.21 + 12.];
    let mut svg = format!(
        r##"<rect x=".5" y="{y}" width="{w}" height="{h}" rx="{r}" fill="#ffd34d" fill-opacity=".14" stroke="#ffd34d" stroke-opacity=".4"/><text x="{x}" y="{ty}" text-anchor="middle" font-family="{FONT}" font-size="15" font-weight="800" fill="#fff3c4">+100 G</text>"##,
        y = HIGH / 2. - chip[1] / 2. + 0.5,
        w = chip[0] - 1.,
        h = chip[1] - 1.,
        r = (chip[1] - 1.) / 2.,
        x = chip[0] / 2.,
        ty = HIGH / 2. + 15. * 0.36,
    );
    let mut x = chip[0] + 12.;
    let mid = HIGH / 2.;
    match rec {
        Some(rec) => {
            svg += &format!(
                r##"<defs><filter id="g" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="7"/></filter></defs><circle cx="{cx}" cy="{mid}" r="23" fill="#ffd34d" fill-opacity=".5" filter="url(#g)"/><circle cx="{cx}" cy="{mid}" r="23" fill="#ffd34d"/>{}"##,
                avatar_svg(rec, x, mid - 21., 42., "a"),
                cx = x + 21.,
            );
            x += 42.;
        }
        None => {
            x += 10.;
            for (i, person) in people.iter().enumerate() {
                if i > 0 {
                    x -= 10.;
                }
                svg += &format!(
                    r##"<circle cx="{cx}" cy="{mid}" r="19" fill="#1d1d22"/>{}"##,
                    avatar_svg(person, x, mid - 17., 34., &format!("a{i}")),
                    cx = x + 17.,
                );
                x += 34.;
            }
        }
    }
    Part::new(svg, x + 16.)
}

/// The pill's left end, a slice of its middle and its right end: dark, ringed in gold and
/// lit along its top edge.
fn pill() -> [Arc<Look>; 3] {
    let svg = r##"<defs><linearGradient id="b" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#34343c"/><stop offset=".7" stop-color="#141418"/></linearGradient><clipPath id="c"><rect x="2" y="2" width="73" height="72" rx="36"/></clipPath></defs><rect x=".5" y=".5" width="76" height="75" rx="37.5" fill="#ffd34d" fill-opacity=".55"/><rect x="2" y="2" width="73" height="72" rx="36" fill="url(#b)"/><path clip-path="url(#c)" fill="#fff" fill-opacity=".12" fill-rule="evenodd" d="M-10 -10H90V90H-10Z M38 3h1a36 36 0 0 1 0 72h-1a36 36 0 0 1 0-72Z"/>"##;
    [("left", 0., 38.), ("middle", 38., 1.), ("right", 39., 38.)].map(|(name, x, w)| {
        Look::new(
            format!("p8-pill:{name}"),
            [w, 76.],
            format!(r#"<g transform="translate({} 0)">{svg}</g>"#, -x),
        )
    })
}

/// The gold circle's glow, at its dimmest or brightest, inside a pill `reach` wide.
fn icon_glow(bright: bool, reach: f32) -> Arc<Look> {
    let (r, blur, a) = if bright {
        (36., 15., 0.9)
    } else {
        (29., 6., 0.55)
    };
    Look::new(
        format!("p8-glow:{bright}:{reach}"),
        [GLOW, HIGH],
        format!(
            r##"<defs><clipPath id="c"><rect width="{reach}" height="{HIGH}" rx="36"/></clipPath><filter id="b" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="{blur}"/></filter></defs><g clip-path="url(#c)"><circle cx="36" cy="36" r="{r}" fill="#ffc83c" fill-opacity="{a}" filter="url(#b)"/></g>"##
        ),
    )
}

/// The gold circle the trophy sits on, shaded at its foot.
fn coin() -> Arc<Look> {
    Look::new(
        "p8-coin",
        [HIGH, HIGH],
        r##"<defs><radialGradient id="g" cx="36" cy="27.9" r="44.3" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff6cf"/><stop offset=".38" stop-color="#ffd34d"/><stop offset=".78" stop-color="#e08a00"/><stop offset="1" stop-color="#7a4200"/></radialGradient><clipPath id="c"><circle cx="36" cy="36" r="27"/></clipPath><filter id="b" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="4"/></filter></defs><circle cx="36" cy="36" r="27" fill="url(#g)"/><g clip-path="url(#c)"><path fill="#783c00" fill-opacity=".55" fill-rule="evenodd" filter="url(#b)" d="M-20 -20H92V92H-20Z M36 5a27 27 0 1 0 .01 0Z"/></g>"##,
    )
}

/// The trophy, turned `turn` degrees (in steps of two) about its middle.
fn trophy(turn: f32) -> Arc<Look> {
    let turn = (turn / 2.).round() * 2.;
    let side = TROPHY * 1.5;
    let c = side / 2.;
    Look::new(
        format!("p8-trophy:{turn}"),
        [side, side],
        format!(
            r#"<g transform="rotate({turn} {c} {c})"><text x="{c}" y="{y}" text-anchor="middle" font-family="{EMOJI}" font-size="{TROPHY}">🏆</text></g>"#,
            y = c + TROPHY * 0.36,
        ),
    )
}

/// The width `text` takes up in a line, from its start to the next one's.
fn advance(text: &str, size: f32, weight: u16) -> f32 {
    looks::text_width(&format!("|{text}|"), size, weight, FONT)
        - looks::text_width("||", size, weight, FONT)
}

/// The chime: a rising "bloop", then a bright bell-like "bling" with a sparkly shimmer
/// on top.
fn chime(graph: &mut Graph) {
    let bus = Some(graph.dest(0.45));
    graph.tone(Tone {
        freq: 392.,
        glide_to: Some(784.),
        dur: 0.17,
        gain: 0.34,
        attack: 0.01,
        to: bus,
        ..Tone::default()
    });
    graph.tone(Tone {
        freq: 784.,
        glide_to: Some(1568.),
        dur: 0.13,
        wave: Wave::Triangle,
        gain: 0.07,
        attack: 0.01,
        to: bus,
        ..Tone::default()
    });
    let t = 0.16;
    for (i, (freq, gain)) in [(1046.5, 0.2), (1568., 0.15), (2093., 0.1), (2637., 0.07)]
        .into_iter()
        .enumerate()
    {
        let when = t + i as f32 * 0.012;
        graph.tone(Tone {
            when,
            freq,
            gain,
            dur: 1.7,
            attack: 0.004,
            to: bus,
            ..Tone::default()
        });
        graph.tone(Tone {
            when,
            freq,
            wave: Wave::Triangle,
            detune: 6.,
            gain: gain * 0.35,
            dur: 0.9,
            to: bus,
            ..Tone::default()
        });
        // The bell's partial.
        graph.tone(Tone {
            when,
            freq: freq * 2.76,
            gain: gain * 0.18,
            dur: 0.35,
            to: bus,
            ..Tone::default()
        });
    }
    let sparkle = [3136., 3520., 3951., 4186., 4699., 5274.];
    for i in 0..12 {
        graph.tone(Tone {
            when: t + 0.06 + i as f32 * 0.04,
            freq: sparkle[(i * 5) % 6],
            dur: 0.4,
            gain: 0.025,
            to: bus,
            ..Tone::default()
        });
    }
    graph.noise(Noise {
        when: t,
        dur: 1.1,
        gain: 0.035,
        filter: Filter::Highpass,
        freq: 8000.,
        sweep_to: Some(12000.),
        attack: 0.03,
        to: bus,
        ..Noise::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Reaction;

    #[test]
    fn the_banner_says_who_it_was_for() {
        let me = Who::new("room:r:u-sam", "Sam Rivera", [1, 2, 3], true);
        let alex = Who::new("room:r:u-alex", "Alex Morgan", [4, 5, 6], false);
        let audio = Who::new("room:r:u-alex:audio", "Alex Morgan", [4, 5, 6], false);
        assert_eq!(says(&alex, None), "unlocked an achievement for everyone");
        assert_eq!(says(&alex, Some(&audio)), "unlocked an achievement");
        assert_eq!(says(&alex, Some(&me)), "awarded you an achievement");
        assert_eq!(says(&me, Some(&alex)), "awarded Alex an achievement");
    }

    #[test]
    fn a_newer_one_sends_it_off() {
        let mut stage = Stage::new(1440., 900., 2);
        let alex = Who::new("room:r:u-alex", "Alex Morgan", [4, 5, 6], false);
        let mut toast = Achievement::new(&mut stage, &alex, "", None);
        let step = |stage: &mut Stage, toast: &mut Achievement, until: f32| {
            let mut alive = true;
            while stage.now < until && alive {
                let before = stage.now;
                stage.now += 1. / 60.;
                alive = toast.step(
                    stage,
                    Clock {
                        before,
                        now: stage.now,
                    },
                );
            }
            alive
        };
        assert!(step(&mut stage, &mut toast, 1.));
        assert!(!toast.again(&mut stage, &alex, &Reaction::new("achievement")));
        assert!(!step(&mut stage, &mut toast, 1.6), "it stayed");
    }
}
