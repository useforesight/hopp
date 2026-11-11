//! 10. Ship it: a mission-control countdown, a rocket launching up through everyone's
//! screen in a cloud of smoke, then SHIPPED with confetti and a fanfare. `text` names
//! what shipped, on the rocket and under the title. Another ship it starts it again.

use std::sync::Arc;

use super::Effect;
use crate::{
    protocol::Reaction,
    ui::reactions::{
        looks::{self, FONT, Look, MONO, Words},
        particles::{Emit, Shape},
        scene::Canvas,
        stage::{Clock, Stage, Who, bezier, ease, keyframes, rgb},
        synth::{Filter, Graph, Id, Noise, Tone, Wave},
    },
};

const THREE: f32 = 0.25;
const IGNITE: f32 = 2.65;
const LIFT: f32 = 3.45;
const FLIGHT: f32 = 1.9;
const SHIPPED: f32 = 5.2;
const DONE: f32 = 8.6;
const TAG: &str = "ship_it";
/// The title's size, and how far its letters are pulled together.
const TITLE: f32 = 150.;
const SPACING: f32 = -3.;
const STARS: [[u8; 3]; 3] = [rgb(0xffd166), rgb(0xffffff), rgb(0xffe58a)];

/// A puff of smoke, spreading and fading.
struct Puff {
    started: f32,
    seconds: f32,
    at: [f32; 2],
    to: [f32; 2],
    size: f32,
    big: bool,
    warm: bool,
}

/// One of SHIPPED's letters (or its rocket), dropping in.
struct Letter {
    look: Arc<Look>,
    /// Its middle, from the title's middle.
    at: [f32; 2],
    delay: f32,
}

/// A countdown number: crisp, with its glow drawn a quarter size.
struct Number {
    words: Words,
    glow: Arc<Look>,
}

pub struct Ship {
    /// When this launch began on the effect's clock: another ship it starts it again.
    began: f32,
    /// Seconds since this launch began.
    now: f32,
    scale: f32,
    /// This frame's rocket: shaken sideways, raised, squatting.
    jx: f32,
    dy: f32,
    sx: f32,
    flame: [f32; 2],
    glow: f32,
    last_puff: f32,
    puffs: Vec<Puff>,
    rocket: Arc<Look>,
    outer: Arc<Look>,
    inner: Arc<Look>,
    vignette: Arc<Look>,
    lights: Arc<Look>,
    smoke: [Arc<Look>; 2],
    caption: Arc<Look>,
    numbers: Vec<Number>,
    letters: Vec<Letter>,
    line: Arc<Look>,
}

impl Ship {
    pub fn new(stage: &mut Stage, who: &Who, text: &str) -> Self {
        let name: String = text.trim().chars().take(12).collect();
        let sound = stage.sound();
        // Countdown: 3, 2, 1 with blips; the last one higher.
        for i in 0..3 {
            let when = THREE + i as f32 * 0.8;
            let last = i == 2;
            sound.tone(Tone {
                when,
                freq: if last { 1760. } else { 880. },
                wave: Wave::Square,
                dur: if last { 0.32 } else { 0.11 },
                gain: 0.08,
                attack: 0.003,
                wet: 0.2,
                ..Tone::default()
            });
            sound.tone(Tone {
                when,
                freq: if last { 3520. } else { 1760. },
                dur: 0.08,
                gain: 0.03,
                ..Tone::default()
            });
        }
        rumble(stage.tagged(TAG));
        let caption = if name.is_empty() {
            "T-MINUS".to_owned()
        } else {
            format!("T-MINUS · {}", name.to_uppercase())
        };
        let said = if name.is_empty() {
            "is shipping it".to_owned()
        } else {
            format!("is shipping {name}")
        };
        stage.banner(who, &said, "🚀", 3000.);
        let (letters, line) = title(who, &name);
        Self {
            began: 0.,
            now: 0.,
            scale: (stage.h / 900.).clamp(0.75, 1.3),
            jx: 0.,
            dy: 420.,
            sx: 1.,
            flame: [0.2, 0.],
            glow: 0.,
            last_puff: f32::NEG_INFINITY,
            puffs: Vec::new(),
            rocket: rocket_look(if name.is_empty() { "HOPP" } else { &name }),
            outer: flame_look(false),
            inner: flame_look(true),
            vignette: vignette_look(),
            lights: lights_look(),
            smoke: [smoke_look(false), smoke_look(true)],
            caption: caption_look(&caption),
            numbers: ["3", "2", "1"].map(number).into(),
            letters,
            line,
        }
    }

    /// Where the bottom of the rocket's box is.
    fn base(&self, stage: &Stage) -> [f32; 2] {
        [stage.w / 2. + self.jx, stage.h - 74. + self.dy]
    }

