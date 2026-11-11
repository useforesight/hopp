//! One frame of the reactions layer as a list of simple pieces, and the painters for it:
//! GPUI's, for the screen, and tiny-skia's, for the preview images the tests write.
//!
//! The pieces are what GPUI draws quickly: polygons (CPU-transformed, so they turn and
//! squash freely, and batched one path per colour), rounded rectangles, discs and rings
//! (quads), and pictures (see [`super::looks`]), which can't turn and fade in cached
//! steps of [`ALPHA_STEPS`].

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use gpui_kit::{Bounds, Corners, Pixels, RenderImage, Window, point, px, size};

use super::looks::Look;
use crate::ui::marks::colour;

/// Pictures fade through this many cached opacities.
pub const ALPHA_STEPS: f32 = 16.;

pub fn step_alpha(alpha: f32) -> f32 {
    (alpha.clamp(0., 1.) * ALPHA_STEPS).round() / ALPHA_STEPS
}

#[derive(Clone)]
pub enum Item {
    /// Polygons in one colour; `ends` closes each one in `points`. Each is fanned from its
    /// first point, so it must be convex or star-shaped around that point.
    Shapes {
        rgb: [u8; 3],
        alpha: f32,
        points: Vec<[f32; 2]>,
        ends: Vec<usize>,
    },
    Rect {
        at: [f32; 2],
        size: [f32; 2],
        radius: f32,
        rgb: [u8; 3],
        alpha: f32,
    },
    Disc {
        at: [f32; 2],
        radius: f32,
        rgb: [u8; 3],
        alpha: f32,
    },
    Ring {
        at: [f32; 2],
        radius: f32,
        width: f32,
        rgb: [u8; 3],
        alpha: f32,
    },
    /// `look` stretched over the box at `at` of `size`.
    Picture {
        look: Arc<Look>,
        at: [f32; 2],
        size: [f32; 2],
        alpha: f32,
    },
}

/// What a frame shows: `items` shaken by `shake`, then the banners, which stay still.
#[derive(Default)]
pub struct Frame {
    pub items: Vec<Item>,
    pub banners: Vec<Item>,
    pub shake: [f32; 2],
}

impl Frame {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.banners.is_empty()
    }
}

/// Pieces added in order, with helpers for the common ones.
#[derive(Default)]
pub struct Canvas {
    pub items: Vec<Item>,
}

impl Canvas {
    pub fn polygon(
        &mut self,
        points: impl IntoIterator<Item = [f32; 2]>,
        rgb: [u8; 3],
        alpha: f32,
    ) {
        if alpha <= 0.004 {
            return;
        }
        let points: Vec<_> = points.into_iter().collect();
        let end = points.len();
        self.items.push(Item::Shapes {
            rgb,
            alpha,
            points,
            ends: vec![end],
        });
    }

    pub fn rect(&mut self, at: [f32; 2], size: [f32; 2], radius: f32, rgb: [u8; 3], alpha: f32) {
        if alpha > 0.004 && size[0] > 0. && size[1] > 0. {
            self.items.push(Item::Rect {
                at,
                size,
                radius,
                rgb,
                alpha,
            });
        }
    }

    pub fn disc(&mut self, at: [f32; 2], radius: f32, rgb: [u8; 3], alpha: f32) {
        if alpha > 0.004 && radius > 0. {
            self.items.push(Item::Disc {
                at,
                radius,
                rgb,
                alpha,
            });
        }
    }

    pub fn ring(&mut self, at: [f32; 2], radius: f32, width: f32, rgb: [u8; 3], alpha: f32) {
        if alpha > 0.004 && radius > 0. && width > 0. {
            self.items.push(Item::Ring {
                at,
                radius,
                width,
                rgb,
                alpha,
            });
        }
    }

    /// `look` with its middle at `centre`, scaled by `scale`.
    pub fn picture(&mut self, look: &Arc<Look>, centre: [f32; 2], scale: f32, alpha: f32) {
        let [w, h] = look.size;
        self.picture_in(
            look,
            [centre[0] - w * scale / 2., centre[1] - h * scale / 2.],
            [w * scale, h * scale],
            alpha,
        );
    }

