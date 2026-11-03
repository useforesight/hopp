//! Remote cursors in the "Crisp" style: a compact arrow, a small name tag tucked under it,
//! a slim pencil while drawing. Drawn as SVG and rendered with resvg.
//!
//! The pieces are separate sprites, so the name can fade on its own while the cursor rests.
//! Each sprite's offset puts its point (the arrow's tip, the pencil's lead) on the anchor.

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

use crate::colours::hex;

/// The arrow's tip, the hand's fingertip and the pencil's lead, in logical pixels.
const ARROW_TIP: (f32, f32) = (3.0, 2.5);
const HAND_TIP: (f32, f32) = (3.6, 1.7);
const PENCIL_TIP: (f32, f32) = (2.6, 19.4);
/// Where each tag's top left sits, in its cursor's drawing.
const ARROW_TAG: (f32, f32) = (13.0, 17.0);
const HAND_TAG: (f32, f32) = (13.5, 21.5);
const PENCIL_TAG: (f32, f32) = (20.5, 2.0);
const TAG_HEIGHT: f32 = 20.;
/// Room round a shape for all of its soft shadow, which would otherwise end in a hard edge.
const PAD: f32 = 16.;
/// Names wider than this are cut with an ellipsis.
const MAX_TAG: f32 = 180.;

const FONT: &str = "Inter, 'Adwaita Sans', 'Helvetica Neue', 'Segoe UI', Arial, sans-serif";

/// A rendered piece, straight-alpha RGBA. `offset` is from the anchor to its top left, in
/// device pixels.
#[derive(Debug, Clone)]
pub struct Sprite {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub offset: (f32, f32),
}

/// The arrow's tag, relative to the pointer, in logical pixels.
pub fn tag_offset_arrow() -> (f32, f32) {
    (ARROW_TAG.0 - ARROW_TIP.0, ARROW_TAG.1 - ARROW_TIP.1)
}

/// The hand's tag, relative to the fingertip, in logical pixels.
pub fn tag_offset_hand() -> (f32, f32) {
    (HAND_TAG.0 - HAND_TIP.0, HAND_TAG.1 - HAND_TIP.1)
}

/// The pencil's tag, relative to the lead, in logical pixels.
pub fn tag_offset_pencil() -> (f32, f32) {
    (PENCIL_TAG.0 - PENCIL_TIP.0, PENCIL_TAG.1 - PENCIL_TIP.1)
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            #[cfg(target_os = "linux")]
            db.set_sans_serif_family("Adwaita Sans");
            Arc::new(db)
        })
        .clone()
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