    fn puff(&mut self, stage: &mut Stage, at: [f32; 2], big: bool, warm: bool) {
        let size = if big {
            stage.rand(90., 170.)
        } else {
            stage.rand(36., 70.)
        };
        let seconds = if big {
            stage.rand(2.4, 3.4)
        } else {
            stage.rand(1.5, 2.3)
        };
        let to = if big {
            let side = stage.rng.sign();
            let w = stage.w;
            [side * stage.rand(140., w * 0.42), stage.rand(-80., 10.)]
        } else {
            [stage.rand(-40., 40.), stage.rand(10., 60.)]
        };
        self.puffs.push(Puff {
            started: self.now,
            seconds,
            at,
            to,
            size,
            big,
            warm,
        });
    }

    fn shipped(&mut self, stage: &mut Stage) {
        fanfare(stage.sound());
        stage.flash(rgb(0xffffff), 450., 0.35);
        let [x, y] = title_middle(stage);
        let y = y - TITLE_HEIGHT / 2. + TITLE / 2.;
        stage.emit(Emit {
            x,
            y,
            count: 120,
            angle: -90.,
            spread: 360.,
            speed: [6., 18.],
            gravity: 0.3,
            life: [90., 150.],
            size: [7., 12.],
            ..Emit::default()
        });
        stage.emit(Emit {
            x,
            y,
            count: 30,
            angle: 0.,
            spread: 360.,
            speed: [3., 10.],
            gravity: 0.1,
            drag: 0.95,
            life: [50., 90.],
            size: [10., 16.],
            shape: Shape::Star,
            colours: &STARS,
            glow: true,
            ..Emit::default()
        });
    }

    fn cannon(stage: &mut Stage, x: f32, angle: f32) {
        stage.emit(Emit {
            x,
            y: stage.h + 10.,
            count: 90,
            angle,
            spread: 26.,
            speed: [16., 26.],
            gravity: 0.32,
            life: [120., 190.],
            size: [7., 12.],
            ..Emit::default()
        });
    }

    fn draw_rocket(&self, stage: &Stage, canvas: &mut Canvas) {
        let [bx, by] = self.base(stage);
        let (sx, sy) = (self.scale * self.sx, self.scale / self.sx);
        // The flame hangs from the nozzle, flickering.
        let flicker = |seconds: f32| {
            let p = (self.now / seconds).rem_euclid(2.);
            ease(if p < 1. { p } else { 2. - p })
        };
        let [fx, fy] = self.flame;
        if fy > 0.01 {
            let top = by - (280. - 214.) * sy;
            let (w, h) = (52. * fx * sx, 130. * fy * sy);
            let k = flicker(0.07);
            let (kx, ky) = (1.05 - 0.11 * k, 0.88 + 0.22 * k);
            let middle = [bx, top + h / 2.];
            let [lw, lh] = self.outer.size;
            let (ow, oh) = (w * kx * lw / 52., h * ky * lh / 130.);
            canvas.picture_in(
                &self.outer,
                [middle[0] - ow / 2., middle[1] - oh / 2.],
                [ow, oh],
                1.,
            );
            let k = flicker(0.05);
            let (kx, ky) = (1.05 - 0.11 * k, 0.88 + 0.22 * k);
            let (iw, ih) = (26. / 52. * w * kx, 0.55 * h * ky);
            let middle = [bx, top + 0.55 * h / 2.];
            canvas.picture_in(
                &self.inner,
                [middle[0] - iw / 2., middle[1] - ih / 2.],
                [iw, ih],
                1.,
            );
        }
        canvas.picture_in(
            &self.rocket,
            [bx - 60. * sx, by - 280. * sy],
            [120. * sx, 280. * sy],
            1.,
        );
    }

    fn draw_puffs(&self, canvas: &mut Canvas) {
        for puff in &self.puffs {
            let k = bezier(0.15, 0.7, 0.3, 1., (self.now - puff.started) / puff.seconds);
            let travel = keyframes(&[(0., 0.), (0.4, 0.6), (1., 1.)], k, |k| k);
            let grown = if puff.big {
                keyframes(&[(0., 0.3), (0.4, 2.2), (1., 3.2)], k, |k| k)
            } else {
                keyframes(&[(0., 0.3), (0.4, 1.6), (1., 2.6)], k, |k| k)
            };
            let alpha = keyframes(&[(0., 0.95), (0.4, 0.8), (1., 0.)], k, |k| k);
            let size = puff.size * grown;
            let at = [
                puff.at[0] + puff.to[0] * travel,
                puff.at[1] + puff.to[1] * travel,
            ];
            canvas.picture_in(
                &self.smoke[usize::from(puff.warm)],
                [at[0] - size / 2., at[1] - size / 2.],
                [size, size],
                alpha,
            );
        }
    }

