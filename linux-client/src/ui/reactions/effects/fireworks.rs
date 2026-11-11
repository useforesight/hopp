//! 7. Fireworks: `level` rockets (the sender held the fuse longer for more) go up across
//! everyone's darkened screen and burst as peonies, rings, gold willows and crackles;
//! from seven up the last three are a finale. More fireworks while it plays join the show.

use std::sync::Arc;

use super::Effect;
use crate::{
    protocol::Reaction,
    ui::reactions::{
        looks::{self, Look},
        particles::{Emit, MOST, Shape},
        scene::Canvas,
        stage::{Clock, Stage, Who, bezier, ease, rgb},
        synth::{Filter, Graph, Id, Noise, Tone, Wave},
    },
};

static PALETTES: [[[u8; 3]; 3]; 6] = [
    [rgb(0xff4d6d), rgb(0xff9eb5), rgb(0xffffff)],
    [rgb(0x4cc9f0), rgb(0xa8f0ff), rgb(0xffffff)],
    [rgb(0xffd166), rgb(0xfff1b8), rgb(0xff9f1c)],
    [rgb(0xb26bff), rgb(0xe6c3ff), rgb(0xffffff)],
    [rgb(0x06d6a0), rgb(0xb9ffe6), rgb(0xffffff)],
    [rgb(0xff7a3d), rgb(0xffd0a8), rgb(0xfff3c4)],
];
static GOLD: [[u8; 3]; 4] = [rgb(0xffcf70), rgb(0xffb347), rgb(0xffe8b0), rgb(0xfff6dc)];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const SMOKE: [[u8; 3]; 2] = [rgb(0xffd9a0), WHITE];
const TRAIL: [[u8; 3]; 3] = [rgb(0xffcf8a), rgb(0xff9f5a), rgb(0xfff3c4)];
const POP: [[u8; 3]; 2] = [WHITE, rgb(0xfff6c8)];
const EMBER: [[u8; 3]; 1] = [rgb(0xfff3c4)];
/// Willow trails stop dropping gold once there are this many sparks, so the bursts
/// still to come have room under the particle limit.
const TRAILS_UP_TO: usize = MOST - 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Peony,
    Ring,
    Willow,
    Crackle,
}

/// A rocket waiting for its turn.
struct Planned {
    at: f32,
    x: f32,
    kind: Kind,
    big: f32,
    flight: f32,
    /// The finale's last, which shakes the screen.
    last: bool,
}

struct Rocket {
    from: [f32; 2],
    to: [f32; 2],
    started: f32,
    flight: f32,
    wob: f32,
    kind: Kind,
    big: f32,
}

/// A willow's stars, moved here so they can leave long drooping trails.
struct Willow {
    stars: Vec<[f32; 4]>,
    frame: u32,
}

/// A crackle's late pop.
struct Pop {
    at: f32,
    x: f32,
    y: f32,
}

/// The soft light a burst throws.
struct Glow {
    at: [f32; 2],
    r: f32,
    look: Arc<Look>,
    started: f32,
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
        let k = ease((now - self.started) / self.seconds);
        self.from + (self.to - self.from) * k
    }

    fn towards(&mut self, to: f32, seconds: f32, now: f32) {
        *self = Self {
            from: self.at(now),
            to,
            started: now,
            seconds,
        };
    }
}

pub struct Fireworks {
    now: f32,
    planned: Vec<Planned>,
    rockets: Vec<Rocket>,
    willows: Vec<Willow>,
    pops: Vec<Pop>,
    glows: Vec<Glow>,
    /// When the finales' last bursts shake the screen.
    finales: Vec<f32>,
    dark: Fade,
    /// When the darkness lifts.
    dark_until: f32,
    dark_look: Arc<Look>,
}

impl Fireworks {
    pub fn new(stage: &mut Stage, who: &Who, level: u8) -> Self {
        let mut fireworks = Self {
            now: 0.,
            planned: Vec::new(),
            rockets: Vec::new(),
            willows: Vec::new(),
            pops: Vec::new(),
            glows: Vec::new(),
            finales: Vec::new(),
            dark: Fade {
                from: 0.,
                to: 0.,
                started: 0.,
                seconds: 1.,
            },
            dark_until: 0.,
            dark_look: dark_look(),
        };
        fireworks.play(stage, who, level);
        fireworks
    }

