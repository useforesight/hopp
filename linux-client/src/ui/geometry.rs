//! Pure layout math for video: fit, cover, and the follow viewport.
//!
//! A tiled window has whatever shape the compositor gives it, while a shared screen
//! has a fixed one. Rather than letterbox, the viewer treats its window as a viewport
//! onto the remote screen: `image_bounds` may be larger than the view and is clipped,
//! and `frame_point` maps a click back through that crop.
use gpui_kit::{Bounds, Pixels, point, px, size};

use crate::{protocol::Point, settings::FacesInShared};

/// The whole image inside `view`, preserving aspect ratio (letterboxed).
pub fn contain(view: Bounds<Pixels>, source: [u32; 2]) -> Bounds<Pixels> {
    let scale = (f32::from(view.size.width) / source[0].max(1) as f32)
        .min(f32::from(view.size.height) / source[1].max(1) as f32);
    centered(view, source, scale)
}

/// Fill `view` completely, preserving aspect ratio and cropping the overflow.
pub fn cover(view: Bounds<Pixels>, source: [u32; 2]) -> Bounds<Pixels> {
    let scale = (f32::from(view.size.width) / source[0].max(1) as f32)
        .max(f32::from(view.size.height) / source[1].max(1) as f32);
    centered(view, source, scale)
}

fn centered(view: Bounds<Pixels>, source: [u32; 2], scale: f32) -> Bounds<Pixels> {
    let dimensions = size(px(source[0] as f32 * scale), px(source[1] as f32 * scale));
    Bounds {
        origin: point(
            view.origin.x + (view.size.width - dimensions.width) / 2.,
            view.origin.y + (view.size.height - dimensions.height) / 2.,
        ),
        size: dimensions,
    }
}

/// A normalized frame point painted inside `image`.
pub fn point_on_image(image: Bounds<Pixels>, position: Point) -> gpui_kit::Point<Pixels> {
    point(
        image.origin.x + image.size.width * position.x as f32,
        image.origin.y + image.size.height * position.y as f32,
    )
}

/// Space left around a fitted screen. `sideways` bands are left and right of the screen;
/// otherwise they are above and below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    pub sideways: bool,
    /// Thickness of each band in logical pixels.
    pub size: f32,
}

/// Bands thinner than this can't hold faces or controls; the floating pill is used instead.
pub const MIN_DOCK: f32 = 90.;

/// Bands around `source` fitted into `view`, if they are wide enough to dock into.
pub fn dock_bands(view: Bounds<Pixels>, source: [u32; 2]) -> Option<Bands> {
    let image = contain(view, source);
    let x = f32::from(image.origin.x - view.origin.x);
    let y = f32::from(image.origin.y - view.origin.y);
    let bands = Bands {
        sideways: x >= y,
        size: x.max(y),
    };
    (bands.size >= MIN_DOCK).then_some(bands)
}

/// Where the faces go in a shared screen's window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dock {
    /// Not here: the call window has them.
    Away,
    /// In the bands either side of a fitted screen.
    Bands(Bands),
    /// In a rail of their own down the right (`sideways`) or along the bottom; the screen
    /// has the rest of the window.
    Rail(Bands),
    /// Round faces over the screen's top-right corner.
    Bubbles,
}

/// A rail's share of the window, and the least and most it takes, in logical pixels.
const SIDE_RAIL: (f32, f32, f32) = (0.18, 128., 260.);
const BOTTOM_RAIL: (f32, f32, f32) = (0.2, 112., 220.);