    fn draw_countdown(&self, stage: &Stage, canvas: &mut Canvas) {
        let middle = [stage.w / 2., stage.h * 0.46];
        for (i, number) in self.numbers.iter().enumerate() {
            let since = self.now - (THREE + i as f32 * 0.8);
            if !(0. ..0.8).contains(&since) {
                continue;
            }
            let k = bezier(0.2, 0.9, 0.3, 1., since / 0.78);
            let scale = keyframes(&[(0., 1.7), (0.22, 1.), (0.75, 0.94), (1., 0.6)], k, |k| k);
            let alpha = keyframes(&[(0., 0.), (0.22, 1.), (0.75, 1.), (1., 0.)], k, |k| k);
            let size = stage.w * 0.4;
            let font = size.min(380.);
            // The digit's middle sits a little above its line's.
            let centre = [middle[0], middle[1] - font * 0.05];
            canvas.picture(&number.glow, centre, 4. * scale * font / 380., alpha);
            let words = &number.words;
            let s = scale * font / 380.;
            let [w, h] = words.look.size;
            canvas.picture_in(
                &words.look,
                [
                    centre[0] - words.anchor[0] * s,
                    centre[1] - words.anchor[1] * s,
                ],
                [w * s, h * s],
                alpha,
            );
            // The ring going out from it.
            let k = bezier(0.1, 0.7, 0.3, 1., since / 0.75);
            let ring = 0.4 + 2.2 * k;
            canvas.ring(
                middle,
                148.5 * ring,
                3. * ring,
                rgb(0x8cc8ff),
                0.8 * (1. - k),
            );
        }
    }

    fn draw_title(&self, stage: &Stage, canvas: &mut Canvas) {
        let since = self.now - SHIPPED;
        if since < 0. {
            return;
        }
        let middle = title_middle(stage);
        // Away again before the end: up a little, smaller, fading.
        let out = since - (DONE - SHIPPED - 0.8);
        let (shown, lift, shrink) = if out > 0. {
            let k = bezier(0.5, 0., 0.8, 0.4, out / 0.6);
            (1. - ease(out / 0.5), TITLE_HEIGHT * 0.1 * k, 1. - 0.08 * k)
        } else {
            (1., 0., 1.)
        };
        let place = |p: [f32; 2]| [middle[0] + p[0] * shrink, middle[1] + p[1] * shrink - lift];
        for letter in &self.letters {
            let k = bezier(0.2, 1.7, 0.4, 1., (since - letter.delay) / 0.7);
            if since < letter.delay {
                continue;
            }
            let at = place([letter.at[0], letter.at[1] - 120. * (1. - k)]);
            let s = (0.4 + 0.6 * k) * shrink;
            canvas.picture(&letter.look, at, s, k.min(1.) * shown);
        }
        let k = bezier(0.2, 1.4, 0.4, 1., (since - 0.55) / 0.5);
        if since > 0.55 {
            let at = place([0., TITLE_HEIGHT / 2. - 13.2 + 16. * (1. - k)]);
            canvas.picture(&self.line, at, shrink, (0.92 * k).min(0.92) * shown);
        }
    }
}

/// The title's block: SHIPPED, a gap, and the line under it.
const TITLE_HEIGHT: f32 = TITLE + 10. + 26.4;

fn title_middle(stage: &Stage) -> [f32; 2] {
    [stage.w / 2., stage.h * 0.4]
}

/// SHIPPED's letters and its rocket, laid out from the title's middle, and the line that
/// goes under it.
fn title(who: &Who, name: &str) -> (Vec<Letter>, Arc<Look>) {
    let word = "SHIPPED";
    let y = -TITLE_HEIGHT / 2. + TITLE / 2.;
    let mut letters = Vec::new();
    let mut middles = Vec::new();
    let mut end = 0.;
    for (i, ch) in word.chars().enumerate() {
        // Each letter ends where the word so far does.
        let prefix: String = word.chars().take(i + 1).collect();
        end = looks::text_width(&prefix, TITLE, 900, FONT) + SPACING * i as f32;
        let ink = looks::text_width(&ch.to_string(), TITLE, 900, FONT);
        middles.push((ch, end - ink / 2.));
    }
    let space = TITLE * 0.28;
    let emoji = TITLE * 1.2;
    let width = end + space + emoji;
    for (i, (ch, x)) in middles.into_iter().enumerate() {
        letters.push(Letter {
            look: letter_look(ch),
            at: [x - width / 2., y],
            delay: i as f32 * 0.045,
        });
    }
    letters.push(Letter {
        look: looks::emoji_sized("🚀", TITLE * 128. / 104.),
        at: [end + space + emoji / 2. - width / 2., y],
        delay: 0.34,
    });
    let by = match (who.mine, name.is_empty()) {
        (true, false) => "you",
        (true, true) => "You",
        (false, _) => &who.name,
    };
    (letters, line_look(name, &format!("{by} shipped it")))
}

