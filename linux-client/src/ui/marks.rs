//! Everything people put on a shared screen, painted: cursors, ink, click ripples and
//! stamps. The viewer paints them over the remote screen; the sharer's overlay paints the
//! same over their own monitor, so both sides look alike.
//!
//! Ink, cursors and pins are rasterised in software at the display's pixel density by
//! [`crate::ink_raster`] (true anti-aliasing, the same on every GPU) and drawn as images,
//! cached until they change. Rings, discs, pills and bubbles are GPUI's rounded quads,
//! which are anti-aliased everywhere already.
//!
//! - **Cursors** move at an even speed between network updates, in the person's colour
//!   with their name in a pill; a pencil while they draw, sitting exactly on the tip of
//!   their ink. The name hides after a few idle seconds, the cursor fades a little later.
//! - **Ink** is flat and even, drawn in tiles so only its tip is redrawn as it grows, and
//!   thins as it fades out instead of vanishing.
//! - **Ripples** are a dot and two rings that pulse out where someone clicks.
//! - **Stamps** are a pin that drops in with a little bounce, pulses while it listens, and
//!   carries a bubble with what was said; words still being recognised are dimmed.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{
    App, Bounds, BoxShadow, Corners, FontWeight, Hsla, Pixels, RenderImage, Rgba, TextAlign,
    TextRun, Window, point, px, size,
};

use super::session::{Ripple, Stroke};
use crate::{ink_raster, protocol::Point};

/// Your strokes stay this long after you finish them, then fade over [`INK_FADE`].
pub const INK_HOLD: Duration = Duration::from_millis(2_400);
pub const INK_FADE: Duration = Duration::from_millis(600);
const RIPPLE: Duration = Duration::from_millis(700);
const NAME_IDLE: Duration = Duration::from_millis(2_500);
const POINTER_IDLE: Duration = Duration::from_secs(8);
const STAMP_DROP: Duration = Duration::from_millis(460);
const STAMP_FADE: Duration = Duration::from_millis(260);

type P = gpui_kit::Point<Pixels>;

pub fn colour(rgb: [u8; 3], alpha: f32) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(rgb[0]) / 255.,
        g: f32::from(rgb[1]) / 255.,
        b: f32::from(rgb[2]) / 255.,
        a: alpha.clamp(0., 1.),
    })
}

fn black(alpha: f32) -> Hsla {
    colour([0, 0, 0], alpha)
}

fn white(alpha: f32) -> Hsla {
    colour([255, 255, 255], alpha)
}

fn progress(since: Duration, over: Duration) -> f32 {
    (since.as_secs_f32() / over.as_secs_f32()).clamp(0., 1.)
}

fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

/// Overshoots a little, then settles: for things that land.
fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.4, 2.4);
    1. + c3 * (t - 1.).powi(3) + c1 * (t - 1.).powi(2)
}

/// Everyone's colour, the one their marks have in captured frames: you (`own_id`, your
/// user id) first, then people in the order given. The result maps a sender (empty for
/// you) to a colour.
pub fn colours_for(
    own_id: Option<String>,
    senders: impl Iterator<Item = String>,
) -> impl Fn(&str) -> [u8; 3] + Clone + 'static {
    let mut colours = crate::ink::Colours::default();
    if let Some(own) = &own_id {
        colours.of(own);
    }
    for sender in senders {
        if !sender.is_empty() {
            colours.of(&sender);
        }
    }
    move |sender: &str| {
        let who = if sender.is_empty() {
            own_id.as_deref()
        } else {
            Some(sender)
        };
        who.map(|who| colours.get(who).unwrap_or_else(|| crate::ink::colour(who)))
            .unwrap_or(crate::ink::PALETTE[0])
    }
}

// ---- Cursors --------------------------------------------------------------------------

/// A remote cursor, played back a moment behind the network so it moves at an even
/// speed. Updates arrive about 60 to 120 times a second, unevenly and sometimes in
/// pairs; each is given a time no closer to the previous one than half the usual gap,
/// and the cursor is drawn where the path was a little over one gap ago, moving in a
/// straight line between updates. So it never surges after a late update or stalls
/// waiting for the next.
#[derive(Clone, Debug)]
pub struct Pointer {
    samples: VecDeque<(Instant, Point)>,
    /// The usual time between updates, in seconds.
    gap: f32,
    /// The longest recent wait between updates, in seconds, slowly forgotten.
    slow: f32,
    /// How far behind the network it plays, in seconds: `lag` at `lag_at`, moving
    /// towards `lag_to` gradually, since a sudden change would show as the cursor
    /// speeding up or slowing down.
    lag: f32,
    lag_to: f32,
    lag_at: Instant,
    arrived: Instant,
    pub moved: Instant,
}

