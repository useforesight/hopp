//! 9. Disco: a mirror ball drops onto everyone's screen, the lights go down and coloured
//! beams sweep to three bars of a 124 BPM groove, then a cymbal and it's gone. Whoever
//! started it can end it early; another disco starts it again.

use std::sync::Arc;

use super::Effect;
use crate::{
    protocol::Reaction,
    ui::reactions::{
        looks::{self, Look},
        scene::{Canvas, place},
        stage::{Clock, Stage, Who, bezier, ease, ease_in_out, keyframes, rgb},
        synth::{Filter, Graph, Id, Noise, Tone, Wave},
    },
};

const BPM: f32 = 124.;
const BEAT: f32 = 60. / BPM;
const S16: f32 = BEAT / 4.;
/// Three bars, then the cymbal.
const BEATS: f32 = 12.;
/// When the groove's first beat lands.
const T0: f32 = 0.08;
const END: f32 = T0 + BEATS * BEAT;
/// How long the lights take to go once it's over.
const OUTRO: f32 = 1.1;
/// Where the beams come from: under the ball (the menu bar, string, cap and radius down).
const BALL_Y: f32 = 26. + 96. + 8. + 75.;
/// The ball's middle, from the top of its string.
const BALL_MIDDLE: f32 = 102. + 75.;
const TAG: &str = "disco";
const COLOURS: [[u8; 3]; 6] = [
    rgb(0xff2bd6),
    rgb(0x22e4ff),
    rgb(0xffe14d),
    rgb(0x6dff5c),
    rgb(0x9b5cff),
    rgb(0xff7a2b),
];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];

/// A coloured spot of light wandering the room.
struct Spot {
    look: Arc<Look>,
    /// Where it goes, as fractions of the screen, back to the first.
    path: [[f32; 2]; 6],
    seconds: f32,
}

/// One of the ball's reflections, turning slowly round it.
struct Dot {
    r: f32,
    angle: f32,
    size: f32,
    rgb: [u8; 3],
    glow: Arc<Look>,
    /// Where its twinkle starts.
    phase: f32,
}

/// An opacity moving as a CSS transition with `ease` does.
#[derive(Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    started: f32,
    seconds: f32,
}

impl Fade {
    fn at(&self, now: f32) -> f32 {
        self.from + (self.to - self.from) * ease((now - self.started) / self.seconds)
    }
}

pub struct Disco {
    /// Who started it: only they can end it early.
    by: String,
    /// When this party began on the effect's clock: another disco starts it again.
    began: f32,
    /// Seconds since this party began.
    now: f32,
    /// When it ended.
    ended: Option<f32>,
    /// How far the lights are down with the beat, as the prototype's opacity follows it.
    beat: f32,
    dim_out: Option<Fade>,
    layer_out: Option<Fade>,
    facets: Vec<[u8; 3]>,
    spots: Vec<Spot>,
    dots: Vec<Dot>,
    dim_look: Arc<Look>,
    halo: Arc<Look>,
    shade: Arc<Look>,
    sparkle: Arc<Look>,
}