    fn play(&mut self, stage: &mut Stage, who: &Who, level: u8) {
        let n = usize::from(level.clamp(1, 10));
        let finale = n >= 7;
        let gap = if n <= 3 { 0.52 } else { 0.33 };
        let times: Vec<f32> = (0..n)
            .map(|i| {
                if finale && i >= n - 3 {
                    (n - 3) as f32 * gap + (i - (n - 3)) as f32 * 0.09
                } else {
                    i as f32 * gap
                }
            })
            .collect();
        let mut xs: Vec<f32> = (0..n)
            .map(|i| stage.w * (0.1 + 0.8 * (i as f32 + 0.5) / n as f32) + stage.rand(-40., 40.))
            .collect();
        shuffle(&mut xs, stage);
        let mut kinds = [Kind::Peony, Kind::Ring, Kind::Willow, Kind::Crackle];
        shuffle(&mut kinds, stage);
        let last = times[n - 1] + 1.3;
        self.darken(last + 1.8);
        let said = if n == 1 {
            "launched a firework".to_owned()
        } else if finale {
            format!("launched a {n}-rocket finale")
        } else {
            format!("launched {n} fireworks")
        };
        stage.banner(who, &said, "🎆", (last * 1000.).max(2600.));
        for (i, (&at, &x)) in times.iter().zip(&xs).enumerate() {
            let in_finale = finale && i >= n - 3;
            let last = in_finale && i == n - 1;
            let kind = if last {
                Kind::Peony
            } else if n == 1 {
                *stage.rng.pick(&[Kind::Peony, Kind::Ring, Kind::Willow])
            } else {
                kinds[i % kinds.len()]
            };
            let big = if in_finale {
                1.35
            } else if n == 1 {
                1.2
            } else {
                1.
            };
            self.planned.push(Planned {
                at: self.now + at,
                x,
                kind,
                big: big * stage.rand(0.9, 1.1),
                flight: stage.rand(1., 1.3),
                last,
            });
        }
        self.launch_due(stage);
    }

    /// The darkness comes down, and lifts `seconds` from now.
    fn darken(&mut self, seconds: f32) {
        if self.dark.to < 1. {
            self.dark.towards(1., 0.5, self.now);
        }
        self.dark_until = self.now + seconds;
    }

