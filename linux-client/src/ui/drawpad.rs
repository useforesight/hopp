//! Drawing on your own screen while you share it, as Tuple lets the sharer do.
//!
//! The marks overlay over the shared monitor lets every click through, so you can work
//! while others draw. To draw yourself (Draw on the sharing pill, Super+Alt+D, or
//! `hopp draw`), this pad covers the same monitor and turns the pointer into ink: your
//! strokes go to everyone in the call as Hopp's usual draw messages, and the overlay
//! underneath paints them for you. It takes the keyboard when it opens: Escape stops,
//! right-click or C clears your ink, Ctrl+Z undoes your last stroke and K keeps your ink
//! instead of letting it fade (the Mac app's drawing rules). Closing it gives focus back
//! to your app.
//!
//! The pad shows nothing itself: the sharing pill turns into the drawing bar, so there's
//! one overlay to look at. The pad takes the pointer only over the usable area minus the
//! pill, so the pill and the bars (Omarchy's included) stay clickable while you draw.
//! It asks for the keyboard on demand, not exclusively: while any exclusive layer is up,
//! Hyprland sends every click to it, gaps in its input region included, so the pill's
//! buttons would draw instead. Like the overlay it is left out of the screen share:
//! viewers paint your ink themselves.

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    Pixels, Render, Styled, Window, WindowHandle, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    point, px, size,
};

use super::{session::Session, sharing};
use crate::protocol::Point;

pub const NAMESPACE: &str = "hopp-draw";

pub struct DrawPad {
    session: Entity<Session>,
    focus: FocusHandle,
}

/// Where `position` falls on the pad, from 0 to 1 across and down: the share's
/// coordinates, since the pad covers exactly the shared monitor.
fn normalised(position: gpui_kit::Point<Pixels>, window: &Window) -> Point {
    let size = window.viewport_size();
    let across = |at: Pixels, length: Pixels| {
        (f32::from(at) / f32::from(length).max(1.)).clamp(0., 1.) as f64
    };
    Point {
        x: across(position.x, size.width),
        y: across(position.y, size.height),
    }
}

/// Where the sharing pill is on a monitor `width` wide, with `reserved` space (left,
/// top, right, bottom) taken by bars: centred in the usable width, [`sharing::MARGIN`]
/// under the top of the usable area, as layer-shell places it.
fn pill_box(width: f32, [left, top, right, _]: [f32; 4]) -> Bounds<Pixels> {
    let usable = (width - left - right).max(0.);
    let x = left + ((usable - sharing::WIDTH) / 2.).max(0.);
    Bounds {
        origin: point(px(x), px(top + sharing::MARGIN)),
        size: size(px(sharing::WIDTH.min(usable)), px(sharing::HEIGHT)),
    }
}

/// Where the pad takes the pointer on a monitor `width` × `height` with `reserved` space
/// (left, top, right, bottom) taken by bars: the usable area minus the sharing pill
/// (`pill`), as four bands around the pill, so the pill's buttons and the bars still work.
fn input_region(
    width: f32,
    height: f32,
    [reserved_left, reserved_top, reserved_right, reserved_bottom]: [f32; 4],
    pill: Bounds<Pixels>,
) -> Vec<Bounds<Pixels>> {
    let (area_left, area_top) = (reserved_left, reserved_top);
    let (area_right, area_bottom) = (width - reserved_right, height - reserved_bottom);
    let (left, top) = (f32::from(pill.origin.x), f32::from(pill.origin.y));
    let (right, bottom) = (
        left + f32::from(pill.size.width),
        top + f32::from(pill.size.height),
    );
    let band = |x: f32, y: f32, w: f32, h: f32| Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w.max(0.)), px(h.max(0.))),
    };
    vec![
        band(area_left, area_top, area_right - area_left, top - area_top),
        band(area_left, top, left - area_left, bottom - top),
        band(right, top, area_right - right, bottom - top),
        band(
            area_left,
            bottom,
            area_right - area_left,
            area_bottom - bottom,
        ),
    ]
}

impl DrawPad {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            focus: cx.focus_handle(),
        }
    }

    fn stop(&self, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            if session.drawing {
                session.toggle_drawing(cx);
            }
        });
    }
}