impl Disco {
    pub fn new(stage: &mut Stage, who: &Who) -> Self {
        let sound = stage.sound();
        // A slide whistle as it drops, then a springy "doing" as it lands.
        sound.tone(Tone {
            freq: 1500.,
            glide_to: Some(380.),
            dur: 0.5,
            gain: 0.07,
            wet: 0.3,
            ..Tone::default()
        });
        sound.tone(Tone {
            when: 0.55,
            freq: 300.,
            glide_to: Some(520.),
            dur: 0.25,
            wave: Wave::Triangle,
            gain: 0.06,
            wet: 0.2,
            ..Tone::default()
        });
        groove(stage.tagged(TAG));

        // A faceted mirror: greys, with a few bright and tinted tiles.
        let tints = [rgb(0xffd6f6), rgb(0xc8f4ff), rgb(0xfff3b8), rgb(0xd9c8ff)];
        let facets = (0..15 * 30)
            .map(|_| {
                let k = stage.rng.unit();
                if k > 0.93 {
                    WHITE
                } else if k > 0.86 {
                    *stage.rng.pick(&tints)
                } else {
                    let s = 8. + stage.rng.unit() * 12.;
                    let l = 35. + stage.rng.unit() * 45.;
                    hsl(240., s / 100., l / 100.)
                }
            })
            .collect();
        let spots = COLOURS[..5]
            .iter()
            .enumerate()
            .map(|(i, &colour)| {
                let mut path = [[0.; 2]; 6];
                for point in &mut path[..5] {
                    *point = [stage.rand(0.05, 0.95), stage.rand(0.2, 0.95)];
                }
                path[5] = path[0];
                Spot {
                    look: looks::blob(colour, &[(0., 0.8), (0.96, 0.)]),
                    path,
                    seconds: BEAT * 8. * (1. + i as f32 * 0.15),
                }
            })
            .collect();
        let far = stage.w.hypot(stage.h);
        let dots = (0..90)
            .map(|_| {
                let r = stage.rand(120., far);
                let angle = stage.rand(0., std::f32::consts::TAU);
                let size = stage.rand(3., 9.);
                let colour = if stage.rng.chance(0.25) {
                    *stage.rng.pick(&COLOURS)
                } else {
                    WHITE
                };
                Dot {
                    r,
                    angle,
                    size,
                    rgb: colour,
                    glow: looks::glow(colour),
                    phase: stage.rand(0., 1.2),
                }
            })
            .collect();
        stage.banner(who, "started disco mode", "🪩", 2600.);
        Self {
            by: who.key.clone(),
            began: 0.,
            now: 0.,
            ended: None,
            beat: 1.,
            dim_out: None,
            layer_out: None,
            facets,
            spots,
            dots,
            dim_look: dim_look(),
            halo: halo_look(),
            shade: shade_look(),
            sparkle: looks::blob(WHITE, &[(0., 1.), (0.1, 1.), (0.85, 0.)]),
        }
    }

    /// The lights come up, the ball goes back up, and a cymbal; cut short if `early`.
    fn finish(&mut self, stage: &mut Stage, early: bool) {
        if self.ended.is_some() {
            return;
        }
        let now = self.now;
        self.ended = Some(now);
        if early {
            stage.hush(TAG);
        }
        let sound = stage.sound();
        let when = if early { 0.02 } else { (END - now).max(0.) };
        cymbal(sound, when);
        let out = sound.dest(0.1);
        kick(sound, when, out);
        // Zip up.
        sound.tone(Tone {
            when: 0.15,
            freq: 300.,
            glide_to: Some(2400.),
            dur: 0.45,
            gain: 0.06,
            wet: 0.3,
            ..Tone::default()
        });
        stage.flash(WHITE, 400., 0.25);
        self.dim_out = Some(Fade {
            from: self.dim(now),
            to: 0.,
            started: now,
            seconds: 0.9,
        });
        self.layer_out = Some(Fade {
            from: self.layer(now),
            to: 0.,
            started: now,
            seconds: 0.45,
        });
    }

    /// The lights down: in over half a second, then with the beat.
    fn dim(&self, now: f32) -> f32 {
        match self.dim_out {
            Some(fade) => fade.at(now),
            None => ease(now / 0.5) * self.beat,
        }
    }

    /// The beams, spots and reflections: in after the ball's dropped, out at the end.
    fn layer(&self, now: f32) -> f32 {
        match self.layer_out {
            Some(fade) => fade.at(now),
            None => ease((now - 0.45) / 0.45),
        }
    }

    /// Where the ball's rig is: dropped by this much and turned by this many degrees.
    fn rig(&self) -> (f32, f32) {
        let t = self.now;
        if let Some(ended) = self.ended {
            let k = (t - ended - 0.12) / 0.65;
            // `element.animate` eases the whole run, not each step.
            let k = bezier(0.5, 0., 0.9, 0.4, k);
            let y = keyframes(&[(0., 0.), (0.25, 14.), (1., -420.)], k, |k| k);
            return (y, 0.);
        }
        let k = bezier(0.3, 0.6, 0.4, 1., t / 0.9);
        let y = keyframes(
            &[(0., -300.), (0.55, 22.), (0.75, -9.), (0.9, 3.), (1., 0.)],
            k,
            |k| k,
        );
        let turn = keyframes(
            &[(0., 0.), (0.55, 2.), (0.75, -1.5), (0.9, 0.6), (1., 0.)],
            k,
            |k| k,
        );
        (y, turn)
    }