/// A letter of SHIPPED: white to gold to orange, with a warm glow below.
fn letter_look(ch: char) -> Arc<Look> {
    let ink = looks::text_width(&ch.to_string(), TITLE, 900, FONT);
    let pad = 44.;
    let (w, h) = (ink + pad * 2., TITLE + pad * 2.);
    Look::new(
        format!("ship:letter:{ch}"),
        [w, h],
        format!(
            r##"<defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="0" y1="{pad}" x2="0" y2="{bottom}"><stop offset=".2" stop-color="#fff"/><stop offset=".6" stop-color="#ffd166"/><stop offset="1" stop-color="#ff7a2b"/></linearGradient><filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="6" stdDeviation="12" flood-color="#ff8c28" flood-opacity=".55"/></filter></defs><text x="{x}" y="{baseline}" text-anchor="middle" font-family="{FONT}" font-size="{TITLE}" font-weight="900" fill="url(#g)" filter="url(#s)">{ch}</text>"##,
            bottom = pad + TITLE,
            x = w / 2.,
            baseline = pad + TITLE * 0.84,
        ),
    )
}

/// "`v2.0` is out · Sam shipped it", the name in a code pill.
fn line_look(name: &str, said: &str) -> Arc<Look> {
    let size = 22.;
    let h = size * 1.2 + 8.;
    let baseline = h / 2. + size * 0.36;
    let mut body = String::new();
    let mut x = 4.;
    let rest = if name.is_empty() {
        said.to_owned()
    } else {
        let code = looks::text_width(name, size, 800, MONO);
        body.push_str(&format!(
            r##"<rect x="{x}" y="{y}" width="{w}" height="{ph}" rx="8" fill="#fff" fill-opacity=".14"/><text x="{tx}" y="{baseline}" font-family="{MONO}" font-size="{size}" font-weight="800" fill="#fff">{}</text>"##,
            looks::escape(name),
            y = h / 2. - size * 0.7,
            w = code + 20.,
            ph = size * 1.4,
            tx = x + 10.,
        ));
        x += code + 20. + size * 0.28;
        format!("is out · {said}")
    };
    let width = looks::text_width(&rest, size, 700, FONT);
    body.push_str(&format!(
        r##"<text x="{x}" y="{baseline}" font-family="{FONT}" font-size="{size}" font-weight="700" fill="#fff">{}</text>"##,
        looks::escape(&rest),
    ));
    Look::new(
        format!("ship:line:{name}:{said}"),
        [x + width + 8., h],
        body,
    )
}

/// The countdown's "3", "2" and "1" in mission-control blue light.
fn number(digit: &str) -> Number {
    let words = looks::words(digit, 380., 800, MONO, "#fff", "", None);
    // Its glow at a quarter size: it's all blur.
    let font = 95.;
    let (w, h) = (font * 0.62 + 70., font * 1.3 + 70.);
    let glow = Look::new(
        format!("ship:glow:{digit}"),
        [w, h],
        format!(
            r##"<defs><filter id="a" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="11.25"/></filter><filter id="b" x="-1" y="-1" width="3" height="3"><feGaussianBlur stdDeviation="3.75"/></filter></defs><text x="{x}" y="{y}" text-anchor="middle" font-family="{MONO}" font-size="{font}" font-weight="800" fill="#3c8cff" fill-opacity=".6" filter="url(#a)">{digit}</text><text x="{x}" y="{y}" text-anchor="middle" font-family="{MONO}" font-size="{font}" font-weight="800" fill="#78beff" fill-opacity=".9" filter="url(#b)">{digit}</text>"##,
            x = w / 2.,
            y = h / 2. + font * 0.36,
        ),
    );
    Number { words, glow }
}

/// "T-MINUS · V2.0", spaced out.
fn caption_look(text: &str) -> Arc<Look> {
    let spacing = 6.;
    let width = looks::text_width(text, 15., 700, MONO) + spacing * text.chars().count() as f32;
    Look::new(
        format!("ship:caption:{text}"),
        [width + 8., 24.],
        format!(
            r##"<text x="4" y="17" font-family="{MONO}" font-size="15" font-weight="700" letter-spacing="{spacing}" fill="#8fd0ff">{}</text>"##,
            looks::escape(text),
        ),
    )
}