/// Where the faces go in `window`, showing `source` (if it has arrived), as `faces` asks.
/// A fitted screen's own bands are used when they're wide enough; peek uses nothing else.
pub fn dock(
    faces: FacesInShared,
    fit: bool,
    peek: bool,
    window: Bounds<Pixels>,
    source: Option<[u32; 2]>,
) -> Dock {
    let Some(source) = source else {
        return Dock::Away;
    };
    match faces {
        FacesInShared::Off => Dock::Away,
        FacesInShared::Bubbles => Dock::Bubbles,
        FacesInShared::Side | FacesInShared::Bottom => {
            if fit && let Some(bands) = dock_bands(window, source) {
                return Dock::Bands(bands);
            }
            if peek {
                return Dock::Away;
            }
            let sideways = faces == FacesInShared::Side;
            let (share, least, most) = if sideways { SIDE_RAIL } else { BOTTOM_RAIL };
            let length = f32::from(if sideways {
                window.size.width
            } else {
                window.size.height
            });
            // A small window keeps all of itself for the screen.
            if length < 3. * least {
                return Dock::Away;
            }
            Dock::Rail(Bands {
                sideways,
                size: (length * share).clamp(least, most),
            })
        }
    }
}

/// The part of `window` the screen has: all of it, unless a rail takes some.
pub fn screen_area(window: Bounds<Pixels>, dock: Dock) -> Bounds<Pixels> {
    match dock {
        Dock::Rail(rail) if rail.sideways => Bounds {
            origin: window.origin,
            size: size(window.size.width - px(rail.size), window.size.height),
        },
        Dock::Rail(rail) => Bounds {
            origin: window.origin,
            size: size(window.size.width, window.size.height - px(rail.size)),
        },
        _ => window,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// Whole screen, letterboxed. Used by peek.
    Fit,
    /// Readable scale, cropped to the window and panned to follow the sharer.
    Follow,
    /// One remote pixel per device pixel, panned to follow.
    Actual,
}

/// Viewport state for one viewer window.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub mode: ViewMode,
    /// Multiplier on the readable scale in follow mode.
    pub zoom: f32,
    /// Normalized frame point at the centre of the view.
    pub center: [f32; 2],
    /// Where the centre is easing towards.
    pub target: [f32; 2],
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            mode: ViewMode::Follow,
            zoom: 1.,
            center: [0.5, 0.5],
            target: [0.5, 0.5],
        }
    }
}

pub const MIN_ZOOM: f32 = 0.5;
pub const MAX_ZOOM: f32 = 3.;

impl Viewport {
    /// Device pixels per source pixel for the current mode.
    pub fn scale(&self, view: Bounds<Pixels>, source: [u32; 2], scale_factor: f32) -> f32 {
        let width = f32::from(view.size.width);
        let height = f32::from(view.size.height);
        let fit = (width / source[0].max(1) as f32).min(height / source[1].max(1) as f32);
        let fill = (width / source[0].max(1) as f32).max(height / source[1].max(1) as f32);
        // One source pixel per physical pixel keeps remote text crisp and close to the
        // size the sharer sees. Never smaller than filling the window, so a crop never
        // shows bars, and never so large that a pixel is blown up past 3×.
        let actual = 1. / scale_factor.max(0.5);
        match self.mode {
            ViewMode::Fit => fit,
            ViewMode::Actual => actual.max(fill),
            ViewMode::Follow => (actual * self.zoom).max(fill).min(fill.max(3. * actual)),
        }
    }

    /// Where the full frame lands for this view; may extend past the view (cropped).
    pub fn image_bounds(
        &self,
        view: Bounds<Pixels>,
        source: [u32; 2],
        scale_factor: f32,
    ) -> Bounds<Pixels> {
        if self.mode == ViewMode::Fit {
            return contain(view, source);
        }
        let scale = self.scale(view, source, scale_factor);
        let width = source[0] as f32 * scale;
        let height = source[1] as f32 * scale;
        let [cx, cy] = self.clamped(self.center, view, width, height);
        let left = f32::from(view.origin.x) + f32::from(view.size.width) / 2. - cx * width;
        let top = f32::from(view.origin.y) + f32::from(view.size.height) / 2. - cy * height;
        Bounds {
            origin: point(px(left), px(top)),
            size: size(px(width), px(height)),
        }
    }

    /// Keep the image covering the view on any axis where it is larger, and centred
    /// on any axis where it is smaller.
    fn clamped(&self, center: [f32; 2], view: Bounds<Pixels>, width: f32, height: f32) -> [f32; 2] {
        let axis = |value: f32, visible: f32, total: f32| {
            if total <= visible {
                0.5
            } else {
                let half = visible / total / 2.;
                value.clamp(half, 1. - half)
            }
        };
        [
            axis(center[0], f32::from(view.size.width), width),
            axis(center[1], f32::from(view.size.height), height),
        ]
    }

