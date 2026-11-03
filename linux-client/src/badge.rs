//! Remote cursors and stamp markers, in the "Crisp" style chosen on 3 November 2026: a
//! compact arrow, a small name tag tucked under it, a slim pencil while drawing, and a
//! numbered marker for each stamp. Drawn as SVG and rendered with resvg.
//!
//! The pieces are separate sprites, so the name can fade on its own while the cursor
//! rests. Each sprite's offset puts its point (the arrow's tip, the pencil's lead, the
//! marker's foot) on the anchor.

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

use crate::ink_raster::Sprite;

/// Logical pixels per SVG unit.
pub const UNIT: f32 = 1.0;

/// The arrow's tip and the pencil's lead, in units.
const ARROW_TIP: (f32, f32) = (3.0, 2.5);
const PENCIL_TIP: (f32, f32) = (2.6, 19.4);
/// Where each tag's top left sits, in units of its cursor's drawing.
const ARROW_TAG: (f32, f32) = (13.0, 17.0);
const PENCIL_TAG: (f32, f32) = (15.0, 2.0);
const TAG_HEIGHT: f32 = 20.;
/// Room round a shape for its soft shadow.
const PAD: f32 = 8.;
/// Names longer than this (in units) are cut with an ellipsis.
const MAX_TAG: f32 = 180.;

/// How far a stamp marker's head sits above its foot, and its radius, in logical pixels.
pub const MARKER_RISE: f32 = 17.8;
pub const MARKER_RADIUS: f32 = 11.;

/// The arrow's tag, relative to the pointer, in logical pixels.
pub fn tag_offset_arrow() -> (f32, f32) {
    (
        (ARROW_TAG.0 - ARROW_TIP.0) * UNIT,
        (ARROW_TAG.1 - ARROW_TIP.1) * UNIT,
    )
}

/// The pencil's tag, relative to the lead, in logical pixels.
pub fn tag_offset_pencil() -> (f32, f32) {
    (
        (PENCIL_TAG.0 - PENCIL_TIP.0) * UNIT,
        (PENCIL_TAG.1 - PENCIL_TIP.1) * UNIT,
    )
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.set_sans_serif_family("Adwaita Sans");
            Arc::new(db)
        })
        .clone()
}

const FONT: &str = "Inter, 'Adwaita Sans', sans-serif";

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Text that reads on `rgb`: near-black on light colours (yellow, cyan, lime), white on
/// the rest, as multiplayer tools do.
fn text_on([r, g, b]: [u8; 3]) -> &'static str {
    let linear = |c: u8| {
        let c = f32::from(c) / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    if luminance > 0.5 {
        "#14151a"
    } else {
        "#ffffff"
    }
}

/// A soft two-layer shadow, over the filter region `w` × `h` from the origin.
fn shadow(id: &str, w: f32, h: f32) -> String {
    format!(
        r##"<filter id="{id}" x="0" y="0" width="{w}" height="{h}" filterUnits="userSpaceOnUse" color-interpolation-filters="sRGB">
<feDropShadow dx="0" dy="1.5" stdDeviation="1.4" flood-color="#000" flood-opacity=".32"/>
<feDropShadow dx="0" dy="4" stdDeviation="4" flood-color="#000" flood-opacity=".14"/></filter>"##
    )
}

/// Render `body` (`size` units) at `scale` device pixels per logical pixel; the sprite's
/// offset puts unit point `anchor` on the anchor.
fn render(body: &str, size: (f32, f32), anchor: (f32, f32), scale: f32) -> Option<Sprite> {
    let px = UNIT * scale;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" fill="none">{body}</svg>"#,
        w = size.0,
        h = size.1,
    );
    let options = usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(&svg, &options).ok()?;
    let (width, height) = ((size.0 * px).ceil() as u32, (size.1 * px).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(px, px),
        &mut pixmap.as_mut(),
    );
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        bgra.extend_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
    }
    Some(Sprite {
        width,
        height,
        bgra,
        offset: (-anchor.0 * px, -anchor.1 * px),
    })
}

/// The arrow in `rgb`: compact, with a white edge and a soft shadow.
pub fn arrow(rgb: [u8; 3], scale: f32) -> Option<Sprite> {
    let (w, h) = (20. + PAD * 2., 24. + PAD * 2.);
    let body = format!(
        r##"<g filter="url(#s)"><path transform="translate({PAD} {PAD})" d="M3 2.5 L3 20.5 L8 15.6 L15.6 14.8 Z" fill="{color}" stroke="white" stroke-width="1.6" stroke-linejoin="round"/></g><defs>{shadow}</defs>"##,
        color = hex(rgb),
        shadow = shadow("s", w, h),
    );
    render(&body, (w, h), (ARROW_TIP.0 + PAD, ARROW_TIP.1 + PAD), scale)
}