    fn launch_due(&mut self, stage: &mut Stage) {
        let now = self.now;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.planned)
            .into_iter()
            .partition(|planned| planned.at <= now);
        self.planned = later;
        for planned in due {
            self.launch(stage, &planned);
            if planned.last {
                self.finales.push(planned.at + 1.25);
            }
        }
    }

    fn launch(&mut self, stage: &mut Stage, planned: &Planned) {
        let x0 = planned.x;
        let to = [x0 + stage.rand(-70., 70.), stage.h * stage.rand(0.14, 0.4)];
        let wob = stage.rand(0., 6.);
        let whistle = stage.rng.chance(0.75);
        let w = stage.w;
        launch_sound(stage.sound(), x0 / w, planned.flight, whistle);
        stage.emit(Emit {
            x: x0,
            y: stage.h - 4.,
            count: 14,
            angle: -90.,
            spread: 70.,
            speed: [1., 4.],
            gravity: 0.15,
            life: [14., 26.],
            size: [2., 4.],
            shape: Shape::Circle,
            colours: &SMOKE,
            spin: false,
            ..Emit::default()
        });
        self.rockets.push(Rocket {
            from: [x0, stage.h + 10.],
            to,
            started: self.now,
            flight: planned.flight,
            wob,
            kind: planned.kind,
            big: planned.big,
        });
    }

    /// The rocket where it is at `t`: on its way, trailing sparks, or burst.
    fn fly(&mut self, stage: &mut Stage, index: usize, t: f32) -> bool {
        let rocket = &self.rockets[index];
        let p = ((t - rocket.started) / rocket.flight).min(1.);
        let e = 1. - (1. - p) * (1. - p);
        let x = rocket.from[0]
            + (rocket.to[0] - rocket.from[0]) * p
            + (p * 9. + rocket.wob).sin() * 3. * (1. - p);
        let y = rocket.from[1] + (rocket.to[1] - rocket.from[1]) * e;
        stage.emit(Emit {
            x,
            y,
            count: 2,
            angle: 90.,
            spread: 40.,
            speed: [0.2, 1.4],
            gravity: 0.06,
            life: [14., 30.],
            size: [1.6, 3.4],
            shape: Shape::Circle,
            colours: &TRAIL,
            spin: false,
            wobble: false,
            ..Emit::default()
        });
        stage.emit(Emit {
            x,
            y,
            count: 1,
            angle: 0.,
            spread: 0.,
            speed: [0., 0.],
            gravity: 0.,
            life: [2., 3.],
            size: [6., 7.],
            shape: Shape::Circle,
            colours: &[WHITE],
            glow: true,
            ..Emit::default()
        });
        if p < 1. {
            return true;
        }
        let (kind, big) = (rocket.kind, rocket.big);
        let crackles = if kind == Kind::Crackle { 4 } else { 14 };
        let w = stage.w;
        let fizzes = self.burst(stage, x, y, kind, big);
        let sound = stage.sound();
        let bus = Bus::new(sound);
        boom_sound(sound, &bus, x / w, big, crackles);
        // The crackle's pops tick as they go.
        let p = bus.pan(sound, x / w, 0.25);
        for when in fizzes {
            fizz(sound, p, when, 0.14);
        }
        false
    }

    /// A burst at (`x`, `y`). Returns when its pops fizz, from now.
    fn burst(&mut self, stage: &mut Stage, x: f32, y: f32, kind: Kind, big: f32) -> Vec<f32> {
        let palette: &'static [[u8; 3]] = if kind == Kind::Willow {
            &GOLD
        } else {
            stage.rng.pick(&PALETTES)
        };
        let v = 9.5 * big;
        self.glows.push(Glow {
            at: [x, y],
            r: 160. * big,
            look: looks::blob(palette[0], &[(0., 0.45), (0.92, 0.)]),
            started: self.now,
        });
        stage.emit(Emit {
            x,
            y,
            count: 18,
            angle: 0.,
            spread: 360.,
            speed: [0.5, 3.],
            gravity: 0.,
            drag: 0.9,
            life: [8., 14.],
            size: [8., 14.],
            shape: Shape::Circle,
            colours: &[WHITE],
            glow: true,
            ..Emit::default()
        });
        let spark = Emit {
            x,
            y,
            count: 1,
            spread: 0.,
            gravity: 0.05,
            drag: 0.95,
            life: [70., 110.],
            size: [3., 5.5],
            shape: Shape::Circle,
            colours: palette,
            glow: true,
            spin: false,
            wobble: false,
            ..Emit::default()
        };
        let mut fizzes = Vec::new();
        match kind {
            Kind::Peony => {
                sphere(stage, (115. * big).round() as usize, v, &spark);
                sphere(
                    stage,
                    32,
                    v * 0.45,
                    &Emit {
                        colours: &[WHITE],
                        size: [2., 3.5],
                        ..spark.clone()
                    },
                );
            }
            Kind::Ring => {
                let n = (70. * big).round() as usize;
                let off = stage.rand(0., 360.);
                for i in 0..n {
                    stage.emit(Emit {
                        angle: off + i as f32 * 360. / n as f32,
                        spread: 2.,
                        speed: [v * 0.98, v],
                        gravity: 0.04,
                        life: [80., 110.],
                        size: [3.5, 5.5],
                        colours: &palette[..2],
                        ..spark.clone()
                    });
                }
                sphere(
                    stage,
                    26,
                    v * 0.35,
                    &Emit {
                        colours: &palette[2..],
                        size: [2.5, 4.],
                        ..spark.clone()
                    },
                );
            }
            Kind::Crackle => {
                sphere(
                    stage,
                    (60. * big).round() as usize,
                    v * 0.85,
                    &Emit {
                        colours: &GOLD,
                        life: [40., 60.],
                        size: [2.5, 4.],
                        ..spark.clone()
                    },
                );
                // Late pops scattered where the sparks ended up, each with its own tick.
                for i in 0..(34. * big).round() as usize {
                    let ms = stage.rand(500., 1200.);
                    let a = stage.rand(0., std::f32::consts::TAU);
                    let r = v * 18. * stage.rng.unit().sqrt();
                    self.pops.push(Pop {
                        at: self.now + ms / 1000.,
                        x: x + a.cos() * r,
                        y: y + a.sin() * r + ms / 40.,
                    });
                    if i % 4 == 0 {
                        fizzes.push(ms / 1000.);
                    }
                }
            }
            Kind::Willow => {
                let stars = (0..(60. * big).round() as usize)
                    .map(|_| {
                        let u = stage.rng.unit();
                        let s = v * 0.8 * (1. - u * u).sqrt();
                        let a = stage.rand(0., std::f32::consts::TAU);
                        [x, y, a.cos() * s, a.sin() * s]
                    })
                    .collect();
                self.willows.push(Willow { stars, frame: 0 });
            }
        }
        fizzes
    }
}