    /// Normalized frame point under a window position, or `None` when it falls outside
    /// the visible image (letterbox or clipped area), so it can never control the peer.
    pub fn frame_point(
        &self,
        position: gpui_kit::Point<Pixels>,
        view: Bounds<Pixels>,
        source: [u32; 2],
        scale_factor: f32,
    ) -> Option<Point> {
        if !view.contains(&position) {
            return None;
        }
        let image = self.image_bounds(view, source, scale_factor);
        let x = f32::from(position.x - image.origin.x) / f32::from(image.size.width);
        let y = f32::from(position.y - image.origin.y) / f32::from(image.size.height);
        ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some(Point {
            x: f64::from(x),
            y: f64::from(y),
        })
    }

    /// The visible part of the frame, normalized, for the minimap.
    pub fn visible(&self, view: Bounds<Pixels>, source: [u32; 2], scale_factor: f32) -> [f32; 4] {
        let image = self.image_bounds(view, source, scale_factor);
        let width = f32::from(image.size.width).max(1.);
        let height = f32::from(image.size.height).max(1.);
        let left = (f32::from(view.origin.x - image.origin.x) / width).clamp(0., 1.);
        let top = (f32::from(view.origin.y - image.origin.y) / height).clamp(0., 1.);
        let right =
            (f32::from(view.origin.x + view.size.width - image.origin.x) / width).clamp(0., 1.);
        let bottom =
            (f32::from(view.origin.y + view.size.height - image.origin.y) / height).clamp(0., 1.);
        [left, top, right - left, bottom - top]
    }

    /// Whether panning can reveal anything (the image is bigger than the view).
    pub fn is_cropped(&self, view: Bounds<Pixels>, source: [u32; 2], scale_factor: f32) -> bool {
        let [_, _, width, height] = self.visible(view, source, scale_factor);
        width < 0.999 || height < 0.999
    }

    /// Retarget so `focus` stays inside the middle 60% of the view. Moving the cursor
    /// within that dead zone never pans, which keeps the picture calm while reading.
    pub fn follow(
        &mut self,
        focus: Point,
        view: Bounds<Pixels>,
        source: [u32; 2],
        scale_factor: f32,
    ) {
        let scale = self.scale(view, source, scale_factor);
        let visible_w = f32::from(view.size.width) / (source[0] as f32 * scale);
        let visible_h = f32::from(view.size.height) / (source[1] as f32 * scale);
        let axis = |target: f32, focus: f32, visible: f32| {
            let margin = visible * 0.3;
            if focus < target - margin {
                focus + margin
            } else if focus > target + margin {
                focus - margin
            } else {
                target
            }
        };
        self.target = [
            axis(self.target[0], focus.x as f32, visible_w),
            axis(self.target[1], focus.y as f32, visible_h),
        ];
        self.target = self.clamped(
            self.target,
            view,
            source[0] as f32 * scale,
            source[1] as f32 * scale,
        );
    }

    /// Jump straight to a frame point (minimap click).
    pub fn jump(
        &mut self,
        focus: [f32; 2],
        view: Bounds<Pixels>,
        source: [u32; 2],
        scale_factor: f32,
    ) {
        let scale = self.scale(view, source, scale_factor);
        self.target = self.clamped(
            focus,
            view,
            source[0] as f32 * scale,
            source[1] as f32 * scale,
        );
        self.center = self.target;
    }