    fn draw_beams(&self, stage: &Stage, canvas: &mut Canvas, alpha: f32, beat: Option<[f32; 2]>) {
        let origin = [stage.w / 2., BALL_Y];
        let length = stage.h * 1.6;
        // Back and forth every bar, easing at each end.
        let cycle = (self.now / (BEAT * 4.)).rem_euclid(2.);
        let swing = bezier(
            0.45,
            0.,
            0.55,
            1.,
            if cycle < 1. { cycle } else { 2. - cycle },
        );
        const STEPS: usize = 24;
        for (i, &colour) in COLOURS.iter().enumerate() {
            let from = -70. + i as f32 * 26.;
            let to = from + if i % 2 == 1 { -50. } else { 50. };
            let turn = (from + (to - from) * swing).to_radians();
            let lit = match beat {
                Some([pulse, half]) => 0.3 + 0.45 * if i % 2 == 1 { half } else { pulse },
                None => 0.55,
            };
            // Narrow at the ball, wide at the far end, fading out by 85% of the way.
            for step in 0..STEPS {
                let (s0, s1) = (
                    0.85 * step as f32 / STEPS as f32,
                    0.85 * (step + 1) as f32 / STEPS as f32,
                );
                let half = |s: f32| (13.8 + (230. - 13.8) * s) / 2.;
                let fade = 0.8 * (1. - (s0 + s1) / 2. / 0.85);
                let corners = [
                    [-half(s0), s0 * length],
                    [half(s0), s0 * length],
                    [half(s1), s1 * length],
                    [-half(s1), s1 * length],
                ];
                canvas.polygon(
                    corners.map(|p| place(p, origin, turn, 1.)),
                    colour,
                    fade * lit * alpha,
                );
            }
        }
    }

    fn draw_ball(&self, stage: &Stage, canvas: &mut Canvas) {
        let (drop, turn) = self.rig();
        let top = [stage.w / 2., drop];
        let turn = turn.to_radians();
        let at = |p: [f32; 2]| place(p, top, turn, 1.);
        let middle = at([0., BALL_MIDDLE]);
        canvas.picture(&self.halo, middle, 4., 1.);
        // The string.
        for (i, grey) in [0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc].iter().enumerate() {
            let y = i as f32 * 16.;
            canvas.polygon(
                [[-1., y], [1., y], [1., y + 16.], [-1., y + 16.]].map(at),
                [*grey; 3],
                1.,
            );
        }
        // The mirror: its tiles scroll past behind the round window it's seen through.
        canvas.disc(middle, 75., rgb(0x1b1830), 1.);
        let offset = 300. * (self.now / 3.6).rem_euclid(1.);
        for row in 0..15 {
            let y0 = row as f32 * 10. + 0.6;
            let y1 = y0 + 8.8;
            let dy = (y0 - 75.).abs().max((y1 - 75.).abs());
            if dy >= 75. {
                continue;
            }
            let chord = (75. * 75. - dy * dy).sqrt();
            for column in 0..30 {
                let mut x0 = (column as f32 * 10. - offset).rem_euclid(300.) + 0.6;
                if x0 > 291.2 {
                    x0 -= 300.;
                }
                let (left, right) = (x0.max(75. - chord), (x0 + 8.8).min(75. + chord));
                if right <= left {
                    continue;
                }
                let (left, right) = (left - 75., right - 75.);
                let (top, bottom) = (y0 + 102., y1 + 102.);
                canvas.polygon(
                    [[left, top], [right, top], [right, bottom], [left, bottom]].map(at),
                    self.facets[row * 30 + column],
                    1.,
                );
            }
        }
        canvas.picture(&self.shade, middle, 1., 1.);
        // The cap, over the top of the ball.
        for (x, grey) in [(-11., 0x80), (-4., 0xdd), (3., 0x99)] {
            canvas.polygon(
                [[x, 94.], [x + 8., 94.], [x + 8., 106.], [x, 106.]].map(at),
                [grey; 3],
                1.,
            );
        }
        // Glints, popping and turning.
        for (x, y, delay, seconds) in [
            (0.38, 0.3, 0., 0.9),
            (0.68, 0.58, 0.45, 0.9),
            (0.3, 0.7, 0.2, 1.3),
        ] {
            let p = ((self.now + delay) / seconds).rem_euclid(1.);
            let k = ease_in_out(if p < 0.5 { p * 2. } else { (1. - p) * 2. });
            if k < 0.02 {
                continue;
            }
            let centre = at([x * 150. - 75., 102. + y * 150.]);
            let size = 26. * k;
            canvas.picture_in(
                &self.sparkle,
                [centre[0] - size / 2., centre[1] - size / 2.],
                [size, size],
                1.,
            );
            let spin = turn + (45. * k).to_radians();
            for arm in [0., std::f32::consts::FRAC_PI_2] {
                let diamond = [[0., -13.], [1.2, 0.], [0., 13.], [-1.2, 0.]];
                canvas.polygon(
                    diamond.map(|[px, py]| place([px * k, py * k], centre, spin + arm, 1.)),
                    WHITE,
                    0.9,
                );
            }
        }
    }
}