    /// `look` stretched over the box at `at` of `size`.
    pub fn picture_in(&mut self, look: &Arc<Look>, at: [f32; 2], size: [f32; 2], alpha: f32) {
        if step_alpha(alpha) > 0. && size[0] > 0.5 && size[1] > 0.5 {
            self.items.push(Item::Picture {
                look: look.clone(),
                at,
                size,
                alpha,
            });
        }
    }
}

/// `p` squashed vertically by `squash`, turned by `turn` and moved to `centre`: a canvas
/// `translate(centre) rotate(turn) scale(1, squash)`.
pub fn place(p: [f32; 2], centre: [f32; 2], turn: f32, squash: f32) -> [f32; 2] {
    let (sin, cos) = turn.sin_cos();
    let (x, y) = (p[0], p[1] * squash);
    [centre[0] + x * cos - y * sin, centre[1] + x * sin + y * cos]
}

// ---- GPUI -----------------------------------------------------------------------------

/// A picture at one scale and opacity, uploaded: the image and its size in device pixels.
type Uploaded = Arc<RenderImage>;

/// Pictures for one window: rendered once per scale, uploaded once per opacity step, and
/// released once a frame no longer shows them.
#[derive(Default)]
pub struct Pictures {
    rendered: HashMap<(String, u32), Option<Arc<resvg::tiny_skia::Pixmap>>>,
    rendered_used: HashSet<(String, u32)>,
    uploaded: HashMap<(String, u32, u32), Uploaded>,
    used: HashSet<(String, u32, u32)>,
    /// Frames since the rendered pictures were last pruned.
    frames: u32,
}

impl Pictures {
    fn rendered(&mut self, look: &Look, scale: f32) -> Option<Arc<resvg::tiny_skia::Pixmap>> {
        let key = (look.key.clone(), scale.to_bits());
        self.rendered_used.insert(key.clone());
        self.rendered
            .entry(key)
            .or_insert_with(|| look.render(scale).map(Arc::new))
            .clone()
    }

    fn get(&mut self, look: &Look, scale: f32, alpha: f32) -> Option<Uploaded> {
        let key = (look.key.clone(), scale.to_bits(), alpha.to_bits());
        self.used.insert(key.clone());
        if let Some(found) = self.uploaded.get(&key) {
            return Some(found.clone());
        }
        let pixmap = self.rendered(look, scale)?;
        // GPUI wants straight-alpha BGRA.
        let mut bgra = Vec::with_capacity(pixmap.data().len());
        for pixel in pixmap.pixels() {
            let c = pixel.demultiply();
            let a = (f32::from(c.alpha()) * alpha).round() as u8;
            bgra.extend_from_slice(&[c.blue(), c.green(), c.red(), a]);
        }
        let buffer = image::RgbaImage::from_raw(pixmap.width(), pixmap.height(), bgra)?;
        let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
        self.uploaded.insert(key, image.clone());
        Some(image)
    }

    /// Release what this frame didn't show. Rendered pictures are kept a couple of
    /// seconds, so one that fades in and out isn't rendered again for each step.
    pub fn finish(&mut self, window: &mut Window) {
        let used = std::mem::take(&mut self.used);
        let stale: Vec<_> = self
            .uploaded
            .keys()
            .filter(|key| !used.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            if let Some(image) = self.uploaded.remove(&key) {
                let _ = window.drop_image(image);
            }
        }
        self.frames += 1;
        if self.frames >= 120 {
            self.frames = 0;
            let used = std::mem::take(&mut self.rendered_used);
            self.rendered.retain(|key, _| used.contains(key));
        }
    }
}

/// How finely to render `look` drawn `extent` big: at the display's density, or twice or
/// four times that for a picture drawn larger than its own size. In steps, so a picture
/// that grows is rendered only a few times.
fn density(look: &Look, extent: [f32; 2], scale: f32) -> f32 {
    let drawn = (extent[0] / look.size[0]).max(extent[1] / look.size[1]);
    if drawn > 2.1 {
        scale * 4.
    } else if drawn > 1.1 {
        scale * 2.
    } else {
        scale
    }
}

