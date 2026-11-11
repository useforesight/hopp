//! 4. Emoji: one of eight flies HUGE from the sender's tile, swells, then bursts into a
//! shower of itself, each with its own sound.

use std::sync::Arc;

use super::Effect;
use crate::ui::reactions::{
    looks::{self, EMOJI, Look},
    particles::{CONFETTI, Emit, Shape},
    scene::Canvas,
    stage::{Clock, Stage, Who, bezier, ease_out, lerp, rgb},
    synth::{Filter, Graph, Id, Noise, Tone, Wave},
};

/// One of the eight.
struct Kind {
    key: &'static str,
    emoji: &'static [&'static str],
    verb: &'static str,
    hue: [u8; 3],
    /// How its shower falls.
    gravity: f32,
    /// Whether it points the way it flies (the rocket).
    tilt: bool,
}

const KINDS: [Kind; 8] = [
    Kind {
        key: "party",
        emoji: &["🎉"],
        verb: "started a party",
        hue: rgb(0xffd166),
        gravity: 0.32,
        tilt: false,
    },
    Kind {
        key: "fire",
        emoji: &["🔥"],
        verb: "says that's fire",
        hue: rgb(0xff6a00),
        gravity: -0.14,
        tilt: false,
    },
    Kind {
        key: "applause",
        emoji: &["👏"],
        verb: "is applauding",
        hue: rgb(0xffcf8a),
        gravity: 0.32,
        tilt: false,
    },
    Kind {
        key: "lol",
        emoji: &["😂"],
        verb: "is crying laughing",
        hue: rgb(0x7fd3ff),
        gravity: 0.32,
        tilt: false,
    },
    Kind {
        key: "rocket",
        emoji: &["🚀"],
        verb: "says ship it",
        hue: rgb(0x8b5cf6),
        gravity: 0.32,
        tilt: true,
    },
    Kind {
        key: "hundred",
        emoji: &["💯"],
        verb: "says 100",
        hue: rgb(0xff3b5c),
        gravity: 0.32,
        tilt: false,
    },
    Kind {
        key: "love",
        emoji: &["❤️"],
        verb: "sent love",
        hue: rgb(0xff5d8f),
        gravity: -0.07,
        tilt: false,
    },
    Kind {
        key: "shock",
        emoji: &["😱"],
        verb: "is shocked",
        hue: rgb(0xa78bfa),
        gravity: 0.32,
        tilt: false,
    },
];

const FIRE_TRAIL: [[u8; 3]; 3] = [rgb(0xffb000), rgb(0xff6a00), rgb(0xffdd55)];
const FIRE: [[u8; 3]; 3] = [rgb(0xffb000), rgb(0xff5a00), rgb(0xffe066)];
const APPLAUSE_TRAIL: [[u8; 3]; 2] = [rgb(0xffffff), rgb(0xffe2a8)];
const APPLAUSE: [[u8; 3]; 2] = [rgb(0xffd166), rgb(0xffffff)];
const LOL_TRAIL: [[u8; 3]; 2] = [rgb(0x7fd3ff), rgb(0xbfe9ff)];
const LOL: [[u8; 3]; 3] = [rgb(0x7fd3ff), rgb(0xbfe9ff), rgb(0x4cc9f0)];
/// Smoke (a little see-through in the prototype) and flame.
const ROCKET_TRAIL: [[u8; 3]; 3] = [rgb(0xe6e6f0), rgb(0xffb347), rgb(0xff6a00)];
const HUNDRED_TRAIL: [[u8; 3]; 2] = [rgb(0xffd166), rgb(0xfff3b0)];
const HUNDRED: [[u8; 3]; 3] = [rgb(0xffd166), rgb(0xffe066), rgb(0xffffff)];
const SHOCK_TRAIL: [[u8; 3]; 2] = [rgb(0xc4b5fd), rgb(0xffffff)];
const SHOCK: [[u8; 3]; 3] = [rgb(0xa78bfa), rgb(0x6d28d9), rgb(0xffffff)];

/// How long it flies, then swells before it bursts.
const FLY: f32 = 0.9;
const WIND: f32 = 0.23;
const BURST: f32 = FLY + WIND;
/// The flying emoji's box: 240px type, as the prototype's `.p4-fly`.
const SIZE: f32 = 288.;

