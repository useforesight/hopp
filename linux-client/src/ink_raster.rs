//! Ink, cursors and pins rasterised in software at the display's own pixel density, with
//! proper anti-aliasing (tiny-skia).
//!
//! The viewer, the sharer's overlay and captured frames all draw through here, so a mark
//! looks the same on both sides of a call and in what an agent sees, whatever the GPU
//! does (or doesn't do) about anti-aliasing paths.
//!
//! - **Ink** is flat colour of an even width with round ends and joins. Its centre line
//!   runs through the midpoints between the points drawn, curving through each point
//!   (quadratic segments), so it is smooth without lagging the pen, and adding a point
//!   changes only the last stretch: a stroke never reshapes behind the pen. It is drawn
//!   in square tiles, so a stroke being drawn re-renders only the tiles at its tip.
//! - **Cursor**: a pointer in the person's colour inside a white edge, on a crisp shadow;
//!   a pencil instead while they draw.
//! - **Pin**: a teardrop with the same treatment and a white centre.

use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, Point, Rect, SpreadMode, Stroke, Transform,
};

/// A rasterised mark: straight-alpha BGRA pixels (what GPUI's sprites take), and where
/// its top-left sits relative to the mark's anchor (a cursor's tip, a pin's point, or
/// the origin for ink), in device pixels.
#[derive(Clone)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub offset: (f32, f32),
}

fn color([r, g, b]: [u8; 3], alpha: f32) -> Color {
    Color::from_rgba8(r, g, b, (alpha.clamp(0., 1.) * 255.).round() as u8)
}

fn solid(rgb: [u8; 3], alpha: f32) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color(rgb, alpha));
    paint.anti_alias = true;
    paint
}

pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8
    })
}

/// Three box blurs, close to a gaussian of the given radius, on an alpha mask.
fn blur(mask: &mut Mask, radius: f32) {
    let r = (radius / 1.7).round() as usize;
    if r == 0 {
        return;
    }
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let data = mask.data_mut();
    let mut line = vec![0u16; w.max(h)];
    for _ in 0..3 {
        for y in 0..h {
            box_pass(&mut data[y * w..(y + 1) * w], 1, w, r, &mut line);
        }
        for x in 0..w {
            box_pass(&mut data[x..], w, h, r, &mut line);
        }
    }
}

/// A running-sum box blur of `count` values `stride` apart.
fn box_pass(data: &mut [u8], stride: usize, count: usize, r: usize, line: &mut [u16]) {
    for i in 0..count {
        line[i] = u16::from(data[i * stride]);
    }
    let window = (2 * r + 1) as u32;
    let mut sum: u32 = 0;
    for value in &line[..=r.min(count - 1)] {
        sum += u32::from(*value);
    }
    for i in 0..count {
        data[i * stride] = (sum / window) as u8;
        if i + r + 1 < count {
            sum += u32::from(line[i + r + 1]);
        }
        if i >= r {
            sum -= u32::from(line[i - r]);
        }
    }
}

/// A soft shadow of `path` (already in pixmap space), offset and blurred.
fn shadow(pixmap: &mut Pixmap, path: &Path, offset: (f32, f32), radius: f32, alpha: f32) {
    let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    mask.fill_path(
        path,
        FillRule::Winding,
        true,
        Transform::from_translate(offset.0, offset.1),
    );
    blur(&mut mask, radius);
    let rect = Rect::from_xywh(0., 0., pixmap.width() as f32, pixmap.height() as f32);
    if let Some(rect) = rect {
        pixmap.fill_rect(
            rect,
            &solid([0, 0, 0], alpha),
            Transform::identity(),
            Some(&mask),
        );
    }
}

fn sprite(pixmap: Pixmap, offset: (f32, f32)) -> Sprite {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        bgra.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
    }
    Sprite {
        width,
        height,
        bgra,
        offset,
    }
}

/// A sprite of one flat colour: its coverage from `pixmap`, the colour exact everywhere
/// (no fringe from un-premultiplying faint edge pixels).
fn flat_sprite(pixmap: &Pixmap, rgb: [u8; 3], offset: (f32, f32)) -> Sprite {
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.pixels() {
        bgra.extend_from_slice(&[rgb[2], rgb[1], rgb[0], pixel.alpha()]);
    }
    Sprite {
        width,
        height,
        bgra,
        offset,
    }
}