impl Render for DrawPad {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drawpad")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .cursor_crosshair()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, window, cx| {
                    let at = normalised(event.position, window);
                    view.session.update(cx, |session, cx| {
                        session.pointer_down(
                            at,
                            MouseButton::Left,
                            event.modifiers,
                            event.click_count as u32,
                            cx,
                        )
                    });
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, window, cx| {
                    let at = normalised(event.position, window);
                    let dragging = event.pressed_button == Some(MouseButton::Left);
                    let straight = event.modifiers.shift;
                    view.session.update(cx, |session, cx| {
                        session.pointer_moved(at, dragging, straight, cx)
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseUpEvent, window, cx| {
                    let at = normalised(event.position, window);
                    view.session.update(cx, |session, cx| {
                        session.pointer_up(Some(at), MouseButton::Left, event.modifiers, cx)
                    });
                }),
            )
            // Right-click clears your ink, as in the Mac app; Escape stops drawing.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|view, _: &gpui_kit::MouseDownEvent, _, cx| {
                    view.session
                        .update(cx, |session, cx| session.clear_own_drawings(cx))
                }),
            )
            .on_key_down(cx.listener(|view, event: &gpui_kit::KeyDownEvent, _, cx| {
                if view
                    .session
                    .update(cx, |session, cx| session.drawing_key(&event.keystroke, cx))
                {
                    return;
                }
                let plain = !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt;
                match event.keystroke.key.as_str() {
                    "escape" => view.stop(cx),
                    "c" | "backspace" | "delete" if plain => view
                        .session
                        .update(cx, |session, cx| session.clear_own_drawings(cx)),
                    "k" if plain => view
                        .session
                        .update(cx, |session, cx| session.toggle_keep_ink(cx)),
                    _ => {}
                }
            }))
    }
}

/// Open the pad over the shared monitor, with the keyboard.
pub fn open(
    session: Entity<Session>,
    target: &crate::screencast::CaptureTarget,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<DrawPad>> {
    let (width, height) = (target.logical_width as f32, target.logical_height as f32);
    let display_id = super::overlay::shared_display(target, cx)?;
    let bounds = Bounds {
        origin: Default::default(),
        size: size(px(width), px(height)),
    };
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: NAMESPACE.into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            // Over the bar too, as the share sees it.
            exclusive_zone: Some(px(-1.)),
            exclusive_edge: None,
            margin: None,
            // On demand, not exclusive: Hyprland gives an exclusive layer every click,
            // even through the hole left for the pill. On demand still takes the
            // keyboard when the pad opens, for Escape and the other keys.
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
        }),
        display_id,
        app_id: Some("hopp.draw".into()),
        focus: true,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    let open_pad = |window: &mut Window, cx: &mut gpui_kit::App, session| {
        let pad = cx.new(|cx| DrawPad::new(session, cx));
        let focus = pad.read(cx).focus.clone();
        focus.focus(window, cx);
        pad
    };
    let handle = match cx.open_window(options, |window, cx| open_pad(window, cx, layer_session)) {
        Ok(handle) => handle,
        // Compositors without layer-shell (the screenshot harness) get a plain window.
        Err(_) => cx
            .open_window(
                gpui_kit::WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
                    app_id: Some("hopp.draw".into()),
                    ..Default::default()
                },
                |window, cx| open_pad(window, cx, session),
            )
            .ok()?,
    };
    let reserved = target
        .output_name
        .as_deref()
        .map(super::hypr::reserved)
        .unwrap_or_default();
    let region = input_region(width, height, reserved, pill_box(width, reserved));
    let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&region)));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_region_leaves_a_hole_exactly_over_the_sharing_pill() {
        // A 1728-wide laptop with a 37-pixel bar along the top.
        let pill = pill_box(1_728., [0., 37., 0., 0.]);
        assert_eq!(f32::from(pill.origin.y), 37. + sharing::MARGIN);
        assert_eq!(f32::from(pill.origin.x), (1_728. - sharing::WIDTH) / 2.);
        let region = input_region(1_728., 1_117., [0., 37., 0., 0.], pill);
        let inside = |x: f32, y: f32| {
            region.iter().any(|band| {
                let (left, top) = (f32::from(band.origin.x), f32::from(band.origin.y));
                x >= left
                    && y >= top
                    && x < left + f32::from(band.size.width)
                    && y < top + f32::from(band.size.height)
            })
        };
        let middle = 1_728. / 2.;
        assert!(!inside(middle, 80.), "the pill takes its own clicks");
        assert!(!inside(middle, 110.), "all the way down the pill");
        assert!(!inside(5., 5.), "the bar takes its own clicks");
        assert!(!inside(middle, 20.), "the bar takes its own clicks");
        for (x, y) in [
            (middle, 40.),
            (middle, 60.),
            (5., 90.),
            (1_720., 90.),
            (middle, 120.),
        ] {
            assert!(inside(x, y), "({x}, {y}) is drawable");
        }
        let total: f32 = region
            .iter()
            .map(|band| f32::from(band.size.width) * f32::from(band.size.height))
            .sum();
        assert_eq!(
            total,
            1_728. * (1_117. - 37.) - sharing::WIDTH * sharing::HEIGHT
        );
    }

    #[test]
    fn the_pill_is_centred_in_the_usable_width() {
        // A side panel on the left moves the pill's centre right.
        let pill = pill_box(2_000., [200., 0., 0., 0.]);
        assert_eq!(
            f32::from(pill.origin.x),
            200. + (1_800. - sharing::WIDTH) / 2.
        );
        assert_eq!(f32::from(pill.origin.y), sharing::MARGIN);
    }
}