fn gpoint([x, y]: [f32; 2], [dx, dy]: [f32; 2]) -> gpui_kit::Point<Pixels> {
    point(px(x + dx), px(y + dy))
}

fn bounds(at: [f32; 2], extent: [f32; 2], shift: [f32; 2]) -> Bounds<Pixels> {
    Bounds {
        origin: gpoint(at, shift),
        size: size(px(extent[0]), px(extent[1])),
    }
}

/// Paint `frame` with its top left at `origin`.
pub fn paint(window: &mut Window, pictures: &mut Pictures, frame: &Frame, origin: [f32; 2]) {
    let shaken = [origin[0] + frame.shake[0], origin[1] + frame.shake[1]];
    for item in &frame.items {
        paint_item(window, pictures, item, shaken);
    }
    for item in &frame.banners {
        paint_item(window, pictures, item, origin);
    }
    pictures.finish(window);
}

fn paint_item(window: &mut Window, pictures: &mut Pictures, item: &Item, shift: [f32; 2]) {
    match item {
        Item::Shapes {
            rgb,
            alpha,
            points,
            ends,
        } => {
            let mut path: Option<gpui_kit::Path<Pixels>> = None;
            let mut start = 0;
            for &end in ends {
                let shape = &points[start..end];
                start = end;
                if shape.len() < 3 {
                    continue;
                }
                let first = gpoint(shape[0], shift);
                let path = match &mut path {
                    Some(path) => {
                        path.move_to(first);
                        path
                    }
                    None => path.insert(gpui_kit::Path::new(first)),
                };
                for &p in &shape[1..] {
                    path.line_to(gpoint(p, shift));
                }
            }
            if let Some(path) = path {
                window.paint_path(path, colour(*rgb, *alpha));
            }
        }
        Item::Rect {
            at,
            size: extent,
            radius,
            rgb,
            alpha,
        } => window.paint_quad(
            gpui_kit::fill(bounds(*at, *extent, shift), colour(*rgb, *alpha))
                .corner_radii(Corners::all(px(*radius))),
        ),
        Item::Disc {
            at,
            radius,
            rgb,
            alpha,
        } => window.paint_quad(
            gpui_kit::fill(
                bounds(
                    [at[0] - radius, at[1] - radius],
                    [radius * 2., radius * 2.],
                    shift,
                ),
                colour(*rgb, *alpha),
            )
            .corner_radii(Corners::all(px(*radius))),
        ),
        Item::Ring {
            at,
            radius,
            width,
            rgb,
            alpha,
        } => {
            let outer = radius + width / 2.;
            window.paint_quad(
                gpui_kit::fill(
                    bounds(
                        [at[0] - outer, at[1] - outer],
                        [outer * 2., outer * 2.],
                        shift,
                    ),
                    colour(*rgb, 0.),
                )
                .corner_radii(Corners::all(px(outer)))
                .border_widths(px(*width))
                .border_color(colour(*rgb, *alpha)),
            );
        }
        Item::Picture {
            look,
            at,
            size: extent,
            alpha,
        } => {
            let density = density(look, *extent, window.scale_factor());
            if let Some(image) = pictures.get(look, density, step_alpha(*alpha)) {
                let place = bounds(*at, *extent, shift);
                let _ = window.paint_image(place, place, Corners::default(), image, 0, false);
            }
        }
    }
}

// ---- Previews -------------------------------------------------------------------------