// ---- Ink ------------------------------------------------------------------------------

type Xy = (f32, f32);

/// A stretch of a stroke's centre line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    /// A single point: a dot.
    Dot(Xy),
    Line(Xy, Xy),
    /// From, through (the control point), to.
    Quad(Xy, Xy, Xy),
}

impl Piece {
    fn start(&self) -> Xy {
        match *self {
            Piece::Dot(p) | Piece::Line(p, _) | Piece::Quad(p, _, _) => p,
        }
    }

    /// Left, top, right, bottom: a box the piece stays inside.
    pub fn bounds(&self) -> [f32; 4] {
        let points: &[Xy] = match self {
            Piece::Dot(p) => &[*p],
            Piece::Line(a, b) => &[*a, *b],
            Piece::Quad(a, c, b) => &[*a, *c, *b],
        };
        points.iter().fold(
            [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            |[l, t, r, b], &(x, y)| [l.min(x), t.min(y), r.max(x), b.max(y)],
        )
    }
}

fn mid(a: Xy, b: Xy) -> Xy {
    ((a.0 + b.0) / 2., (a.1 + b.1) / 2.)
}

/// The centre line through `points`: points closer than `min` to the last one kept are
/// skipped (but it always reaches the latest), then straight to the first midpoint,
/// curving through each point to the next midpoint, and straight to the last point.
pub fn pieces(points: &[Xy], min: f32) -> Vec<Piece> {
    let mut kept: Vec<Xy> = Vec::with_capacity(points.len());
    for &p in points {
        if kept
            .last()
            .is_none_or(|last| (last.0 - p.0).hypot(last.1 - p.1) >= min)
        {
            kept.push(p);
        }
    }
    if let (Some(&last), Some(&end)) = (kept.last(), points.last())
        && last != end
    {
        kept.push(end);
    }
    match kept.len() {
        0 => vec![],
        1 => vec![Piece::Dot(kept[0])],
        2 => vec![Piece::Line(kept[0], kept[1])],
        n => {
            let mut line = Vec::with_capacity(n);
            line.push(Piece::Line(kept[0], mid(kept[0], kept[1])));
            for i in 1..n - 1 {
                line.push(Piece::Quad(
                    mid(kept[i - 1], kept[i]),
                    kept[i],
                    mid(kept[i], kept[i + 1]),
                ));
            }
            line.push(Piece::Line(mid(kept[n - 2], kept[n - 1]), kept[n - 1]));
            line
        }
    }
}

/// How far apart points need to be to keep them, at `scale` device pixels per point.
pub fn min_spacing(scale: f32) -> f32 {
    0.75 * scale
}

fn overlaps([l, t, r, b]: [f32; 4], [l2, t2, r2, b2]: [f32; 4]) -> bool {
    l <= r2 && r >= l2 && t <= b2 && b >= t2
}

fn grow([l, t, r, b]: [f32; 4], by: f32) -> [f32; 4] {
    [l - by, t - by, r + by, b + by]
}

/// The pieces within `reach` of `area`, as one path of runs of consecutive pieces. A run
/// ends where the next piece is out of reach, so its round end is out of reach too.
fn ink_path(line: &[Piece], area: [f32; 4], reach: f32) -> (Option<Path>, Vec<Xy>) {
    let mut path = PathBuilder::new();
    let mut dots = Vec::new();
    let mut open = false;
    for piece in line {
        if !overlaps(grow(piece.bounds(), reach), area) {
            open = false;
            continue;
        }
        if let Piece::Dot(p) = *piece {
            dots.push(p);
            open = false;
            continue;
        }
        if !open {
            let (x, y) = piece.start();
            path.move_to(x, y);
            open = true;
        }
        match *piece {
            Piece::Dot(_) => {}
            Piece::Line(_, (x, y)) => path.line_to(x, y),
            Piece::Quad(_, (cx, cy), (x, y)) => path.quad_to(cx, cy, x, y),
        }
    }
    (path.finish(), dots)
}

/// Draw the ink of `line` that falls on `pixmap`, whose top-left is at `origin`.
fn draw_ink(pixmap: &mut Pixmap, line: &[Piece], width: f32, origin: Xy) {
    let area = [
        origin.0,
        origin.1,
        origin.0 + pixmap.width() as f32,
        origin.1 + pixmap.height() as f32,
    ];
    let reach = width / 2. + 2.;
    let (path, dots) = ink_path(line, area, reach);
    let transform = Transform::from_translate(-origin.0, -origin.1);
    // Coverage only: the colour goes on when the pixels are handed over.
    let paint = solid([255, 255, 255], 1.);
    if let Some(path) = path {
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, transform, None);
    }
    for (x, y) in dots {
        if let Some(disc) = PathBuilder::from_circle(x, y, width / 2.) {
            pixmap.fill_path(&disc, &paint, FillRule::Winding, transform, None);
        }
    }
}