    /// Ease the centre towards the target. Returns whether another frame is needed.
    pub fn step(&mut self, dt: f32) -> bool {
        let t = (dt * 9.).clamp(0., 1.);
        let mut moving = false;
        for axis in 0..2 {
            let delta = self.target[axis] - self.center[axis];
            if delta.abs() < 0.0005 {
                self.center[axis] = self.target[axis];
            } else {
                self.center[axis] += delta * t;
                moving = true;
            }
        }
        moving
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(width), px(height)),
        }
    }

    #[test]
    fn the_faces_take_a_rail_unless_the_screen_leaves_bands_for_them() {
        let screen = Some([2560, 1600]);
        let tile = view(0., 0., 1600., 900.);
        // Cropped: a rail down the side, and the screen has the rest.
        let side = dock(FacesInShared::Side, false, false, tile, screen);
        assert_eq!(
            side,
            Dock::Rail(Bands {
                sideways: true,
                size: 260.
            })
        );
        assert_eq!(screen_area(tile, side), view(0., 0., 1340., 900.));
        let bottom = dock(FacesInShared::Bottom, false, false, tile, screen);
        assert_eq!(
            bottom,
            Dock::Rail(Bands {
                sideways: false,
                size: 180.
            })
        );
        assert_eq!(screen_area(tile, bottom), view(0., 0., 1600., 720.));
        // Fitted into an ultrawide tile, the bands either side are room enough.
        let wide = view(0., 0., 1600., 675.);
        assert!(matches!(
            dock(FacesInShared::Bottom, true, false, wide, screen),
            Dock::Bands(Bands { sideways: true, .. })
        ));
        // Fitted with no room around it, a rail all the same; peek leaves it be.
        assert!(matches!(
            dock(FacesInShared::Side, true, false, tile, screen),
            Dock::Rail(_)
        ));
        assert_eq!(
            dock(FacesInShared::Side, true, true, tile, screen),
            Dock::Away
        );
    }

    #[test]
    fn the_faces_stay_in_the_call_window_when_asked_or_theres_no_room() {
        let screen = Some([2560, 1600]);
        let tile = view(0., 0., 1600., 900.);
        assert_eq!(
            dock(FacesInShared::Off, false, false, tile, screen),
            Dock::Away
        );
        assert_eq!(
            dock(FacesInShared::Side, false, false, tile, None),
            Dock::Away
        );
        let narrow = view(0., 0., 360., 900.);
        assert_eq!(
            dock(FacesInShared::Side, false, false, narrow, screen),
            Dock::Away
        );
        assert_eq!(
            dock(FacesInShared::Bubbles, false, false, narrow, screen),
            Dock::Bubbles
        );
        assert_eq!(screen_area(tile, Dock::Bubbles), tile);
    }

    #[test]
    fn docks_only_into_bands_wide_enough_for_faces() {
        // 2560×1600 on an ultrawide tile: 260 px either side.
        let wide = dock_bands(view(0., 0., 1600., 675.), [2560, 1600]).unwrap();
        assert!(wide.sideways && (wide.size - 260.).abs() < 0.5);
        // A tall tile puts the bands above and below.
        let tall = dock_bands(view(0., 0., 620., 860.), [2560, 1600]).unwrap();
        assert!(!tall.sideways && tall.size > 200.);
        // 16:10 on a 16:9 tile leaves slivers: no dock.
        assert!(dock_bands(view(0., 0., 1600., 900.), [2560, 1600]).is_none());
    }

    #[test]
    fn contain_letterboxes_and_cover_crops() {
        let fitted = contain(view(100., 100., 400., 400.), [1920, 1080]);
        assert_eq!(f32::from(fitted.size.height), 225.);
        assert_eq!(f32::from(fitted.origin.y), 187.5);
        let covered = cover(view(0., 0., 120., 120.), [1280, 720]);
        assert_eq!(f32::from(covered.size.height), 120.);
        assert!((f32::from(covered.size.width) - 213.333).abs() < 0.01);
        assert!(f32::from(covered.origin.x) < 0.);
    }

    #[test]
    fn fit_round_trips_and_rejects_letterbox() {
        let viewport = Viewport {
            mode: ViewMode::Fit,
            ..Default::default()
        };
        let frame = view(100., 100., 400., 400.);
        let image = viewport.image_bounds(frame, [1920, 1080], 1.);
        let centre = point_on_image(image, Point { x: 0.5, y: 0.5 });
        let back = viewport
            .frame_point(centre, frame, [1920, 1080], 1.)
            .unwrap();
        assert!((back.x - 0.5).abs() < 1e-6 && (back.y - 0.5).abs() < 1e-6);
        assert!(
            viewport
                .frame_point(point(px(300.), px(120.)), frame, [1920, 1080], 1.)
                .is_none(),
            "a click in the letterbox must not reach the peer"
        );
    }

    #[test]
    fn follow_crop_maps_clicks_through_the_pan() {
        // 2560×1600 remote in a narrow 600×800 tile on a 2× display.
        let mut viewport = Viewport::default();
        let frame = view(0., 0., 600., 800.);
        let source = [2560, 1600];
        // A tall tile must fill its height, so ~47% of the width is visible and the
        // centre can travel between ~0.23 and ~0.77.
        viewport.jump([0.6, 0.3], frame, source, 2.);
        let image = viewport.image_bounds(frame, source, 2.);
        assert!(
            f32::from(image.size.width) > 600.,
            "follow crops instead of shrinking"
        );
        assert!(
            f32::from(image.size.height) >= 800.,
            "no bars in a tall tile"
        );
        let centre = viewport
            .frame_point(point(px(300.), px(400.)), frame, source, 2.)
            .unwrap();
        assert!((centre.x - 0.6).abs() < 0.01, "{centre:?}");
        assert!(
            (centre.y - 0.5).abs() < 0.01,
            "no vertical room to pan: {centre:?}"
        );
        let mapped = point_on_image(image, centre);
        assert!((f32::from(mapped.x) - 300.).abs() < 0.01);
        assert!(
            viewport
                .frame_point(point(px(700.), px(400.)), frame, source, 2.)
                .is_none(),
            "outside the window is never forwarded"
        );
    }

    #[test]
    fn invalid_positions_and_empty_frames_never_control_the_peer() {
        let viewport = Viewport::default();
        let frame = view(0., 0., 400., 400.);
        assert!(
            viewport
                .frame_point(point(px(f32::NAN), px(10.)), frame, [1920, 1080], 1.)
                .is_none()
        );
        assert!(
            viewport
                .frame_point(point(px(200.), px(200.)), frame, [0, 0], 1.)
                .is_none_or(|point| point.valid())
        );
    }

    #[test]
    fn pan_is_clamped_to_the_frame_edges() {
        let mut viewport = Viewport::default();
        let frame = view(0., 0., 600., 400.);
        viewport.jump([0., 0.], frame, [2560, 1600], 1.);
        let image = viewport.image_bounds(frame, [2560, 1600], 1.);
        assert_eq!(f32::from(image.origin.x), 0.);
        assert_eq!(f32::from(image.origin.y), 0.);
        let [left, top, width, height] = viewport.visible(frame, [2560, 1600], 1.);
        assert_eq!([left, top], [0., 0.]);
        assert!((width - 600. / 2560.).abs() < 1e-4 && (height - 0.25).abs() < 1e-4);
    }

    #[test]
    fn follow_keeps_a_dead_zone_then_tracks_the_cursor() {
        let mut viewport = Viewport::default();
        let frame = view(0., 0., 640., 400.);
        let source = [2560, 1600];
        viewport.jump([0.5, 0.5], frame, source, 1.);
        viewport.follow(Point { x: 0.52, y: 0.5 }, frame, source, 1.);
        assert_eq!(viewport.target, [0.5, 0.5], "small moves do not pan");
        viewport.follow(Point { x: 0.95, y: 0.5 }, frame, source, 1.);
        assert!(viewport.target[0] > 0.5);
        while viewport.step(1. / 60.) {}
        let visible = viewport.visible(frame, source, 1.);
        assert!(0.95 >= visible[0] && 0.95 <= visible[0] + visible[2]);
    }

    #[test]
    fn follow_never_letterboxes_even_when_the_tile_is_larger_than_the_frame() {
        let viewport = Viewport::default();
        let frame = view(0., 0., 3000., 1200.);
        let image = viewport.image_bounds(frame, [1280, 800], 1.);
        assert!(f32::from(image.size.width) >= 3000.);
        assert!(f32::from(image.size.height) >= 1200.);
    }
}
