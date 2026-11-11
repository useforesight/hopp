//! The tray of reactions to send, opened by the call window's reactions button: eight
//! quick emoji, eight effects and a soundboard (keys 1 to 8), with a switch for their
//! sounds. A layer-shell surface beside the call window that takes the keyboard while
//! it's open; it stays until Escape, the button again, or the call ending.
use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    WindowHandle,
    base::input::{InputEvent, InputState},
    div,
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    prelude::FluentBuilder,
    px, rgba, size,
};
use gpui_omarchy::{ActiveTheme, input, with_tooltip};

use super::super::{Shell, session::Session};
use crate::protocol::Reaction;

pub const WIDTH: f32 = 336.;
pub const HEIGHT: f32 = 404.;
/// Between the call window and the tray.
pub const GAP: f32 = 10.;

const EMOJI: [(&str, &str, &str); 8] = [
    ("party", "🎉", "Party"),
    ("fire", "🔥", "Fire"),
    ("applause", "👏", "Applause"),
    ("lol", "😂", "LOL"),
    ("rocket", "🚀", "Ship it"),
    ("hundred", "💯", "100"),
    ("love", "❤️", "Love"),
    ("shock", "😱", "Shock"),
];

/// In the order of the soundboard's pads.
const SOUNDS: [(&str, &str, &str); 8] = [
    ("airhorn", "📯", "Airhorn"),
    ("badumtss", "🥁", "Ba-dum-tss"),
    ("sad_trombone", "🎺", "Sad trombone"),
    ("crickets", "🦗", "Crickets"),
    ("drumroll", "🥁", "Drumroll"),
    ("chaching", "💰", "Cha-ching"),
    ("wooow", "😮", "Wooow"),
    ("applause", "👏", "Applause"),
];

/// Confetti taps closer together than this build a streak, up to the biggest burst.
const CONFETTI_STREAK: Duration = Duration::from_millis(900);
const CONFETTI_MOST: u8 = 4;
/// Fireworks: a click sends a few rockets; holding charges up to ten.
const FIREWORKS_CLICK: u8 = 3;
const FIREWORKS_HOLD: f32 = 0.25;
const FIREWORKS_FULL: f32 = 3.;
/// A disco lasts about this long; until then the button ends it early.
const DISCO: Duration = Duration::from_secs(7);

const BACKGROUND: u32 = 0x17131ff2;
const TILE: u32 = 0xffffff0f;
const TILE_HOVER: u32 = 0xffffff1f;
const FUCHSIA: u32 = 0xd946ef;

/// Rockets for a press of the fireworks tile `held` seconds long.
pub fn rockets(held: f32) -> u8 {
    if held < FIREWORKS_HOLD {
        return FIREWORKS_CLICK;
    }
    let charge = ((held - FIREWORKS_HOLD) / (FIREWORKS_FULL - FIREWORKS_HOLD)).min(1.);
    1 + (charge * 9.).round() as u8
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Achievement,
    ShipIt,
}

pub struct Tray {
    session: Entity<Session>,
    focus: FocusHandle,
    form: Option<Form>,
    text: Entity<InputState>,
    /// Who the achievement is for; empty for everyone.
    to: String,
    /// The last confetti tap and the streak so far.
    confetti: Option<(Instant, u8)>,
    /// When the fireworks tile was pressed, while it's held.
    pressed: Option<Instant>,
    disco_until: Option<Instant>,
}

