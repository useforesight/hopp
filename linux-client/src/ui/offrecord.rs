//! While the call is off the record, a pill under the bar says so, counts down, and
//! offers two more minutes or going back on the record now. Either does the same on
//! every client in the call (see `capture::pause`).
//!
//! Someone says "tinfoil hat" or "off the record" and every capture in the call stops
//! keeping anything for two minutes. The last five seconds turn amber and beep. A
//! layer-shell surface in the call's namespace, so a screen share never shows it.
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FontWeight, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, WindowHandle, div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon};

use super::{
    session::Session,
    sharing,
    style::{self, Tone},
};

const WIDTH: f32 = 380.;
/// The last seconds: amber, and a beep each.
pub const COUNTDOWN: u64 = 5;

/// Whole seconds left, counting a started second as left: 2:00 at the start, 0:01
/// through the last second.
pub fn seconds_left(ends: Instant, now: Instant) -> u64 {
    let left = ends.saturating_duration_since(now);
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// How long until the clock next changes, so that it ticks on the second.
pub fn next_tick(ends: Instant, now: Instant) -> Duration {
    let left = ends.saturating_duration_since(now);
    match left.subsec_nanos() {
        0 => Duration::from_secs(1).min(left),
        nanos => Duration::from_nanos(nanos.into()),
    }
}

/// `m:ss`.
pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

pub struct OffRecordView {
    session: Entity<Session>,
}

impl OffRecordView {
    fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self { session }
    }
}

impl Render for OffRecordView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let left = self
            .session
            .read(cx)
            .off_record
            .map_or(0, |ends| seconds_left(ends, Instant::now()));
        let ending = left <= COUNTDOWN;
        let edge = if ending { theme.warning } else { theme.accent };
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
            .rounded(px(sharing::HEIGHT / 2.))
            .bg(theme.background)
            .border_1()
            .border_color(edge)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(px(12.))
            .child(icon(IconName::HatGlasses).size(px(15.)).text_color(edge))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.bright)
                    .child("Off the record"),
            )
            .child(
                // As wide as the widest time, so the buttons don't shift each second.
                div()
                    .w(px(34.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if ending {
                        theme.warning
                    } else {
                        theme.foreground
                    })
                    .child(clock(left)),
            )
            .child(
                chip("more", "+2 min", None, Tone::Plain, cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.session
                            .update(cx, |session, _| session.off_record_more())
                    },
                )),
            )
            .child(
                chip("resume", "Resume", None, Tone::On, cx).on_click(cx.listener(
                    |view, _, _, cx| {
                        view.session
                            .update(cx, |session, _| session.off_record_end())
                    },
                )),
            );
        div().size_full().flex().justify_center().child(pill)
    }
}

/// Open the pill under the bar; `below_sharing` puts it under the "You're sharing" pill.
pub fn open(
    session: Entity<Session>,
    below_sharing: bool,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<OffRecordView>> {
    let top = if below_sharing {
        sharing::MARGIN + sharing::HEIGHT + 8.
    } else {
        sharing::MARGIN
    };
    let bounds = Some(gpui_kit::WindowBounds::Windowed(Bounds {
        origin: Default::default(),
        size: size(px(WIDTH), px(sharing::HEIGHT)),
    }));
    let options = gpui_kit::WindowOptions {
        window_bounds: bounds,
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            // The call card's namespace, which the Hyprland rules already keep out of
            // screen shares.
            namespace: "hopp-call".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP,
            exclusive_zone: None,
            exclusive_edge: None,
            margin: Some((px(top), px(0.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::None,
        }),
        app_id: Some("hopp.offrecord".into()),
        focus: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    if let Ok(handle) = cx.open_window(options, |_, cx| {
        cx.new(|cx| OffRecordView::new(layer_session, cx))
    }) {
        return Some(handle);
    }
    // Compositors without layer-shell (the screenshot harness) get a plain window.
    let fallback = gpui_kit::WindowOptions {
        window_bounds: bounds,
        app_id: Some("hopp.offrecord".into()),
        focus: false,
        is_resizable: false,
        ..Default::default()
    };
    cx.open_window(fallback, |_, cx| {
        cx.new(|cx| OffRecordView::new(session, cx))
    })
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_starts_at_two_minutes_and_ends_after_one_second() {
        let now = Instant::now();
        let at = |ms: u64| seconds_left(now + Duration::from_millis(ms), now);
        assert_eq!(clock(at(120_000)), "2:00");
        assert_eq!(clock(at(119_999)), "2:00");
        assert_eq!(clock(at(119_000)), "1:59");
        assert_eq!(clock(at(5_000)), "0:05");
        assert_eq!(clock(at(1)), "0:01");
        assert_eq!(at(0), 0);
        assert_eq!(clock(600), "10:00");
    }

    #[test]
    fn it_ticks_on_the_second() {
        let now = Instant::now();
        let tick = |ms: u64| next_tick(now + Duration::from_millis(ms), now);
        assert_eq!(tick(119_400), Duration::from_millis(400));
        assert_eq!(tick(120_000), Duration::from_secs(1));
        assert_eq!(tick(300), Duration::from_millis(300));
        assert_eq!(tick(0), Duration::ZERO);
    }
}
