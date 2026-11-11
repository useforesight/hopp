//! Pictures for the reactions layer: emoji, words, banners and set pieces (the gong, the
//! table, the rocket…), drawn as SVG and rendered with resvg at the display's density.
//!
//! The resvg here is built without raster images, so it skips colour emoji, whose
//! glyphs are PNGs in the font (Noto Color Emoji). Those are drawn over the rest from the
//! glyph images usvg lays out.

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use resvg::{tiny_skia, usvg};

/// The type the prototypes are set in: the system's sans, bold.
pub const FONT: &str = "Inter, 'Adwaita Sans', sans-serif";
pub const MONO: &str = "'JetBrains Mono', 'DejaVu Sans Mono', monospace";
pub const EMOJI: &str = "'Noto Color Emoji', 'Apple Color Emoji', 'Twemoji', sans-serif";

/// A picture, `size` logical pixels, drawn from SVG `body` (the inside of an `<svg>` of
/// that size). `key` names it in the caches, so two looks with one key must be the same.
pub struct Look {
    pub key: String,
    pub size: [f32; 2],
    body: String,
}

impl Look {
    pub fn new(key: impl Into<String>, size: [f32; 2], body: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            key: key.into(),
            size,
            body: body.into(),
        })
    }

    /// Rendered at `scale` device pixels per logical pixel.
    pub fn render(&self, scale: f32) -> Option<tiny_skia::Pixmap> {
        let [w, h] = self.size;
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" fill="none">{}</svg>"#,
            self.body
        );
        let tree = usvg::Tree::from_str(&svg, &options()).ok()?;
        let (width, height) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1))?;
        let transform = tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        glyph_images(tree.root(), transform, &mut pixmap);
        Some(pixmap)
    }
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            // Omarchy's interface font, else whichever common sans is there.
            let present = |family: &str| {
                db.faces()
                    .any(|face| face.families.iter().any(|(name, _)| name == family))
            };
            if let Some(family) = ["Adwaita Sans", "Inter", "Noto Sans", "DejaVu Sans"]
                .into_iter()
                .find(|family| present(family))
            {
                db.set_sans_serif_family(family);
            }
            Arc::new(db)
        })
        .clone()
}

fn options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: fonts(),
        ..Default::default()
    }
}

/// Draw the colour emoji resvg skipped, as resvg would place them.
fn glyph_images(
    group: &usvg::Group,
    transform: tiny_skia::Transform,
    pixmap: &mut tiny_skia::Pixmap,
) {
    let transform = transform.pre_concat(group.transform());
    for node in group.children() {
        match node {
            usvg::Node::Group(group) => glyph_images(group, transform, pixmap),
            usvg::Node::Text(text) => glyph_images(text.flattened(), transform, pixmap),
            usvg::Node::Image(image) => {
                let usvg::ImageKind::PNG(data) = image.kind() else {
                    continue;
                };
                let Some(glyph) = decode(data) else { continue };
                let paint = tiny_skia::Paint {
                    shader: tiny_skia::Pattern::new(
                        glyph.as_ref(),
                        tiny_skia::SpreadMode::Pad,
                        tiny_skia::FilterQuality::Bicubic,
                        1.,
                        tiny_skia::Transform::default(),
                    ),
                    ..Default::default()
                };
                if let Some(rect) =
                    tiny_skia::Rect::from_xywh(0., 0., glyph.width() as f32, glyph.height() as f32)
                {
                    pixmap.fill_rect(rect, &paint, transform, None);
                }
            }
            usvg::Node::Path(_) => {}
        }
    }
}

/// A PNG as a premultiplied pixmap.
fn decode(data: &[u8]) -> Option<tiny_skia::Pixmap> {
    let image = image::load_from_memory(data).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    for (out, pixel) in pixmap.pixels_mut().iter_mut().zip(image.pixels()) {
        let [r, g, b, a] = pixel.0;
        *out = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pixmap)
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The width of `text` set at `size` in `family` and `weight`, in logical pixels.
pub fn text_width(text: &str, size: f32, weight: u16, family: &str) -> f32 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="2000" height="400"><text x="0" y="200" font-family="{family}" font-size="{size}" font-weight="{weight}">{}</text></svg>"#,
        escape(text)
    );
    usvg::Tree::from_str(&svg, &options())
        .ok()
        .map(|tree| tree.root().abs_bounding_box().width())
        .filter(|width| *width > 0.)
        .unwrap_or_else(|| size * 0.58 * text.chars().count() as f32)
}