impl Tray {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let text = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&text, |view, _, event: &InputEvent, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => view.submit(cx),
            _ => {}
        })
        .detach();
        Self {
            session,
            focus: cx.focus_handle(),
            form: None,
            text,
            to: String::new(),
            confetti: None,
            pressed: None,
            disco_until: None,
        }
    }

    fn send(&self, reaction: Reaction, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, _| session.react(reaction));
    }

    fn choose(kind: &str, choice: &str) -> Reaction {
        Reaction {
            choice: Some(choice.into()),
            ..Reaction::new(kind)
        }
    }

    fn confetti(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let streak = match self.confetti {
            Some((last, streak)) if now - last < CONFETTI_STREAK => (streak + 1).min(CONFETTI_MOST),
            _ => 0,
        };
        self.confetti = Some((now, streak));
        self.send(
            Reaction {
                level: streak,
                ..Reaction::new("confetti")
            },
            cx,
        );
    }

    fn charge(&mut self, cx: &mut Context<Self>) {
        if self.pressed.is_some() {
            return;
        }
        self.pressed = Some(Instant::now());
        // Fill the tile while it's held.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let held = this.update(cx, |view, cx| {
                    cx.notify();
                    view.pressed.is_some()
                });
                if !matches!(held, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn launch(&mut self, cx: &mut Context<Self>) {
        let Some(pressed) = self.pressed.take() else {
            return;
        };
        self.send(
            Reaction {
                level: rockets(pressed.elapsed().as_secs_f32()),
                ..Reaction::new("fireworks")
            },
            cx,
        );
        cx.notify();
    }

    fn disco(&mut self, cx: &mut Context<Self>) {
        if self
            .disco_until
            .take()
            .is_some_and(|until| Instant::now() < until)
        {
            self.send(Reaction::new("disco_end"), cx);
        } else {
            let until = Instant::now() + DISCO;
            self.disco_until = Some(until);
            self.send(Reaction::new("disco"), cx);
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DISCO).await;
                let _ = this.update(cx, |view, cx| {
                    if view.disco_until == Some(until) {
                        view.disco_until = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn open_form(&mut self, form: Form, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(form);
        self.to.clear();
        self.text.update(cx, |text, cx| {
            text.set_placeholder(
                if form == Form::Achievement {
                    "Nailed it"
                } else {
                    "v2.0"
                },
                window,
                cx,
            );
            text.set_value("", window, cx);
            text.focus(window, cx);
        });
        cx.notify();
    }

    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.take() else {
            return;
        };
        let text = self.text.read(cx).value().trim().to_owned();
        let achievement = form == Form::Achievement;
        self.send(
            Reaction {
                text: (!text.is_empty()).then_some(text),
                to: (achievement && !self.to.is_empty()).then(|| self.to.clone()),
                ..Reaction::new(if achievement {
                    "achievement"
                } else {
                    "ship_it"
                })
            },
            cx,
        );
        cx.notify();
    }

    fn key(&mut self, event: &gpui_kit::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.key == "escape" {
            if self.form.is_some() {
                self.close_form(window, cx);
            } else {
                cx.defer(Shell::close_reactions_tray);
            }
            return;
        }
        let modifiers = keystroke.modifiers;
        if self.form.is_some()
            || event.is_held
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
        {
            return;
        }
        let pad = keystroke
            .key
            .parse::<usize>()
            .ok()
            .and_then(|key| key.checked_sub(1))
            .and_then(|pad| SOUNDS.get(pad));
        if let Some((choice, _, _)) = pad {
            self.send(Self::choose("sound", choice), cx);
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let on = self.session.read(cx).settings.reaction_sounds;
        let switch = div()
            .id("sounds")
            .w(px(30.))
            .h(px(18.))
            .p(px(2.))
            .flex()
            .rounded_full()
            .cursor_pointer()
            .bg(if on {
                rgba(FUCHSIA << 8 | 0xff)
            } else {
                rgba(0xffffff26)
            })
            .when(on, |track| track.justify_end())
            .child(div().size(px(14.)).rounded_full().bg(rgba(0xffffffff)))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.defer(move |cx| Shell::set_reaction_sounds(!on, cx))
            }));
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.))
            .pt(px(12.))
            .pb(px(8.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Reactions"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(11.))
                    .text_color(rgba(0xffffff99))
                    .child("Sounds")
                    .child(switch),
            )
    }

    fn section(title: &'static str, body: impl IntoElement) -> gpui_kit::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .px(px(4.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(0xffffff73))
                    .child(title.to_uppercase()),
            )
            .child(body)
    }

    fn tile(
        id: &str,
        emoji: &str,
        label: SharedString,
        small: bool,
        on: bool,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(SharedString::from(id.to_owned()))
            .relative()
            .h(px(58.))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(px(12.))
            .cursor_pointer()
            .bg(if on {
                rgba(FUCHSIA << 8 | 0x4d)
            } else {
                rgba(TILE)
            })
            .hover(move |tile| {
                tile.bg(if on {
                    rgba(FUCHSIA << 8 | 0x66)
                } else {
                    rgba(TILE_HOVER)
                })
            })
            .child(
                div()
                    .relative()
                    .when(small, |emoji| {
                        emoji.text_size(px(13.)).font_weight(FontWeight::BOLD)
                    })
                    .when(!small, |emoji| emoji.text_size(px(22.)))
                    .child(emoji.to_owned()),
            )
            .child(
                div()
                    .relative()
                    .mt(px(6.))
                    .text_size(px(10.))
                    .text_color(rgba(0xffffffb3))
                    .child(label),
            )
    }

    fn effects(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let charge = self.pressed.map_or(0., |pressed| {
            (pressed.elapsed().as_secs_f32() / FIREWORKS_FULL).min(1.)
        });
        let disco = self.disco_until.is_some_and(|until| Instant::now() < until);
        let simple = |id: &str,
                      emoji: &str,
                      label: &'static str,
                      small: bool,
                      kind: &'static str,
                      cx: &mut Context<Self>| {
            Self::tile(id, emoji, label.into(), small, false)
                .on_click(cx.listener(move |view, _, _, cx| view.send(Reaction::new(kind), cx)))
        };
        let fireworks_label: SharedString = if charge > 0.05 {
            format!("{} rockets", rockets(charge * FIREWORKS_FULL)).into()
        } else {
            "Fireworks".into()
        };
        let fireworks = Self::tile("fireworks", "🎆", fireworks_label, false, false)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(gpui_kit::relative(charge))
                    .bg(gpui_kit::linear_gradient(
                        0.,
                        gpui_kit::linear_color_stop(rgba(0xfbbf2473), 0.),
                        gpui_kit::linear_color_stop(rgba(FUCHSIA << 8 | 0x40), 1.),
                    )),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.charge(cx)),
            );
        div()
            .grid()
            .grid_cols(4)
            .gap(px(6.))
            .child(
                Self::tile("confetti", "🎉", "Confetti".into(), false, false)
                    .on_click(cx.listener(|view, _, _, cx| view.confetti(cx))),
            )
            .child(simple("gong", "🔔", "Gong", false, "gong", cx))
            .child(simple("flip", "┻━┻", "Flip table", true, "table_flip", cx))
            .child(simple("clap", "👏", "Slow clap", false, "clap", cx))
            .child(fireworks)
            .child(
                Self::tile("achievement", "🏆", "Achievement".into(), false, false).on_click(
                    cx.listener(|view, _, window, cx| {
                        view.open_form(Form::Achievement, window, cx)
                    }),
                ),
            )
            .child(
                Self::tile(
                    "disco",
                    "🪩",
                    if disco { "Stop disco" } else { "Disco" }.into(),
                    false,
                    disco,
                )
                .on_click(cx.listener(|view, _, _, cx| view.disco(cx))),
            )
            .child(
                Self::tile("ship", "🚀", "Ship it".into(), false, false).on_click(
                    cx.listener(|view, _, window, cx| view.open_form(Form::ShipIt, window, cx)),
                ),
            )
    }

    fn picker(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let emoji =
            div()
                .grid()
                .grid_cols(8)
                .gap(px(4.))
                .children(EMOJI.map(|(choice, emoji, label)| {
                    let tile = div()
                        .id(SharedString::from(format!("emoji-{choice}")))
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(12.))
                        .cursor_pointer()
                        .text_size(px(22.))
                        .hover(|tile| tile.bg(rgba(0xffffff1a)))
                        .child(emoji);
                    with_tooltip(tile, label).on_click(cx.listener(move |view, _, _, cx| {
                        view.send(Self::choose("emoji", choice), cx)
                    }))
                }));
        let pads = div()
            .grid()
            .grid_cols(4)
            .gap(px(6.))
            .children(
                SOUNDS
                    .iter()
                    .enumerate()
                    .map(|(i, (choice, emoji, label))| {
                        let choice = *choice;
                        div()
                            .id(SharedString::from(format!("sound-{choice}")))
                            .relative()
                            .h(px(46.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .cursor_pointer()
                            .bg(rgba(TILE))
                            .hover(|pad| pad.bg(rgba(TILE_HOVER)))
                            .child(div().text_size(px(17.)).child(*emoji))
                            .child(
                                div()
                                    .mt(px(4.))
                                    .px(px(4.))
                                    .max_w_full()
                                    .text_ellipsis()
                                    .text_size(px(9.5))
                                    .text_color(rgba(0xffffffa6))
                                    .child(*label),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(4.))
                                    .right(px(6.))
                                    .text_size(px(9.))
                                    .text_color(rgba(0xffffff59))
                                    .child((i + 1).to_string()),
                            )
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Self::choose("sound", choice), cx)
                            }))
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(12.))
            .pb(px(12.))
            .child(emoji)
            .child(Self::section("Effects", self.effects(cx)))
            .child(Self::section("Soundboard", pads))
    }

    fn form(&self, form: Form, window: &Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let achievement = form == Form::Achievement;
        let people: Vec<(String, String)> = std::iter::once((String::new(), "Everyone".into()))
            .chain(
                self.session
                    .read(cx)
                    .participants
                    .iter()
                    .map(|participant| {
                        (
                            crate::protocol::canonical_identity(&participant.identity).to_owned(),
                            super::super::session::sanitize(&participant.name),
                        )
                    }),
            )
            .fold(Vec::new(), |mut people, person| {
                if !people.iter().any(|(known, _)| *known == person.0) {
                    people.push(person);
                }
                people
            });
        let chips = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .children(people.into_iter().map(|(identity, name)| {
                let chosen = self.to == identity;
                div()
                    .id(SharedString::from(format!("for-{identity}")))
                    .h(px(28.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .when(chosen, |chip| {
                        chip.border_color(rgba(0xe879f9b3))
                            .bg(rgba(FUCHSIA << 8 | 0x40))
                            .text_color(rgba(0xffffffff))
                    })
                    .when(!chosen, |chip| {
                        chip.border_color(rgba(0xffffff1a))
                            .bg(rgba(0xffffff0a))
                            .text_color(rgba(0xffffffb3))
                            .hover(|chip| chip.bg(rgba(0xffffff1a)))
                    })
                    .child(name)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.to = identity.clone();
                        cx.notify();
                    }))
            }));
        let label = |text: &'static str| {
            div()
                .px(px(4.))
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(0xffffff73))
                .child(text.to_uppercase())
        };
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap(px(12.))
            .px(px(16.))
            .pb(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .rounded(px(12.))
                    .bg(rgba(TILE))
                    .child(div().text_size(px(30.)).child(if achievement {
                        "🏆"
                    } else {
                        "🚀"
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(rgba(0xffffffb3))
                            .child(if achievement {
                                "Award an achievement. Everyone sees it unlock."
                            } else {
                                "Launch a release. Everyone sees the countdown and lift-off."
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(label(if achievement {
                        "Achievement"
                    } else {
                        "What shipped"
                    }))
                    .child(input("reaction-text", &self.text, window, cx)),
            )
            .when(achievement, |form| {
                form.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(label("For"))
                        .child(chips),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("back")
                            // A third of the row; the button beside it takes the rest.
                            .w(px(94.))
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .bg(rgba(TILE))
                            .hover(|button| button.bg(rgba(TILE_HOVER)))
                            .text_size(px(13.))
                            .text_color(rgba(0xffffffbf))
                            .child("Back")
                            .on_click(
                                cx.listener(|view, _, window, cx| view.close_form(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("submit")
                            .flex_1()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .bg(gpui_kit::linear_gradient(
                                90.,
                                gpui_kit::linear_color_stop(rgba(FUCHSIA << 8 | 0xff), 0.),
                                gpui_kit::linear_color_stop(rgba(0xfbbf24ff), 1.),
                            ))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(0xffffffff))
                            .child(if achievement { "Unlock it" } else { "Launch" })
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.submit(cx);
                                view.focus.focus(window, cx);
                            })),
                    ),
            )
    }
}

impl Render for Tray {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let body = match self.form {
            Some(form) => self.form(form, window, cx),
            None => self.picker(cx),
        };
        div()
            .id("reactions-tray")
            .track_focus(&self.focus)
            .size_full()
            .p(px(6.))
            .flex()
            .font_family(theme.font.clone())
            .text_color(rgba(0xffffffff))
            .on_key_down(cx.listener(|view, event, window, cx| view.key(event, window, cx)))
            // Fireworks go up wherever the press ends.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.launch(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(18.))
                    .border_1()
                    .border_color(rgba(0xffffff1a))
                    .bg(rgba(BACKGROUND))
                    .shadow_lg()
                    .child(self.header(cx))
                    .child(body),
            )
    }
}