impl Pointer {
    pub fn new(point: Point) -> Self {
        let now = Instant::now();
        Self {
            samples: VecDeque::from([(now, point)]),
            gap: 0.016,
            slow: 0.016,
            lag: 0.035,
            lag_to: 0.035,
            lag_at: now,
            arrived: now,
            moved: now,
        }
    }

    /// The lag at `now`: playback runs up to 10% slow while it grows and 3% fast while
    /// it shrinks.
    fn lag_at(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.lag_at).as_secs_f32();
        let rate = if self.lag_to > self.lag { 0.1 } else { 0.03 };
        self.lag + (self.lag_to - self.lag).clamp(-rate * elapsed, rate * elapsed)
    }

    fn delay(&self, now: Instant) -> Duration {
        Duration::from_secs_f32(self.lag_at(now))
    }

    /// Aim the lag at what the network needs: past the longest recent wait, so playback
    /// never catches up with the last update and stops.
    fn adjust_lag(&mut self, now: Instant) {
        self.lag = self.lag_at(now);
        self.lag_at = now;
        self.lag_to = (self.gap * 2.2).max(self.slow * 1.3).clamp(0.016, 0.09);
    }

    pub fn moved_to(&mut self, point: Point) {
        self.moved_to_at(point, Instant::now());
    }

    fn moved_to_at(&mut self, point: Point, now: Instant) {
        let since = now.duration_since(self.arrived).as_secs_f32();
        self.arrived = now;
        self.moved = now;
        let previous = self.samples.back().copied();
        if since > 0.25 || previous.is_none() {
            // It was resting: start from where it rests, one gap ago, not from whenever
            // it last moved.
            let rest = previous.map_or(point, |(_, point)| point);
            let gap = Duration::from_secs_f32(self.gap);
            self.samples.clear();
            self.samples
                .push_back((now.checked_sub(gap).unwrap_or(now), rest));
            self.samples.push_back((now, point));
            return;
        }
        self.gap += (since.clamp(0.004, 0.05) - self.gap) * 0.05;
        self.slow = (self.slow * 0.985).max(since.min(0.065));
        self.adjust_lag(now);
        // When it was probably sent: the network's jitter averaged out of the arrival
        // times (a late update and the next arriving together are still spaced out), but
        // a pause taken as it comes.
        let last = self.samples.back().map_or(now, |(at, _)| *at);
        let gap = Duration::from_secs_f32(self.gap);
        let predicted = last + gap;
        let at = if now > predicted + gap * 4 {
            now
        } else if now >= predicted {
            predicted + (now - predicted).mul_f32(0.08)
        } else {
            predicted - (predicted - now).mul_f32(0.08)
        };
        let delay = self.delay(now);
        let at = at
            .max(last + gap * 2 / 5)
            // Not so far back that playback has already passed it.
            .max(now.checked_sub(delay * 4 / 5).unwrap_or(now))
            .min(now + delay / 2);
        self.samples.push_back((at, point));
        // Keep what playback still needs.
        let shown = now.checked_sub(delay + Duration::from_millis(100));
        while self.samples.len() > 2 && shown.is_some_and(|shown| self.samples[1].0 < shown) {
            self.samples.pop_front();
        }
    }

    pub fn position(&self, now: Instant) -> Point {
        let at = now.checked_sub(self.delay(now)).unwrap_or(now);
        let Some(&(_, last)) = self.samples.back() else {
            return Point { x: 0., y: 0. };
        };
        let mut previous: Option<(Instant, Point)> = None;
        for &(time, point) in &self.samples {
            if time > at {
                return match previous {
                    None => point,
                    Some((from_time, from)) => {
                        let span = time.duration_since(from_time).as_secs_f64();
                        let t = (at.duration_since(from_time).as_secs_f64() / span.max(1e-6))
                            .clamp(0., 1.);
                        Point {
                            x: from.x + (point.x - from.x) * t,
                            y: from.y + (point.y - from.y) * t,
                        }
                    }
                };
            }
            previous = Some((time, point));
        }
        last
    }

    /// Whether the cursor is still visible: it fades out after [`POINTER_IDLE`].
    pub fn visible(&self, now: Instant) -> bool {
        now.duration_since(self.moved) < POINTER_IDLE + Duration::from_millis(400)
    }

    /// Whether painting it again would show something different.
    pub fn animating(&self, now: Instant) -> bool {
        let idle = now.duration_since(self.moved);
        let playing = self.samples.back().is_some_and(|(at, _)| {
            now.checked_sub(self.delay(now))
                .is_none_or(|shown| shown < *at)
        });
        playing
            || (NAME_IDLE..NAME_IDLE + Duration::from_millis(350)).contains(&idle)
            || (POINTER_IDLE..POINTER_IDLE + Duration::from_millis(450)).contains(&idle)
    }
}

