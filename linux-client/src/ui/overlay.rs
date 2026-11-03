//! What people put on your screen while you share it: their cursors, ink, ripples and
//! stamps, painted over the shared monitor by the same painter the viewer uses.
//!
//! It is one transparent layer-shell surface over the whole monitor, above everything,
//! that takes no input (clicks go straight through to your apps) and is excluded from the
//! screen share by Hopp's Hyprland rules: viewers paint their own copy, crisp at their
//! own scale, instead of seeing this one in the video.
use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Pixels, Render, Styled,
    Window, WindowHandle, canvas, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    px, size,
};

use super::session::{Session, SessionEvent};

pub const NAMESPACE: &str = "hopp-marks";

pub struct MarksOverlay {
    session: Entity<Session>,
    sprites: std::rc::Rc<std::cell::RefCell<super::marks::Sprites>>,
}

impl MarksOverlay {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&session, |_, _, event, cx| {
            if matches!(event, SessionEvent::Screen) {
                cx.notify();
            }
        })
        .detach();
        Self {
            session,
            sprites: Default::default(),
        }
    }
}

impl Render for MarksOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scene = self.session.read(cx).scene(true);
        let sprites = self.sprites.clone();
        div().size_full().child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds: Bounds<Pixels>, window, cx| {
                    let mut sprites = sprites.borrow_mut();
                    let animating =
                        super::marks::paint_scene(window, cx, &mut sprites, &scene, bounds, bounds);
                    if animating {
                        window.request_animation_frame();
                    }
                },
            )
            .size_full(),
        )
    }
}

/// The display showing the shared monitor, found by its Wayland output name ("DP-2").
///
/// Not by size: GPUI's Wayland backend derives display bounds from the integer
/// `wl_output.scale`, so a monitor at a fractional scale (1.6) reports the wrong logical
/// size. GPUI names each display's uuid after its output name, so that is the match.
/// GPUI also keeps outputs that were unplugged and plugged back in; the stale ones share
/// the name, and the live one is the latest bound (the highest id).
///
/// `None` when the shared monitor isn't among the displays; `Some(None)` when the share
/// names no monitor, or no displays are known (the screenshot harness), and the
/// compositor places the surface.
pub(super) fn shared_display(
    target: &crate::screencast::CaptureTarget,
    cx: &gpui_kit::App,
) -> Option<Option<gpui_kit::DisplayId>> {
    let displays = cx.displays();
    let Some(name) = target.output_name.as_deref() else {
        return Some(None);
    };
    if displays.is_empty() {
        return Some(None);
    }
    let wanted = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes());
    let found = displays
        .iter()
        .filter(|display| display.uuid().is_ok_and(|uuid| uuid == wanted))
        .map(|display| display.id())
        .max_by_key(|&id| u64::from(id));
    if found.is_none() {
        crate::logger::line(format!(
            "Marks overlay: the shared monitor {name} isn't among {} displays",
            displays.len()
        ));
    }
    found.map(Some)
}

/// Open the overlay over the shared monitor.
pub fn open(
    session: Entity<Session>,
    target: &crate::screencast::CaptureTarget,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<MarksOverlay>> {
    let (width, height) = (target.logical_width as f32, target.logical_height as f32);
    let display_id = shared_display(target, cx)?;
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(width), px(height)),
        })),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: NAMESPACE.into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            // Over the bar too: the whole monitor, as the share sees it.
            exclusive_zone: Some(px(-1.)),
            exclusive_edge: None,
            margin: None,
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        display_id,
        app_id: Some("hopp.marks".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    let handle = match cx.open_window(options, |_, cx| {
        cx.new(|cx| MarksOverlay::new(layer_session, cx))
    }) {
        Ok(handle) => handle,
        // Compositors without layer-shell (the screenshot harness) get a plain window.
        Err(_) => cx
            .open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(width), px(height)),
                    })),
                    app_id: Some("hopp.marks".into()),
                    focus: false,
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| MarksOverlay::new(session, cx)),
            )
            .ok()?,
    };
    // Clicks and scrolling go through to whatever is underneath.
    let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&[])));
    Some(handle)
}