/// Where the tray goes: on monitor `output`, its top left `left`, `top` from the
/// monitor's corner.
pub struct Spot {
    pub output: String,
    pub left: f32,
    pub top: f32,
}

/// Beside a call window at `window` on a monitor of `monitor` size: on whichever side
/// has room, its foot level with the window's, where the reactions button is.
pub fn beside(window: super::super::hypr::Rect, monitor: (f64, f64)) -> (f32, f32) {
    let (x, y, w, h) = (
        window.x as f32,
        window.y as f32,
        window.width as f32,
        window.height as f32,
    );
    let (mw, mh) = (monitor.0 as f32, monitor.1 as f32);
    let left = if x - GAP - WIDTH >= 0. || x + w + GAP + WIDTH > mw {
        (x - GAP - WIDTH).max(0.)
    } else {
        x + w + GAP
    };
    let top = (y + h - HEIGHT).clamp(0., (mh - HEIGHT).max(0.));
    (left, top)
}

/// Open the tray at `spot`, or at the top right under the bar without one.
pub fn open(
    session: Entity<Session>,
    spot: Option<Spot>,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<Tray>> {
    let bounds = Some(gpui_kit::WindowBounds::Windowed(Bounds {
        origin: Default::default(),
        size: size(px(WIDTH), px(HEIGHT)),
    }));
    let display_id = spot
        .as_ref()
        .and_then(|spot| super::super::overlay::display_named(&spot.output, cx));
    let spot = spot.filter(|_| display_id.is_some());
    let (anchor, margin, exclusive_zone) = match &spot {
        Some(spot) => (
            Anchor::TOP | Anchor::LEFT,
            (px(spot.top), px(0.), px(0.), px(spot.left)),
            // Measured from the monitor's corner, not from under the bar.
            Some(px(-1.)),
        ),
        None => (
            Anchor::TOP | Anchor::RIGHT,
            (px(8.), px(10.), px(0.), px(0.)),
            None,
        ),
    };
    let layer = gpui_kit::WindowOptions {
        window_bounds: bounds,
        kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
            namespace: "hopp-reactions".into(),
            layer: Layer::Overlay,
            anchor,
            exclusive_zone,
            exclusive_edge: None,
            margin: Some(margin),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
        }),
        display_id,
        app_id: Some("hopp.reactions-tray".into()),
        focus: true,
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let layer_session = session.clone();
    let handle = match cx.open_window(layer, |window, cx| {
        cx.new(|cx| Tray::new(layer_session, window, cx))
    }) {
        Ok(handle) => handle,
        Err(_) => cx
            .open_window(
                gpui_kit::WindowOptions {
                    window_bounds: bounds,
                    titlebar: Some(gpui_kit::TitlebarOptions {
                        title: Some("Reactions".into()),
                        ..Default::default()
                    }),
                    app_id: Some("hopp.reactions-tray".into()),
                    is_resizable: false,
                    window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Tray::new(session, window, cx)),
            )
            .ok()?,
    };
    let _ = handle.update(cx, |view, window, cx| view.focus.focus(window, cx));
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_sends_three_rockets_and_a_hold_up_to_ten() {
        assert_eq!(rockets(0.1), 3);
        assert_eq!(rockets(0.25), 1);
        assert_eq!(rockets(1.625), 6);
        assert_eq!(rockets(3.), 10);
        assert_eq!(rockets(9.), 10);
    }

    #[test]
    fn the_tray_goes_on_the_side_with_room() {
        use super::super::super::hypr::Rect;
        let monitor = (1920., 1080.);
        let right = Rect {
            x: 1700.,
            y: 600.,
            width: 210.,
            height: 470.,
        };
        assert_eq!(beside(right, monitor), (1700. - GAP - WIDTH, 666.));
        let left = Rect {
            x: 10.,
            y: 40.,
            width: 210.,
            height: 300.,
        };
        assert_eq!(beside(left, monitor), (220. + GAP, 0.));
    }
}