/// What a remote cursor looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    /// They're drawing.
    Pencil,
    /// Only their name: they control your machine, so your own cursor is theirs.
    Name,
}

/// A sprite uploaded for painting: the image, its offset from its anchor and its size,
/// both in device pixels.
type Cached = (Arc<RenderImage>, (f32, f32), (u32, u32));

fn upload(sprite: ink_raster::Sprite) -> Option<Cached> {
    // The bytes stay BGRA, the order GPUI's renderer expects (straight alpha).
    let buffer = image::RgbaImage::from_raw(sprite.width, sprite.height, sprite.bgra)?;
    let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
    Some((image, sprite.offset, (sprite.width, sprite.height)))
}

/// A stroke's tiles as last drawn: what they were drawn from, and the images.
struct InkTiles {
    look: String,
    pieces: Vec<ink_raster::Piece>,
    tiles: HashMap<(i32, i32), Cached>,
}

/// Images of rasterised marks, kept per window until they stop being painted.
#[derive(Default)]
pub struct Sprites {
    images: HashMap<String, Cached>,
    used: HashSet<String>,
    ink: HashMap<String, InkTiles>,
    ink_used: HashSet<String>,
    retired: Vec<Arc<RenderImage>>,
}

impl Sprites {
    /// The image for `key`, rasterising it with `make` the first time.
    fn get(
        &mut self,
        key: String,
        make: impl FnOnce() -> Option<ink_raster::Sprite>,
    ) -> Option<Cached> {
        self.used.insert(key.clone());
        if let Some(found) = self.images.get(&key) {
            return Some(found.clone());
        }
        let entry = upload(make()?)?;
        self.images.insert(key, entry.clone());
        Some(entry)
    }

    /// Release the images not painted this frame.
    pub fn finish(&mut self, window: &mut Window) {
        let used = std::mem::take(&mut self.used);
        let stale: Vec<String> = self
            .images
            .keys()
            .filter(|key| !used.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            if let Some((image, _, _)) = self.images.remove(&key) {
                self.retired.push(image);
            }
        }
        let used = std::mem::take(&mut self.ink_used);
        let retired = &mut self.retired;
        self.ink.retain(|key, stroke| {
            let keep = used.contains(key);
            if !keep {
                retired.extend(stroke.tiles.drain().map(|(_, (image, _, _))| image));
            }
            keep
        });
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
    }
}

/// Paint a sprite anchored at `anchor` (logical pixels), its pixels on the device grid.
fn paint_sprite(window: &mut Window, anchor: P, (image, offset, (width, height)): Cached) {
    let scale = window.scale_factor();
    let x = (f32::from(anchor.x) * scale).round() + offset.0;
    let y = (f32::from(anchor.y) * scale).round() + offset.1;
    let bounds = Bounds {
        origin: point(px(x.round() / scale), px(y.round() / scale)),
        size: size(px(width as f32 / scale), px(height as f32 / scale)),
    };
    let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
}

/// Opacity in steps fine enough to look continuous, coarse enough to cache.
fn step(alpha: f32) -> f32 {
    (alpha.clamp(0., 1.) * 24.).round() / 24.
}

fn with_alpha(mut sprite: ink_raster::Sprite, alpha: f32) -> ink_raster::Sprite {
    if alpha < 1. {
        for pixel in sprite.bgra.as_chunks_mut::<4>().0 {
            pixel[3] = (f32::from(pixel[3]) * alpha).round() as u8;
        }
    }
    sprite
}

fn circle(centre: P, radius: f32) -> Bounds<Pixels> {
    Bounds {
        origin: point(centre.x - px(radius), centre.y - px(radius)),
        size: size(px(radius * 2.), px(radius * 2.)),
    }
}