pub struct Emoji {
    kind: &'static Kind,
    now: f32,
    /// Its flight: a curve from the tile, up over, to where it bursts.
    from: [f32; 2],
    over: [f32; 2],
    to: [f32; 2],
    frames: u32,
    shadow: Arc<Look>,
}

impl Emoji {
    pub fn new(stage: &mut Stage, who: &Who, choice: &str) -> Option<Self> {
        let kind = KINDS.iter().find(|kind| kind.key == choice)?;
        let from = stage.tile();
        let to = [
            stage.w * stage.rand(0.36, 0.52),
            stage.h * stage.rand(0.34, 0.46),
        ];
        let over = [(from[0] + to[0]) / 2., from[1].min(to[1]) - stage.h * 0.45];
        stage.banner(who, kind.verb, kind.emoji[0], 2600.);
        let sound = stage.sound();
        sound.tone(Tone {
            freq: 320.,
            glide_to: Some(720.),
            dur: 0.12,
            gain: 0.25,
            wet: 0.1,
            ..Tone::default()
        });
        sound.noise(Noise {
            filter: Filter::Bandpass,
            freq: 400.,
            sweep_to: Some(2400.),
            q: 1.2,
            dur: FLY,
            attack: 0.5,
            gain: 0.12,
            wet: 0.25,
            ..Noise::default()
        });
        if kind.key == "rocket" {
            rocket_launch(sound, FLY);
        }
        Some(Self {
            kind,
            now: 0.,
            from,
            over,
            to,
            frames: 0,
            shadow: looks::blob([0, 0, 0], &[(0., 0.3), (0.5, 0.18), (1., 0.)]),
        })
    }

    /// Where it is `t` seconds into its flight, slowing into the burst.
    fn flying(&self, t: f32) -> [f32; 2] {
        let k = 1. - (1. - (t / FLY).clamp(0., 1.)).powf(2.4);
        let u = 1. - k;
        [0, 1].map(|i| u * u * self.from[i] + 2. * u * k * self.over[i] + k * k * self.to[i])
    }

