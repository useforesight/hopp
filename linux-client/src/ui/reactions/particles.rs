//! Confetti, sparks, stars and flying emoji: the prototypes' particle system. Particles
//! move in the prototypes' units, per 60 Hz frame, so their numbers carry over as they
//! are.

use std::{collections::HashMap, sync::Arc};

use super::{
    looks::{self, Look},
    scene::{Canvas, Item, place},
    stage::Rng,
};

/// Never more than this many at once, so a big show can't bog the machine down.
pub const MOST: usize = 1400;

pub const CONFETTI: [[u8; 3]; 8] = [
    [0xff, 0x5d, 0x8f],
    [0xff, 0xd1, 0x66],
    [0x06, 0xd6, 0xa0],
    [0x4c, 0xc9, 0xf0],
    [0x9b, 0x5d, 0xe5],
    [0xf1, 0x5b, 0xb5],
    [0xfe, 0xe4, 0x40],
    [0xff, 0xff, 0xff],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Circle,
    Tri,
    Star,
    Emoji,
}

/// `H.emit`'s options, with its defaults.
#[derive(Clone)]
pub struct Emit {
    pub x: f32,
    pub y: f32,
    pub count: usize,
    /// Degrees; −90 is up.
    pub angle: f32,
    pub spread: f32,
    pub speed: [f32; 2],
    pub gravity: f32,
    pub drag: f32,
    /// In frames.
    pub life: [f32; 2],
    pub size: [f32; 2],
    pub colours: &'static [[u8; 3]],
    pub shape: Shape,
    pub emoji: &'static [&'static str],
    pub spin: bool,
    pub wobble: bool,
    pub glow: bool,
    pub fade: bool,
}

impl Default for Emit {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            count: 60,
            angle: -90.,
            spread: 60.,
            speed: [6., 14.],
            gravity: 0.35,
            drag: 0.985,
            life: [60., 120.],
            size: [6., 11.],
            colours: &CONFETTI,
            shape: Shape::Rect,
            emoji: &[],
            spin: true,
            wobble: true,
            glow: false,
            fade: true,
        }
    }
}

#[derive(Clone)]
struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    g: f32,
    drag: f32,
    life: f32,
    max: f32,
    size: f32,
    colour: [u8; 3],
    shape: Shape,
    emoji: Option<Arc<Look>>,
    rot: f32,
    vr: f32,
    wob: f32,
    glow: bool,
    fade: bool,
}

#[derive(Default)]
pub struct Particles {
    parts: Vec<Particle>,
}

impl Particles {
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    pub fn clear(&mut self) {
        self.parts.clear();
    }

    pub fn emit(&mut self, o: &Emit, rng: &mut Rng) {
        let n = o.count.min(MOST.saturating_sub(self.parts.len()));
        for _ in 0..n {
            let angle = (o.angle + rng.range(-o.spread / 2., o.spread / 2.)).to_radians();
            let v = rng.range(o.speed[0], o.speed[1]);
            let emoji = (!o.emoji.is_empty()).then(|| looks::emoji(rng.pick(o.emoji)));
            self.parts.push(Particle {
                x: o.x,
                y: o.y,
                vx: angle.cos() * v,
                vy: angle.sin() * v,
                g: o.gravity,
                drag: o.drag,
                life: 0.,
                max: rng.range(o.life[0], o.life[1]),
                size: rng.range(o.size[0], o.size[1]),
                colour: *rng.pick(o.colours),
                shape: o.shape,
                emoji,
                rot: rng.range(0., std::f32::consts::TAU),
                vr: if o.spin { rng.range(-0.3, 0.3) } else { 0. },
                wob: if o.wobble {
                    rng.range(0., std::f32::consts::TAU)
                } else {
                    0.
                },
                glow: o.glow,
                fade: o.fade,
            });
        }
    }

    /// One frame on, for a screen `height` high: the old and fallen go, the rest move.
    pub fn tick(&mut self, height: f32) {
        self.parts.retain(|p| p.life < p.max && p.y < height + 80.);
        for p in &mut self.parts {
            p.life += 1.;
            p.vx *= p.drag;
            p.vy = p.vy * p.drag + p.g;
            p.x += p.vx;
            p.y += p.vy;
            p.rot += p.vr;
            p.wob += 0.1;
        }
    }

