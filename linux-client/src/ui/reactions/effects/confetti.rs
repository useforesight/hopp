//! 1. Party popper: confetti cannons fire from both bottom corners of everyone's screen,
//! with a proper pop. Quick repeats (`level`, up to 4) make it bigger.

use super::Effect;
use crate::ui::reactions::{
    particles::{Emit, Shape},
    scene::Canvas,
    stage::{Clock, Stage, Who},
    synth::{Filter, Noise, Tone},
};

pub struct Confetti {
    big: f32,
    shape: Shape,
}

impl Confetti {
    pub fn new(stage: &mut Stage, who: &Who, streak: u8) -> Self {
        let big = 1. + f32::from(streak) * 0.5;
        let sound = stage.sound();
        for when in [0., 0.07] {
            sound.noise(Noise {
                when,
                dur: 0.09,
                gain: 0.7,
                filter: Filter::Highpass,
                freq: 900.,
                q: 0.7,
                wet: 0.1,
                ..Noise::default()
            });
            sound.tone(Tone {
                when,
                freq: 180.,
                glide_to: Some(50.),
                dur: 0.12,
                gain: 0.5,
                wet: 0.05,
                ..Tone::default()
            });
            // Paper flutter afterwards.
            sound.noise(Noise {
                when: when + 0.05,
                dur: 1.2,
                gain: 0.08,
                filter: Filter::Bandpass,
                freq: 5000.,
                q: 0.6,
                sweep_to: Some(2500.),
                attack: 0.05,
                ..Noise::default()
            });
        }
        let shape = *stage.rng.pick(&[Shape::Rect, Shape::Rect, Shape::Tri]);
        let mut confetti = Self { big, shape };
        confetti.fire(stage, 0., -62.);
        stage.shake(4. + f32::from(streak) * 2., 300.);
        let said = if streak > 0 {
            format!("popped confetti ×{}", streak + 1)
        } else {
            "popped confetti".into()
        };
        stage.banner(who, &said, "🎉", 2600.);
        confetti
    }

    fn fire(&mut self, stage: &mut Stage, x: f32, angle: f32) {
        let shape = self.shape;
        // A fresh pick for the second cannon, as the prototype's second call makes.
        self.shape = *stage.rng.pick(&[Shape::Rect, Shape::Rect, Shape::Tri]);
        stage.emit(Emit {
            x,
            y: stage.h + 10.,
            count: (110. * self.big).round() as usize,
            angle,
            spread: 26.,
            speed: [16., 27.],
            gravity: 0.32,
            drag: 0.983,
            life: [120., 200.],
            size: [7., 13.],
            shape,
            ..Emit::default()
        });
    }
}

impl Effect for Confetti {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        if clock.crossed(0.07) {
            let w = stage.w;
            self.fire(stage, w, -118.);
        }
        if clock.crossed(0.16) {
            let count = (30. * self.big).round() as usize;
            for (x, angle) in [(40., -60.), (stage.w - 40., -120.)] {
                stage.emit(Emit {
                    x,
                    y: stage.h,
                    count,
                    angle,
                    spread: 30.,
                    speed: [14., 22.],
                    shape: Shape::Circle,
                    size: [5., 8.],
                    ..Emit::default()
                });
            }
        }
        clock.now < 0.2
    }

    fn draw(&self, _: &Stage, _: &mut Canvas) {}
}