    /// The rocket's turn as it flies, in degrees: nose first.
    fn heading(&self, t: f32) -> f32 {
        let (a, b) = (self.flying(t - 1. / 60.), self.flying(t));
        (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees() + 45.
    }

    /// What it leaves behind as it flies.
    fn trail(&self, [x, y]: [f32; 2]) -> Emit {
        let base = Emit {
            x,
            y,
            count: 2,
            angle: 0.,
            spread: 360.,
            speed: [0.5, 2.5],
            life: [25., 45.],
            drag: 0.95,
            gravity: 0.1,
            ..Emit::default()
        };
        match self.kind.key {
            "party" => Emit {
                shape: Shape::Rect,
                size: [5., 9.],
                ..base
            },
            "fire" => sparks(&FIRE_TRAIL, base),
            "applause" => sparks(&APPLAUSE_TRAIL, base),
            "lol" => Emit {
                shape: Shape::Circle,
                colours: &LOL_TRAIL,
                size: [5., 9.],
                gravity: 0.2,
                ..base
            },
            "rocket" => Emit {
                shape: Shape::Circle,
                colours: &ROCKET_TRAIL,
                size: [8., 16.],
                gravity: -0.03,
                ..base
            },
            "hundred" => sparks(&HUNDRED_TRAIL, base),
            "love" => Emit {
                shape: Shape::Emoji,
                emoji: &["💗"],
                size: [12., 20.],
                gravity: -0.05,
                ..base
            },
            _ => sparks(&SHOCK_TRAIL, base),
        }
    }

    /// The shower after the burst, besides the emoji themselves.
    fn shower(&self, [x, y]: [f32; 2]) -> Emit {
        let base = Emit {
            x,
            y,
            angle: -90.,
            spread: 360.,
            speed: [4., 15.],
            life: [50., 100.],
            drag: 0.97,
            ..Emit::default()
        };
        match self.kind.key {
            "party" => Emit {
                shape: Shape::Rect,
                count: 90,
                size: [7., 12.],
                ..base
            },
            "fire" => Emit {
                gravity: -0.12,
                count: 70,
                ..sparks(&FIRE, base)
            },
            "applause" => Emit {
                shape: Shape::Star,
                count: 30,
                colours: &APPLAUSE,
                ..base
            },
            "lol" => Emit {
                shape: Shape::Circle,
                count: 50,
                colours: &LOL,
                size: [6., 12.],
                ..base
            },
            "rocket" => Emit {
                count: 120,
                speed: [6., 20.],
                gravity: 0.2,
                ..sparks(&CONFETTI, base)
            },
            "hundred" => Emit {
                shape: Shape::Star,
                count: 50,
                colours: &HUNDRED,
                size: [8., 14.],
                ..base
            },
            "love" => Emit {
                shape: Shape::Emoji,
                emoji: &["💖", "💕", "💗"],
                count: 26,
                size: [16., 30.],
                gravity: -0.06,
                ..base
            },
            _ => Emit {
                shape: Shape::Tri,
                count: 50,
                colours: &SHOCK,
                ..base
            },
        }
    }

    fn burst(&self, stage: &mut Stage) {
        let [x, y] = self.to;
        sound(stage.sound(), self.kind.key);
        stage.emit(Emit {
            x,
            y,
            count: 34,
            angle: -90.,
            spread: 360.,
            speed: [5., 16.],
            gravity: self.kind.gravity,
            drag: 0.975,
            life: [70., 120.],
            size: [30., 66.],
            shape: Shape::Emoji,
            emoji: self.kind.emoji,
            spin: true,
            ..Emit::default()
        });
        stage.emit(self.shower(self.to));
        stage.flash(self.kind.hue, 380., 0.16);
        stage.shake(if self.kind.key == "rocket" { 14. } else { 7. }, 380.);
    }

    /// The flying emoji, its middle at `at`, turned `turn` degrees, `scale` (across and
    /// down) times its size.
    fn draw_flying(
        &self,
        canvas: &mut Canvas,
        at: [f32; 2],
        turn: f32,
        scale: [f32; 2],
        alpha: f32,
    ) {
        // Turns are baked into its picture, in steps.
        let step = if self.kind.tilt { 5. } else { 2. };
        let look = flying_look(self.kind.emoji[0], (turn / step).round() * step);
        let (w, h) = (SIZE * scale[0], SIZE * scale[1]);
        // Its drop shadow, soft and below.
        let shade = SIZE * 0.8 * scale[0];
        canvas.picture_in(
            &self.shadow,
            [at[0] - shade / 2., at[1] + 18. * scale[1] - shade / 2.],
            [shade, shade],
            alpha,
        );
        canvas.picture_in(&look, [at[0] - w / 2., at[1] - h / 2.], [w, h], alpha);
    }
}

impl Effect for Emoji {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        self.now = clock.now;
        if clock.now < FLY {
            let at = self.flying(clock.now);
            for _ in 0..stage.ticks {
                if self.frames % 2 == 0 && clock.now > 0.06 {
                    stage.emit(self.trail(at));
                }
                self.frames += 1;
            }
        }
        if clock.crossed(BURST) {
            self.burst(stage);
        }
        if self.kind.key == "shock" && clock.crossed(BURST + 0.56) {
            stage.flash([0, 0, 0], 1100., 0.4);
            stage.shake(12., 600.);
        }
        clock.now < BURST + 1.2
    }

    fn draw(&self, _: &Stage, canvas: &mut Canvas) {
        let t = self.now;
        let ms = t * 1000.;
        if t < FLY {
            let k = 1. - (1. - t / FLY).powf(2.4);
            let turn = if self.kind.tilt {
                self.heading(t)
            } else {
                (ms / 90.).sin() * 14. * (1. - k)
            };
            let scale = 0.22 + 0.78 * k.powf(0.7);
            self.draw_flying(canvas, self.flying(t), turn, [scale, scale], 1.);
        } else if t < BURST {
            // Swelling and quivering before it bursts.
            let k = (t - FLY) / WIND;
            let turn = if self.kind.tilt {
                self.heading(FLY)
            } else {
                (ms / 30.).sin() * 6. * k
            };
            let at = [self.to[0] + (ms / 18.).sin() * 5. * k, self.to[1]];
            let scale = [1. + 0.28 * k * k, 1. + 0.2 * k * k];
            self.draw_flying(canvas, at, turn, scale, 1.);
        } else if t < BURST + 0.18 {
            let k = ease_out((t - BURST) / 0.18);
            let scale = lerp(1.3, 1.9, k);
            self.draw_flying(canvas, self.to, 0., [scale, scale], 1. - k);
        }
        // Two shock rings in its colour.
        if t >= BURST {
            for i in 0..2 {
                let i = i as f32;
                let u = ((t - BURST - i * 0.09) / (0.8 + i * 0.3)).max(0.);
                if u >= 1. {
                    continue;
                }
                let k = bezier(0.1, 0.7, 0.3, 1., u);
                let scale = lerp(0.4, 7. + i * 4., k);
                let alpha = 1. - k;
                canvas.ring(
                    self.to,
                    62. * scale,
                    14. * scale,
                    self.kind.hue,
                    0.25 * alpha,
                );
                canvas.ring(self.to, 57.5 * scale, 5. * scale, self.kind.hue, alpha);
            }
        }
    }
}

