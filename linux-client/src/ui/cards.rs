//! Cards from agents and scripts (see `crate::cards`), stacked where Omarchy's
//! notifications appear.
//!
//! Like the call card it is a layer-shell surface that floats above tiles and only takes
//! keyboard focus when clicked, so a card never steals what you're typing. Questions
//! wait for Accept or Reject; notices close with ×, or go by themselves when posted as
//! ephemeral. Hidden from your own screen share by the `hopp-cards` layer rule.

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, WindowHandle,
    div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::FluentBuilder,
    px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon};

use super::{
    session::{Card, Session},
    style::{self, Tone},
};
use crate::cards::CardState;

pub const WIDTH: f32 = 360.;
/// Every card is this tall: a sender line, a title line, two lines of body and a row
/// of buttons, so the stack's height follows from the count.
const CARD_HEIGHT: f32 = 136.;
const GAP: f32 = 8.;
const MORE_HEIGHT: f32 = 22.;
/// Shown at once; the rest wait behind a "+N more" line.
pub const SHOWN: usize = 3;
/// From the top of the screen, as the call card.
const TOP: f32 = 38.;

/// The window's height for `count` cards waiting.
pub fn height(count: usize) -> f32 {
    let shown = count.min(SHOWN) as f32;
    let more = if count > SHOWN { GAP + MORE_HEIGHT } else { 0. };
    (shown * CARD_HEIGHT + (shown - 1.).max(0.) * GAP + more).max(CARD_HEIGHT)
}

/// Below the call card when that is showing too.
pub fn top(below_call: bool) -> f32 {
    if below_call {
        TOP + super::toast::HEIGHT + GAP
    } else {
        TOP
    }
}

pub struct CardsView {
    session: Entity<Session>,
    focus: FocusHandle,
}

impl CardsView {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            focus: cx.focus_handle(),
        }
    }

    fn settle(&self, id: &str, state: CardState, cx: &mut Context<Self>) {
        let id = id.to_owned();
        self.session
            .update(cx, |session, cx| session.settle_card(&id, state, cx));
    }

    fn card(&self, card: &Card, cx: &mut Context<Self>) -> gpui_kit::Stateful<gpui_kit::Div> {
        let theme = cx.omarchy().clone();
        let request = &card.request;
        let id = request.id.clone();
        let question = request.question.clone();
        let sender = request.sender.clone().unwrap_or_else(|| "Agent".into());
        let close_id = id.clone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .text_size(px(11.))
            .text_color(theme.secondary.opacity(0.75))
            .child(icon(IconName::Bot).size(px(12.)))
            .child(div().flex_1().min_w_0().text_ellipsis().child(sender))
            .child(
                div()
                    .id(SharedString::from(format!("close-{id}")))
                    .p(px(2.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|close| close.bg(theme.surface))
                    .child(icon(IconName::X).size(px(12.)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.settle(&close_id, CardState::Dismissed, cx)
                    })),
            );
        let actions = question.map(|question| {
            let (accept_id, reject_id) = (id.clone(), id.clone());
            div()
                .flex()
                .gap(px(6.))
                .child(
                    style::chip(
                        SharedString::from(format!("accept-{id}")),
                        question.accept,
                        None,
                        Tone::On,
                        cx,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.settle(&accept_id, CardState::Accepted, cx)
                    })),
                )
                .child(
                    style::chip(
                        SharedString::from(format!("reject-{id}")),
                        question.reject,
                        None,
                        Tone::Plain,
                        cx,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.settle(&reject_id, CardState::Rejected, cx)
                    })),
                )
        });
        let body_lines = if actions.is_some() { 2 } else { 3 };
        div()
            .id(SharedString::from(format!("card-{id}")))
            .h(px(CARD_HEIGHT))
            .flex_shrink_0()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .overflow_hidden()
            .rounded(px(style::RADIUS))
            .bg(theme.background)
            .border_1()
            .border_color(if actions.is_some() {
                theme.accent
            } else {
                theme.border
            })
            .child(header)
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.bright)
                    .text_ellipsis()
                    .child(request.title.clone()),
            )
            .children(request.body.clone().map(|body| {
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .line_clamp(body_lines)
                    .text_color(theme.foreground.opacity(0.85))
                    .child(body)
            }))
            .child(div().flex_1())
            .children(actions)
    }
}

impl Render for CardsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let showing = self.session.read(cx).showing_cards();
        let more = showing.len().saturating_sub(SHOWN);
        let cards: Vec<_> = showing
            .iter()
            .take(SHOWN)
            .map(|card| self.card(card, cx))
            .collect();
        // Keys act on the newest card: Enter accepts a question, Escape rejects it or
        // closes a notice.
        let newest = showing
            .first()
            .map(|card| (card.request.id.clone(), card.request.question.is_some()));
        div()
            .id("cards")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(GAP))
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, _, cx| {
                    let Some((id, question)) = &newest else {
                        return;
                    };
                    match (event.keystroke.key.as_str(), question) {
                        ("enter", true) => view.settle(id, CardState::Accepted, cx),
                        ("escape", true) => view.settle(id, CardState::Rejected, cx),
                        ("escape", false) => view.settle(id, CardState::Dismissed, cx),
                        _ => {}
                    }
                }),
            )
            .children(cards)
            .when(more > 0, |stack| {
                stack.child(
                    div()
                        .h(px(MORE_HEIGHT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(style::RADIUS))
                        .bg(theme.background.opacity(0.9))
                        .child(style::muted_text(format!("+{more} more"), cx)),
                )
            })
    }
}

/// Open the stack for `count` cards. Falls back to an ordinary small window on
/// compositors without layer-shell (for example the headless screenshot harness).
pub fn open(
    session: Entity<Session>,
    count: usize,
    below_call: bool,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<CardsView>> {
    let bounds = Bounds {
        origin: Default::default(),
        size: size(px(WIDTH), px(height(count))),
    };
    let layer = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: "hopp-cards".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((px(top(below_call)), px(10.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
        }),
        app_id: Some("hopp.cards".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let session_for_layer = session.clone();
    if let Ok(handle) = cx.open_window(layer, |_, cx| {
        cx.new(|cx| CardsView::new(session_for_layer, cx))
    }) {
        return Some(handle);
    }
    let fallback = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp".into()),
            ..Default::default()
        }),
        app_id: Some("hopp.cards".into()),
        is_resizable: false,
        ..Default::default()
    };
    cx.open_window(fallback, |_, cx| cx.new(|cx| CardsView::new(session, cx)))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_grows_to_three_cards_then_a_more_line() {
        assert_eq!(height(0), CARD_HEIGHT, "never zero-sized");
        assert_eq!(height(1), CARD_HEIGHT);
        assert_eq!(height(2), 2. * CARD_HEIGHT + GAP);
        assert_eq!(height(3), 3. * CARD_HEIGHT + 2. * GAP);
        assert_eq!(height(7), 3. * CARD_HEIGHT + 3. * GAP + MORE_HEIGHT);
    }
}