/// An anti-aliased ring (GPUI quads are drawn with a distance field).
fn ring(window: &mut Window, centre: P, radius: f32, width: f32, fill: Hsla) {
    let mut clear = fill;
    clear.a = 0.;
    window.paint_quad(
        gpui_kit::fill(circle(centre, radius + width / 2.), clear)
            .corner_radii(Corners::all(px(radius + width / 2.)))
            .border_widths(px(width))
            .border_color(fill),
    );
}

fn disc(window: &mut Window, centre: P, radius: f32, fill: Hsla) {
    window.paint_quad(
        gpui_kit::fill(circle(centre, radius), fill).corner_radii(Corners::all(px(radius))),
    );
}

/// Where the name tag sits from the cursor's point, tucked under it.
fn pill_offset(tool: Tool) -> (f32, f32) {
    match tool {
        Tool::Pencil => crate::badge::tag_offset_pencil(),
        // The arrow, and beside the system cursor for whoever controls your screen.
        Tool::Arrow | Tool::Name => crate::badge::tag_offset_arrow(),
    }
}

/// A remote cursor at `at`, with `name` beside it (see [`crate::badge`]).
#[allow(clippy::too_many_arguments)]
pub fn paint_pointer(
    window: &mut Window,
    _cx: &mut App,
    sprites: &mut Sprites,
    at: P,
    name: &str,
    rgb: [u8; 3],
    idle: Duration,
    tool: Tool,
) {
    let fade = 1.
        - progress(
            idle.saturating_sub(POINTER_IDLE),
            Duration::from_millis(400),
        );
    let fade = step(fade);
    if fade <= 0. {
        return;
    }
    let scale = window.scale_factor();
    let key = format!("cursor:{tool:?}:{rgb:?}:{scale}:{fade}");
    let made = match tool {
        Tool::Arrow => sprites.get(key, || {
            crate::badge::arrow(rgb, scale).map(|sprite| with_alpha(sprite, fade))
        }),
        Tool::Pencil => sprites.get(key, || {
            crate::badge::pencil(rgb, scale).map(|sprite| with_alpha(sprite, fade))
        }),
        Tool::Name => None,
    };
    if let Some(sprite) = made {
        paint_sprite(window, at, sprite);
    }

    // The name, in a pill that hides once the cursor rests (but not while they draw).
    let label = if tool == Tool::Pencil {
        1.
    } else {
        1. - progress(idle.saturating_sub(NAME_IDLE), Duration::from_millis(350))
    };
    let label = step(ease_in_out(label) * fade);
    if label <= 0.01 || name.is_empty() {
        return;
    }
    let key = format!("tag:{name}:{rgb:?}:{scale}:{label}");
    let Some(pill) = sprites.get(key, || {
        crate::badge::tag(name, rgb, scale).map(|sprite| with_alpha(sprite, label))
    }) else {
        return;
    };
    let (dx, dy) = pill_offset(tool);
    paint_sprite(
        window,
        point(at.x + px(dx), at.y + px(dy) + px(3. * (1. - label))),
        pill,
    );
}

// ---- Ripples --------------------------------------------------------------------------

/// Where someone clicked to point something out: a dot that lands, and two rings that
/// pulse out from it, one after the other. Returns false once it has finished.
pub fn paint_ripple(window: &mut Window, at: P, rgb: [u8; 3], age: Duration) -> bool {
    let mut running = false;
    for (delay, strength) in [(0u64, 0.9f32), (180, 0.6)] {
        let since = age.saturating_sub(Duration::from_millis(delay));
        let t = progress(since, RIPPLE);
        if t >= 1. {
            continue;
        }
        running = true;
        if since.is_zero() {
            continue;
        }
        let radius = 5. + 24. * ease_out_cubic(t);
        // A soft wash inside the ring, then the ring itself with a white rim beside it
        // so it shows on any colour.
        disc(window, at, radius, colour(rgb, 0.10 * strength * (1. - t)));
        let alpha = strength * (1. - t).powf(1.4);
        ring(window, at, radius + 1.4, 1.2, white(0.55 * alpha));
        ring(window, at, radius, 2.6 - 1.2 * t, colour(rgb, alpha));
    }
    if age < Duration::from_millis(320) {
        let t = progress(age, Duration::from_millis(320));
        let r = 5. * (1. - 0.3 * ease_in_out(t));
        disc(window, at, r + 1.5, white(0.9 * (1. - t)));
        disc(window, at, r, colour(rgb, 1. - ease_in_out(t)));
    }
    running || age < RIPPLE + Duration::from_millis(180)
}

// ---- Ink ------------------------------------------------------------------------------