fn sparks(colours: &'static [[u8; 3]], base: Emit) -> Emit {
    Emit {
        shape: Shape::Circle,
        glow: true,
        colours,
        size: [4., 8.],
        gravity: 0.05,
        ..base
    }
}

/// The flying emoji, turned `turn` degrees, in its box.
fn flying_look(emoji: &str, turn: f32) -> Arc<Look> {
    let font = SIZE / 1.2;
    let c = SIZE / 2.;
    Look::new(
        format!("emoji:flying:{emoji}:{turn}"),
        [SIZE, SIZE],
        format!(
            r#"<g transform="rotate({turn} {c} {c})"><text x="{c}" y="{y}" text-anchor="middle" font-family="{EMOJI}" font-size="{font}">{}</text></g>"#,
            looks::escape(emoji),
            y = c + font * 0.36,
        ),
    )
}

// ---- Sound ----------------------------------------------------------------------------

/// A filter after a [`voice`]'s envelope, and how much of it goes to the hall.
struct Voicing {
    filter: Filter,
    freq: f32,
    q: f32,
    wet: f32,
}

/// An oscillator held, then let go, through an optional filter. Returns the oscillator
/// and the filter.
fn voice(
    sound: &mut Graph,
    freq: f32,
    when: f32,
    dur: f32,
    gain: f32,
    wave: Wave,
    voicing: Option<Voicing>,
) -> (Id, Option<Id>) {
    let osc = sound.osc(wave, freq);
    sound.frequency(osc).set(freq, when);
    let envelope = sound.gain(1.);
    sound
        .gain_of(envelope)
        .set(0.0001, when)
        .exp(gain, when + 0.015)
        .set(gain, when + dur * 0.6)
        .exp(0.0001, when + dur);
    sound.connect(osc, envelope);
    let (out, filter, wet) = match voicing {
        Some(voicing) => {
            let filter = sound.filter(voicing.filter, voicing.freq, voicing.q);
            sound.frequency(filter).set(voicing.freq, when);
            sound.connect(envelope, filter);
            (filter, Some(filter), voicing.wet)
        }
        None => (envelope, None, 0.25),
    };
    let to = sound.dest(wet);
    sound.connect(out, to);
    sound.play(osc, when, when + dur + 0.05);
    (osc, filter)
}

fn pop(sound: &mut Graph, when: f32, g: f32) {
    sound.noise(Noise {
        when,
        filter: Filter::Highpass,
        freq: 1100.,
        dur: 0.07,
        gain: g,
        wet: 0.15,
        ..Noise::default()
    });
    sound.tone(Tone {
        when,
        freq: 240.,
        glide_to: Some(60.),
        dur: 0.1,
        gain: g * 0.8,
        wet: 0.05,
        ..Tone::default()
    });
}

/// A brassy chord stab: three detuned saws through an opening filter, over the octave
/// below; long ones with vibrato.
fn brass(sound: &mut Graph, f: f32, when: f32, dur: f32, g: f32) {
    for cents in [-12., 0., 9.] {
        let (osc, filter) = voice(
            sound,
            f * 2f32.powf(cents / 1200.),
            when,
            dur,
            g,
            Wave::Saw,
            Some(Voicing {
                filter: Filter::Lowpass,
                freq: 300.,
                q: 2.,
                wet: 0.45,
            }),
        );
        if let Some(filter) = filter {
            sound
                .frequency(filter)
                .exp(2600., when + 0.05)
                .exp(700., when + dur);
        }
        if dur > 1. {
            let lfo = sound.osc(Wave::Sine, 5.5);
            let depth = sound.gain(f * 0.012);
            sound.connect(lfo, depth);
            sound.modulate(depth, osc, |sound, id| sound.frequency(id));
            sound.play(lfo, when + 0.3, when + dur);
        }
    }
    sound.tone(Tone {
        when,
        freq: f / 2.,
        dur,
        gain: g * 1.4,
        attack: 0.02,
        wet: 0.2,
        ..Tone::default()
    });
}