/// Paint `frame` into an RGBA image `width` × `height` logical pixels at `scale`, over
/// `backdrop` if given: what the overlay would show, for the preview tests.
#[cfg(test)]
pub fn preview(
    frame: &Frame,
    width: u32,
    height: u32,
    scale: f32,
    backdrop: Option<&resvg::tiny_skia::Pixmap>,
) -> resvg::tiny_skia::Pixmap {
    use resvg::tiny_skia::{
        Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
    };
    let mut pixmap = Pixmap::new(
        (width as f32 * scale) as u32,
        (height as f32 * scale) as u32,
    )
    .unwrap();
    if let Some(backdrop) = backdrop {
        pixmap.draw_pixmap(
            0,
            0,
            backdrop.as_ref(),
            &PixmapPaint::default(),
            Transform::default(),
            None,
        );
    }
    let mut rendered: HashMap<(String, u32), Option<Pixmap>> = HashMap::new();
    let solid = |rgb: [u8; 3], alpha: f32| {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(
            rgb[0],
            rgb[1],
            rgb[2],
            (alpha.clamp(0., 1.) * 255.).round() as u8,
        ));
        paint.anti_alias = true;
        paint
    };
    let rounded = |at: [f32; 2], extent: [f32; 2], radius: f32| {
        let r = radius.min(extent[0] / 2.).min(extent[1] / 2.);
        let (x, y, w, h) = (at[0], at[1], extent[0], extent[1]);
        let mut pb = PathBuilder::new();
        // Arcs as cubics: k is the usual circle constant.
        let k = 0.5523 * r;
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
        pb.line_to(x, y + r);
        pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
        pb.close();
        pb.finish()
    };
    let mut paint_items = |items: &[Item], shift: [f32; 2]| {
        let transform = Transform::from_scale(scale, scale).pre_translate(shift[0], shift[1]);
        for item in items {
            match item {
                Item::Shapes {
                    rgb,
                    alpha,
                    points,
                    ends,
                } => {
                    let mut pb = PathBuilder::new();
                    let mut start = 0;
                    for &end in ends {
                        let shape = &points[start..end];
                        start = end;
                        if shape.len() < 3 {
                            continue;
                        }
                        pb.move_to(shape[0][0], shape[0][1]);
                        for p in &shape[1..] {
                            pb.line_to(p[0], p[1]);
                        }
                        pb.close();
                    }
                    if let Some(path) = pb.finish() {
                        pixmap.fill_path(
                            &path,
                            &solid(*rgb, *alpha),
                            FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                }
                Item::Rect {
                    at,
                    size: extent,
                    radius,
                    rgb,
                    alpha,
                } => {
                    if let Some(path) = rounded(*at, *extent, *radius) {
                        pixmap.fill_path(
                            &path,
                            &solid(*rgb, *alpha),
                            FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                }
                Item::Disc {
                    at,
                    radius,
                    rgb,
                    alpha,
                } => {
                    if let Some(path) = PathBuilder::from_circle(at[0], at[1], *radius) {
                        pixmap.fill_path(
                            &path,
                            &solid(*rgb, *alpha),
                            FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                }
                Item::Ring {
                    at,
                    radius,
                    width,
                    rgb,
                    alpha,
                } => {
                    if let Some(path) = PathBuilder::from_circle(at[0], at[1], *radius) {
                        let stroke = Stroke {
                            width: *width,
                            ..Default::default()
                        };
                        pixmap.stroke_path(&path, &solid(*rgb, *alpha), &stroke, transform, None);
                    }
                }
                Item::Picture {
                    look,
                    at,
                    size: extent,
                    alpha,
                } => {
                    let density = density(look, *extent, scale);
                    let Some(image) = rendered
                        .entry((look.key.clone(), density.to_bits()))
                        .or_insert_with(|| look.render(density))
                        .as_ref()
                    else {
                        continue;
                    };
                    let Some(rect) =
                        Rect::from_xywh(0., 0., image.width() as f32, image.height() as f32)
                    else {
                        continue;
                    };
                    let place = transform.pre_translate(at[0], at[1]).pre_scale(
                        extent[0] / image.width() as f32,
                        extent[1] / image.height() as f32,
                    );
                    let paint = Paint {
                        shader: resvg::tiny_skia::Pattern::new(
                            image.as_ref(),
                            resvg::tiny_skia::SpreadMode::Pad,
                            resvg::tiny_skia::FilterQuality::Bilinear,
                            step_alpha(*alpha),
                            Transform::default(),
                        ),
                        ..Default::default()
                    };
                    pixmap.fill_rect(rect, &paint, place, None);
                }
            }
        }
    };
    paint_items(&frame.items, frame.shake);
    paint_items(&frame.banners, [0., 0.]);
    pixmap
}