/// The pencil in `rgb`, its lead on the anchor.
pub fn pencil(rgb: [u8; 3], scale: f32) -> Option<Sprite> {
    let (w, h) = (22. + PAD * 2., 22. + PAD * 2.);
    let body = format!(
        r##"<g filter="url(#s)"><g transform="translate({PAD} {PAD})"><path d="M14.5 3.6 18.4 7.5 7.5 18.4 2.6 19.4 3.6 14.5Z" fill="{color}" stroke="white" stroke-width="1.5" stroke-linejoin="round"/><path d="M12.6 5.5 16.5 9.4" stroke="white" stroke-width="1.5"/></g></g><defs>{shadow}</defs>"##,
        color = hex(rgb),
        shadow = shadow("s", w, h),
    );
    render(
        &body,
        (w, h),
        (PENCIL_TIP.0 + PAD, PENCIL_TIP.1 + PAD),
        scale,
    )
}

/// The width of `text` set at `size` units, semibold.
fn text_width(text: &str, size: f32) -> f32 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="600" height="40"><text x="0" y="20" font-family="{FONT}" font-size="{size}" font-weight="600">{}</text></svg>"#,
        escape(text)
    );
    let options = usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    };
    usvg::Tree::from_str(&svg, &options)
        .ok()
        .map(|tree| tree.root().abs_bounding_box().width())
        .filter(|width| *width > 0.)
        .unwrap_or_else(|| size * 0.56 * text.chars().count() as f32)
}

/// The name tag in `rgb`. The anchor is the tag's top left.
pub fn tag(name: &str, rgb: [u8; 3], scale: f32) -> Option<Sprite> {
    let mut shown = name.to_owned();
    let mut width = text_width(&shown, 11.) + 14.;
    while width > MAX_TAG && shown.chars().count() > 2 {
        shown = shown.chars().take(shown.chars().count() - 2).collect();
        width = text_width(&format!("{shown}…"), 11.) + 14.;
    }
    if shown != name {
        shown.push('…');
    }
    let (w, h) = (width + PAD * 2., TAG_HEIGHT + PAD * 2.);
    let body = format!(
        r##"<g filter="url(#s)"><rect x="{PAD}" y="{PAD}" width="{width}" height="{TAG_HEIGHT}" rx="5" fill="{color}"/></g>
<text x="{tx}" y="{ty}" font-family="{FONT}" font-size="11" font-weight="600" fill="{ink}">{text}</text><defs>{shadow}</defs>"##,
        color = hex(rgb),
        ink = text_on(rgb),
        tx = PAD + 7.,
        ty = PAD + 14.,
        text = escape(&shown),
        shadow = shadow("s", w, h),
    );
    render(&body, (w, h), (PAD, PAD), scale)
}