/// How wide ink is on a screen shown `width` logical pixels across: about 4 pixels on a
/// big monitor, never hairline-thin when the screen is shown small.
pub fn ink_width(width: f32) -> f32 {
    (width / 760.).clamp(2.6, 5.)
}

/// A stroke on a screen shown at `area` (logical pixels), its `points` fractions of that
/// screen, `width` logical pixels wide, at `alpha`; it thins a little as it fades.
///
/// It is drawn in tiles, kept per stroke (`key`): as points arrive only the tiles at the
/// tip are drawn again; the whole stroke only when the screen's size, the colour or the
/// fade step changes. Panning costs nothing.
#[allow(clippy::too_many_arguments)]
pub fn paint_stroke(
    window: &mut Window,
    sprites: &mut Sprites,
    key: &str,
    points: &[Point],
    area: Bounds<Pixels>,
    rgb: [u8; 3],
    alpha: f32,
    width: f32,
) {
    let alpha = step(alpha);
    if points.is_empty() || alpha <= 0. {
        return;
    }
    let scale = window.scale_factor();
    let (w, h) = (
        f32::from(area.size.width) * scale,
        f32::from(area.size.height) * scale,
    );
    let width = width * scale * (0.45 + 0.55 * alpha);
    let look = format!("{w:.0}x{h:.0}:{width:.2}:{alpha}:{rgb:?}");
    let device: Vec<(f32, f32)> = points
        .iter()
        .map(|p| (p.x as f32 * w, p.y as f32 * h))
        .collect();
    let pieces = ink_raster::pieces(&device, ink_raster::min_spacing(scale));
    sprites.ink_used.insert(key.to_owned());
    let stroke = sprites
        .ink
        .entry(key.to_owned())
        .or_insert_with(|| InkTiles {
            look: String::new(),
            pieces: Vec::new(),
            tiles: HashMap::new(),
        });
    let mut dirty: HashSet<(i32, i32)> = HashSet::new();
    if stroke.look != look {
        sprites
            .retired
            .extend(stroke.tiles.drain().map(|(_, (image, _, _))| image));
        dirty.extend(ink_raster::tiles_touched(&pieces, width));
    } else {
        let same = stroke
            .pieces
            .iter()
            .zip(&pieces)
            .take_while(|(a, b)| a == b)
            .count();
        dirty.extend(ink_raster::tiles_touched(&stroke.pieces[same..], width));
        dirty.extend(ink_raster::tiles_touched(&pieces[same..], width));
    }
    for tile in dirty {
        if let Some((image, _, _)) = stroke.tiles.remove(&tile) {
            sprites.retired.push(image);
        }
        if let Some(made) = ink_raster::ink_tile(&pieces, width, rgb, alpha, tile).and_then(upload)
        {
            stroke.tiles.insert(tile, made);
        }
    }
    stroke.look = look;
    stroke.pieces = pieces;
    for tile in stroke.tiles.values() {
        paint_sprite(window, area.origin, tile.clone());
    }
}

/// How visible a stroke is now: 1 until `fading` starts, then down to 0 over [`INK_FADE`].
pub fn ink_alpha(fading: Option<Instant>, now: Instant) -> f32 {
    match fading {
        None => 1.,
        Some(since) => 1. - ease_in_out(progress(now.duration_since(since), INK_FADE)),
    }
}

// ---- Everything on one screen -------------------------------------------------------------

/// A cursor to paint, with how it looks and, while its owner draws, the tip of their ink,
/// which it sits on exactly.
#[derive(Clone)]
pub struct ScenePointer {
    pub sender: String,
    pub pointer: Pointer,
    pub name: String,
    pub tool: Tool,
    pub pen_tip: Option<Point>,
}

/// Each person's colour, by sender (empty for you).
pub type Colours = std::rc::Rc<dyn Fn(&str) -> [u8; 3]>;

/// Everything people have put on one shared screen, ready to paint.
pub struct Scene {
    pub paths: Vec<Stroke>,
    pub ripples: Vec<Ripple>,
    pub stamps: Vec<Stamp>,
    pub pointers: Vec<ScenePointer>,
    pub colours: Colours,
}