/// Emoji `ch` in a square of `size` logical pixels, as the prototypes draw them: large
/// in the middle of their square, and drawn once.
pub fn emoji_sized(ch: &str, size: f32) -> Arc<Look> {
    thread_local! {
        static EMOJI_LOOKS: RefCell<HashMap<(String, u32), Arc<Look>>> = Default::default();
    }
    EMOJI_LOOKS.with(|looks| {
        looks
            .borrow_mut()
            .entry((ch.to_owned(), size.to_bits()))
            .or_insert_with(|| {
                // The prototypes' 104px glyph in a 128px square, its middle a little low.
                let font = size * 104. / 128.;
                Look::new(
                    format!("emoji:{ch}:{size}"),
                    [size, size],
                    format!(
                        r#"<text x="{x}" y="{y}" text-anchor="middle" font-family="{EMOJI}" font-size="{font}">{}</text>"#,
                        escape(ch),
                        x = size / 2.,
                        y = size * 0.5 + font * 0.38,
                    ),
                )
            })
            .clone()
    })
}

/// An emoji for particles and small pieces (up to about 70 logical pixels across).
pub fn emoji(ch: &str) -> Arc<Look> {
    emoji_sized(ch, 72.)
}

/// A soft round glow in `rgb`: white at the heart, the colour, then nothing. The
/// prototypes stamp it for glowing sparks.
pub fn glow(rgb: [u8; 3]) -> Arc<Look> {
    thread_local! {
        static GLOWS: RefCell<HashMap<[u8; 3], Arc<Look>>> = Default::default();
    }
    GLOWS.with(|glows| {
        glows
            .borrow_mut()
            .entry(rgb)
            .or_insert_with(|| {
                let c = hex(rgb);
                Look::new(
                    format!("glow:{c}"),
                    [32., 32.],
                    format!(
                        r##"<defs><radialGradient id="g"><stop offset="0" stop-color="#fff"/><stop offset=".18" stop-color="{c}"/><stop offset=".45" stop-color="{c}" stop-opacity=".33"/><stop offset="1" stop-color="{c}" stop-opacity="0"/></radialGradient></defs><rect width="32" height="32" fill="url(#g)"/>"##
                    ),
                )
            })
            .clone()
    })
}

/// A soft disc of `rgb` fading out from `alpha` at its middle: lights, smoke and glows.
pub fn blob(rgb: [u8; 3], stops: &[(f32, f32)]) -> Arc<Look> {
    let c = hex(rgb);
    let stops: String = stops
        .iter()
        .map(|(at, alpha)| {
            format!(r#"<stop offset="{at}" stop-color="{c}" stop-opacity="{alpha}"/>"#)
        })
        .collect();
    Look::new(
        format!("blob:{c}:{stops}"),
        [48., 48.],
        format!(
            r#"<defs><radialGradient id="g">{stops}</radialGradient></defs><rect width="48" height="48" fill="url(#g)"/>"#
        ),
    )
}

/// Words in one line, `size` high, centred in their picture: the big captions.
pub struct Words {
    pub look: Arc<Look>,
    /// Where the baseline's middle sits, from the top left.
    pub anchor: [f32; 2],
}

/// `text` set in `fill` (any SVG paint; `defs` for gradients) with an optional glow
/// (`shadow`: colour, blur and opacity), room left round it for the glow.
pub fn words(
    text: &str,
    size: f32,
    weight: u16,
    family: &str,
    fill: &str,
    defs: &str,
    shadow: Option<(&str, f32, f32)>,
) -> Words {
    let width = text_width(text, size, weight, family);
    let pad = shadow.map_or(4., |(_, blur, _)| blur * 2.5 + 4.);
    let (w, h) = (width + pad * 2., size * 1.3 + pad * 2.);
    let filter = shadow
        .map(|(colour, blur, opacity)| {
            format!(
                r#"<filter id="s" x="-50%" y="-50%" width="200%" height="200%"><feDropShadow dx="0" dy="0" stdDeviation="{blur}" flood-color="{colour}" flood-opacity="{opacity}"/></filter>"#
            )
        })
        .unwrap_or_default();
    let baseline = pad + size * 1.02;
    let body = format!(
        r#"<defs>{defs}{filter}</defs><text x="{x}" y="{baseline}" text-anchor="middle" font-family="{family}" font-size="{size}" font-weight="{weight}" fill="{fill}"{f}>{}</text>"#,
        escape(text),
        x = w / 2.,
        f = if shadow.is_some() {
            r#" filter="url(#s)""#
        } else {
            ""
        },
    );
    let key = format!("words:{text}:{size}:{weight}:{family}:{fill}:{defs}:{shadow:?}");
    Words {
        look: Look::new(key, [w, h], body),
        anchor: [w / 2., baseline - size * 0.36],
    }
}
