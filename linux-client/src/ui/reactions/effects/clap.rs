//! 6. Slow clap: every clap reaction is one clap, from whoever sends it. Claps from anyone
//! fill a meter over the call window as the room joins in; eight of them erupt into a
//! standing ovation (crowd roar, spotlights, roses, BRAVO), and two seconds without one
//! let it fizzle out.

use std::sync::{Arc, OnceLock};

use super::Effect;
use crate::ui::reactions::{
    looks::{self, EMOJI, Look},
    particles::{Emit, Particles, Shape},
    scene::{Canvas, place},
    stage::{
        Clock, Rng, Stage, Who, bezier, clamp01, ease, ease_in_out, keyframes, lerp, mix, rgb,
    },
    synth::{self, Filter, Graph, Id, Noise, Response, Tone, Wave},
};

/// Claps to the ovation.
const PEAK: u32 = 8;
/// Seconds without a clap before it fizzles.
const FIZZLE: f32 = 2.;
const OVATION: f32 = 6.5;
/// The meter's place: a strip this high just above the call window (or below it, when
/// there's no room above), with the meter in it.
const METER_GAP: f32 = 6.;
const METER_HEIGHT: f32 = 32.;
const METER: [f32; 2] = [140., 18.];
/// Steps the meter's fill is drawn in.
const FILL_STEPS: f32 = 70.;
/// The rest of the room, joining in a fraction late.
const CROWD: [&str; 3] = ["crowd-b", "crowd-c", "crowd-a"];
/// Each person's hands sound a little different, and sit in a different place in the
/// stereo field: the band they ring at, its Q and their pan.
const VOICES: [(f32, f32, f32); 3] = [(1650., 1.2, 0.), (1250., 1., -0.55), (2250., 1.4, 0.55)];
/// Hands are drawn this big and scaled down.
const HAND: f32 = 64.;
const BRAVO_FONT: &str = "Didot, 'Bodoni 72', 'Bodoni MT', 'Playfair Display', Georgia, 'Times New Roman', 'DejaVu Serif', serif";
/// BRAVO's letters are drawn this much larger than they show.
const BIG: f32 = 1.3;

fn spring(t: f32) -> f32 {
    bezier(0.2, 1.5, 0.4, 1., t)
}

/// The meter's slow drain.
fn slow(t: f32) -> f32 {
    bezier(0.45, 0., 0.2, 1., t)
}