/// `stage.rng`'s shuffle of `items`.
fn shuffle<T>(items: &mut [T], stage: &mut Stage) {
    for i in (1..items.len()).rev() {
        let j = ((stage.rng.unit() * (i + 1) as f32) as usize).min(i);
        items.swap(i, j);
    }
}

/// A spherical shell seen from the front: sparks bunch towards the rim, as real ones do.
fn sphere(stage: &mut Stage, n: usize, v: f32, spark: &Emit) {
    for _ in 0..n {
        let u = stage.rng.unit();
        let s = v * (1. - u * u).sqrt() * stage.rand(0.96, 1.04);
        let angle = stage.rand(0., 360.);
        stage.emit(Emit {
            angle,
            speed: [s, s],
            ..spark.clone()
        });
    }
}

/// A willow's next frame: its stars droop on, dropping gold.
fn droop(stage: &mut Stage, willow: &mut Willow) -> bool {
    willow.frame += 1;
    let f = willow.frame;
    for star in &mut willow.stars {
        star[2] *= 0.955;
        star[3] = star[3] * 0.955 + 0.045;
        star[0] += star[2];
        star[1] += star[3];
        if f % 2 == 0 && f < 150 && stage.particles.len() < TRAILS_UP_TO {
            stage.emit(Emit {
                x: star[0],
                y: star[1],
                count: 1,
                angle: 90.,
                spread: 30.,
                speed: [0., 0.25],
                gravity: 0.012,
                drag: 1.,
                life: if f < 30 { [40., 60.] } else { [26., 44.] },
                size: [1.8, 3.2],
                shape: Shape::Circle,
                colours: &GOLD,
                spin: false,
                wobble: false,
                ..Emit::default()
            });
        }
        if f % 6 == 0 && f < 110 {
            stage.emit(Emit {
                x: star[0],
                y: star[1],
                count: 1,
                angle: 0.,
                spread: 0.,
                speed: [0., 0.],
                gravity: 0.,
                life: [5., 7.],
                size: [4., 5.],
                shape: Shape::Circle,
                colours: &EMBER,
                glow: true,
                ..Emit::default()
            });
        }
    }
    f < 150
}

/// The night coming down: darker at the top, as `.p7-dark`.
fn dark_look() -> Arc<Look> {
    Look::new(
        "fireworks:dark",
        [160., 100.],
        r##"<defs><radialGradient id="d" cx=".5" cy="1" r="1.3" gradientTransform="translate(0 1) scale(1 .769) translate(0 -1)"><stop offset="0" stop-color="#060416" stop-opacity=".4"/><stop offset="1" stop-color="#02020c" stop-opacity=".68"/></radialGradient></defs><rect width="160" height="100" fill="url(#d)"/>"##,
    )
}

// ---- Sound ----------------------------------------------------------------------------

/// The show's two buses: close by, and far off with most of it in the hall.
struct Bus {
    out: Id,
    far: Id,
}

impl Bus {
    fn new(sound: &mut Graph) -> Self {
        let out = sound.dest(0.);
        let far = sound.dest(0.9);
        sound.gain_of(far).set(0.6, 0.);
        Self { out, far }
    }