fn scale_alpha(pixmap: &mut Pixmap, alpha: f32) {
    if alpha < 1. {
        let k = (alpha.clamp(0., 1.) * 256.).round() as u32;
        for pixel in pixmap.data_mut().as_chunks_mut::<4>().0 {
            for c in pixel {
                *c = ((u32::from(*c) * k) >> 8) as u8;
            }
        }
    }
}

/// The side of an ink tile, in device pixels.
pub const TILE: u32 = 256;

/// The tiles (column, row) that ink along `pieces`, `width` wide, touches.
pub fn tiles_touched(pieces: &[Piece], width: f32) -> impl Iterator<Item = (i32, i32)> + '_ {
    let reach = width / 2. + 2.;
    pieces.iter().flat_map(move |piece| {
        let [l, t, r, b] = grow(piece.bounds(), reach);
        let tile = TILE as f32;
        let (x0, x1) = ((l / tile).floor() as i32, (r / tile).floor() as i32);
        let (y0, y1) = ((t / tile).floor() as i32, (b / tile).floor() as i32);
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (x, y)))
    })
}

/// One tile of ink: the part of `pieces` (device pixels) in tile `(column, row)`, or
/// `None` when nothing lands there. Neighbouring tiles meet seamlessly: each draws the
/// same path, clipped.
pub fn ink_tile(
    pieces: &[Piece],
    width: f32,
    rgb: [u8; 3],
    alpha: f32,
    (column, row): (i32, i32),
) -> Option<Sprite> {
    let origin = (column as f32 * TILE as f32, row as f32 * TILE as f32);
    let mut pixmap = Pixmap::new(TILE, TILE)?;
    draw_ink(&mut pixmap, pieces, width, origin);
    if pixmap.pixels().iter().all(|pixel| pixel.alpha() == 0) {
        return None;
    }
    scale_alpha(&mut pixmap, alpha);
    Some(flat_sprite(&pixmap, rgb, origin))
}

/// A whole stroke through `points` (device pixels), `width` device pixels wide, at
/// `alpha`, in one sprite whose offset is its top-left in the same space as `points`.
pub fn stroke(
    points: &[(f32, f32)],
    width: f32,
    rgb: [u8; 3],
    alpha: f32,
    scale: f32,
) -> Option<Sprite> {
    if points.is_empty() || alpha <= 0. {
        return None;
    }
    let line = pieces(points, min_spacing(scale));
    let [l, t, r, b] = line
        .iter()
        .map(Piece::bounds)
        .reduce(|[l, t, r, b], [l2, t2, r2, b2]| [l.min(l2), t.min(t2), r.max(r2), b.max(b2)])?;
    let pad = (width / 2. + 2.).ceil();
    let (left, top) = ((l - pad).floor(), (t - pad).floor());
    let (w, h) = ((r + pad).ceil() - left, (b + pad).ceil() - top);
    if w > 16_384. || h > 16_384. {
        return None;
    }
    let mut pixmap = Pixmap::new(w as u32, h as u32)?;
    draw_ink(&mut pixmap, &line, width, (left, top));
    scale_alpha(&mut pixmap, alpha);
    Some(flat_sprite(&pixmap, rgb, (left, top)))
}

