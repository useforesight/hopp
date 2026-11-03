//! Drawing on your own screen while you share it, as Tuple lets the sharer do.
//!
//! The marks overlay over the shared monitor lets every click through, so you can work
//! while others draw. To draw yourself (Draw on the sharing pill, Super+Alt+D, or
//! `hopp draw`), this pad covers the same monitor and turns the pointer into ink: your
//! strokes go to everyone in the call as Hopp's usual draw messages, and the overlay
//! underneath paints them for you. It takes the keyboard while it is up: Escape stops,
//! right-click or C clears your ink, Ctrl+Z undoes your last stroke and K keeps your ink
//! instead of letting it fade (the Mac app's drawing rules). Closing it gives focus back
//! to your app.
//! The sharing pill stays clickable through a hole in the pad's input region. Like the
//! overlay it is left out of the screen share: viewers paint your ink themselves.

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, Styled, Window, WindowHandle, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    point, px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon};

use super::{session::Session, sharing, style};
use crate::protocol::Point;

pub const NAMESPACE: &str = "hopp-draw";
/// From the top of the monitor: the sharing pill's margin.
const PILL_TOP: f32 = 38.;

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

/// The pad takes input everywhere except over the sharing pill, so Draw and Stop on
/// it still work while you draw: four bands around the pill's box.
fn input_region(width: f32, height: f32) -> Vec<Bounds<Pixels>> {
    let pill_left = ((width - sharing::WIDTH) / 2.).max(0.);
    let pill_right = (pill_left + sharing::WIDTH).min(width);
    let pill_bottom = PILL_TOP + sharing::HEIGHT;
    let band = |x: f32, y: f32, w: f32, h: f32| Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w.max(0.)), px(h.max(0.))),
    };
    vec![
        band(0., 0., width, PILL_TOP),
        band(0., PILL_TOP, pill_left, sharing::HEIGHT),
        band(pill_right, PILL_TOP, width - pill_right, sharing::HEIGHT),
        band(0., pill_bottom, width, height - pill_bottom),
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
        let theme = cx.omarchy().clone();
        let hint = div()
            .absolute()
            .top(px(PILL_TOP + sharing::HEIGHT + 10.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(14.))
                    .bg(theme.background.opacity(0.92))
                    .border_1()
                    .border_color(theme.accent)
                    .text_size(px(12.))
                    .text_color(theme.foreground)
                    .font_family(theme.font.clone())
                    .child(icon(IconName::Pencil).size(px(13.)))
                    .child("Drawing on your screen")
                    .child(style::muted_text(
                        "Esc stops · right-click clears · Ctrl+Z undoes · K keeps ink",
                        cx,
                    )),
            );
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
            .child(hint)
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
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
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
    let region = input_region(width, height);
    let _ = handle.update(cx, |_, window, _| window.set_input_region(Some(&region)));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_region_leaves_a_hole_for_the_sharing_pill() {
        let region = input_region(2_000., 1_000.);
        let inside = |x: f32, y: f32| {
            region.iter().any(|band| {
                let (left, top) = (f32::from(band.origin.x), f32::from(band.origin.y));
                x >= left
                    && y >= top
                    && x < left + f32::from(band.size.width)
                    && y < top + f32::from(band.size.height)
            })
        };
        assert!(
            !inside(1_000., PILL_TOP + 5.),
            "the pill takes its own clicks"
        );
        for (x, y) in [
            (5., 5.),
            (1_000., 10.),
            (5., 60.),
            (1_995., 60.),
            (1_000., 500.),
        ] {
            assert!(inside(x, y), "({x}, {y}) is drawable");
        }
        let total: f32 = region
            .iter()
            .map(|band| f32::from(band.size.width) * f32::from(band.size.height))
            .sum();
        assert_eq!(total, 2_000. * 1_000. - sharing::WIDTH * sharing::HEIGHT);
    }
}
