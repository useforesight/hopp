//! Incoming and outgoing call card, shown where Omarchy notifications appear. With no
//! call ringing, it shows the newest missed call until you call back or dismiss it.
//!
//! It is a layer-shell surface so it floats above tiles without taking one, and it only
//! takes keyboard focus when clicked, so a call never steals what you're typing.
use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, WindowHandle, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::FluentBuilder,
    px, size,
};
use gpui_omarchy::{ActiveTheme, IconName};

use super::{
    session::{Ringing, Session},
    style::{self, Tone},
};

pub const WIDTH: f32 = 360.;
pub const HEIGHT: f32 = 92.;

pub struct ToastView {
    session: Entity<Session>,
    focus: FocusHandle,
}

impl ToastView {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for ToastView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let ringing = session.ringing.clone();
        let dialing = session.dialing.clone();
        let missed = session.missed_shown().cloned();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        let missed_label = missed
            .as_ref()
            .map(|missed| crate::missed::describe(missed, now))
            .unwrap_or_default();
        let (name, title, detail) = match (&ringing, &dialing) {
            (Some(Ringing::Call { name, .. }), _) => (
                name.clone(),
                format!("{name} wants to pair"),
                "Incoming call",
            ),
            (Some(Ringing::Invite { name, .. }), _) => (
                name.clone(),
                format!("{name} invited you"),
                "Join their pairing session",
            ),
            (None, Some((_, name, invite))) => (
                name.clone(),
                format!("Calling {name}…"),
                if invite.is_some() {
                    "Inviting them to this call"
                } else {
                    "Waiting for them to pick up"
                },
            ),
            (None, None) => match &missed {
                Some(missed) => (
                    missed.name.clone(),
                    if missed.invite {
                        format!("{} invited you", missed.name)
                    } else {
                        format!("{} tried to call", missed.name)
                    },
                    missed_label.as_str(),
                ),
                None => (String::new(), String::new(), ""),
            },
        };
        let incoming = ringing.is_some();
        let missed_id = missed
            .as_ref()
            .filter(|_| !incoming && dialing.is_none())
            .map(|missed| missed.caller_id.clone());

        let actions = if incoming {
            div()
                .flex()
                .gap(px(6.))
                .child(
                    style::chip("accept", "Accept", Some(IconName::Phone), Tone::On, cx).on_click(
                        cx.listener(|view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.accept_ringing(cx))
                        }),
                    ),
                )
                .child(
                    style::chip("decline", "Decline", None, Tone::Plain, cx).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.decline_ringing(cx))
                        },
                    )),
                )
        } else if let Some(id) = missed_id.clone() {
            let back = id.clone();
            div()
                .flex()
                .gap(px(6.))
                .child(
                    style::chip(
                        "call-back",
                        "Call back",
                        Some(IconName::Phone),
                        Tone::On,
                        cx,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.call_back(&back, cx))
                    })),
                )
                .child(
                    style::chip("dismiss", "Dismiss", None, Tone::Plain, cx).on_click(cx.listener(
                        move |view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.dismiss_missed(Some(&id), cx))
                        },
                    )),
                )
        } else {
            div().flex().child(
                style::chip("cancel", "Cancel", None, Tone::Plain, cx).on_click(cx.listener(
                    |view, _, _, cx| view.session.update(cx, |session, cx| session.leave(cx)),
                )),
            )
        };

        div()
            .id("toast")
            .track_focus(&self.focus)
            .size_full()
            .p(px(14.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(style::RADIUS))
            .bg(theme.background)
            .border_1()
            .border_color(if missed_id.is_some() {
                theme.danger
            } else {
                theme.accent
            })
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, _, cx| {
                    let missed = missed_id.clone();
                    match event.keystroke.key.as_str() {
                        "enter" => view.session.update(cx, |session, cx| match &missed {
                            Some(id) => session.call_back(id, cx),
                            None => session.accept_ringing(cx),
                        }),
                        "escape" => view.session.update(cx, |session, cx| {
                            if session.ringing.is_some() {
                                session.decline_ringing(cx)
                            } else if let Some(id) = &missed {
                                session.dismiss_missed(Some(id), cx)
                            } else {
                                session.leave(cx)
                            }
                        }),
                        _ => {}
                    }
                }),
            )
            .child(
                div()
                    .relative()
                    .child(style::avatar(&name, 46., cx))
                    .when(incoming, |avatar| {
                        avatar.child(
                            div()
                                .absolute()
                                .inset(px(-3.))
                                .rounded_full()
                                .border_2()
                                .border_color(theme.accent.opacity(0.6)),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .text_ellipsis()
                            .child(title),
                    )
                    .child(style::muted_text(detail, cx))
                    .child(div().pt(px(6.)).child(actions)),
            )
    }
}

/// Open the card. Falls back to an ordinary small window on compositors without
/// layer-shell (for example the headless screenshot harness).
pub fn open(session: Entity<Session>, cx: &mut gpui_kit::App) -> Option<WindowHandle<ToastView>> {
    let layer = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(WIDTH), px(HEIGHT)),
        })),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: "hopp-call".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((px(38.), px(10.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
        }),
        app_id: Some("hopp.call".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let session_for_layer = session.clone();
    if let Ok(handle) = cx.open_window(layer, |_, cx| {
        cx.new(|cx| ToastView::new(session_for_layer, cx))
    }) {
        return Some(handle);
    }
    let fallback = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(WIDTH), px(HEIGHT)),
        })),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp call".into()),
            ..Default::default()
        }),
        app_id: Some("hopp.call".into()),
        is_resizable: false,
        ..Default::default()
    };
    cx.open_window(fallback, |_, cx| cx.new(|cx| ToastView::new(session, cx)))
        .ok()
}