// ---- Shapes with rounded corners --------------------------------------------------------

/// A fill that is a touch lighter at `from` than at `to`: enough to give a shape some
/// light without looking glossy.
fn sheen(rgb: [u8; 3], from: Xy, to: Xy, lift: f32) -> Paint<'static> {
    let mut paint = solid(rgb, 1.);
    if let Some(shader) = LinearGradient::new(
        Point::from_xy(from.0, from.1),
        Point::from_xy(to.0, to.1),
        vec![
            GradientStop::new(0., color(mix(rgb, [255, 255, 255], lift), 1.)),
            GradientStop::new(1., color(mix(rgb, [0, 0, 0], lift * 0.4), 1.)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        paint.shader = shader;
    }
    paint
}

fn fill(pixmap: &mut Pixmap, path: &Path, paint: &Paint) {
    pixmap.fill_path(path, paint, FillRule::Winding, Transform::identity(), None);
}

/// The shadow a cursor or pin casts: tight and fairly dark right under it, with a wide
/// soft one further out, so it lifts off any background.
fn lift(pixmap: &mut Pixmap, outline: &Path, scale: f32) {
    shadow(pixmap, outline, (0., 2.2 * scale), 4.5 * scale, 0.16);
    shadow(pixmap, outline, (0., 0.9 * scale), 1.1 * scale, 0.34);
}

/// A white-edged shape: `outline` filled white, a hairline round it for white
/// backgrounds, and `inner` in the person's colour.
fn chrome(pixmap: &mut Pixmap, outline: &Path, inner: &Path, paint: &Paint, scale: f32) {
    lift(pixmap, outline, scale);
    fill(pixmap, outline, &solid([255, 255, 255], 1.));
    let hairline = Stroke {
        width: 0.55 * scale,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    pixmap.stroke_path(
        outline,
        &solid([0, 0, 0], 0.16),
        &hairline,
        Transform::identity(),
        None,
    );
    fill(pixmap, inner, paint);
}

// ---- Pin ----------------------------------------------------------------------------------

/// A pin's head radius and how far its centre sits above the point, at 1×.
pub const PIN_RADIUS: f32 = 11.5;
pub const PIN_RISE: f32 = 27.;

/// A pin in `rgb` at `scale`; its offset is from the point. `scale` may include the
/// drop animation's growth.
pub fn pin(rgb: [u8; 3], scale: f32) -> Option<Sprite> {
    let (r, rise) = (PIN_RADIUS * scale, PIN_RISE * scale);
    let pad = (9. * scale).ceil();
    let (w, h) = ((2. * r + 2. * pad).ceil(), (rise + r + 2. * pad).ceil());
    let mut pixmap = Pixmap::new(w as u32, h as u32)?;
    let (cx, cy) = (pad + r, pad + r);
    let tip = (cx, cy + rise);
    let teardrop = |r: f32, tip: Xy| -> Option<Path> {
        // The sides leave the head where tangents from the tip touch it, then meet at
        // the tip in a gentle curve rather than straight lines.
        let distance = tip.1 - cy;
        let angle = (r / distance).asin();
        let side = std::f32::consts::FRAC_PI_2 - angle;
        let touch = |s: f32| {
            (
                cx + r * (std::f32::consts::FRAC_PI_2 + s).cos(),
                cy + r * (std::f32::consts::FRAC_PI_2 + s).sin(),
            )
        };
        let (right, left) = (touch(-side), touch(side));
        let mut path = PathBuilder::new();
        path.move_to(tip.0, tip.1);
        path.quad_to(
            cx + 0.18 * r,
            tip.1 - 0.55 * (tip.1 - right.1),
            right.0,
            right.1,
        );
        // Round the head the long way over the top, in small arcs.
        let (start, end) = (
            (right.1 - cy).atan2(right.0 - cx),
            (left.1 - cy).atan2(left.0 - cx) - std::f32::consts::TAU,
        );
        let steps = 24;
        for i in 1..=steps {
            let a = start + (end - start) * i as f32 / steps as f32;
            let mid = start + (end - start) * (i as f32 - 0.5) / steps as f32;
            let k = 1. / ((end - start).abs() / steps as f32 / 2.).cos();
            path.quad_to(
                cx + r * k * mid.cos(),
                cy + r * k * mid.sin(),
                cx + r * a.cos(),
                cy + r * a.sin(),
            );
        }
        path.quad_to(cx - 0.18 * r, tip.1 - 0.55 * (tip.1 - left.1), tip.0, tip.1);
        path.close();
        path.finish()
    };
    let edge = 1.8 * scale;
    let outline = teardrop(r, tip)?;
    let inner = teardrop(r - edge, (tip.0, tip.1 - edge * 1.9))?;
    let paint = sheen(rgb, (cx, pad), (cx, tip.1), 0.16);
    chrome(&mut pixmap, &outline, &inner, &paint, scale);
    // The white centre.
    let eye = PathBuilder::from_circle(cx, cy, 4.2 * scale)?;
    fill(&mut pixmap, &eye, &solid([255, 255, 255], 1.));
    Some(sprite(pixmap, (-tip.0, -tip.1)))
}

/// Draw a sprite onto an RGBA image with its anchor at `at` (image pixels), at `alpha`.
pub fn composite(image: &mut image::RgbaImage, sprite: &Sprite, at: (f32, f32), alpha: f32) {
    let x0 = (at.0 + sprite.offset.0).round() as i64;
    let y0 = (at.1 + sprite.offset.1).round() as i64;
    for y in 0..sprite.height as i64 {
        let iy = y0 + y;
        if iy < 0 || iy >= i64::from(image.height()) {
            continue;
        }
        for x in 0..sprite.width as i64 {
            let ix = x0 + x;
            if ix < 0 || ix >= i64::from(image.width()) {
                continue;
            }
            let i = ((y as u32 * sprite.width + x as u32) * 4) as usize;
            let a = f32::from(sprite.bgra[i + 3]) / 255. * alpha;
            if a <= 0. {
                continue;
            }
            let src = [sprite.bgra[i + 2], sprite.bgra[i + 1], sprite.bgra[i]];
            let dst = image.get_pixel_mut(ix as u32, iy as u32);
            for c in 0..3 {
                let d = f32::from(dst[c]);
                dst[c] = (d + (f32::from(src[c]) - d) * a).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(sprite: &Sprite, x: u32, y: u32) -> u8 {
        sprite.bgra[((y * sprite.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn ink_edges_are_anti_aliased_and_its_middle_is_solid() {
        // Off the pixel grid, so its edges fall part-way across pixels.
        let line: Vec<(f32, f32)> = (0..=40).map(|i| (10. + i as f32 * 5., 30.4)).collect();
        let sprite = stroke(&line, 8., [255, 0, 0], 1., 1.).unwrap();
        let (ox, oy) = sprite.offset;
        let (cx, cy) = ((110. - ox) as u32, (30.4 - oy) as u32);
        assert_eq!(alpha_at(&sprite, cx, cy), 255, "solid in the middle");
        // Partial coverage somewhere across the edge, not a hard step from 0 to 255.
        let across: Vec<u8> = (cy..sprite.height)
            .map(|y| alpha_at(&sprite, cx, y))
            .collect();
        assert!(
            across.iter().any(|a| *a > 20 && *a < 235),
            "a soft edge: {across:?}"
        );
        // Flat colour everywhere it shows, faint edges included.
        let i = ((cy + 3) * sprite.width + cx) as usize * 4;
        assert_eq!(&sprite.bgra[i..i + 3], &[0, 0, 255]);
    }

    #[test]
    fn ink_is_even_in_width_to_its_ends() {
        let line: Vec<(f32, f32)> = (0..=40).map(|i| (10. + i as f32 * 5., 30.)).collect();
        let sprite = stroke(&line, 8., [255, 0, 0], 1., 1.).unwrap();
        let (ox, oy) = sprite.offset;
        let thickness = |x: f32| {
            (0..sprite.height)
                .filter(|&y| alpha_at(&sprite, (x - ox) as u32, y) > 127)
                .count()
        };
        assert_eq!(thickness(20.), thickness(110.));
        let _ = oy;
    }

    #[test]
    fn a_loop_that_crosses_itself_has_no_holes() {
        // A circle drawn past its start, overlapping itself.
        let ring: Vec<(f32, f32)> = (0..=120)
            .map(|i| {
                let a = i as f32 / 100. * std::f32::consts::TAU;
                (60. + 40. * a.cos(), 60. + 40. * a.sin())
            })
            .collect();
        let sprite = stroke(&ring, 6., [0, 0, 255], 1., 1.).unwrap();
        let (ox, oy) = sprite.offset;
        // Where the stroke overlaps itself (around angle 0), it is still solid.
        let (x, y) = ((100. - ox) as u32, (60. - oy) as u32);
        assert_eq!(alpha_at(&sprite, x, y), 255);
    }

    #[test]
    fn adding_a_point_changes_only_the_tip() {
        let points: Vec<(f32, f32)> = (0..30)
            .map(|i| (i as f32 * 9., (i as f32 * 0.4).sin() * 40.))
            .collect();
        let before = pieces(&points[..29], 0.75);
        let after = pieces(&points, 0.75);
        let same = before
            .iter()
            .zip(&after)
            .take_while(|(a, b)| a == b)
            .count();
        assert!(same >= before.len() - 1, "{same} of {}", before.len());
    }

    #[test]
    fn tiles_join_seamlessly() {
        // A diagonal across the corner where four tiles meet: drawn whole and in tiles,
        // the pixels agree.
        let t = TILE as f32;
        let line = [(t - 40., t - 30.), (t + 35., t + 42.)];
        let whole = stroke(&line, 7., [10, 20, 30], 1., 1.).unwrap();
        let line = pieces(&line, 0.75);
        for tile in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let Some(part) = ink_tile(&line, 7., [10, 20, 30], 1., tile) else {
                continue;
            };
            for y in 0..TILE {
                for x in 0..TILE {
                    let (gx, gy) = (part.offset.0 + x as f32, part.offset.1 + y as f32);
                    let (wx, wy) = (gx - whole.offset.0, gy - whole.offset.1);
                    let expected = if wx >= 0.
                        && wy >= 0.
                        && (wx as u32) < whole.width
                        && (wy as u32) < whole.height
                    {
                        alpha_at(&whole, wx as u32, wy as u32)
                    } else {
                        0
                    };
                    let got = alpha_at(&part, x, y);
                    assert!(
                        got.abs_diff(expected) <= 1,
                        "tile {tile:?} at {x},{y}: {got} vs {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_single_point_is_a_dot() {
        let sprite = stroke(&[(20., 20.)], 8., [255, 0, 0], 1., 1.).unwrap();
        let (ox, oy) = sprite.offset;
        assert_eq!(alpha_at(&sprite, (20. - ox) as u32, (20. - oy) as u32), 255);
    }

    #[test]
    fn sprites_are_straight_alpha_bgra() {
        // A cursor's body, well inside it: its blue channel first, fully opaque.
        let sprite = crate::badge::arrow([0x0a, 0x84, 0xff], 2.).unwrap();
        let (x, y) = (
            (-sprite.offset.0 + 14.) as u32,
            (-sprite.offset.1 + 18.) as u32,
        );
        let i = ((y * sprite.width + x) * 4) as usize;
        let [b, g, r, a] = [
            sprite.bgra[i],
            sprite.bgra[i + 1],
            sprite.bgra[i + 2],
            sprite.bgra[i + 3],
        ];
        assert_eq!(a, 255);
        assert!(b > 200 && r < 120 && g > 100, "{b} {g} {r}");
        let pin = pin([0xff, 0x3b, 0x30], 1.).unwrap();
        assert!(pin.width > 20 && pin.height > 40);
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// `cargo test --bin hopp ink_raster::preview -- --ignored` writes
    /// /tmp/hopp-marks-preview.png: ink, cursors, pencils and pins on light and dark.
    #[test]
    #[ignore]
    fn write_preview() {
        let scale = 2.;
        let (w, h) = (1400u32, 640u32);
        let mut image = image::RgbaImage::from_fn(w, h, |x, y| {
            let dark = y > h / 2;
            let stripe = (x / 140) % 2 == 0;
            match (dark, stripe) {
                (false, true) => image::Rgba([250, 250, 250, 255]),
                (false, false) => image::Rgba([232, 236, 242, 255]),
                (true, true) => image::Rgba([30, 31, 36, 255]),
                (true, false) => image::Rgba([52, 56, 66, 255]),
            }
        });
        let palette = [
            [0xff, 0x3b, 0x30],
            [0x0a, 0x84, 0xff],
            [0x30, 0xd1, 0x58],
            [0xff, 0x9f, 0x0a],
            [0xbf, 0x5a, 0xf2],
            [0xff, 0xd6, 0x0a],
        ];
        for half in 0..2 {
            let y0 = (half * h / 2) as f32;
            // A scribble: a loop and a wave, as a hand would draw them.
            let scribble: Vec<(f32, f32)> = (0..220)
                .map(|i| {
                    let t = i as f32 / 219.;
                    let a = t * 9.;
                    (
                        60. + t * 520. + 40. * a.cos(),
                        y0 + 150. + 70. * (a * 0.7).sin() - 30. * a.sin(),
                    )
                })
                .collect();
            let sprite = stroke(&scribble, 4.4 * scale, palette[half as usize], 1., scale).unwrap();
            composite(&mut image, &sprite, (0., 0.), 1.);
            // Sparse points (a fast stroke), to show the curves between them.
            let fast: Vec<(f32, f32)> = (0..9)
                .map(|i| {
                    (
                        640. + i as f32 * 38.,
                        y0 + 120. + if i % 2 == 0 { 0. } else { 70. },
                    )
                })
                .collect();
            let sprite = stroke(&fast, 4.4 * scale, palette[2 + half as usize], 1., scale).unwrap();
            composite(&mut image, &sprite, (0., 0.), 1.);
            for (i, rgb) in palette.iter().enumerate() {
                let x = 1000. + (i % 3) as f32 * 120.;
                let y = y0 + 40. + (i / 3) as f32 * 130.;
                let tool = if i % 2 == 0 {
                    crate::badge::arrow(*rgb, scale)
                } else {
                    crate::badge::pencil(*rgb, scale)
                };
                composite(&mut image, &tool.unwrap(), (x, y), 1.);
            }
            let pin = pin(palette[4], scale).unwrap();
            composite(&mut image, &pin, (620., y0 + 290.), 1.);
        }
        image.save("/tmp/hopp-marks-preview.png").unwrap();
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// `cargo test --release --bin hopp ink_raster::timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn tile_costs() {
        // A long scribble across a 5K screen, drawn point by point.
        let points: Vec<(f32, f32)> = (0..1500)
            .map(|i| {
                let t = i as f32 / 1499.;
                (
                    300. + 4200. * t + 120. * (t * 60.).cos(),
                    1000. + 500. * (t * 7.).sin() + 120. * (t * 60.).sin(),
                )
            })
            .collect();
        let width = 6.7;
        let start = std::time::Instant::now();
        let mut worst = std::time::Duration::ZERO;
        let mut previous: Vec<Piece> = Vec::new();
        let mut renders = 0;
        for n in 1..=points.len() {
            let frame = std::time::Instant::now();
            let line = pieces(&points[..n], min_spacing(1.6));
            let same = previous
                .iter()
                .zip(&line)
                .take_while(|(a, b)| a == b)
                .count();
            let mut dirty: std::collections::HashSet<(i32, i32)> =
                tiles_touched(&previous[same..], width).collect();
            dirty.extend(tiles_touched(&line[same..], width));
            for tile in dirty {
                renders += 1;
                let _ = ink_tile(&line, width, [255, 0, 0], 1., tile);
            }
            previous = line;
            worst = worst.max(frame.elapsed());
        }
        let all: std::collections::HashSet<(i32, i32)> = tiles_touched(&previous, width).collect();
        let full = std::time::Instant::now();
        for tile in &all {
            let _ = ink_tile(&previous, width, [255, 0, 0], 0.5, *tile);
        }
        eprintln!(
            "growing: {:?} total, worst frame {:?}, {renders} tile renders; whole stroke ({} tiles) again: {:?}",
            start.elapsed(),
            worst,
            all.len(),
            full.elapsed()
        );
    }
}