/// Paint `scene` over a screen shown at `area`; bubbles stay inside `visible`. Returns
/// whether anything is still moving.
pub fn paint_scene(
    window: &mut Window,
    cx: &mut App,
    sprites: &mut Sprites,
    scene: &Scene,
    area: Bounds<Pixels>,
    visible: Bounds<Pixels>,
) -> bool {
    let at = |p: Point| super::geometry::point_on_image(area, p);
    let now = Instant::now();
    let mut animating = false;
    let width = ink_width(f32::from(area.size.width));
    let rgb_of = &scene.colours;
    for path in &scene.paths {
        let fade = path.fade_start();
        animating |= fade
            .is_some_and(|start| now + Duration::from_millis(1_200) >= start && !path.faded(now));
        paint_stroke(
            window,
            sprites,
            &format!("{}:{}", path.sender, path.id),
            &path.points,
            area,
            rgb_of(&path.sender),
            ink_alpha(fade, now),
            width,
        );
    }
    for ripple in &scene.ripples {
        animating |= paint_ripple(
            window,
            at(ripple.point),
            rgb_of(&ripple.sender),
            now.duration_since(ripple.at),
        );
    }
    for stamp in &scene.stamps {
        paint_stamp(
            window,
            cx,
            sprites,
            stamp,
            at(stamp.point),
            rgb_of(&stamp.owner),
            now,
            visible,
        );
        animating |= stamp.animating(now);
    }
    for shown in &scene.pointers {
        let position = shown.pen_tip.unwrap_or_else(|| shown.pointer.position(now));
        let idle = if shown.pen_tip.is_some() {
            Duration::ZERO
        } else {
            now.duration_since(shown.pointer.moved)
        };
        paint_pointer(
            window,
            cx,
            sprites,
            at(position),
            &shown.name,
            rgb_of(&shown.sender),
            idle,
            shown.tool,
        );
        animating |= shown.pointer.animating(now);
    }
    sprites.finish(window);
    animating
}

// ---- Stamps ---------------------------------------------------------------------------

/// A pin someone dropped and spoke over.
#[derive(Clone, Debug)]
pub struct Stamp {
    pub id: u64,
    /// Numbered through the call, in the order stamps were placed (as the capture
    /// numbers pins), and shown on its marker.
    pub number: u32,
    /// Participant identity; empty for yours.
    pub owner: String,
    pub name: String,
    pub point: Point,
    /// Final words.
    pub text: String,
    /// Words still being recognised, shown dimmed after `text`.
    pub draft: String,
    /// Still taking speech.
    pub listening: bool,
    pub placed: Instant,
    pub removed: Option<Instant>,
}

impl Stamp {
    pub fn gone(&self, now: Instant) -> bool {
        self.removed
            .is_some_and(|removed| now.duration_since(removed) >= STAMP_FADE)
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.listening
            || now.duration_since(self.placed) < STAMP_DROP + Duration::from_millis(400)
            || self.removed.is_some()
    }
}

