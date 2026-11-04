//! While you share, a small pill under the bar says so, says who can control your
//! screen, and offers Draw and Stamp (on your own screen, see `drawpad`), Revoke and Stop.
//!
//! It is the one control you see while you share. When you draw, it turns into the
//! drawing bar (Keep ink, Undo, Clear, Stamp, Done); with the stamp tool, it says how to
//! leave a stamp and what to do next. No second overlay appears: the draw pad underneath
//! only catches the pen, with a hole where this pill is. It is a
//! layer-shell surface on the shared monitor, excluded from screen sharing by Hopp's
//! Hyprland rules, so viewers never see it.
use gpui_kit::{
    AppContext, Bounds, Context, Entity, FontWeight, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, WindowHandle, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::FluentBuilder,
    px, size,
};
use gpui_omarchy::{ActiveTheme, IconName};

use super::{
    session::Session,
    style::{self, Tone},
};

/// The pill's window: wide enough for the drawing bar; the pill is centred in it.
pub const WIDTH: f32 = 720.;
pub const HEIGHT: f32 = 40.;
/// Below the top of the usable area (under the bar).
pub const MARGIN: f32 = 38.;

pub struct SharingView {
    session: Entity<Session>,
}

impl SharingView {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self { session }
    }
}

impl Render for SharingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let output = session
            .capture_name
            .clone()
            .map(|name| format!("Sharing {name}"))
            .unwrap_or_else(|| "Sharing your screen".into());
        let controller = session
            .grant
            .as_deref()
            .map(|grant| session.name_for(grant));
        let viewers = session.participants.len();
        // A window share has no fixed place on screen to draw over.
        let can_draw = session
            .share_target
            .as_ref()
            .is_some_and(|target| target.output_name.is_some());
        let drawing = session.drawing && can_draw;
        let stamping = session.stamping && can_draw;
        let stamp_hint = session.stamp_hint();
        let active_stamp = session.active_stamp_number();
        let keep_ink = session.keep_ink;
        let has_ink = session
            .paths
            .iter()
            .any(|path| path.sender.is_empty() && path.fading.is_none());
        let edge = if drawing || stamping {
            theme.accent
        } else {
            theme.danger
        };
        let chip = |id: &'static str, label: &'static str, glyph, tone, cx: &Context<Self>| {
            style::chip(id, label, glyph, tone, cx).rounded(px(12.))
        };
        let pill = div()
            .h_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(5.))
            .rounded(px(HEIGHT / 2.))
            .bg(theme.background)
            .border_1()
            .border_color(edge)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(px(12.))
            .child(div().size(px(8.)).rounded_full().bg(edge));
        let pill = if drawing {
            // The drawing bar: what you can do while drawing, and how to stop.
            pill.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.bright)
                    .child("Drawing"),
            )
            .child(style::muted_text("Esc to stop", cx))
            .child(
                chip(
                    "keep",
                    "Keep ink",
                    Some(IconName::Pin),
                    if keep_ink { Tone::On } else { Tone::Plain },
                    cx,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.session
                        .update(cx, |session, cx| session.toggle_keep_ink(cx))
                })),
            )
            .child(
                chip("undo", "Undo", Some(IconName::Undo2), Tone::Plain, cx)
                    .when(!has_ink, |chip| chip.opacity(0.5))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.undo_stroke(cx))
                    })),
            )
            .child(
                chip("clear", "Clear", Some(IconName::Eraser), Tone::Plain, cx)
                    .when(!has_ink, |chip| chip.opacity(0.5))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.clear_own_drawings(cx))
                    })),
            )
            .child(
                chip("stamp", "Stamp", Some(IconName::MapPin), Tone::Plain, cx).on_click(
                    cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_stamping(cx))
                    }),
                ),
            )
            .child(
                chip("done", "Done", Some(IconName::Check), Tone::On, cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_drawing(cx))
                    },
                )),
            )
        } else if stamping {
            // The stamp tool: how to leave one, and once it's down, how to finish it.
            let pill = pill
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child(match active_stamp {
                            Some(number) => format!("Stamp {number}"),
                            None => "Stamps".into(),
                        }),
                )
                .child(style::muted_text(stamp_hint, cx));
            if active_stamp.is_some() {
                pill.child(
                    chip("unstamp", "Remove", Some(IconName::X), Tone::Plain, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.cancel_stamp(cx))
                        }),
                    ),
                )
                .child(
                    chip("finish", "Finish", Some(IconName::Check), Tone::On, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.finish_stamp(cx))
                        }),
                    ),
                )
            } else {
                pill.child(
                    chip("pen", "Draw", Some(IconName::Pencil), Tone::Plain, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.toggle_drawing(cx))
                        }),
                    ),
                )
                .child(
                    chip("done", "Done", Some(IconName::Check), Tone::On, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.stop_marking(cx))
                        }),
                    ),
                )
            }
        } else {
            pill.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.bright)
                    .child(output),
            )
            .child(style::muted_text(
                match (&controller, viewers) {
                    (Some(name), _) => format!("{name} has control"),
                    (None, 1) => "1 viewer".to_owned(),
                    (None, count) => format!("{count} viewers"),
                },
                cx,
            ))
            .when(can_draw, |pill| {
                pill.child(
                    chip("draw", "Draw", Some(IconName::Pencil), Tone::Plain, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.toggle_drawing(cx))
                        }),
                    ),
                )
                .child(
                    chip("stamp", "Stamp", Some(IconName::MapPin), Tone::Plain, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.toggle_stamping(cx))
                        }),
                    ),
                )
            })
            .when(controller.is_some(), |pill| {
                pill.child(
                    chip("revoke", "Revoke", Some(IconName::Hand), Tone::Plain, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session.update(cx, |session, cx| session.revoke(cx))
                        }),
                    ),
                )
            })
        };
        div().size_full().flex().justify_center().child(pill.child(
            chip("stop", "Stop", None, Tone::Danger, cx).on_click(cx.listener(|view, _, _, cx| {
                view.session
                    .update(cx, |session, cx| session.toggle_share(false, cx))
            })),
        ))
    }
}

/// Open the pill on the monitor being shared (`target`), or the focused one for a
/// window share.
pub fn open(
    session: Entity<Session>,
    target: Option<&crate::screencast::CaptureTarget>,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<SharingView>> {
    let display_id = target.and_then(|target| super::overlay::shared_display(target, cx));
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(WIDTH), px(HEIGHT)),
        })),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: "hopp-sharing".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((px(MARGIN), px(0.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        display_id: display_id.flatten(),
        app_id: Some("hopp.sharing".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    if let Ok(handle) = cx.open_window(options, |_, cx| {
        cx.new(|cx| SharingView::new(layer_session, cx))
    }) {
        return Some(handle);
    }
    // Compositors without layer-shell (the screenshot harness) get a plain window.
    let fallback = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(WIDTH), px(HEIGHT)),
        })),
        app_id: Some("hopp.sharing".into()),
        focus: false,
        is_resizable: false,
        ..Default::default()
    };
    cx.open_window(fallback, |_, cx| cx.new(|cx| SharingView::new(session, cx)))
        .ok()
}
