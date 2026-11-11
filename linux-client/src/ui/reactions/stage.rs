//! What every effect plays on: the screen's size and the call window on it, the
//! particles, shakes, flashes and "Sam hit the gong" banners, and this frame's sounds.

use std::sync::Arc;

use super::{
    looks::{self, FONT, Look},
    particles::{Emit, Particles},
    scene::{Canvas, Item},
    synth::Graph,
};

/// A small, quick random source (SplitMix64), seeded so previews come out the same.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// From 0 up to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// `H.rand`.
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + self.unit() * (b - a)
    }

    /// `H.pick`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[((self.unit() * items.len() as f32) as usize).min(items.len() - 1)]
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// −1 or 1.
    pub fn sign(&mut self) -> f32 {
        if self.chance(0.5) { -1. } else { 1. }
    }
}

/// An effect's time: where it was last frame and where it is now, in seconds since it
/// began. What the prototypes do with `setTimeout`, an effect does when its clock
/// crosses that time.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub before: f32,
    pub now: f32,
}

impl Clock {
    pub fn crossed(&self, at: f32) -> bool {
        self.before < at && at <= self.now
    }
}

pub const fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

pub fn mix(a: [u8; 3], b: [u8; 3], k: f32) -> [u8; 3] {
    let k = k.clamp(0., 1.);
    [0, 1, 2].map(|i| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * k).round() as u8)
}