/// A stamp's marker in `rgb`: a round head with `number` in it, on a short point whose
/// foot is the anchor.
pub fn marker(rgb: [u8; 3], number: u32, scale: f32) -> Option<Sprite> {
    let (w, h) = (24. + PAD * 2., 32. + PAD * 2.);
    let (fx, fy) = (12. + PAD, 30. + PAD);
    let digits = number.to_string();
    let size = if digits.len() > 2 { 9. } else { 11.5 };
    let body = format!(
        r##"<g filter="url(#s)"><g transform="translate({fx} {fy})"><path d="M0 0 L-5 -8 A 11 11 0 1 1 5 -8 Z" fill="{color}" stroke="white" stroke-width="1.6" stroke-linejoin="round"/>
<text x="0" y="-13.6" font-family="{FONT}" font-size="{size}" font-weight="700" fill="{ink}" text-anchor="middle">{digits}</text></g></g><defs>{shadow}</defs>"##,
        color = hex(rgb),
        ink = text_on(rgb),
        shadow = shadow("s", w, h),
    );
    render(&body, (w, h), (fx, fy), scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(s: &Sprite, x: f32, y: f32) -> u8 {
        let (x, y) = ((x - s.offset.0) as u32, (y - s.offset.1) as u32);
        s.bgra[((y * s.width + x) * 4 + 3) as usize]
    }

    /// Every piece renders, straight-alpha BGRA, with its point on the anchor.
    #[test]
    fn the_pieces_render_with_their_points_on_the_anchor() {
        let blue = [0x0a, 0x84, 0xff];
        for scale in [1., 1.6, 2.] {
            let arrow = arrow(blue, scale).expect("arrow");
            let pencil = pencil(blue, scale).expect("pencil");
            let tag = tag("Jonas Weber", blue, scale).expect("tag");
            let marker = marker(blue, 3, scale).expect("marker");
            for sprite in [&arrow, &pencil, &tag, &marker] {
                assert_eq!(sprite.bgra.len() as u32, sprite.width * sprite.height * 4);
                assert!(
                    sprite.bgra.chunks(4).any(|p| p[3] == 255),
                    "something opaque"
                );
            }
            // Just inside each point is covered.
            assert!(
                alpha_at(&arrow, 0.5 * scale, 3. * scale) > 0,
                "the arrow's tip"
            );
            assert!(
                alpha_at(&pencil, 1.5 * scale, -1.5 * scale) > 0,
                "the pencil's lead"
            );
            assert!(alpha_at(&marker, 0., -2. * scale) > 0, "the marker's foot");
            assert_eq!(alpha_at(&marker, 0., -MARKER_RISE * scale), 255, "its head");
        }
    }

    /// Light colours get dark text; the rest white.
    #[test]
    fn tag_text_reads_on_its_colour() {
        assert_eq!(text_on([0xff, 0xd6, 0x0a]), "#14151a", "yellow");
        assert_eq!(text_on([0x64, 0xd2, 0xff]), "#14151a", "cyan");
        assert_eq!(text_on([0xa3, 0x5c, 0xff]), "#ffffff", "purple");
        assert_eq!(text_on([0x0a, 0x84, 0xff]), "#ffffff", "blue");
    }

    /// A long name is cut to the tag's maximum width.
    #[test]
    fn a_long_name_is_cut() {
        let short = tag("Mira", [200, 120, 255], 1.).expect("tag");
        let long = tag("Maximiliana Konstantinopoulou-Smythe", [200, 120, 255], 1.).expect("tag");
        assert!(short.width < long.width);
        assert!(
            long.width <= (MAX_TAG + PAD * 2.).ceil() as u32 + 1,
            "{}",
            long.width
        );
    }

    /// Writes `/tmp/hopp-badges.png` to look at: `cargo test badges_preview -- --ignored`.
    #[test]
    #[ignore]
    fn badges_preview() {
        let (w, h) = (560u32, 300u32);
        let mut out = image::RgbaImage::from_pixel(w, h, image::Rgba([26, 27, 38, 255]));
        let mut put = |s: &Sprite, x: f32, y: f32| {
            for sy in 0..s.height {
                for sx in 0..s.width {
                    let i = ((sy * s.width + sx) * 4) as usize;
                    let a = f32::from(s.bgra[i + 3]) / 255.;
                    let (px, py) = (
                        (x + s.offset.0) as i64 + sx as i64,
                        (y + s.offset.1) as i64 + sy as i64,
                    );
                    if px < 0 || py < 0 || px >= w as i64 || py >= h as i64 {
                        continue;
                    }
                    let d = out.get_pixel_mut(px as u32, py as u32);
                    for (c, k) in [(2, 0), (1, 1), (0, 2)] {
                        d[k] = (f32::from(s.bgra[i + c]) * a + f32::from(d[k]) * (1. - a)).round()
                            as u8;
                    }
                }
            }
        };
        let scale = 2.;
        let (ax, ay) = tag_offset_arrow();
        let (px, py) = tag_offset_pencil();
        let people = [
            ("Jonas Weber", [0xff, 0x7a, 0x1a]),
            ("Mira Chen", [0xa3, 0x5c, 0xff]),
            ("Lee Park", [0xff, 0xd6, 0x0a]),
        ];
        for (row, (name, rgb)) in people.into_iter().enumerate() {
            let y = 30. + row as f32 * 90.;
            put(&arrow(rgb, scale).unwrap(), 30., y);
            put(
                &tag(name, rgb, scale).unwrap(),
                30. + ax * scale,
                y + ay * scale,
            );
            put(&pencil(rgb, scale).unwrap(), 260., y + 40.);
            put(
                &tag(name, rgb, scale).unwrap(),
                260. + px * scale,
                y + 40. + py * scale,
            );
            put(&marker(rgb, row as u32 + 1, scale).unwrap(), 480., y + 70.);
        }
        out.save("/tmp/hopp-badges.png").unwrap();
    }
}
