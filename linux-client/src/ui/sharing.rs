//! While you share, a small pill under the bar says so, says who can control your
//! screen, and offers Draw (on your own screen, see `drawpad`), Revoke and Stop. It is a layer-shell surface excluded from screen
//! sharing by Hopp's Hyprland rules, so viewers never see it.
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

pub const WIDTH: f32 = 520.;
pub const HEIGHT: f32 = 40.;

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
        let drawing = session.drawing;
        // A window share has no fixed place on screen to draw over.
        let can_draw = session
            .share_target
            .as_ref()
            .is_some_and(|target| target.output_name.is_some());
        div().size_full().flex().justify_center().child(
            div()
                .h_full()
                .flex()
                .items_center()
                .gap(px(10.))
                .pl(px(12.))
                .pr(px(5.))
                .rounded(px(HEIGHT / 2.))
                .bg(theme.background)
                .border_1()
                .border_color(theme.danger)
                .text_color(theme.foreground)
                .font_family(theme.font.clone())
                .text_size(px(12.))
                .child(div().size(px(8.)).rounded_full().bg(theme.danger))
                .child(
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
                        style::chip(
                            "draw",
                            if drawing { "Drawing" } else { "Draw" },
                            Some(IconName::Pencil),
                            if drawing { Tone::On } else { Tone::Plain },
                            cx,
                        )
                        .rounded(px(12.))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.toggle_drawing(cx))
                        })),
                    )
                })
                .when(controller.is_some(), |pill| {
                    pill.child(
                        style::chip("revoke", "Revoke", Some(IconName::Hand), Tone::Plain, cx)
                            .rounded(px(12.))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.session.update(cx, |session, cx| session.revoke(cx))
                            })),
                    )
                })
                .child(
                    style::chip("stop", "Stop", None, Tone::Danger, cx)
                        .rounded(px(12.))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.toggle_share(false, cx))
                        })),
                ),
        )
    }
}

pub fn open(session: Entity<Session>, cx: &mut gpui_kit::App) -> Option<WindowHandle<SharingView>> {
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
            margin: Some((px(38.), px(0.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
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