pub fn clamp01(t: f32) -> f32 {
    t.clamp(0., 1.)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// CSS's `cubic-bezier(x1, y1, x2, y2)` at `t`.
pub fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = clamp01(t);
    let curve = |a: f32, b: f32, s: f32| {
        3. * a * s * (1. - s).powi(2) + 3. * b * s * s * (1. - s) + s.powi(3)
    };
    // Find s where x(s) = t, by bisection: monotonic for x1, x2 in [0, 1].
    let (mut low, mut high) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (low + high) / 2.;
        if curve(x1, x2, mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    curve(y1, y2, (low + high) / 2.)
}

/// CSS's `ease-out`.
pub fn ease_out(t: f32) -> f32 {
    bezier(0., 0., 0.58, 1., t)
}

/// CSS's `ease-in-out`.
pub fn ease_in_out(t: f32) -> f32 {
    bezier(0.42, 0., 0.58, 1., t)
}

/// CSS's `ease`.
pub fn ease(t: f32) -> f32 {
    bezier(0.25, 0.1, 0.25, 1., t)
}

/// A value through keyframes `(offset, value)` with `easing` between each pair, as
/// `element.animate` plays them.
pub fn keyframes(frames: &[(f32, f32)], t: f32, easing: impl Fn(f32) -> f32) -> f32 {
    let t = clamp01(t);
    let Some(first) = frames.first() else {
        return 0.;
    };
    if t <= first.0 {
        return first.1;
    }
    for pair in frames.windows(2) {
        let ((a, va), (b, vb)) = (pair[0], pair[1]);
        if t <= b {
            let k = if b > a { (t - a) / (b - a) } else { 1. };
            return lerp(va, vb, easing(k));
        }
    }
    frames[frames.len() - 1].1
}

/// Someone in the call, as the effects show them.
#[derive(Clone, Debug)]
pub struct Who {
    /// Their identity (canonical), to tell people apart.
    pub key: String,
    /// Their first name.
    pub name: String,
    pub initials: String,
    pub rgb: [u8; 3],
    /// Whether this is you.
    pub mine: bool,
}

impl Who {
    pub fn new(key: &str, full: &str, rgb: [u8; 3], mine: bool) -> Self {
        let full = full.trim();
        let name = full
            .split_whitespace()
            .next()
            .unwrap_or("Someone")
            .to_owned();
        let initials: String = full
            .split_whitespace()
            .take(2)
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect();
        Self {
            key: key.to_owned(),
            name,
            initials: if initials.is_empty() {
                "?".into()
            } else {
                initials
            },
            rgb,
            mine,
        }
    }

    /// "You" for you, their first name for anyone else: how a banner starts.
    pub fn called(&self) -> &str {
        if self.mine { "You" } else { &self.name }
    }

    /// Their avatar's two colours, lighter to deeper, as the prototypes' gradients go.
    pub fn gradient(&self) -> (String, String) {
        (
            looks::hex(mix(self.rgb, [255, 255, 255], 0.28)),
            looks::hex(mix(self.rgb, [0, 0, 0], 0.18)),
        )
    }
}

/// An avatar `size` across at (`x`, `y`) (its top left) as SVG: initials on their colour.
pub fn avatar_svg(who: &Who, x: f32, y: f32, size: f32, id: &str) -> String {
    let (from, to) = who.gradient();
    let r = size / 2.;
    format!(
        r##"<defs><linearGradient id="{id}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{from}"/><stop offset="1" stop-color="{to}"/></linearGradient></defs><circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#{id})"/><text x="{cx}" y="{ty}" text-anchor="middle" font-family="{FONT}" font-size="{fs}" font-weight="800" fill="#fff">{}</text>"##,
        looks::escape(&who.initials),
        cx = x + r,
        cy = y + r,
        ty = y + r + size * 0.42 * 0.36,
        fs = size * 0.42,
    )
}

/// A sound for this frame, played as one; `tag` lets it be stopped early.
pub struct Sound {
    pub graph: Graph,
    pub tag: Option<String>,
}

struct Shake {
    intensity: f32,
    started: f32,
    seconds: f32,
}

struct Flash {
    rgb: [u8; 3],
    opacity: f32,
    started: f32,
    seconds: f32,
}

/// "Sam hit the gong", at the top of everyone's screen.
struct Banner {
    look: Arc<Look>,
    /// The pill's own size, inside the room left for its shadow.
    pill: [f32; 2],
    started: f32,
    seconds: f32,
}

/// Room round a banner's pill for its shadow.
const BANNER_SHADOW: f32 = 56.;
/// Banners are stacked down from here; no more than this many show.
const BANNER_TOP: f32 = 44.;
const BANNER_GAP: f32 = 8.;
const MOST_BANNERS: usize = 3;

pub struct Stage {
    pub w: f32,
    pub h: f32,
    /// The call window: x, y, width, height, from the top left of the screen.
    pub call: [f32; 4],
    pub rng: Rng,
    pub particles: Particles,
    /// Everyone in the call, you among them.
    pub people: Vec<Who>,
    /// Seconds since the layer started.
    pub now: f32,
    /// 60 Hz frames passed since the last step: for things that move per frame.
    pub ticks: u32,
    shakes: Vec<Shake>,
    flashes: Vec<Flash>,
    banners: Vec<Banner>,
    sounds: Vec<Sound>,
    hushed: Vec<String>,
}

impl Stage {
    pub fn new(w: f32, h: f32, seed: u64) -> Self {
        Self {
            w,
            h,
            call: [w / 2. - 160., h - 280., 320., 240.],
            rng: Rng::new(seed),
            particles: Particles::default(),
            people: Vec::new(),
            now: 0.,
            ticks: 0,
            shakes: Vec::new(),
            flashes: Vec::new(),
            banners: Vec::new(),
            sounds: Vec::new(),
            hushed: Vec::new(),
        }
    }

    /// Where the sender's tile is: the middle of the call window.
    pub fn tile(&self) -> [f32; 2] {
        let [x, y, w, h] = self.call;
        [x + w / 2., y + h / 2.]
    }

    pub fn rand(&mut self, a: f32, b: f32) -> f32 {
        self.rng.range(a, b)
    }

    pub fn emit(&mut self, emit: Emit) {
        self.particles.emit(&emit, &mut self.rng);
    }

    /// This frame's sound, started now.
    pub fn sound(&mut self) -> &mut Graph {
        self.tagged_sound(None)
    }

    /// A sound of its own that [`Stage::hush`] can stop, started now.
    pub fn tagged(&mut self, tag: &str) -> &mut Graph {
        self.tagged_sound(Some(tag))
    }

    fn tagged_sound(&mut self, tag: Option<&str>) -> &mut Graph {
        let index = match self
            .sounds
            .iter()
            .position(|sound| sound.tag.as_deref() == tag)
        {
            Some(index) => index,
            None => {
                let seed = self.rng.next_u64();
                self.sounds.push(Sound {
                    graph: Graph::new(seed),
                    tag: tag.map(str::to_owned),
                });
                self.sounds.len() - 1
            }
        };
        &mut self.sounds[index].graph
    }

    /// Stop the sounds tagged `tag` that are playing.
    pub fn hush(&mut self, tag: &str) {
        self.sounds
            .retain(|sound| sound.tag.as_deref() != Some(tag));
        self.hushed.push(tag.to_owned());
    }

    /// The sounds started and stopped since this was last asked.
    pub fn take_sounds(&mut self) -> (Vec<Sound>, Vec<String>) {
        let sounds = std::mem::take(&mut self.sounds)
            .into_iter()
            .filter(|sound| !sound.graph.is_silent())
            .collect();
        (sounds, std::mem::take(&mut self.hushed))
    }

    /// `H.shake`: everything but the banners jumps about, less and less, for `ms`.
    pub fn shake(&mut self, intensity: f32, ms: f32) {
        self.shakes.push(Shake {
            intensity,
            started: self.now,
            seconds: ms / 1000.,
        });
    }

    /// `H.flash`: the whole screen in `rgb` at `opacity`, fading over `ms`.
    pub fn flash(&mut self, rgb: [u8; 3], ms: f32, opacity: f32) {
        self.flashes.push(Flash {
            rgb,
            opacity,
            started: self.now,
            seconds: ms / 1000.,
        });
    }

    /// `H.banner`: "<who> <text>", for `ms`.
    pub fn banner(&mut self, who: &Who, text: &str, emoji: &str, ms: f32) {
        let (look, pill) = banner_look(who, text, emoji);
        if self.banners.len() >= MOST_BANNERS {
            self.banners.remove(0);
        }
        self.banners.push(Banner {
            look,
            pill,
            started: self.now,
            seconds: ms / 1000.,
        });
    }

    /// Whether the stage itself still shows anything.
    pub fn busy(&self) -> bool {
        !self.particles.is_empty()
            || !self.banners.is_empty()
            || !self.flashes.is_empty()
            || !self.shakes.is_empty()
    }

    /// Everything off at once: the call is over.
    pub fn clear(&mut self) {
        self.particles.clear();
        self.shakes.clear();
        self.flashes.clear();
        self.banners.clear();
        self.sounds.clear();
    }

    /// Drop the shakes, flashes and banners that are over.
    pub fn tidy(&mut self) {
        let now = self.now;
        self.shakes
            .retain(|shake| now - shake.started < shake.seconds);
        self.flashes
            .retain(|flash| now - flash.started < flash.seconds + 0.05);
        // The banner slides out over 0.4 s once its time is up.
        self.banners
            .retain(|banner| now - banner.started < banner.seconds + 0.4);
    }

    /// Where the shakes put everything this frame.
    pub fn shake_offset(&mut self) -> [f32; 2] {
        let now = self.now;
        let mut strongest = 0f32;
        for shake in &self.shakes {
            let k = 1. - (now - shake.started) / shake.seconds;
            strongest = strongest.max(shake.intensity * k.max(0.));
        }
        if strongest <= 0. {
            return [0., 0.];
        }
        [
            self.rng.range(-1., 1.) * strongest,
            self.rng.range(-1., 1.) * strongest,
        ]
    }

    pub fn draw_flashes(&self, canvas: &mut Canvas) {
        for flash in &self.flashes {
            let k = ease_out((self.now - flash.started) / flash.seconds);
            canvas.rect(
                [-40., -40.],
                [self.w + 80., self.h + 80.],
                0.,
                flash.rgb,
                flash.opacity * (1. - k),
            );
        }
    }

    pub fn draw_banners(&self, out: &mut Vec<Item>) {
        let mut canvas = Canvas::default();
        let mut top = BANNER_TOP;
        for banner in &self.banners {
            let age = self.now - banner.started;
            let leaving = (age - banner.seconds).max(0.);
            // In: a little drop with a springy settle; out: back up, fading.
            let (shown, settle) = if leaving > 0. {
                let k = clamp01(leaving / 0.35);
                (
                    1. - clamp01(leaving / 0.25),
                    1. - bezier(0.2, 1.4, 0.4, 1., k),
                )
            } else {
                (clamp01(age / 0.25), bezier(0.2, 1.4, 0.4, 1., age / 0.35))
            };
            let scale = 0.96 + 0.04 * settle;
            let y = top - 20. * (1. - settle);
            let [w, h] = banner.look.size;
            let s = scale;
            canvas.picture_in(
                &banner.look,
                [
                    self.w / 2. - w * s / 2.,
                    y + banner.pill[1] / 2. - h * s / 2.,
                ],
                [w * s, h * s],
                shown,
            );
            top += banner.pill[1] + BANNER_GAP;
        }
        out.extend(canvas.items);
    }
}

/// The banner's picture, and its pill's size: as the prototypes' `.h-banner`.
fn banner_look(who: &Who, text: &str, emoji: &str) -> (Arc<Look>, [f32; 2]) {
    let said = format!("{} {}", who.called(), text);
    let small = "everyone sees this";
    let said_width = looks::text_width(&said, 14., 650, FONT);
    let small_width = looks::text_width(small, 11.7, 500, FONT);
    let emoji_width = if emoji.is_empty() { 0. } else { 22. + 10. };
    let (pad, avatar) = (9., 26.);
    let width =
        1. + pad + avatar + 10. + emoji_width + said_width + 4. + 10. + small_width + 16. + 1.;
    let height = 46.;
    let m = BANNER_SHADOW;
    let (x0, y0) = (m, m);
    let mut x = x0 + 1. + pad;
    let mut body = format!(
        r##"<defs><filter id="s" x="-50%" y="-100%" width="200%" height="300%"><feDropShadow dx="0" dy="14" stdDeviation="20" flood-color="#000" flood-opacity=".35"/></filter></defs>
<rect x="{rx}" y="{ry}" width="{rw}" height="{rh}" rx="{rr}" fill="#0f0a1c" fill-opacity=".82" stroke="#fff" stroke-opacity=".14" filter="url(#s)"/>"##,
        rx = x0 + 0.5,
        ry = y0 + 0.5,
        rw = width - 1.,
        rh = height - 1.,
        rr = (height - 1.) / 2.,
    );
    body.push_str(&avatar_svg(who, x, y0 + 1. + pad, avatar, "av"));
    x += avatar + 10.;
    let middle = y0 + height / 2.;
    if !emoji.is_empty() {
        body.push_str(&format!(
            r#"<text x="{x}" y="{y}" font-family="{emo}" font-size="18">{}</text>"#,
            looks::escape(emoji),
            y = middle + 6.5,
            emo = looks::EMOJI,
        ));
        x += emoji_width;
    }
    body.push_str(&format!(
        r##"<text x="{x}" y="{y}" font-family="{FONT}" font-size="14" font-weight="650" fill="#fff">{}</text>"##,
        looks::escape(&said),
        y = middle + 5.,
    ));
    x += said_width + 4. + 10.;
    body.push_str(&format!(
        r##"<text x="{x}" y="{y}" font-family="{FONT}" font-size="11.7" font-weight="500" fill="#fff" fill-opacity=".55">{small}</text>"##,
        y = middle + 4.,
    ));
    let key = format!(
        "banner:{}:{}:{:?}:{said}:{emoji}",
        who.key, who.initials, who.rgb
    );
    (
        Look::new(key, [width + m * 2., height + m * 2.], body),
        [width, height],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_crosses_each_time_once() {
        let mut fired = 0;
        let mut before = -1.;
        for frame in 0..100 {
            let now = frame as f32 / 60.;
            let clock = Clock { before, now };
            if clock.crossed(0.5) {
                fired += 1;
            }
            if clock.crossed(0.) {
                fired += 10;
            }
            before = now;
        }
        assert_eq!(fired, 11);
    }

    #[test]
    fn css_curves_end_where_they_should() {
        for t in [0., 1.] {
            assert!((bezier(0.2, 1.4, 0.4, 1., t) - t).abs() < 1e-3);
            assert!((ease_out(t) - t).abs() < 1e-3);
        }
        // This one overshoots on the way.
        assert!(bezier(0.2, 1.4, 0.4, 1., 0.5) > 1.);
        assert!((keyframes(&[(0., 0.), (0.5, 10.), (1., 0.)], 0.75, |t| t) - 5.).abs() < 1e-4);
    }

    #[test]
    fn people_are_called_by_their_first_name_or_you() {
        let them = Who::new("a", "Sam Rivers", [1, 2, 3], false);
        assert_eq!(them.called(), "Sam");
        assert_eq!(them.initials, "SR");
        assert_eq!(Who::new("b", "Alex", [1, 2, 3], true).called(), "You");
        assert_eq!(Who::new("c", "", [1, 2, 3], false).initials, "?");
    }
}