/// Render `body` (`size` logical pixels) at `scale` device pixels per logical pixel; the
/// sprite's offset puts logical point `anchor` on the anchor.
fn render(body: &str, size: (f32, f32), anchor: (f32, f32), scale: f32) -> Option<Sprite> {
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
    let (width, height) = (
        (size.0 * scale).ceil() as u32,
        (size.1 * scale).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(Sprite {
        width,
        height,
        rgba,
        offset: (-anchor.0 * scale, -anchor.1 * scale),
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

/// The hand in `rgb`, its fingertip on the anchor: for when they point at something
/// clickable.
pub fn hand(rgb: [u8; 3], scale: f32) -> Option<Sprite> {
    let (w, h) = (21. + PAD * 2., 22. + PAD * 2.);
    let body = format!(
        r##"<g filter="url(#s)"><path transform="translate({PAD} {PAD}) scale(.85)" d="M2.4 13.65 2.72 15.57 3.85 16.46 5.12 16.95 11.95 23.78 20.62 20.7 20.8 17.86 20.71 14.27 18.46 7.02 16.56 6.13 14.61 5.95 10.13 6.13 8.21 6.36 5.33 1.75 3.2 2.52 6.99 14.03 4.09 12.67Z" fill="{color}" stroke="white" stroke-width="1.76" stroke-linejoin="round"/></g><defs>{shadow}</defs>"##,
        color = hex(rgb),
        shadow = shadow("s", w, h),
    );
    render(&body, (w, h), (HAND_TIP.0 + PAD, HAND_TIP.1 + PAD), scale)
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

/// The width of `text` set at `size`, semibold.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(s: &Sprite, x: f32, y: f32) -> u8 {
        let (x, y) = ((x - s.offset.0) as u32, (y - s.offset.1) as u32);
        s.rgba[((y * s.width + x) * 4 + 3) as usize]
    }

    /// Every piece renders, with its point on the anchor.
    #[test]
    fn the_pieces_render_with_their_points_on_the_anchor() {
        let blue = [0x0a, 0x84, 0xff];
        for scale in [1., 1.6, 2.] {
            let arrow = arrow(blue, scale).expect("arrow");
            let hand = hand(blue, scale).expect("hand");
            let pencil = pencil(blue, scale).expect("pencil");
            let tag = tag("Jonas Weber", blue, scale).expect("tag");
            for sprite in [&arrow, &hand, &pencil, &tag] {
                assert_eq!(sprite.rgba.len() as u32, sprite.width * sprite.height * 4);
                assert!(
                    sprite.rgba.chunks(4).any(|p| p[3] == 255),
                    "something opaque"
                );
            }
            // Just inside each point is covered.
            assert!(
                alpha_at(&arrow, 0.5 * scale, 3. * scale) > 0,
                "the arrow's tip"
            );
            assert!(alpha_at(&hand, 0., 1. * scale) > 0, "the hand's fingertip");
            assert!(
                alpha_at(&pencil, 1.5 * scale, -1.5 * scale) > 0,
                "the pencil's lead"
            );
        }
    }

    /// The shadow fades to nothing before the sprite ends, leaving no hard edge.
    #[test]
    fn the_shadow_ends_before_the_sprite_does() {
        let blue = [0x0a, 0x84, 0xff];
        for sprite in [
            arrow(blue, 2.).expect("arrow"),
            hand(blue, 2.).expect("hand"),
            pencil(blue, 2.).expect("pencil"),
            tag("Mira", blue, 2.).expect("tag"),
        ] {
            let alpha = |x: u32, y: u32| sprite.rgba[((y * sprite.width + x) * 4 + 3) as usize];
            let (last_x, last_y) = (sprite.width - 1, sprite.height - 1);
            let edges = (0..sprite.width)
                .flat_map(|x| [alpha(x, 0), alpha(x, last_y)])
                .chain((0..sprite.height).flat_map(|y| [alpha(0, y), alpha(last_x, y)]));
            let strongest = edges.max().unwrap_or(0);
            // One level of alpha in 255 can't show as an edge.
            assert!(strongest <= 1, "alpha {strongest} at the edge");
        }
    }

    /// The name doesn't cover the mark it belongs to.
    #[test]
    fn the_tag_sits_clear_of_the_pencil_and_hand() {
        let (blue, scale) = ([0x0a, 0x84, 0xff], 2.);
        let under_tag = |sprite: &Sprite, (tag_x, tag_y): (f32, f32)| {
            let left = (tag_x * scale - sprite.offset.0) as u32;
            let top = (tag_y * scale - sprite.offset.1) as u32;
            (top..sprite.height).any(|y| {
                (left..sprite.width)
                    .any(|x| sprite.rgba[((y * sprite.width + x) * 4 + 3) as usize] == 255)
            })
        };
        let pencil = pencil(blue, scale).expect("pencil");
        assert!(!under_tag(&pencil, tag_offset_pencil()), "the pencil");
        let hand = hand(blue, scale).expect("hand");
        assert!(!under_tag(&hand, tag_offset_hand()), "the hand");
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

    /// A name is text, whatever's in it.
    #[test]
    fn a_name_can_hold_anything() {
        for name in ["R&D", "<b>", "\"quoted\"", "Zoë 🎉"] {
            assert!(tag(name, [200, 120, 255], 2.).is_some(), "{name}");
        }
    }
}