/// The rocket, `label` up its side.
fn rocket_look(label: &str) -> Arc<Look> {
    Look::new(
        format!("ship:rocket:{label}"),
        [120., 280.],
        format!(
            r##"<defs>
<linearGradient id="b" x1="0" x2="1"><stop offset="0" stop-color="#8e95a3"/><stop offset=".3" stop-color="#fff"/><stop offset=".62" stop-color="#e3e7ee"/><stop offset="1" stop-color="#7d8494"/></linearGradient>
<linearGradient id="r" x1="0" x2="1"><stop offset="0" stop-color="#8a0d1d"/><stop offset=".35" stop-color="#ff5a6a"/><stop offset=".7" stop-color="#df2a3e"/><stop offset="1" stop-color="#6d0915"/></linearGradient>
<linearGradient id="n" x1="0" x2="1"><stop offset="0" stop-color="#2c303a"/><stop offset=".45" stop-color="#8a909c"/><stop offset="1" stop-color="#23262e"/></linearGradient>
<radialGradient id="w" cx=".38" cy=".35" r=".75"><stop offset="0" stop-color="#d8f3ff"/><stop offset=".45" stop-color="#3aa0ff"/><stop offset="1" stop-color="#0a2f73"/></radialGradient>
<clipPath id="c"><path d="M60 6C90 38 97 92 96 202H24C23 92 30 38 60 6Z"/></clipPath>
</defs>
<path d="M27 136C8 154 2 196 5 236L27 214Z" fill="url(#r)"/>
<path d="M93 136C112 154 118 196 115 236L93 214Z" fill="url(#r)"/>
<path d="M38 202H82L89 224H31Z" fill="url(#n)"/>
<path d="M60 6C90 38 97 92 96 202H24C23 92 30 38 60 6Z" fill="url(#b)"/>
<g clip-path="url(#c)"><rect x="0" y="0" width="120" height="74" fill="url(#r)"/><rect x="0" y="74" width="120" height="5" fill="#2a2f3b" opacity=".55"/><rect x="0" y="190" width="120" height="12" fill="url(#r)"/></g>
<circle cx="60" cy="102" r="17" fill="#c9ced8" stroke="#7d8494" stroke-width="1.5"/>
<circle cx="60" cy="102" r="12" fill="url(#w)"/>
<ellipse cx="55.5" cy="97" rx="4.5" ry="2.8" fill="#fff" opacity=".85" transform="rotate(-30 55.5 97)"/>
<text transform="translate(64.5 156) rotate(-90)" text-anchor="middle" font-family="{MONO}" font-size="12.5" font-weight="800" fill="#1d2433" letter-spacing=".5">{}</text>
<rect x="56" y="196" width="8" height="40" rx="3" fill="#7a0b19"/>"##,
            looks::escape(label),
        ),
    )
}