/// `hsl(h, s, l)` with `s` and `l` from 0 to 1.
fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1. - (2. * l - 1.).abs()) * s;
    let x = c * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
    let m = l - c / 2.;
    let (r, g, b) = match (h / 60.) as u32 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    [r, g, b].map(|v| ((v + m) * 255.).round().clamp(0., 255.) as u8)
}

/// The room with the lights down: `.p9-dim`, lighter round the ball.
fn dim_look() -> Arc<Look> {
    Look::new(
        "disco:dim",
        [160., 100.],
        r##"<defs><radialGradient id="d" cx=".5" cy=".1" r="1.2" gradientTransform="translate(0 .1) scale(1 .75) translate(0 -.1)"><stop offset="0" stop-color="#1e063c" stop-opacity=".7"/><stop offset="1" stop-color="#04000e" stop-opacity=".9"/></radialGradient></defs><rect width="160" height="100" fill="url(#d)"/>"##,
    )
}

/// The ball's glow, a quarter size: drawn four times as big.
fn halo_look() -> Arc<Look> {
    Look::new(
        "disco:halo",
        [140., 140.],
        r##"<defs><filter id="a" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="17.5"/></filter><filter id="b" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="7.5"/></filter></defs><circle cx="70" cy="70" r="18.75" fill="#a078ff" fill-opacity=".45" filter="url(#a)"/><circle cx="70" cy="70" r="18.75" fill="#fff" fill-opacity=".35" filter="url(#b)"/>"##,
    )
}

/// The light on the ball, from the top left, and its dark rim.
fn shade_look() -> Arc<Look> {
    Look::new(
        "disco:shade",
        [150., 150.],
        r##"<defs><radialGradient id="s" gradientUnits="userSpaceOnUse" cx="51" cy="42" r="146.5"><stop offset="0" stop-color="#fff" stop-opacity=".75"/><stop offset=".22" stop-color="#fff" stop-opacity=".1"/><stop offset=".45" stop-color="#000" stop-opacity="0"/><stop offset=".78" stop-color="#0a001e" stop-opacity=".55"/><stop offset="1" stop-color="#000" stop-opacity=".92"/></radialGradient></defs><circle cx="75" cy="75" r="75" fill="url(#s)"/>"##,
    )
}

// ---- Sound ----------------------------------------------------------------------------

fn mtof(m: f32) -> f32 {
    440. * 2f32.powf((m - 69.) / 12.)
}

fn kick(sound: &mut Graph, t: f32, out: Id) {
    let o = sound.osc(Wave::Sine, 170.);
    sound.frequency(o).set(170., t).exp(48., t + 0.11);
    let g = sound.gain(1.);
    sound.gain_of(g).set(1., t).exp(0.0001, t + 0.42);
    sound.connect(o, g);
    sound.connect(g, out);
    sound.play(o, t, t + 0.45);
}

#[allow(clippy::too_many_arguments)]
fn saw(
    sound: &mut Graph,
    t: f32,
    freq: f32,
    dur: f32,
    gain: f32,
    cutoff: f32,
    out: Id,
    q: f32,
    env: f32,
) {
    let o = sound.osc(Wave::Saw, freq);
    let f = sound.filter(Filter::Lowpass, cutoff, q);
    sound
        .frequency(f)
        .set(cutoff * env, t)
        .exp(cutoff, t + dur * 0.8);
    let g = sound.gain(0.);
    sound
        .gain_of(g)
        .set(0.0001, t)
        .exp(gain, t + 0.006)
        .exp(0.0001, t + dur);
    sound.connect(o, f);
    sound.connect(f, g);
    sound.connect(g, out);
    sound.play(o, t, t + dur + 0.05);
}