/// Where the meter goes for a call window at `call`: centred over it, just above it, or
/// just below it when there's no room above.
fn meter_at([x, y, w, h]: [f32; 4]) -> [f32; 2] {
    let above = y - METER_HEIGHT - METER_GAP;
    let top = if above >= 0. {
        above
    } else {
        y + h + METER_GAP
    };
    [x + w / 2. - METER[0] / 2., top + 7.]
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
        Self {
            from: value,
            to: value,
            start: 0.,
            seconds: 0.,
            curve: ease,
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

    fn go(&mut self, now: f32, to: f32, seconds: f32, curve: fn(f32) -> f32) {
        if to == self.to {
            return;
        }
        *self = Self {
            from: self.at(now),
            to,
            start: now,
            seconds,
            curve,
        };
    }

    fn moving(&self, now: f32) -> bool {
        now < self.start + self.seconds
    }
}

struct Meter {
    fill: Glide,
    heat: f32,
    live: bool,
    fizz: bool,
    label: &'static str,
    shown: Glide,
}

/// A slow clap building.
struct Session {
    n: u32,
    leader: Who,
    join_at: u32,
    last: Option<f32>,
    gap: f32,
    erupted: bool,
}

/// A pair of clapping hands rising off the call window.
struct Hands {
    at: [f32; 2],
    big: f32,
    from: f32,
}

enum Later {
    Hands(f32),
    Ovation,
    FizzEnd,
    Roses,
    Throw,
    WindDown,
}

/// The standing ovation's set pieces.
struct Ovation {
    from: f32,
    /// Each spotlight's swing: seconds each way.
    swings: [f32; 4],
    /// BRAVO!'s letters: each, where its middle is from the word's, and the turn it comes
    /// in from.
    letters: Vec<(char, f32, f32)>,
    size: f32,
    glow: Arc<Look>,
}

pub struct Clap {
    session: Option<Session>,
    meter: Meter,
    dim: Glide,
    fizzle_at: Option<f32>,
    later: Vec<(f32, Later)>,
    hands: Vec<Hands>,
    ovation: Option<Ovation>,
    /// The ovation's roses and confetti, here rather than on the stage so they show over
    /// its darkness, as the prototype's do.
    particles: Particles,
}

impl Clap {
    pub fn new(stage: &mut Stage, who: &Who) -> Self {
        let mut clap = Self {
            session: None,
            meter: Meter {
                fill: Glide::still(0.),
                heat: 0.,
                live: false,
                fizz: false,
                label: "slow clap",
                shown: Glide::still(0.),
            },
            dim: Glide::still(0.),
            fizzle_at: None,
            later: Vec::new(),
            hands: Vec::new(),
            ovation: None,
            particles: Particles::default(),
        };
        clap.clap(stage, who);
        clap
    }

    /// One clap from `who`.
    fn clap(&mut self, stage: &mut Stage, who: &Who) {
        let now = stage.now;
        if self.session.as_ref().is_some_and(|session| session.erupted) {
            // Clapping along to the ovation.
            let graph = stage.sound();
            let mut room = Room::new(graph);
            clap_sound(graph, &mut room, &who.key, 0., 0.8, 1.);
            self.hands(stage, now, 1.);
            return;
        }
        let mut session = match self.session.take() {
            Some(session) => session,
            None => {
                stage.banner(who, "started a slow clap", "👏", 2600.);
                Session {
                    n: 0,
                    leader: who.clone(),
                    join_at: *stage.rng.pick(&[2, 3]),
                    last: None,
                    gap: 0.9,
                    erupted: false,
                }
            }
        };
        if let Some(last) = session.last {
            session.gap = (now - last).clamp(0.14, 1.4);
        }
        session.last = Some(now);
        session.n += 1;
        let heat = session.n as f32 / PEAK as f32;
        let graph = stage.sound();
        let mut room = Room::new(graph);
        clap_sound(
            graph,
            &mut room,
            &who.key,
            0.,
            0.75 + heat * 0.35,
            1. + heat * 0.06,
        );
        // The others join in, a fraction late and not quite together. Once it gets going
        // they double up.
        if session.n >= session.join_at {
            for (i, key) in CROWD[..2].iter().enumerate() {
                let lag = graph.rng.range(0.04, 0.11) + i as f32 * graph.rng.range(0.03, 0.07);
                clap_sound(graph, &mut room, key, lag, 0.6 + heat * 0.3, 1.);
                self.later.push((now + lag, Later::Hands(0.9 + heat * 0.3)));
                if session.n >= 5 && session.gap > 0.26 {
                    clap_sound(graph, &mut room, key, lag + session.gap / 2., 0.45, 1.);
                }
            }
        }
        if session.n >= 4 {
            for _ in 0..(session.n - 3) * 3 {
                let when = graph.rng.range(0., 0.3);
                crowd_clap(graph, &room, when, 0.05 + heat * 0.08);
            }
        }
        self.hands(stage, now, 1. + heat * 0.35);
        let left = PEAK.saturating_sub(session.n);
        let label = if left == 0 {
            "BRAVO!"
        } else if session.n < session.join_at {
            "keep going…"
        } else if left <= 2 {
            "almost there…"
        } else {
            "everyone's in"
        };
        self.set_meter(now, session.n, label, 0.45, spring);
        self.fizzle_at = None;
        if session.n >= PEAK {
            session.erupted = true;
            self.later.push((now + 0.12, Later::Ovation));
        } else {
            self.fizzle_at = Some(now + FIZZLE);
        }
        self.session = Some(session);
    }

    fn hands(&mut self, stage: &mut Stage, from: f32, big: f32) {
        let [x, y] = stage.tile();
        let x = x + stage.rand(-30., 30.);
        self.hands.push(Hands {
            at: [x, y - 6.],
            big,
            from,
        });
    }

    fn set_meter(
        &mut self,
        now: f32,
        n: u32,
        label: &'static str,
        seconds: f32,
        curve: fn(f32) -> f32,
    ) {
        let k = clamp01(n as f32 / PEAK as f32);
        let meter = &mut self.meter;
        meter.fill.go(now, k, seconds, curve);
        meter.live = n > 0;
        meter.heat = k;
        meter.label = label;
        self.show_meter(now);
        self.dim.go(now, k * k * 0.55, 0.6, ease);
    }

    /// The meter shows while a clap is going, dimmer while it fizzles.
    fn show_meter(&mut self, now: f32) {
        let meter = &mut self.meter;
        let shown = if meter.live {
            1.
        } else if meter.fizz {
            0.55
        } else {
            0.
        };
        meter.shown.go(now, shown, 0.3, ease);
    }

    fn end_session(&mut self, now: f32, fade: f32) {
        self.session = None;
        self.dim.go(now, 0., fade, ease);
    }

    fn fizzle(&mut self, stage: &mut Stage, now: f32) {
        let Some(session) = &self.session else {
            return;
        };
        // One last hopeful clap each, then it trails away.
        let joined = session.n >= session.join_at;
        let graph = stage.sound();
        let mut room = Room::new(graph);
        for (i, key) in CROWD[..2].iter().enumerate() {
            if joined || i == 0 {
                let lag = 0.15 + i as f32 * 0.52;
                clap_sound(graph, &mut room, key, lag, 0.35, 1.);
                self.later.push((now + lag, Later::Hands(0.8)));
            }
        }
        self.meter.fizz = true;
        self.set_meter(now, 0, "…it fizzled", 1.4, slow);
        self.later.push((now + 1.8, Later::FizzEnd));
        self.end_session(now, 1.4);
    }

    fn erupt(&mut self, stage: &mut Stage, now: f32) {
        let leader = match &mut self.session {
            Some(session) => {
                session.erupted = true;
                session.leader.clone()
            }
            None => return,
        };
        stage.banner(&leader, "got a standing ovation going", "🌹", 3600.);
        self.set_meter(now, PEAK, "BRAVO!", 0.45, spring);
        // Lights down, spotlights up.
        self.dim.go(now, 0., 0.2, ease);
        ovation_sound(stage.sound());
        let swings = [0; 4].map(|_| stage.rand(1.4, 1.9));
        let size = (stage.w * 0.19).min(240.);
        let font = size * BIG;
        let mut letters = Vec::new();
        let advances: Vec<f32> = "BRAVO!"
            .chars()
            .map(|ch| advance(&ch.to_string(), font, 700, BRAVO_FONT) / BIG)
            .collect();
        let mut left = -advances.iter().sum::<f32>() / 2.;
        for (ch, advance) in "BRAVO!".chars().zip(advances) {
            letters.push((ch, left + advance / 2., stage.rand(-30., 30.)));
            left += advance;
        }
        self.ovation = Some(Ovation {
            from: now,
            swings,
            letters,
            size,
            glow: bravo_glow(size),
        });
        stage.flash(rgb(0xffe2a8), 500., 0.5);
        stage.shake(9., 600.);
        // Roses and confetti rain down; roses get thrown up from the front row too.
        for i in 0..34 {
            self.later
                .push((now + 0.15 + i as f32 * 0.11, Later::Roses));
        }
        for at in [0.4, 0.9, 1.5] {
            self.later.push((now + at, Later::Throw));
        }
        // The crowd stands and claps.
        for i in 0..18 {
            self.later.push((now + i as f32 * 0.21, Later::Hands(1.2)));
        }
        self.later.push((now + OVATION - 0.6, Later::WindDown));
    }

    fn roses(&mut self, stage: &mut Stage) {
        let x = stage.rand(0., stage.w);
        self.particles.emit(
            &Emit {
                x,
                y: -40.,
                count: 2,
                angle: 90.,
                spread: 40.,
                speed: [2., 6.],
                gravity: 0.16,
                drag: 0.99,
                life: [200., 260.],
                size: [30., 46.],
                shape: Shape::Emoji,
                emoji: &["🌹"],
                wobble: false,
                ..Emit::default()
            },
            &mut stage.rng,
        );
        let x = stage.rand(0., stage.w);
        let shape = *stage.rng.pick(&[Shape::Rect, Shape::Tri]);
        self.particles.emit(
            &Emit {
                x,
                y: -20.,
                count: 9,
                angle: 90.,
                spread: 60.,
                speed: [1., 5.],
                gravity: 0.12,
                drag: 0.98,
                life: [200., 260.],
                size: [7., 12.],
                shape,
                ..Emit::default()
            },
            &mut stage.rng,
        );
    }

    fn throw(&mut self, stage: &mut Stage) {
        let x = stage.w * stage.rand(0.3, 0.7);
        self.particles.emit(
            &Emit {
                x,
                y: stage.h + 20.,
                count: 6,
                angle: -90.,
                spread: 50.,
                speed: [15., 22.],
                gravity: 0.32,
                life: [90., 130.],
                size: [34., 46.],
                shape: Shape::Emoji,
                emoji: &["🌹"],
                ..Emit::default()
            },
            &mut stage.rng,
        );
    }

    fn busy(&self, now: f32) -> bool {
        self.session.is_some()
            || !self.later.is_empty()
            || !self.hands.is_empty()
            || !self.particles.is_empty()
            || self
                .ovation
                .as_ref()
                .is_some_and(|ovation| now < ovation.from + OVATION)
            || self.meter.shown.at(now) > 0.004
            || self.meter.shown.moving(now)
            || self.dim.at(now) > 0.004
            || self.dim.moving(now)
    }
}

impl Effect for Clap {
    fn step(&mut self, stage: &mut Stage, _: Clock) -> bool {
        let now = stage.now;
        for _ in 0..stage.ticks {
            self.particles.tick(stage.h);
        }
        if self.fizzle_at.is_some_and(|at| at <= now) {
            self.fizzle_at = None;
            self.fizzle(stage, now);
        }
        let mut due = Vec::new();
        let mut i = 0;
        while i < self.later.len() {
            if self.later[i].0 <= now {
                due.push(self.later.remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (at, later) in due {
            match later {
                Later::Hands(big) => self.hands(stage, at, big),
                Later::Ovation => self.erupt(stage, now),
                Later::FizzEnd => {
                    self.meter.fizz = false;
                    if !self.meter.live {
                        self.meter.label = "slow clap";
                    }
                    self.show_meter(now);
                }
                Later::Roses => self.roses(stage),
                Later::Throw => self.throw(stage),
                Later::WindDown => {
                    self.set_meter(now, 0, "slow clap", 1.2, slow);
                    self.end_session(now, 0.3);
                }
            }
        }
        self.hands.retain(|hands| now - hands.from < 1.);
        if self
            .ovation
            .as_ref()
            .is_some_and(|ovation| now >= ovation.from + OVATION)
        {
            self.ovation = None;
        }
        self.busy(now)
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let now = stage.now;
        let (w, h) = (stage.w, stage.h);
        canvas.picture_in(
            &dim_look(),
            [-40., -40.],
            [w + 80., h + 80.],
            self.dim.at(now),
        );
        if let Some(ovation) = &self.ovation {
            ovation.draw_lights(stage, canvas, now - ovation.from);
        }
        self.draw_meter(stage, canvas, now);
        self.particles.draw(canvas);
        for hands in &self.hands {
            hands.draw(canvas, now - hands.from);
        }
        if let Some(ovation) = &self.ovation {
            ovation.draw_bravo(stage, canvas, now - ovation.from);
        }
    }

    fn again(&mut self, stage: &mut Stage, who: &Who, _: &crate::protocol::Reaction) -> bool {
        self.clap(stage, who);
        true
    }
}

impl Clap {
    fn draw_meter(&self, stage: &Stage, canvas: &mut Canvas, now: f32) {
        let meter = &self.meter;
        let alpha = meter.shown.at(now);
        if alpha <= 0.004 {
            return;
        }
        let [x, y] = meter_at(stage.call);
        let frame = meter_frame(meter.live, (meter.heat * 8.).round() as u32);
        let [fw, fh] = frame.size;
        canvas.picture_in(
            &frame,
            [x - (fw - METER[0]) / 2., y - (fh - METER[1]) / 2.],
            [fw, fh],
            alpha,
        );
        let steps = (clamp01(meter.fill.at(now)) * FILL_STEPS).round() as u32;
        if steps > 0 {
            canvas.picture_in(&meter_fill(steps, meter.fizz), [x, y], METER, alpha);
        }
        canvas.picture_in(&meter_label(meter.label), [x, y], METER, alpha);
    }
}

impl Hands {
    fn draw(&self, canvas: &mut Canvas, t: f32) {
        let k = clamp01(t / 0.95);
        let e = |t| bezier(0.2, 0.9, 0.3, 1., t);
        let rise = keyframes(&[(0., 0.5), (0.25, 0.9), (0.6, 1.1), (1., 1.9)], k, e);
        let scale = keyframes(&[(0., 0.3), (0.25, 1.1), (0.6, 1.), (1., 0.85)], k, e);
        let alpha = keyframes(&[(0., 0.), (0.25, 1.), (0.6, 1.), (1., 0.)], k, e);
        let font = 38. * self.big;
        let [x, y] = self.at;
        let centre = [x, y + (0.5 - rise) * font];
        // The hands swing in and meet, then part a little.
        let k = clamp01(t / 0.52);
        let e = |t| bezier(0.5, 0., 0.3, 1., t);
        let shift = keyframes(&[(0., -22.), (0.3, -3.), (1., -9.)], k, e);
        let turn = keyframes(&[(0., -22.), (0.3, 0.), (1., -8.)], k, e);
        let s = scale * font / HAND;
        for (ch, side, turn) in [("🫱", -1., turn), ("🫲", 1., -turn)] {
            let at = [centre[0] + scale * side * (font / 2. - shift), centre[1]];
            canvas.picture(&hand_look(ch, turn), at, s, alpha);
        }
        // The pow where they meet: spokes of light in a ring.
        let grow = keyframes(&[(0., 0.2), (0.28, 0.3), (0.4, 1.2), (1., 1.6)], k, |t| t);
        let shine = keyframes(&[(0., 0.), (0.28, 0.), (0.4, 1.), (1., 0.)], k, |t| t) * alpha;
        let middle = [centre[0], centre[1] - 0.05 * font * scale];
        let (inner, outer) = (0.349 * font * grow * scale, 0.57 * font * grow * scale);
        for spoke in 0..8 {
            let a = (spoke as f32 * 45.).to_radians();
            let b = (spoke as f32 * 45. + 6.).to_radians();
            let at = |r: f32, a: f32| [middle[0] + a.sin() * r, middle[1] - a.cos() * r];
            canvas.polygon(
                [at(inner, a), at(outer, a), at(outer, b), at(inner, b)],
                rgb(0xfff6c8),
                shine,
            );
        }
    }
}

impl Ovation {
    /// The dark, and four spotlights swinging over the crowd.
    fn draw_lights(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let (w, h) = (stage.w, stage.h);
        let k = t / OVATION;
        let dark = keyframes(&[(0., 0.), (0.06, 1.), (0.82, 1.), (1., 0.)], k, |t| t);
        canvas.picture_in(&dark_look(), [-40., -40.], [w + 80., h + 80.], dark);
        let shown = keyframes(&[(0., 0.), (0.08, 1.), (0.8, 1.), (1., 0.)], k, |t| t);
        for (i, x) in [0.12, 0.37, 0.63, 0.88].into_iter().enumerate() {
            let dir = if i % 2 == 1 { 1. } else { -1. };
            let swing = self.swings[i];
            let p = (t - i as f32 * 0.09) / swing;
            let turn = if p < 0. {
                0.
            } else {
                let f = if (p as u32) % 2 == 1 {
                    1. - p.fract()
                } else {
                    p.fract()
                };
                lerp(dir * 32., -dir * 26., bezier(0.45, 0., 0.55, 1., f))
            };
            beam(canvas, [x * w, -0.06 * h], 0.46 * w, 1.4 * h, turn, shown);
        }
    }

    /// BRAVO!, letter by letter, glowing.
    fn draw_bravo(&self, stage: &Stage, canvas: &mut Canvas, t: f32) {
        let k = t / (OVATION - 0.2);
        if k >= 1. {
            return;
        }
        let scale = keyframes(&[(0., 1.), (0.8, 1.06), (1., 1.1)], k, ease_in_out);
        let alpha = keyframes(&[(0., 1.), (0.8, 1.), (1., 0.)], k, ease_in_out);
        let lift = keyframes(&[(0., 0.), (0.8, 0.), (1., -0.12)], k, ease_in_out) * self.size;
        let centre = [stage.w / 2., stage.h * 0.44 + lift];
        let mut letters = Vec::new();
        let mut shown = 0.;
        for (i, &(ch, x, from)) in self.letters.iter().enumerate() {
            let k = (t - (0.18 + i as f32 * 0.085)) / 0.7;
            if k < 0. {
                continue;
            }
            let e = spring(k.min(1.));
            let grow = lerp(0.2, 1., e);
            let drop = lerp(90., 0., e);
            let turn = lerp(from, 0., e);
            let opacity = clamp01(e);
            shown += opacity / self.letters.len() as f32;
            letters.push((
                bravo_letter(ch, self.size, turn),
                [centre[0] + x * scale, centre[1] + drop * scale],
                scale * grow / BIG,
                alpha * opacity,
            ));
        }
        canvas.picture(&self.glow, centre, scale, alpha * shown);
        for (look, at, s, a) in letters {
            canvas.picture(&look, at, s, a);
        }
    }
}

/// A spotlight from `top`, `width` across at its foot and `length` long, turned `turn`
/// degrees about its top: brightest there, fading to nothing.
fn beam(canvas: &mut Canvas, top: [f32; 2], width: f32, length: f32, turn: f32, alpha: f32) {
    let turn = turn.to_radians();
    let half = |s: f32| lerp(0.03, 0.5, s) * width;
    let bands = 24;
    for band in 0..bands {
        let (a, b) = (band as f32 / bands as f32, (band + 1) as f32 / bands as f32);
        let mid = (a + b) / 2.;
        let (colour, opacity) = if mid < 0.55 {
            let k = mid / 0.55;
            (mix(rgb(0xfff4d6), rgb(0xffe2a0), k), lerp(0.75, 0.28, k))
        } else {
            (rgb(0xffe2a0), lerp(0.28, 0., (mid - 0.55) / 0.45))
        };
        let points = [
            [-half(a), a * length],
            [half(a), a * length],
            [half(b), b * length],
            [-half(b), b * length],
        ]
        .map(|p| place(p, top, turn, 1.));
        canvas.polygon(points, colour, opacity * alpha);
    }
}

/// Stretched over the screen while the clap builds.
fn dim_look() -> Arc<Look> {
    Look::new(
        "p6-dim",
        [64., 40.],
        r##"<defs><radialGradient id="d" cx=".5" cy=".4" r="1" gradientTransform="matrix(1.1 0 0 .9 -.05 .04)"><stop offset="0" stop-color="#0a0618" stop-opacity=".35"/><stop offset="1" stop-color="#06030e" stop-opacity=".85"/></radialGradient></defs><rect width="64" height="40" fill="url(#d)"/>"##,
    )
}

/// Stretched over the screen for the ovation.
fn dark_look() -> Arc<Look> {
    Look::new(
        "p6-dark",
        [64., 40.],
        r##"<defs><radialGradient id="d" cx=".5" cy=".3" r="1" gradientTransform="matrix(1.2 0 0 1 -.1 0)"><stop offset="0" stop-color="#10061e" stop-opacity=".55"/><stop offset="1" stop-color="#04020a" stop-opacity=".9"/></radialGradient></defs><rect width="64" height="40" fill="url(#d)"/>"##,
    )
}

/// The meter's track: its glow (as hot as the clap, `heat` eighths) round the outside.
fn meter_frame(live: bool, heat: u32) -> Arc<Look> {
    let m = 30.;
    let [w, h] = METER;
    let k = heat as f32 / 8.;
    let glow = if live && heat > 0 {
        format!(
            r##"<defs><filter id="g" x="-100%" y="-300%" width="300%" height="700%"><feGaussianBlur stdDeviation="{blur}"/></filter><mask id="o"><rect width="{fw}" height="{fh}" fill="#fff"/><rect x="{m}" y="{m}" width="{w}" height="{h}" rx="9" fill="#000"/></mask></defs><g mask="url(#o)"><rect x="{m}" y="{m}" width="{w}" height="{h}" rx="9" fill="#ffbe5a" fill-opacity="{a}" filter="url(#g)"/></g>"##,
            blur = k * 11.,
            a = k * 0.9,
            fw = w + m * 2.,
            fh = h + m * 2.,
        )
    } else {
        String::new()
    };
    Look::new(
        format!("p6-meter:{live}:{heat}"),
        [w + m * 2., h + m * 2.],
        format!(
            r##"{glow}<rect x="{m}" y="{m}" width="{w}" height="{h}" rx="9" fill="#141020" fill-opacity=".55"/><rect x="{x}" y="{x}" width="{iw}" height="{ih}" rx="8.5" stroke="#fff" stroke-opacity="{ring}"/>"##,
            x = m + 0.5,
            iw = w - 1.,
            ih = h - 1.,
            ring = if live { 0.15 } else { 0.08 },
        ),
    )
}

/// The meter's fill, `steps` of [`FILL_STEPS`] full: warm, or grey as it fizzles.
fn meter_fill(steps: u32, grey: bool) -> Arc<Look> {
    let [w, h] = METER;
    let filled = w * steps as f32 / FILL_STEPS;
    let [a, b, c] = if grey {
        ["#949494", "#797979", "#525252"]
    } else {
        ["#ffd166", "#ff9f43", "#ff4d6d"]
    };
    Look::new(
        format!("p6-fill:{steps}:{grey}"),
        METER,
        format!(
            r##"<defs><linearGradient id="f" x1="0" y1="0" x2="{filled}" y2="0" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="{a}"/><stop offset=".5" stop-color="{b}"/><stop offset="1" stop-color="{c}"/></linearGradient><clipPath id="c"><rect width="{w}" height="{h}" rx="9"/></clipPath></defs><rect width="{filled}" height="{h}" fill="url(#f)" clip-path="url(#c)"/>"##
        ),
    )
}

fn meter_label(text: &str) -> Arc<Look> {
    let [w, h] = METER;
    Look::new(
        format!("p6-label:{text}"),
        METER,
        format!(
            r##"<defs><filter id="s" x="-20%" y="-50%" width="140%" height="200%"><feDropShadow dx="0" dy="1" stdDeviation="1" flood-color="#000" flood-opacity=".6"/></filter></defs><text x="{x}" y="{y}" text-anchor="middle" font-family="{font}" font-size="9.5" font-weight="800" letter-spacing=".57" fill="#fff" filter="url(#s)">{}</text>"##,
            looks::escape(&text.to_uppercase()),
            x = w / 2.,
            y = h / 2. + 9.5 * 0.36,
            font = looks::FONT,
        ),
    )
}

/// A hand, turned `turn` degrees (in steps of two) about its middle.
fn hand_look(ch: &str, turn: f32) -> Arc<Look> {
    let turn = (turn / 2.).round() * 2.;
    let side = HAND * 1.5;
    let c = side / 2.;
    Look::new(
        format!("p6-hand:{ch}:{turn}"),
        [side, side],
        format!(
            r#"<g transform="rotate({turn} {c} {c})"><text x="{c}" y="{y}" text-anchor="middle" font-family="{EMOJI}" font-size="{HAND}">{ch}</text></g>"#,
            y = c + HAND * 0.38,
        ),
    )
}

/// One of BRAVO!'s letters, gold with its two hard shadows, turned `turn` degrees (in
/// steps of three) about its middle.
fn bravo_letter(ch: char, size: f32, turn: f32) -> Arc<Look> {
    let turn = (turn / 3.).round() * 3.;
    let font = size * BIG;
    let side = font * 1.7;
    let c = side / 2.;
    let y = c + font * 0.36;
    let ch = looks::escape(&ch.to_string());
    let text = |dy: f32, fill: &str| {
        format!(
            r#"<text x="{c}" y="{}" text-anchor="middle" fill="{fill}">{ch}</text>"#,
            y + dy * BIG
        )
    };
    Look::new(
        format!("p6-bravo:{ch}:{font}:{turn}"),
        [side, side],
        format!(
            r#"<g transform="rotate({turn} {c} {c})" font-family="{BRAVO_FONT}" font-size="{font}" font-weight="700" font-style="italic" letter-spacing="{ls}">{}{}{}</g>"#,
            text(5., "#8a5a1a"),
            text(2., "#b9802d"),
            text(0., "#ffe6a3"),
            ls = font * 0.02,
        ),
    )
}

/// BRAVO!'s glow, behind its letters.
fn bravo_glow(size: f32) -> Arc<Look> {
    let width = looks::text_width("BRAVO!", size, 700, BRAVO_FONT) * 1.1;
    let pad = 160.;
    let (w, h) = (width + pad * 2., size + pad * 2.);
    let text = |colour: &str, opacity: f32, blur: f32, id: &str| {
        format!(
            r#"<filter id="{id}" x="-50%" y="-100%" width="200%" height="300%"><feGaussianBlur stdDeviation="{blur}"/></filter><text x="{x}" y="{y}" text-anchor="middle" font-family="{BRAVO_FONT}" font-size="{size}" font-weight="700" font-style="italic" fill="{colour}" fill-opacity="{opacity}" filter="url(#{id})">BRAVO!</text>"#,
            x = w / 2.,
            y = h / 2. + size * 0.36,
        )
    };
    Look::new(
        format!("p6-glow:{size}"),
        [w, h],
        format!(
            "{}{}",
            text("#ff8c3c", 0.5, 55., "a"),
            text("#ffc85a", 0.75, 20., "b")
        ),
    )
}

/// The width `text` takes up in a line, from its start to the next one's.
fn advance(text: &str, size: f32, weight: u16, family: &str) -> f32 {
    looks::text_width(&format!("|{text}|"), size, weight, family)
        - looks::text_width("||", size, weight, family)
}

// ---- Sounds ---------------------------------------------------------------------------

fn voice(key: &str) -> (f32, f32, f32) {
    let hash = key
        .encode_utf16()
        .fold(7u32, |h, c| h.wrapping_mul(31).wrapping_add(u32::from(c)));
    VOICES[hash as usize % VOICES.len()]
}

/// A small room the claps ring in (0.45 s), made once.
fn room_response() -> Response {
    static ROOM: OnceLock<Response> = OnceLock::new();
    ROOM.get_or_init(|| {
        let mut rng = Rng::new(0x5c1a9);
        let len = (synth::RATE as f32 * 0.45) as usize;
        let mut channel = || {
            (0..len)
                .map(|i| {
                    let onset = if i < 300 { i as f32 / 300. } else { 1. };
                    rng.range(-1., 1.) * (1. - i as f32 / len as f32).powi(4) * onset
                })
                .collect::<Vec<_>>()
        };
        Arc::new([channel(), channel()])
    })
    .clone()
}

/// The room in a sound: everything goes out through `out`, and each person's hands
/// through their own panned bus, part of it into the room.
struct Room {
    conv: Id,
    out: Id,
    buses: Vec<(String, Id)>,
}

impl Room {
    fn new(graph: &mut Graph) -> Self {
        let conv = graph.convolver(room_response());
        let wet = graph.gain(0.5);
        let out = graph.dest(0.06);
        graph.connect(conv, wet);
        graph.connect(wet, out);
        Self {
            conv,
            out,
            buses: Vec::new(),
        }
    }

    fn bus(&mut self, graph: &mut Graph, key: &str) -> Id {
        if let Some((_, id)) = self.buses.iter().find(|(k, _)| k == key) {
            return *id;
        }
        let g = graph.gain(1.);
        let pan = graph.panner(voice(key).2);
        let send = graph.gain(0.7);
        graph.connect(g, pan);
        graph.connect(pan, self.out);
        graph.connect(pan, send);
        graph.connect(send, self.conv);
        self.buses.push((key.to_owned(), g));
        g
    }
}

/// One crisp clap: a few micro-transients (palms never meet all at once), a bright body
/// and a short tail.
fn clap_sound(graph: &mut Graph, room: &mut Room, key: &str, when: f32, level: f32, bright: f32) {
    let (freq, q, _) = voice(key);
    let to = Some(room.bus(graph, key));
    let f = freq * bright * graph.rng.range(0.94, 1.06);
    for k in 0..3 {
        let when = when + k as f32 * graph.rng.range(0.006, 0.011);
        let freq = f * graph.rng.range(0.9, 1.15);
        graph.noise(Noise {
            when,
            dur: 0.014,
            gain: 0.5 * level * (1. - k as f32 * 0.2),
            filter: Filter::Bandpass,
            freq,
            q,
            attack: 0.0008,
            to,
            ..Noise::default()
        });
    }
    graph.noise(Noise {
        when: when + 0.02,
        dur: 0.11,
        gain: 0.42 * level,
        filter: Filter::Bandpass,
        freq: f,
        q: q * 0.8,
        attack: 0.001,
        to,
        ..Noise::default()
    });
    graph.noise(Noise {
        when: when + 0.02,
        dur: 0.05,
        gain: 0.18 * level,
        filter: Filter::Bandpass,
        freq: 850.,
        q: 2.,
        attack: 0.001,
        to,
        ..Noise::default()
    });
}

/// A clap from somebody further away: the crowd.
fn crowd_clap(graph: &mut Graph, room: &Room, when: f32, level: f32) {
    let dur = graph.rng.range(0.03, 0.08);
    let freq = graph.rng.range(1000., 2600.);
    let q = graph.rng.range(0.8, 1.6);
    graph.noise(Noise {
        when,
        dur,
        gain: level,
        filter: Filter::Bandpass,
        freq,
        q,
        attack: 0.001,
        to: Some(room.conv),
        ..Noise::default()
    });
}

/// The ovation: the crowd's roar and whoops, a few hundred claps swelling then thinning
/// out, the room clapping along and a thump to start it.
fn ovation_sound(graph: &mut Graph) {
    let mut room = Room::new(graph);
    let long = OVATION;
    for (i, (freq, q, level)) in [
        (420., 0.7, 0.32),
        (900., 0.6, 0.26),
        (2200., 0.5, 0.12),
        (160., 0.9, 0.18),
    ]
    .into_iter()
    .enumerate()
    {
        let src = graph.noise_source(true);
        let bp = graph.filter(Filter::Bandpass, freq, q);
        let g = graph.gain(1.);
        let rate = graph.rng.range(0.3, 0.9);
        let wob = graph.osc(Wave::Sine, rate);
        let depth = graph.gain(level * 0.25);
        graph.connect(wob, depth);
        graph.modulate(depth, g, Graph::gain_of);
        graph
            .gain_of(g)
            .set(0.0001, 0.)
            .exp(level, 0.35 + i as f32 * 0.1)
            .set(level, long * 0.55)
            .exp(0.0001, long);
        graph.connect(src, bp);
        graph.connect(bp, g);
        graph.connect(g, room.out);
        graph.play(src, 0., long + 0.1);
        graph.play(wob, 0., long + 0.1);
    }
    // "Whooo!" whistles from the crowd.
    for _ in 0..6 {
        let when = graph.rng.range(0.2, long * 0.55);
        let freq = graph.rng.range(1500., 2200.);
        let glide = freq * graph.rng.range(1.25, 1.5);
        let dur = graph.rng.range(0.35, 0.6);
        graph.tone(Tone {
            when,
            freq,
            glide_to: Some(glide),
            dur,
            gain: 0.05,
            attack: 0.08,
            to: Some(room.out),
            ..Tone::default()
        });
    }
    for _ in 0..4 {
        let when = graph.rng.range(0.3, long * 0.5);
        let freq = graph.rng.range(250., 340.);
        graph.tone(Tone {
            when,
            freq,
            glide_to: Some(freq * 0.75),
            wave: Wave::Saw,
            dur: 0.7,
            gain: 0.025,
            attack: 0.1,
            to: Some(room.conv),
            ..Tone::default()
        });
    }
    for _ in 0..420 {
        let when = long * 0.85 * graph.rng.unit().powf(1.4);
        let swell = (when / 0.4).min(1.) * (1. - when / long);
        let level = 0.06 + 0.22 * swell * graph.rng.unit();
        crowd_clap(graph, &room, when, level);
    }
    for key in CROWD {
        let mut when = 0.05;
        while when < long * 0.6 {
            clap_sound(graph, &mut room, key, when, 0.5, 1.);
            when += graph.rng.range(0.2, 0.28);
        }
    }
    graph.tone(Tone {
        freq: 70.,
        glide_to: Some(38.),
        dur: 0.6,
        gain: 0.5,
        to: Some(room.out),
        ..Tone::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Reaction;

    #[test]
    fn the_meter_sits_above_the_call_window_or_below_it() {
        // Room above: 6 px over it, the meter 7 px into its strip and centred.
        assert_eq!(
            meter_at([1052., 137., 300., 618.]),
            [1132., 137. - 38. + 7.]
        );
        assert_eq!(meter_at([100., 38., 300., 400.]), [180., 7.]);
        // None: just below it.
        assert_eq!(
            meter_at([100., 20., 300., 400.]),
            [180., 20. + 400. + 6. + 7.]
        );
    }

    fn run(stage: &mut Stage, clap: &mut Clap, until: f32) -> bool {
        let mut alive = true;
        while stage.now < until && alive {
            let before = stage.now;
            stage.now += 1. / 60.;
            stage.ticks = 1;
            alive = clap.step(
                stage,
                Clock {
                    before,
                    now: stage.now,
                },
            );
            let _ = stage.take_sounds();
        }
        alive
    }

    #[test]
    fn eight_claps_from_anyone_make_an_ovation_and_one_fizzles() {
        let mut stage = Stage::new(1440., 900., 4);
        stage.call = [1052., 137., 300., 618.];
        let sam = Who::new("a", "Sam Rivera", [1, 2, 3], false);
        let alex = Who::new("b", "Alex Morgan", [4, 5, 6], false);
        let mut clap = Clap::new(&mut stage, &sam);
        for n in 1..PEAK {
            assert!(run(&mut stage, &mut clap, n as f32 * 0.5));
            let who = if n % 2 == 0 { &sam } else { &alex };
            assert!(clap.again(&mut stage, who, &Reaction::new("clap")));
        }
        assert!(run(&mut stage, &mut clap, PEAK as f32 * 0.5));
        assert!(clap.ovation.is_some(), "no ovation");
        assert!(!run(&mut stage, &mut clap, 30.), "the ovation never ended");

        let mut clap = Clap::new(&mut stage, &sam);
        let start = stage.now;
        assert!(run(&mut stage, &mut clap, start + FIZZLE + 0.1));
        assert_eq!(clap.meter.label, "…it fizzled");
        assert!(clap.session.is_none());
        assert!(
            !run(&mut stage, &mut clap, start + 10.),
            "the fizzle never ended"
        );
    }
}