/// The flame (`inner`: its white-hot core), a box with round corners, longer below.
fn flame_look(inner: bool) -> Arc<Look> {
    let (w, h) = if inner { (26., 71.5) } else { (52., 130.) };
    // Room for the outer flame's blur.
    let pad = if inner { 0. } else { 6. };
    let (tx, ty, bx, by) = (w * 0.45, h * 0.2, w * 0.5, h * 0.8);
    let shape = format!(
        "M{l} {t}m{tx} 0H{r1}A{tx} {ty} 0 0 1 {r} {t1}V{b1}A{bx} {by} 0 0 1 {m} {b}A{bx} {by} 0 0 1 {l} {b1}V{t1}A{tx} {ty} 0 0 1 {l2} {t}Z",
        l = pad,
        t = pad,
        r = pad + w,
        r1 = pad + w - tx,
        t1 = pad + ty,
        b = pad + h,
        b1 = pad + h - by,
        m = pad + w / 2.,
        l2 = pad + tx,
    );
    let (fill, blur) = if inner {
        (
            format!(
                r##"<radialGradient id="f" gradientUnits="userSpaceOnUse" cx="0" cy="0" r="1" gradientTransform="translate({cx} {cy}) scale({rx} {ry})"><stop offset="0" stop-color="#fff"/><stop offset=".5" stop-color="#bfe8ff"/><stop offset="1" stop-color="#8cc8ff" stop-opacity="0"/></radialGradient>"##,
                cx = w / 2.,
                cy = h * 0.1,
                rx = w / 2.,
                ry = h * 0.7,
            ),
            "",
        )
    } else {
        (
            format!(
                r##"<radialGradient id="f" gradientUnits="userSpaceOnUse" cx="0" cy="0" r="1" gradientTransform="translate({cx} {cy}) scale({rx} {ry})"><stop offset="0" stop-color="#fff7d6"/><stop offset=".45" stop-color="#ffb02e"/><stop offset=".7" stop-color="#ff4614" stop-opacity=".85"/><stop offset="1" stop-color="#ff2800" stop-opacity="0"/></radialGradient><filter id="s" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="2"/></filter>"##,
                cx = pad + w / 2.,
                cy = pad + h * 0.15,
                rx = w / 2.,
                ry = h * 0.7,
            ),
            r#" filter="url(#s)""#,
        )
    };
    Look::new(
        format!("ship:flame:{inner}"),
        [w + pad * 2., h + pad * 2.],
        format!(r#"<defs>{fill}</defs><path d="{shape}" fill="url(#f)"{blur}/>"#),
    )
}

/// The countdown's darkened edges.
fn vignette_look() -> Arc<Look> {
    Look::new(
        "ship:vignette",
        [160., 100.],
        r##"<defs><radialGradient id="v" cx=".5" cy=".55" r=".7"><stop offset="0" stop-color="#040814" stop-opacity=".25"/><stop offset="1" stop-color="#02040c" stop-opacity=".82"/></radialGradient></defs><rect width="160" height="100" fill="url(#v)"/>"##,
    )
}

/// The engines' light on the ground, a quarter size.
fn lights_look() -> Arc<Look> {
    Look::new(
        "ship:lights",
        [175., 90.],
        r##"<defs><radialGradient id="l"><stop offset="0" stop-color="#ffaa3c" stop-opacity=".85"/><stop offset=".55" stop-color="#ff5a14" stop-opacity=".35"/><stop offset="1" stop-color="#ff3c00" stop-opacity="0"/></radialGradient></defs><rect width="175" height="90" fill="url(#l)"/>"##,
    )
}

fn smoke_look(warm: bool) -> Arc<Look> {
    let stops = if warm {
        r##"<stop offset="0" stop-color="#ffe2be" stop-opacity=".95"/><stop offset=".45" stop-color="#f0c8aa" stop-opacity=".7"/><stop offset="1" stop-color="#d2b4aa" stop-opacity="0"/>"##
    } else {
        r##"<stop offset="0" stop-color="#faf8fc" stop-opacity=".95"/><stop offset=".45" stop-color="#e4e0ea" stop-opacity=".75"/><stop offset="1" stop-color="#c8c4d2" stop-opacity="0"/>"##
    };
    Look::new(
        format!("ship:smoke:{warm}"),
        [64., 64.],
        format!(
            r#"<defs><radialGradient id="p">{stops}</radialGradient></defs><circle cx="32" cy="32" r="32" fill="url(#p)"/>"#
        ),
    )
}

// ---- Sound ----------------------------------------------------------------------------

fn midi(n: f32) -> f32 {
    440. * 2f32.powf((n - 69.) / 12.)
}

/// Brass-ish: three detuned saws per note through a lowpass that swells open.
fn brass(sound: &mut Graph, freq: f32, t: f32, dur: f32, gain: f32, out: Id) {
    let f = sound.filter(Filter::Lowpass, 400., 1.5);
    sound
        .frequency(f)
        .set(400., t)
        .exp(3200., t + 0.09)
        .exp(1500., t + dur);
    let g = sound.gain(1.);
    sound
        .gain_of(g)
        .set(0.0001, t)
        .exp(gain, t + 0.04)
        .set(gain, t + dur * 0.7)
        .exp(0.0001, t + dur);
    sound.connect(f, g);
    sound.connect(g, out);
    for detune in [-11., 0., 9.] {
        let o = sound.osc(Wave::Saw, freq);
        sound.detune(o).set(detune, 0.);
        sound.connect(o, f);
        sound.play(o, t, t + dur + 0.05);
    }
}

fn fanfare(sound: &mut Graph) {
    let out = sound.dest(0.35);
    for when in [0., 0.13, 0.26] {
        for n in [67., 72.] {
            brass(sound, midi(n), when, 0.12, 0.07, out);
        }
    }
    for n in [60., 64., 67., 72., 76.] {
        brass(sound, midi(n), 0.4, 1.9, 0.065, out);
    }
    brass(sound, midi(48.), 0.4, 1.9, 0.09, out);
    sound.tone(Tone {
        when: 0.4,
        freq: midi(84.),
        gain: 0.05,
        dur: 1.6,
        wet: 0.5,
        ..Tone::default()
    });
    // Cymbal.
    sound.noise(Noise {
        when: 0.4,
        dur: 1.8,
        gain: 0.12,
        filter: Filter::Highpass,
        freq: 6000.,
        wet: 0.4,
        ..Noise::default()
    });
    sound.tone(Tone {
        when: 0.4,
        freq: 120.,
        glide_to: Some(45.),
        dur: 0.5,
        gain: 0.5,
        wet: 0.1,
        ..Tone::default()
    });
}

/// The engines' rumble, swelling at ignition and lift-off and gone after the title.
fn rumble(sound: &mut Graph) {
    let src = sound.noise_source(true);
    let lp = sound.filter(Filter::Lowpass, 90., 0.8);
    let bp = sound.filter(Filter::Bandpass, 900., 0.6);
    let g = sound.gain(1.);
    let cg = sound.gain(1.);
    let sub = sound.osc(Wave::Sine, 42.);
    let sg = sound.gain(0.25);
    sound.connect(src, lp);
    sound.connect(lp, g);
    sound.connect(src, bp);
    sound.connect(bp, cg);
    sound.connect(cg, g);
    sound.connect(sub, sg);
    sound.connect(sg, g);
    let out = sound.dest(0.2);
    sound.connect(g, out);
    sound
        .gain_of(g)
        .set(0.0001, 0.)
        .exp(0.25, IGNITE)
        .exp(1.1, LIFT + 0.3)
        .set(1.1, LIFT + 0.8)
        .exp(0.0001, SHIPPED + 1.);
    sound
        .gain_of(cg)
        .set(0.0001, 0.)
        .exp(0.25, LIFT + 0.4)
        .exp(0.02, SHIPPED);
    sound.frequency(lp).set(70., 0.).exp(420., LIFT + 0.5);
    let end = SHIPPED + 1.1;
    sound.play(src, 0., end);
    sound.play(sub, 0., end);
}

impl Effect for Ship {
    fn step(&mut self, stage: &mut Stage, clock: Clock) -> bool {
        let clock = Clock {
            before: clock.before - self.began,
            now: clock.now - self.began,
        };
        let t = clock.now;
        self.now = t;
        if clock.crossed(IGNITE) {
            let sound = stage.sound();
            // Whoomp.
            sound.noise(Noise {
                dur: 0.6,
                gain: 0.5,
                filter: Filter::Lowpass,
                freq: 3000.,
                sweep_to: Some(200.),
                attack: 0.01,
                wet: 0.2,
                ..Noise::default()
            });
            sound.tone(Tone {
                freq: 110.,
                glide_to: Some(38.),
                dur: 0.6,
                gain: 0.5,
                wet: 0.1,
                ..Tone::default()
            });
            stage.flash(rgb(0xffb36b), 300., 0.18);
        }
        if clock.crossed(LIFT) {
            // Whoosh.
            stage.sound().noise(Noise {
                dur: 1.6,
                gain: 0.18,
                filter: Filter::Bandpass,
                freq: 400.,
                sweep_to: Some(3000.),
                q: 0.7,
                attack: 0.5,
                wet: 0.25,
                ..Noise::default()
            });
        }
        if clock.crossed(SHIPPED) {
            self.shipped(stage);
        }
        if clock.crossed(SHIPPED + 0.12) {
            Self::cannon(stage, 0., -62.);
        }
        if clock.crossed(SHIPPED + 0.2) {
            let w = stage.w;
            Self::cannon(stage, w, -118.);
        }

        // The rocket rises into place, squats and shakes as it fires, and goes.
        let (mut dy, mut jx, mut sx) = (0., 0., 1.);
        let mut shake = 0.;
        if t < 0.7 {
            let p = (t / 0.7).clamp(0., 1.);
            let back = 1. + 2.2 * (p - 1.).powi(3) + 1.2 * (p - 1.).powi(2);
            dy = 420. * (1. - back);
        }
        if (IGNITE..LIFT).contains(&t) {
            let k = (t - IGNITE) / (LIFT - IGNITE);
            jx = stage.rand(-1.5, 1.5) * k;
            dy = 4. * k;
            sx = 1. + 0.03 * k;
            self.flame = [0.55 + 0.45 * k, 0.4 + 0.6 * k];
            shake = 2. + 7. * k;
            self.glow = 0.4 + 0.6 * k + stage.rand(-0.1, 0.1);
        } else if t >= LIFT {
            let k = ((t - LIFT) / FLIGHT).clamp(0., 1.);
            dy = -(stage.h + 360. * self.scale) * k.powf(2.4);
            jx = stage.rand(-1., 1.) * (1. - k);
            self.flame = [1. + 0.15 * k, 1.25 + 1.1 * k];
            shake = if t < LIFT + 0.9 {
                9.
            } else {
                (9. * (1. - (t - LIFT - 0.9) / 1.2)).max(0.)
            };
            self.glow = (1. - k * 2.2).max(0.);
        }
        (self.dy, self.jx, self.sx) = (dy, jx, sx);
        if (IGNITE..LIFT).contains(&t) && t - self.last_puff > 0.055 {
            self.last_puff = t;
            let k = (t - IGNITE) / (LIFT - IGNITE);
            let x = self.base(stage)[0];
            for i in 0..3 {
                let at = [x + stage.rand(-30., 30.), stage.h - stage.rand(10., 60.)];
                self.puff(stage, at, true, k < 0.3 && i == 0);
            }
        } else if t >= LIFT && t - self.last_puff > 0.038 {
            let k = ((t - LIFT) / FLIGHT).clamp(0., 1.);
            if k < 1. {
                self.last_puff = t;
                let [x, by] = self.base(stage);
                let y = by - 280. * self.scale + 214. * self.scale;
                if y < stage.h + 40. {
                    let at = [x + stage.rand(-8., 8.), y + stage.rand(0., 30.)];
                    self.puff(stage, at, k < 0.12, false);
                }
            }
        }
        let now = self.now;
        self.puffs.retain(|puff| now - puff.started < puff.seconds);
        if shake > 0.2 {
            stage.shake(shake, 40.);
        }
        t < DONE
    }

    fn draw(&self, stage: &Stage, canvas: &mut Canvas) {
        let t = self.now;
        let vignette = if t < SHIPPED {
            ease(t / 0.5)
        } else {
            1. - ease((t - SHIPPED) / 0.5)
        };
        canvas.picture_in(
            &self.vignette,
            [-40., -40.],
            [stage.w + 80., stage.h + 80.],
            vignette,
        );
        canvas.picture_in(
            &self.lights,
            [stage.w / 2. - 350., stage.h - 200.],
            [700., 360.],
            self.glow.clamp(0., 1.),
        );
        self.draw_rocket(stage, canvas);
        let caption = if t < IGNITE {
            ease(t / 0.3)
        } else {
            1. - ease((t - IGNITE) / 0.3)
        };
        let [w, h] = self.caption.size;
        canvas.picture_in(
            &self.caption,
            [
                stage.w / 2. - w / 2.,
                stage.h * 0.46 - (stage.w * 0.22).min(210.) - 3.,
            ],
            [w, h],
            0.9 * caption,
        );
        self.draw_puffs(canvas);
        self.draw_countdown(stage, canvas);
        self.draw_title(stage, canvas);
    }

    fn again(&mut self, stage: &mut Stage, who: &Who, reaction: &Reaction) -> bool {
        // A new launch stops the one playing, as the prototype's does.
        stage.hush(TAG);
        let began = self.began + self.now;
        *self = Self::new(stage, who, reaction.text.as_deref().unwrap_or(""));
        self.began = began;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::reactions::{Player, scene};

    /// SHIPPED comes after the shared previews' last picture: with
    /// `HOPP_REACTIONS_PREVIEW` set, it's drawn there too.
    #[test]
    fn the_title_previews() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut player = Player::new(1440., 900., 2);
        player.play(
            &alex,
            &Reaction {
                text: Some("v2.0".into()),
                ..Reaction::new("ship_it")
            },
        );
        let mut backdrop = resvg::tiny_skia::Pixmap::new(1440, 900).unwrap();
        backdrop.fill(resvg::tiny_skia::Color::from_rgba8(0x5a, 0x34, 0x4e, 0xff));
        for step in 1..=60 * 7 {
            player.advance(step as f32 / 60.);
            let _ = player.stage.take_sounds();
            if ![5.5, 6.].contains(&(step as f32 / 60.)) {
                continue;
            }
            let pixmap = scene::preview(&player.frame(), 1440, 900, 1., Some(&backdrop));
            let Some(dir) = std::env::var_os("HOPP_REACTIONS_PREVIEW") else {
                continue;
            };
            let rgba: Vec<u8> = pixmap
                .pixels()
                .iter()
                .flat_map(|pixel| {
                    let colour = pixel.demultiply();
                    [colour.red(), colour.green(), colour.blue(), colour.alpha()]
                })
                .collect();
            let path = std::path::Path::new(&dir)
                .join(format!("ship_it-title-{}s.png", step as f32 / 60.));
            image::save_buffer(path, &rgba, 1440, 900, image::ExtendedColorType::Rgba8).unwrap();
        }
    }

    #[test]
    fn it_launches_ships_and_goes() {
        let alex = Who::new("a", "Alex Morgan", [1, 2, 3], false);
        let mut stage = Stage::new(1440., 900., 3);
        let mut ship = Ship::new(&mut stage, &alex, "  a very long release name ");
        let mut before = 0.;
        let mut highest = f32::INFINITY;
        let mut gone = None;
        for step in 1..=60 * 12 {
            let now = step as f32 / 60.;
            stage.now = now;
            stage.ticks = 1;
            if !ship.step(&mut stage, Clock { before, now }) {
                gone = Some(now);
                break;
            }
            highest = highest.min(ship.base(&stage)[1]);
            before = now;
        }
        assert!(highest < 0., "the rocket never left");
        assert!(!stage.particles.is_empty(), "no confetti");
        assert!((DONE..DONE + 0.05).contains(&gone.unwrap()));
    }

    #[test]
    fn what_shipped_is_named_short() {
        let you = Who::new("a", "Sam Rivera", [1, 2, 3], true);
        let mut stage = Stage::new(1440., 900., 3);
        let ship = Ship::new(&mut stage, &you, "release-candidate-2");
        assert_eq!(ship.rocket.key, "ship:rocket:release-cand");
        assert!(ship.line.key.ends_with(":you shipped it"));
    }
}