fn cymbal(sound: &mut Graph, when: f32) {
    let out = sound.dest(0.45);
    sound.noise(Noise {
        when,
        dur: 2.4,
        gain: 0.32,
        filter: Filter::Highpass,
        freq: 5200.,
        q: 0.5,
        attack: 0.003,
        to: Some(out),
        ..Noise::default()
    });
    sound.noise(Noise {
        when,
        dur: 1.2,
        gain: 0.18,
        filter: Filter::Bandpass,
        freq: 9000.,
        q: 0.8,
        to: Some(out),
        ..Noise::default()
    });
    for f in [567., 802., 1131., 1537., 2213.] {
        sound.tone(Tone {
            when,
            freq: f * 2.3,
            wave: Wave::Square,
            gain: 0.012,
            dur: 1.1,
            to: Some(out),
            ..Tone::default()
        });
    }
}

/// The whole groove from `T0`, into two buses that are cut at the end.
fn groove(sound: &mut Graph) {
    let dry = sound.dest(0.06);
    let wet = sound.dest(0.32);
    let bass: &[(f32, f32, f32)] = &[
        (0., 45., 2.),
        (3., 57., 1.),
        (6., 45., 1.),
        (8., 48., 2.),
        (10., 50., 1.),
        (11., 52., 1.),
        (14., 57., 1.),
    ];
    let fill: &[(f32, f32, f32)] = &[
        (0., 45., 2.),
        (3., 57., 1.),
        (6., 45., 1.),
        (8., 55., 1.),
        (10., 57., 1.),
        (12., 60., 1.),
        (13., 59., 1.),
        (14., 57., 1.),
        (15., 55., 1.),
    ];
    let chords = [
        [69., 72., 76., 79.],
        [74., 77., 81., 84.],
        [69., 72., 76., 79.],
    ];
    for bar in 0..(BEATS / 4.) as usize {
        let tb = T0 + bar as f32 * 4. * BEAT;
        for b in 0..4 {
            let t = tb + b as f32 * BEAT;
            kick(sound, t, dry);
            // Open hat.
            sound.noise(Noise {
                when: t + BEAT / 2.,
                dur: 0.2,
                gain: 0.16,
                filter: Filter::Highpass,
                freq: 7500.,
                q: 0.7,
                to: Some(dry),
                ..Noise::default()
            });
            for s in [1., 3.] {
                sound.noise(Noise {
                    when: t + s * S16,
                    dur: 0.035,
                    gain: 0.05,
                    filter: Filter::Highpass,
                    freq: 9000.,
                    to: Some(dry),
                    ..Noise::default()
                });
            }
            if b == 1 || b == 3 {
                // Clap: three quick slaps and a tail.
                for d in [0., 0.011, 0.022] {
                    sound.noise(Noise {
                        when: t + d,
                        dur: 0.03,
                        gain: 0.32,
                        filter: Filter::Bandpass,
                        freq: 1300.,
                        q: 1.4,
                        to: Some(wet),
                        ..Noise::default()
                    });
                }
                sound.noise(Noise {
                    when: t + 0.03,
                    dur: 0.22,
                    gain: 0.16,
                    filter: Filter::Bandpass,
                    freq: 1100.,
                    q: 1.,
                    to: Some(wet),
                    ..Noise::default()
                });
            }
        }
        for &(s, m, len) in if bar == 2 { fill } else { bass } {
            let dur = len * S16 * 0.95;
            saw(sound, tb + s * S16, mtof(m), dur, 0.2, 260., dry, 9., 5.);
            sound.tone(Tone {
                when: tb + s * S16,
                freq: mtof(m - 12.),
                gain: 0.22,
                dur,
                to: Some(dry),
                ..Tone::default()
            });
        }
        for s in [6., 14.] {
            for m in chords[bar] {
                for d in [-9., 9.] {
                    let freq = mtof(m) * 2f32.powf(d / 1200.);
                    saw(sound, tb + s * S16, freq, 0.2, 0.035, 1400., wet, 2., 3.);
                }
            }
        }
    }
    // Cut a little after the end, as the prototype disconnects them; the hall rings on.
    for bus in [dry, wet] {
        sound.gain_of(bus).set(0., END + 0.9);
    }
}