    /// Where something at `x` (0 to 1 across the screen) sounds, sending `send` of
    /// itself far off.
    fn pan(&self, sound: &mut Graph, x: f32, send: f32) -> Id {
        let pan = sound.panner((x * 2. - 1.).clamp(-1., 1.) * 0.75);
        let s = sound.gain(send);
        sound.connect(pan, self.out);
        sound.connect(pan, s);
        sound.connect(s, self.far);
        pan
    }
}

fn fizz(sound: &mut Graph, to: Id, when: f32, gain: f32) {
    let dur = sound.rng.range(0.008, 0.02);
    let gain = gain * sound.rng.range(0.4, 1.);
    let freq = sound.rng.range(3500., 7000.);
    sound.noise(Noise {
        when,
        dur,
        gain,
        filter: Filter::Highpass,
        freq,
        attack: 0.001,
        to: Some(to),
        ..Noise::default()
    });
}

fn launch_sound(sound: &mut Graph, x: f32, flight: f32, whistle: bool) {
    let bus = Bus::new(sound);
    let p = bus.pan(sound, x, 0.2);
    sound.noise(Noise {
        dur: 0.14,
        gain: 0.45,
        filter: Filter::Lowpass,
        freq: 500.,
        q: 0.8,
        attack: 0.002,
        to: Some(p),
        ..Noise::default()
    });
    sound.tone(Tone {
        freq: 150.,
        glide_to: Some(55.),
        dur: 0.16,
        gain: 0.35,
        to: Some(p),
        ..Tone::default()
    });
    sound.noise(Noise {
        when: 0.02,
        dur: flight,
        gain: 0.06,
        filter: Filter::Bandpass,
        freq: 2500.,
        sweep_to: Some(6500.),
        q: 1.2,
        attack: 0.03,
        to: Some(p),
        ..Noise::default()
    });
    if !whistle {
        return;
    }
    let f0 = sound.rng.range(650., 900.);
    let rise = sound.rng.range(2.6, 3.2);
    let o = sound.osc(Wave::Sine, 440.);
    sound.frequency(o).set(f0, 0.04).exp(f0 * rise, flight);
    let v = sound.osc(Wave::Sine, 9.);
    let vg = sound.gain(18.);
    sound.connect(v, vg);
    sound.modulate(vg, o, Graph::frequency);
    let g = sound.gain(0.);
    sound
        .gain_of(g)
        .set(0., 0.)
        .linear(0.05, 0.12)
        .set(0.05, flight * 0.8)
        .linear(0., flight);
    sound.connect(o, g);
    sound.connect(g, p);
    sound.play(o, 0., flight + 0.05);
    sound.play(v, 0., flight + 0.05);
}

fn boom_sound(sound: &mut Graph, bus: &Bus, x: f32, size: f32, crackles: usize) {
    // Light travels faster than sound: the boom lands a beat after the flash.
    let d = sound.rng.range(0.08, 0.2);
    let p = bus.pan(sound, x, 0.6);
    sound.noise(Noise {
        when: d,
        dur: 0.6 + size * 0.3,
        gain: 0.7 + size * 0.25,
        filter: Filter::Lowpass,
        freq: 220.,
        q: 0.7,
        attack: 0.004,
        to: Some(p),
        ..Noise::default()
    });
    sound.tone(Tone {
        when: d,
        freq: 78.,
        glide_to: Some(30.),
        dur: 0.7,
        gain: 0.55 * size,
        to: Some(p),
        ..Tone::default()
    });
    sound.noise(Noise {
        when: d,
        dur: 0.05,
        gain: 0.25,
        filter: Filter::Bandpass,
        freq: 1200.,
        q: 0.8,
        to: Some(p),
        ..Noise::default()
    });
    sound.noise(Noise {
        when: d + 0.05,
        dur: 1.8,
        gain: 0.18,
        filter: Filter::Lowpass,
        freq: 700.,
        sweep_to: Some(140.),
        attack: 0.02,
        to: Some(bus.far),
        ..Noise::default()
    });
    for _ in 0..crackles.min(14) {
        let when = d + 0.25 + sound.rng.range(0., 1.3);
        let gain = sound.rng.range(0.03, 0.12);
        let freq = sound.rng.range(3000., 6000.);
        sound.noise(Noise {
            when,
            dur: 0.012,
            gain,
            filter: Filter::Highpass,
            freq,
            attack: 0.001,
            to: Some(p),
            ..Noise::default()
        });
    }
}