/// A stamp with its pin's tip at `at`. `area` keeps the bubble on screen.
#[allow(clippy::too_many_arguments)]
pub fn paint_stamp(
    window: &mut Window,
    cx: &mut App,
    sprites: &mut Sprites,
    stamp: &Stamp,
    at: P,
    rgb: [u8; 3],
    now: Instant,
    area: Bounds<Pixels>,
) {
    let age = now.duration_since(stamp.placed);
    let leaving = stamp.removed.map_or(1., |removed| {
        1. - progress(now.duration_since(removed), STAMP_FADE)
    });
    let drop = progress(age, STAMP_DROP);
    let landed = ease_out_back(drop);
    let alpha = step((drop * 3.).min(1.) * leaving);
    if alpha <= 0. {
        return;
    }

    // Its shadow on the screen sharpens and widens as it lands.
    let spread = 0.3 + 0.7 * drop;
    window.paint_drop_shadows(
        Bounds {
            origin: point(at.x - px(6. * spread), at.y - px(1.5)),
            size: size(px(12. * spread), px(3.)),
        },
        Corners::all(px(1.5)),
        &[BoxShadow {
            color: black(0.45 * alpha * drop),
            offset: point(px(0.), px(0.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
            inset: false,
        }],
    );

    // Quantised so the drop animates through a handful of cached images.
    let grow = ((0.62 + 0.38 * landed) * 50.).round() / 50.;
    let fall = -22. * (1. - landed);
    let tip = point(at.x, at.y + px(fall));
    let head = point(tip.x, tip.y - px(crate::badge::MARKER_RISE * grow));
    let radius = crate::badge::MARKER_RADIUS * grow;

    if stamp.listening {
        // Rings breathing out from the head while it listens.
        let seconds = age.as_secs_f32();
        for offset in [0., 0.5] {
            let phase = (seconds / 1.6 + offset).fract();
            ring(
                window,
                head,
                radius + 3. + 15. * ease_out_cubic(phase),
                1.8,
                colour(rgb, 0.55 * (1. - phase).powf(1.3) * alpha),
            );
        }
    }
    let scale = window.scale_factor();
    let number = stamp.number;
    let key = format!("marker:{rgb:?}:{number}:{scale}:{grow}:{alpha}");
    if let Some(sprite) = sprites.get(key, || {
        crate::badge::marker(rgb, number, scale * grow).map(|sprite| with_alpha(sprite, alpha))
    }) {
        paint_sprite(window, tip, sprite);
    }
    paint_bubble(
        window,
        cx,
        stamp,
        (f32::from(head.x), f32::from(head.y)),
        radius,
        rgb,
        now,
        area,
        alpha * leaving,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_bubble(
    window: &mut Window,
    cx: &mut App,
    stamp: &Stamp,
    head: (f32, f32),
    radius: f32,
    rgb: [u8; 3],
    now: Instant,
    area: Bounds<Pixels>,
    alpha: f32,
) {
    let age = now.duration_since(stamp.placed);
    let shown = ease_out_cubic(progress(
        age.saturating_sub(Duration::from_millis(180)),
        Duration::from_millis(280),
    )) * alpha;
    if shown <= 0.01 {
        return;
    }
    let text_size = px(13.5);
    let line_height = px(19.);
    let wrap = px(290.);
    let base = window.text_style().font();

    // What it says: final words bright, recognised-so-far words dimmed, or "Listening".
    let (said, draft) = (stamp.text.trim(), stamp.draft.trim());
    let mut body = String::new();
    let mut runs = Vec::new();
    let run = |len: usize, a: f32, italic: bool| TextRun {
        len,
        font: gpui_kit::Font {
            style: if italic {
                gpui_kit::FontStyle::Italic
            } else {
                gpui_kit::FontStyle::Normal
            },
            ..base.clone()
        },
        color: colour([0x1c, 0x1d, 0x26], a),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    if !said.is_empty() {
        body.push_str(said);
        runs.push(run(said.len(), shown, false));
    }
    if !draft.is_empty() {
        let piece = if body.is_empty() {
            draft.to_owned()
        } else {
            format!(" {draft}")
        };
        runs.push(run(piece.len(), 0.45 * shown, false));
        body.push_str(&piece);
    }
    if body.is_empty() {
        // Three dots that fill in turn while it listens.
        let dots = 1 + (age.as_millis() / 380 % 3) as usize;
        let piece = format!("Listening{}", ".".repeat(dots));
        runs.push(run(piece.len(), 0.5 * shown, true));
        body.push_str(&piece);
    }
    let Ok(lines) =
        window
            .text_system()
            .shape_text(body.into(), text_size, &runs, Some(wrap), Some(6))
    else {
        return;
    };
    let text_width = lines
        .iter()
        .map(|line| line.size(line_height).width)
        .fold(px(0.), |a, b| a.max(b));
    let text_height = lines
        .iter()
        .map(|line| line.size(line_height).height)
        .fold(px(0.), |a, b| a + b);

    let mut name_font = base.clone();
    name_font.weight = FontWeight::SEMIBOLD;
    let title: String = stamp.name.clone();
    // Their colour, deepened until it reads on the white card.
    let ink = ink_raster::mix(rgb, [0x1c, 0x1d, 0x26], 0.22);
    let title_line = window.text_system().shape_line(
        title.clone().into(),
        px(11.),
        &[TextRun {
            len: title.len(),
            font: name_font,
            color: colour(ink, shown),
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let (pad_x, pad_y) = (px(11.), px(8.));
    let width = text_width.max(title_line.width).max(px(64.)) + pad_x * 2.;
    let height = px(15.) + text_height + pad_y * 2.;

    // Beside the head, on whichever side has room; it slides out from the pin.
    let gap = radius + 9.;
    let right_room = f32::from(area.origin.x + area.size.width) - (head.0 + gap);
    let on_right = right_room >= f32::from(width) + 8.;
    let slide = 8. * (1. - shown / alpha.max(0.01));
    let x = if on_right {
        head.0 + gap - slide
    } else {
        head.0 - gap - f32::from(width) + slide
    };
    let y = (head.1 - 22.)
        .max(f32::from(area.origin.y) + 6.)
        .min(f32::from(area.origin.y + area.size.height) - f32::from(height) - 6.);
    let bubble = Bounds {
        origin: point(px(x), px(y)),
        size: size(width, height),
    };
    let round = Corners::all(px(8.));
    window.paint_drop_shadows(
        bubble,
        round,
        &[
            BoxShadow {
                color: black(0.14 * shown),
                offset: point(px(0.), px(4.)),
                blur_radius: px(10.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: black(0.3 * shown),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.5),
                spread_radius: px(0.),
                inset: false,
            },
        ],
    );
    window.paint_quad(
        gpui_kit::fill(bubble, white(0.98 * shown))
            .corner_radii(round)
            .border_widths(px(1.))
            .border_color(black(0.06 * shown)),
    );
    let text_x = bubble.origin.x + pad_x;
    let _ = title_line.paint(
        point(text_x, bubble.origin.y + pad_y - px(1.)),
        px(14.),
        TextAlign::Left,
        None,
        window,
        cx,
    );
    let mut y = bubble.origin.y + pad_y + px(15.);
    for line in &lines {
        let _ = line.paint(
            point(text_x, y),
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
        y += line.size(line_height).height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pointer_glides_to_where_it_was_sent() {
        let mut pointer = Pointer::new(Point { x: 0.1, y: 0.1 });
        std::thread::sleep(Duration::from_millis(20));
        pointer.moved_to(Point { x: 0.5, y: 0.5 });
        let now = Instant::now();
        let start = pointer.position(now);
        assert!(start.x < 0.2, "starts where it was drawn: {start:?}");
        let later = pointer.position(now + Duration::from_millis(200));
        assert!((later.x - 0.5).abs() < 1e-9 && (later.y - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_pointer_moves_evenly_through_uneven_updates() {
        // Steady movement sent every 16 ms or so, delivered after 0 to 20 ms (in order,
        // so a late one arrives together with the next).
        let start = Instant::now();
        let speed = 0.4; // screen widths a second
        let mut pointer = Pointer::new(Point { x: 0., y: 0.5 });
        let mut seed = 7u64;
        let mut random = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33) as f32 / (1u64 << 31) as f32
        };
        let (mut sent, mut arrived) = (0f32, 0f32);
        let mut updates = Vec::new();
        while sent < 1.5 {
            arrived = (sent + 0.02 * random() * random()).max(arrived);
            updates.push((sent, arrived));
            sent += 0.014 + 0.004 * random();
        }
        let mut next = 0;
        let mut steps = Vec::new();
        let mut last: Option<f64> = None;
        // Painted at 120 Hz.
        for frame in 0..170 {
            let now = start + Duration::from_secs_f32(frame as f32 / 120.);
            while next < updates.len() && start + Duration::from_secs_f32(updates[next].1) <= now {
                let (sent, arrived) = updates[next];
                let x = f64::from(sent * speed);
                let at = start + Duration::from_secs_f32(arrived);
                pointer.moved_to_at(Point { x, y: 0.5 }, at);
                next += 1;
            }
            let x = pointer.position(now).x;
            // Once it's under way.
            if frame > 20
                && let Some(previous) = last
            {
                steps.push(x - previous);
            }
            last = Some(x);
        }
        let mean = steps.iter().sum::<f64>() / steps.len() as f64;
        let expected = f64::from(speed) / 120.;
        assert!(
            (mean - expected).abs() < expected * 0.1,
            "{mean} vs {expected}"
        );
        let worst = steps
            .iter()
            .map(|step| (step - mean).abs() / mean)
            .fold(0., f64::max);
        assert!(
            worst < 0.3,
            "frame-to-frame steps vary by up to {worst:.2} of the mean"
        );
        // And it changes speed gradually: no frame jolts against the one before.
        let jolt = steps
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs() / mean)
            .fold(0., f64::max);
        assert!(jolt < 0.2, "a jolt of {jolt:.2} of the mean between frames");
    }

    #[test]
    fn opacity_steps_are_fine_but_few() {
        assert_eq!(step(1.), 1.);
        assert_eq!(step(0.), 0.);
        assert_eq!(step(0.501), step(0.499));
        assert!((step(0.3) - 0.3).abs() < 1. / 48. + 1e-6);
    }

    #[test]
    fn ink_fades_out_once_fading_starts() {
        let now = Instant::now();
        assert_eq!(ink_alpha(None, now), 1.);
        assert_eq!(ink_alpha(Some(now), now), 1.);
        assert!(ink_alpha(Some(now), now + INK_FADE / 2) < 0.6);
        assert_eq!(ink_alpha(Some(now), now + INK_FADE), 0.);
    }
}