impl Effect for Disco {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        let clock = Clock {
            before: clock.before - self.began,
            now: clock.now - self.began,
        };
        self.now = clock.now;
        if self.ended.is_none() {
            let pos = (clock.now - T0) / BEAT;
            let target = if pos >= 0. {
                0.9 + 0.1 * (1. - (-pos.rem_euclid(1.) * 5.).exp())
            } else {
                1.
            };
            // Set every frame through a half-second transition: it follows softly.
            let k = 1. - (-(clock.now - clock.before).max(0.) / 0.2).exp();
            self.beat += (target - self.beat) * k;
            if clock.now >= END {
                self.finish(stage, false);
            }
        }
        match self.ended {
            Some(ended) => clock.now < ended + OUTRO,
            None => true,
        }
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let now = self.now;
        let dim = self.dim(now);
        canvas.picture_in(
            &self.dim_look,
            [-40., -40.],
            [stage.w + 80., stage.h + 80.],
            dim,
        );
        let layer = self.layer(now);
        if layer > 0.004 {
            let pos = (now - T0) / BEAT;
            let beat = (pos >= 0.).then(|| {
                [
                    (-pos.rem_euclid(1.) * 5.).exp(),
                    (-(pos + 0.5).rem_euclid(1.) * 7.).exp(),
                ]
            });
            self.draw_beams(stage, canvas, layer, beat);
            for spot in &self.spots {
                let k = ease_in_out((now / spot.seconds).rem_euclid(1.));
                let frames = |axis: usize| -> Vec<(f32, f32)> {
                    (0..6)
                        .map(|i| (i as f32 / 5., spot.path[i][axis]))
                        .collect()
                };
                let x = keyframes(&frames(0), k, |k| k) * stage.w;
                let y = keyframes(&frames(1), k, |k| k) * stage.h;
                canvas.picture_in(&spot.look, [x - 170., y - 170.], [340., 340.], 0.5 * layer);
            }
            let origin = [stage.w / 2., BALL_Y];
            let turn = now / 14. * std::f32::consts::TAU;
            for dot in &self.dots {
                let a = dot.angle + turn;
                let at = [origin[0] + a.cos() * dot.r, origin[1] + a.sin() * dot.r];
                if at[0] < -20. || at[1] < -20. || at[0] > stage.w + 20. || at[1] > stage.h + 20. {
                    continue;
                }
                let p = ((now + dot.phase) / 1.2).rem_euclid(2.);
                let twinkle = 0.15 + 0.85 * ease_in_out(if p < 1. { p } else { 2. - p });
                let glow = dot.size * 1.6;
                canvas.picture_in(
                    &dot.glow,
                    [at[0] - glow, at[1] - glow],
                    [glow * 2., glow * 2.],
                    0.6 * twinkle * layer,
                );
                canvas.disc(at, dot.size / 2., dot.rgb, twinkle * layer);
            }
        }
        self.draw_ball(stage, canvas);
    }

    fn again(&mut self, stage: &mut Stage, who: &Who, _: &Reaction) -> bool {
        // A new party ends the one playing, as the prototype's does; one already on its
        // way out finishes alongside.
        if self.ended.is_some() {
            return false;
        }
        stage.hush(TAG);
        let began = self.began + self.now;
        *self = Self::new(stage, who);
        self.began = began;
        true
    }

    fn end(&mut self, stage: &mut Stage, who: Option<&Who>) {
        if who.is_none_or(|who| who.key == self.by) {
            self.finish(stage, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disco_plays_three_bars_then_goes() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut stage = Stage::new(1440., 900., 3);
        let mut disco = Disco::new(&mut stage, &alex);
        let mut before = 0.;
        let mut gone = None;
        for step in 1..=60 * 10 {
            let now = step as f32 / 60.;
            stage.now = now;
            if !disco.step(&mut stage, Clock { before, now }) {
                gone = Some(now);
                break;
            }
            before = now;
        }
        let gone = gone.expect("it never ended");
        assert!(
            (END + OUTRO..END + OUTRO + 0.1).contains(&gone),
            "gone at {gone}"
        );
    }

    #[test]
    fn another_disco_starts_it_again() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut stage = Stage::new(1440., 900., 3);
        let mut disco = Disco::new(&mut stage, &alex);
        disco.step(
            &mut stage,
            Clock {
                before: 0.,
                now: 2.,
            },
        );
        assert!(disco.again(&mut stage, &alex, &Reaction::new("disco")));
        assert_eq!(disco.now, 0.);
        disco.end(&mut stage, None);
        assert!(!disco.again(&mut stage, &alex, &Reaction::new("disco")));
    }

    #[test]
    fn facets_are_greys_and_tints() {
        assert_eq!(hsl(240., 0., 0.5), [128, 128, 128]);
        assert_eq!(hsl(0., 1., 0.5), [255, 0, 0]);
    }
}