fn rocket_launch(sound: &mut Graph, seconds: f32) {
    sound.noise(Noise {
        filter: Filter::Lowpass,
        freq: 160.,
        sweep_to: Some(2800.),
        q: 0.9,
        dur: seconds + 0.15,
        attack: 0.25,
        gain: 0.45,
        wet: 0.25,
        ..Noise::default()
    });
    let (osc, _) = voice(
        sound,
        55.,
        0.,
        seconds + 0.1,
        0.12,
        Wave::Saw,
        Some(Voicing {
            filter: Filter::Lowpass,
            freq: 700.,
            q: 1.,
            wet: 0.2,
        }),
    );
    sound.frequency(osc).exp(340., seconds);
}

/// Each one's own sound as it bursts.
fn sound(sound: &mut Graph, key: &str) {
    match key {
        "party" => {
            // A party horn, then poppers and paper.
            let (osc, _) = voice(
                sound,
                400.,
                0.,
                0.5,
                0.14,
                Wave::Saw,
                Some(Voicing {
                    filter: Filter::Bandpass,
                    freq: 1500.,
                    q: 3.,
                    wet: 0.2,
                }),
            );
            sound.frequency(osc).linear(520., 0.12);
            for i in 0..7 {
                let when = 0.32 + i as f32 * sound.rng.range(0.05, 0.09);
                let g = sound.rng.range(0.3, 0.55);
                pop(sound, when, g);
            }
            sound.noise(Noise {
                when: 0.4,
                dur: 1.4,
                gain: 0.07,
                freq: 5200.,
                sweep_to: Some(2600.),
                q: 0.6,
                attack: 0.08,
                ..Noise::default()
            });
        }
        "fire" => {
            sound.noise(Noise {
                filter: Filter::Lowpass,
                freq: 250.,
                sweep_to: Some(3200.),
                dur: 0.6,
                attack: 0.2,
                gain: 0.55,
                wet: 0.35,
                ..Noise::default()
            });
            sound.noise(Noise {
                when: 0.3,
                filter: Filter::Lowpass,
                freq: 700.,
                sweep_to: Some(300.),
                dur: 1.5,
                attack: 0.1,
                gain: 0.25,
                wet: 0.3,
                ..Noise::default()
            });
            // Crackles.
            for _ in 0..46 {
                let noise = Noise {
                    when: 0.15 + sound.rng.unit().powf(1.4) * 1.7,
                    filter: Filter::Highpass,
                    freq: sound.rng.range(2500., 6000.),
                    dur: sound.rng.range(0.006, 0.02),
                    gain: sound.rng.range(0.08, 0.4),
                    wet: 0.1,
                    ..Noise::default()
                };
                sound.noise(noise);
            }
        }
        "applause" => {
            for _ in 0..80 {
                let when = sound.rng.unit().powf(1.3) * 2.3;
                let noise = Noise {
                    when,
                    filter: Filter::Bandpass,
                    freq: sound.rng.range(900., 2400.),
                    q: 1.3,
                    dur: sound.rng.range(0.025, 0.05),
                    gain: 0.32 * (1. - when / 2.7),
                    attack: 0.001,
                    wet: 0.35,
                    ..Noise::default()
                };
                sound.noise(noise);
            }
        }
        "lol" => {
            // Five falling "ha"s through the formants of a deep "ah".
            for (i, f) in [232., 214., 200., 186., 170.].into_iter().enumerate() {
                let (when, dur) = (i as f32 * 0.17, 0.14);
                sound.noise(Noise {
                    when: (when - 0.03).max(0.),
                    filter: Filter::Bandpass,
                    freq: 1600.,
                    q: 0.8,
                    dur: 0.05,
                    gain: 0.12,
                    attack: 0.01,
                    ..Noise::default()
                });
                let osc = sound.osc(Wave::Saw, f * 1.08);
                sound
                    .frequency(osc)
                    .set(f * 1.08, when)
                    .exp(f * 0.88, when + dur);
                let envelope = sound.gain(1.);
                sound
                    .gain_of(envelope)
                    .set(0.0001, when)
                    .exp(0.5, when + 0.02)
                    .exp(0.0001, when + dur);
                let out = sound.dest(0.15);
                sound.connect(osc, envelope);
                for (freq, q, level) in [(680., 6., 1.), (1080., 7., 0.6), (2400., 8., 0.15)] {
                    let formant = sound.filter(Filter::Bandpass, freq, q);
                    let g = sound.gain(level * 1.6);
                    sound.connect(envelope, formant);
                    sound.connect(formant, g);
                    sound.connect(g, out);
                }
                sound.play(osc, when, when + dur + 0.05);
            }
        }
        "rocket" => {
            sound.tone(Tone {
                freq: 130.,
                glide_to: Some(32.),
                dur: 0.7,
                gain: 0.85,
                wet: 0.3,
                ..Tone::default()
            });
            sound.noise(Noise {
                filter: Filter::Lowpass,
                freq: 3000.,
                sweep_to: Some(300.),
                dur: 0.6,
                gain: 0.6,
                wet: 0.4,
                ..Noise::default()
            });
            for _ in 0..16 {
                let noise = Noise {
                    when: 0.25 + sound.rng.range(0., 1.1),
                    filter: Filter::Highpass,
                    freq: 4000.,
                    dur: 0.02,
                    gain: sound.rng.range(0.1, 0.3),
                    wet: 0.5,
                    ..Noise::default()
                };
                sound.noise(noise);
            }
        }
        "hundred" => {
            // A coin-up blip, then a sparkle running up.
            sound.tone(Tone {
                freq: 988.,
                dur: 0.09,
                gain: 0.13,
                wave: Wave::Square,
                attack: 0.002,
                wet: 0.1,
                ..Tone::default()
            });
            sound.tone(Tone {
                when: 0.08,
                freq: 1319.,
                dur: 0.55,
                gain: 0.13,
                wave: Wave::Square,
                attack: 0.002,
                wet: 0.15,
                ..Tone::default()
            });
            for (i, freq) in [1319., 1568., 1976., 2637., 3136.].into_iter().enumerate() {
                sound.tone(Tone {
                    when: 0.3 + i as f32 * 0.055,
                    freq,
                    dur: 0.3,
                    gain: 0.08,
                    wave: Wave::Triangle,
                    wet: 0.3,
                    ..Tone::default()
                });
            }
        }
        "love" => {
            sound.tone(Tone {
                freq: 261.6,
                dur: 1.8,
                attack: 0.25,
                gain: 0.1,
                wet: 0.5,
                ..Tone::default()
            });
            for (i, freq) in [523.3, 659.3, 784., 987.8, 1318.5].into_iter().enumerate() {
                let when = i as f32 * 0.07;
                sound.tone(Tone {
                    when,
                    freq,
                    dur: 2.2,
                    gain: 0.11,
                    attack: 0.008,
                    wet: 0.65,
                    ..Tone::default()
                });
                sound.tone(Tone {
                    when,
                    freq: freq * 2.76,
                    dur: 0.6,
                    gain: 0.02,
                    wet: 0.65,
                    ..Tone::default()
                });
            }
        }
        _ => {
            // Dun, dun, DUNNN.
            brass(sound, 196., 0., 0.2, 0.09);
            brass(sound, 185., 0.27, 0.2, 0.09);
            brass(sound, 155.6, 0.56, 1.6, 0.1);
            sound.tone(Tone {
                when: 0.56,
                freq: 82.,
                glide_to: Some(50.),
                dur: 1.3,
                gain: 0.7,
                wet: 0.3,
                ..Tone::default()
            });
            sound.noise(Noise {
                when: 0.56,
                filter: Filter::Lowpass,
                freq: 400.,
                dur: 0.6,
                gain: 0.4,
                wet: 0.3,
                ..Noise::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_the_protocol_knows_flies() {
        let mut stage = Stage::new(1440., 900., 9);
        let who = Who::new("a", "Sam Rivers", [1, 2, 3], false);
        for choice in crate::protocol::REACTION_EMOJI {
            assert!(Emoji::new(&mut stage, &who, choice).is_some(), "{choice}");
        }
        assert!(Emoji::new(&mut stage, &who, "kazoo").is_none());
    }
}