    /// Draw them, one batch per colour and opacity, glows and emoji as pictures.
    pub fn draw(&self, canvas: &mut Canvas) {
        let mut batches: HashMap<([u8; 3], u8), (Vec<[f32; 2]>, Vec<usize>)> = HashMap::new();
        let mut discs = Vec::new();
        let mut pictures = Vec::new();
        for p in &self.parts {
            let alpha = if p.fade {
                (1. - (p.life / p.max).powi(3)).clamp(0., 1.)
            } else {
                1.
            };
            if alpha <= 0.004 {
                continue;
            }
            let centre = [p.x, p.y];
            if p.glow {
                // Glows add light in the prototypes; here they're laid over.
                let r = p.size * 2.2;
                pictures.push((
                    looks::glow(p.colour),
                    [p.x - r, p.y - r],
                    [r * 2., r * 2.],
                    alpha,
                ));
                continue;
            }
            let s = p.size;
            let squash = p.wob.cos().abs();
            let shape: Vec<[f32; 2]> = match p.shape {
                Shape::Rect => [
                    [-s / 2., -s / 4.],
                    [s / 2., -s / 4.],
                    [s / 2., s / 4.],
                    [-s / 2., s / 4.],
                ]
                .iter()
                .map(|&q| place(q, centre, p.rot, squash))
                .collect(),
                Shape::Tri => [[0., -s / 2.], [s / 2., s / 2.], [-s / 2., s / 2.]]
                    .iter()
                    .map(|&q| place(q, centre, p.rot, squash))
                    .collect(),
                Shape::Star => star(centre, s / 2., p.rot),
                Shape::Circle => {
                    discs.push((centre, s / 2., p.colour, alpha));
                    continue;
                }
                Shape::Emoji => {
                    let look = p.emoji.clone().unwrap_or_else(|| looks::emoji("🎉"));
                    pictures.push((look, [p.x - s / 2., p.y - s / 2.], [s, s], alpha));
                    continue;
                }
            };
            let key = (p.colour, (alpha * 32.).round() as u8);
            let (points, ends) = batches.entry(key).or_default();
            points.extend(shape);
            ends.push(points.len());
        }
        let mut batches: Vec<_> = batches.into_iter().collect();
        batches.sort_by_key(|(key, _)| *key);
        for ((rgb, alpha), (points, ends)) in batches {
            canvas.items.push(Item::Shapes {
                rgb,
                alpha: f32::from(alpha) / 32.,
                points,
                ends,
            });
        }
        for (at, radius, rgb, alpha) in discs {
            canvas.disc(at, radius, rgb, alpha);
        }
        for (look, at, size, alpha) in pictures {
            canvas.picture_in(&look, at, size, alpha);
        }
    }
}

/// A five-pointed star of radius `r`, its inner points at 0.45: from its middle, so it
/// fans out right.
pub fn star(centre: [f32; 2], r: f32, turn: f32) -> Vec<[f32; 2]> {
    let mut points = vec![centre];
    for i in 0..=10 {
        let a = (i % 10) as f32 * std::f32::consts::PI / 5. - std::f32::consts::FRAC_PI_2;
        let rr = if i % 2 == 1 { r * 0.45 } else { r };
        points.push(place([a.cos() * rr, a.sin() * rr], centre, turn, 1.));
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_never_too_many() {
        let mut particles = Particles::default();
        let mut rng = Rng::new(1);
        for _ in 0..40 {
            particles.emit(
                &Emit {
                    count: 100,
                    ..Emit::default()
                },
                &mut rng,
            );
        }
        assert_eq!(particles.len(), MOST);
    }

    #[test]
    fn they_fall_and_go() {
        let mut particles = Particles::default();
        let mut rng = Rng::new(2);
        particles.emit(
            &Emit {
                x: 100.,
                y: 100.,
                count: 10,
                life: [30., 40.],
                ..Emit::default()
            },
            &mut rng,
        );
        for _ in 0..42 {
            particles.tick(900.);
        }
        assert!(particles.is_empty());
    }
}