impl Effect for Fireworks {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        self.now = clock.now;
        self.launch_due(stage);
        // Rockets and willows move a 60 Hz frame at a time, as the prototype's do.
        let ticks = stage.ticks;
        for tick in 0..ticks {
            let t = clock.now - (ticks - 1 - tick) as f32 / 60.;
            let mut index = 0;
            while index < self.rockets.len() {
                if self.fly(stage, index, t) {
                    index += 1;
                } else {
                    self.rockets.remove(index);
                }
            }
            self.willows.retain_mut(|willow| droop(stage, willow));
        }
        let now = clock.now;
        self.pops.retain(|pop| {
            if pop.at > now {
                return true;
            }
            stage.emit(Emit {
                x: pop.x,
                y: pop.y,
                count: 5,
                angle: 0.,
                spread: 360.,
                speed: [0.8, 2.6],
                gravity: 0.02,
                drag: 0.9,
                life: [6., 14.],
                size: [2., 4.],
                shape: Shape::Circle,
                colours: &POP,
                glow: true,
                ..Emit::default()
            });
            false
        });
        self.finales.retain(|&at| {
            if at > now {
                return true;
            }
            stage.shake(6., 600.);
            stage.flash(rgb(0xfff1d6), 500., 0.18);
            false
        });
        self.glows.retain(|glow| now - glow.started < 0.9);
        if clock.crossed(self.dark_until) {
            self.dark.towards(0., 1.2, self.dark_until);
        }
        !(self.planned.is_empty()
            && self.rockets.is_empty()
            && self.willows.is_empty()
            && self.pops.is_empty()
            && self.finales.is_empty()
            && self.glows.is_empty()
            && now > self.dark_until + 1.3)
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let dark = self.dark.at(self.now);
        if dark > 0.01 {
            canvas.picture_in(
                &self.dark_look,
                [-40., -40.],
                [stage.w + 80., stage.h + 80.],
                dark,
            );
            // The night is behind the sparks: they're drawn again over it.
            stage.particles.draw(canvas);
        }
        for glow in &self.glows {
            let k = bezier(0.1, 0.7, 0.3, 1., (self.now - glow.started) / 0.85);
            let r = glow.r * (0.3 + 0.7 * k);
            canvas.picture_in(
                &glow.look,
                [glow.at[0] - r, glow.at[1] - r],
                [r * 2., r * 2.],
                1. - k,
            );
        }
    }

    fn again(&mut self, stage: &mut Stage, who: &Who, reaction: &Reaction) -> bool {
        self.play(stage, who, reaction.level.max(1));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::reactions::Player;

    fn fireworks(level: u8) -> Reaction {
        Reaction {
            level,
            ..Reaction::new("fireworks")
        }
    }

    #[test]
    fn a_full_charge_is_a_finale_that_ends() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut player = Player::new(1440., 900., 4);
        player.play(&alex, &fireworks(10));
        let mut most = 0;
        let mut flashed = false;
        for step in 1..=60 * 20 {
            player.advance(step as f32 / 60.);
            let frame = player.frame();
            most = most.max(player.stage.particles.len());
            flashed |= frame.shake != [0., 0.];
            let _ = player.stage.take_sounds();
        }
        assert!(most > 600, "only {most} sparks");
        assert!(most < MOST, "the sparks ran out");
        assert!(flashed, "the finale never shook");
        assert!(!player.busy());
    }

    #[test]
    fn more_fireworks_join_the_show_playing() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut stage = Stage::new(1440., 900., 1);
        let mut show = Fireworks::new(&mut stage, &alex, 2);
        assert_eq!(show.planned.len() + show.rockets.len(), 2);
        assert!(show.again(&mut stage, &alex, &fireworks(3)));
        assert_eq!(show.planned.len() + show.rockets.len(), 5);
    }
}
