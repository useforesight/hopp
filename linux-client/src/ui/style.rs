//! Shared visual language. Everything derives from the live Omarchy theme so the
//! client follows `omarchy-theme-set` without a restart.
use gpui_kit::{
    App, Div, ElementId, FontWeight, Hsla, InteractiveElement, ParentElement, SharedString,
    Stateful, Styled, div, px, rems,
};
use gpui_omarchy::{ActiveTheme, IconName, icon, with_tooltip};

use super::session::initials;

pub const RADIUS: f32 = 10.;
pub const CONTROL: f32 = 28.;

/// A stable colour per person, from the Omarchy theme's palette.
pub fn person_color(name: &str, cx: &App) -> Hsla {
    super::palette::for_name(name, cx)
}

/// Round initials avatar.
pub fn avatar(name: &str, diameter: f32, cx: &App) -> Div {
    let color = person_color(name, cx);
    div()
        .size(px(diameter))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(color.opacity(0.22))
        .text_color(color)
        .text_size(px((diameter * 0.38).max(8.)))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initials(name))
}

/// Presence dot overlaid on an avatar.
pub fn presence_dot(online: bool, cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .absolute()
        .right(px(-1.))
        .bottom(px(-1.))
        .size(px(10.))
        .rounded_full()
        .border_2()
        .border_color(theme.background)
        .bg(if online {
            theme.success
        } else {
            theme.secondary.opacity(0.45)
        })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Neutral control.
    Plain,
    /// Something is on (camera, share, drawing).
    On,
    /// A device is muted or off, worth noticing.
    Off,
    /// Destructive (leave, stop).
    Danger,
}

/// Square icon control used in call surfaces.
pub fn control(
    id: impl Into<ElementId>,
    glyph: IconName,
    tone: Tone,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.omarchy();
    let (background, foreground, hover) = match tone {
        Tone::Plain => (theme.surface, theme.bright, theme.selection),
        Tone::On => (theme.accent, theme.on_accent, theme.accent.opacity(0.85)),
        Tone::Off => (
            theme.danger.opacity(0.16),
            theme.danger,
            theme.danger.opacity(0.26),
        ),
        Tone::Danger => (theme.danger, theme.on_accent, theme.danger.opacity(0.85)),
    };
    with_tooltip(
        div()
            .id(id)
            .size(px(CONTROL))
            .flex_shrink_0()
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .bg(background)
            .text_color(foreground)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(icon(glyph).size(px(15.))),
        tooltip,
    )
}

/// Small text button for pills and cards.
pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: Option<IconName>,
    tone: Tone,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.omarchy();
    let (background, foreground, border, hover) = match tone {
        Tone::Plain => (
            theme.background.opacity(0.),
            theme.foreground,
            theme.border,
            theme.selection,
        ),
        Tone::On => (
            theme.accent,
            theme.on_accent,
            theme.accent,
            theme.accent.opacity(0.85),
        ),
        Tone::Off => (
            theme.selection,
            theme.bright,
            theme.border,
            theme.selection.opacity(0.8),
        ),
        Tone::Danger => (
            theme.danger,
            theme.on_accent,
            theme.danger,
            theme.danger.opacity(0.85),
        ),
    };
    let mut chip = div()
        .id(id)
        .h(px(24.))
        .px(px(9.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(7.))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_color(foreground)
        .text_size(px(11.5))
        .font_weight(if tone == Tone::On || tone == Tone::Danger {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |style| style.bg(hover));
    if let Some(glyph) = glyph {
        chip = chip.child(icon(glyph).size(px(13.)));
    }
    chip.child(label.into())
}

/// Floating surface over video: themed, slightly translucent, softly shadowed.
pub fn pill(cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .p(px(4.))
        .pl(px(6.))
        .rounded(px(RADIUS))
        .bg(theme.background.opacity(0.92))
        .border_1()
        .border_color(theme.border.opacity(0.7))
        .shadow_lg()
}

/// Translucent panel floating over the viewer's blurred bands.
pub fn frosted(cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .flex()
        .rounded(px(12.))
        .bg(theme.inset.opacity(0.55))
        .border_1()
        .border_color(theme.bright.opacity(0.1))
        .shadow_lg()
}

pub fn separator(cx: &App) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .mx(px(2.))
        .bg(cx.omarchy().border.opacity(0.7))
}

pub fn section_label(label: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.omarchy().secondary.opacity(0.6))
        .child(label.into())
}

pub fn muted_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(11.))
        .text_color(cx.omarchy().secondary.opacity(0.65))
        .child(text.into())
}

/// Call duration as mm:ss or h:mm:ss.
pub fn duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// The Hopp mark: a rounded square with a lowercase h, drawn with the text colour.
pub fn mark(size: f32, cx: &App) -> Div {
    let theme = cx.omarchy();
    div()
        .size(px(size))
        .rounded(px(size * 0.28))
        .border_2()
        .border_color(theme.bright)
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.bright)
        .text_size(px(size * 0.62))
        .font_weight(FontWeight::BOLD)
        .line_height(rems(1.))
        .child("h")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(duration(0), "00:00");
        assert_eq!(duration(724), "12:04");
        assert_eq!(duration(3_725), "1:02:05");
    }
}
