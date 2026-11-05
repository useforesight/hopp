//! The Hopp window: teammates, rooms, the current call and settings, laid out like an
//! Omarchy panel. With the bar plugin installed this is mostly the settings surface;
//! without it, it is the way into calls.
use std::time::Duration;

use gpui_kit::{
    AppContext, Bounds, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, WindowHandle,
    base::input::{InputEvent, InputState},
    div,
    prelude::FluentBuilder,
    px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon, input};

use super::{
    Shell,
    session::{Ringing, Session, full_name},
    style::{self, Tone},
};

pub struct HomeView {
    session: Entity<Session>,
    search: Entity<InputState>,
    token: Entity<InputState>,
    room_name: Entity<InputState>,
    token_form: bool,
    room_form: bool,
}

impl HomeView {
    fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search teammates and rooms"));
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste an access token")
        });
        let room_name = cx.new(|cx| InputState::new(window, cx).placeholder("Room name"));
        cx.subscribe(&search, |view, _, event: &InputEvent, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => view.call_first_match(cx),
            _ => {}
        })
        .detach();
        cx.subscribe_in(
            &room_name,
            window,
            |view, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let name = state.read(cx).value().trim().to_owned();
                    view.session
                        .update(cx, |session, cx| session.create_room(name, cx));
                    state.update(cx, |state, cx| state.set_value("", window, cx));
                    view.room_form = false;
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe(&token, |view, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                view.sign_in(true, cx);
            }
        })
        .detach();
        // The call timer ticks once a second while a call is up.
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = view.update(cx, |view, cx| {
                    if view.session.read(cx).in_call() || crate::capture::models::any_downloading()
                    {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            session,
            search,
            token,
            room_name,
            token_form: false,
            room_form: false,
        }
    }

    fn query(&self, cx: &gpui_kit::App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }

    fn call_first_match(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let session = self.session.read(cx);
        let own = session.user.as_ref().map(|user| user.id.clone());
        let target = session
            .teammates
            .iter()
            .filter(|teammate| Some(&teammate.id) != own.as_ref())
            .filter(|teammate| teammate.is_active)
            .find(|teammate| {
                full_name(&teammate.first_name, &teammate.last_name)
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned();
        if let Some(teammate) = target {
            self.session
                .update(cx, |session, cx| session.call(&teammate, cx));
        }
    }

    fn sign_in(&mut self, with_token: bool, cx: &mut Context<Self>) {
        let server = self.session.read(cx).settings.server.clone();
        let token = if with_token {
            self.token.read(cx).value().trim().to_owned()
        } else {
            String::new()
        };
        self.session.update(cx, |session, cx| {
            if with_token && token.is_empty() {
                session.push_error("Paste your access token before connecting.");
                cx.notify();
            } else {
                session.sign_in(server, token, cx);
            }
        });
    }

    // ---- sections -------------------------------------------------------------------

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let own = session.user.as_ref().map(|user| user.id.clone());
        let available = session
            .teammates
            .iter()
            .filter(|teammate| teammate.is_active && Some(&teammate.id) != own.as_ref())
            .count();
        let meta = if session.user.is_none() {
            "Pairing for Omarchy".to_owned()
        } else if session.in_call() {
            session.status.clone()
        } else {
            format!("{available} available")
        };
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .size(px(34.))
                    .rounded(px(9.))
                    .bg(theme.surface)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(style::mark(18., cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.bright)
                            .child("Hopp"),
                    )
                    .child(style::muted_text(meta, cx).text_ellipsis()),
            )
            .when(session.user.is_some(), |header| {
                header.child(
                    style::control("refresh", IconName::RefreshCw, Tone::Plain, "Refresh", cx)
                        .bg(theme.background.opacity(0.))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.session.update(cx, |session, _| session.refresh())
                        })),
                )
            })
            .child(
                style::control("settings", IconName::Settings, Tone::Plain, "Settings", cx)
                    .bg(theme.background.opacity(0.))
                    .on_click(|_, _, cx| Shell::show_settings(cx)),
            )
    }

    fn errors(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let errors = self.session.read(cx).errors.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(errors.into_iter().enumerate().map(|(index, error)| {
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(theme.danger.opacity(0.1))
                    .border_1()
                    .border_color(theme.danger.opacity(0.3))
                    .text_color(theme.danger)
                    .text_size(px(11.5))
                    .child(icon(IconName::TriangleAlert).size(px(14.)))
                    .child(div().flex_1().child(error))
                    .child(
                        div()
                            .id(SharedString::from(format!("dismiss-{index}")))
                            .cursor_pointer()
                            .child(icon(IconName::X).size(px(13.)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.session
                                    .update(cx, |session, cx| session.dismiss_error(index, cx))
                            })),
                    )
            }))
    }

    fn ringing_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let (name, title, incoming) = match (&session.ringing, &session.dialing) {
            (Some(ringing @ Ringing::Call { .. }), _) => (
                ringing.name().to_owned(),
                format!("{} wants to pair", ringing.name()),
                true,
            ),
            (Some(ringing @ Ringing::Invite { .. }), _) => (
                ringing.name().to_owned(),
                format!("{} invited you to their call", ringing.name()),
                true,
            ),
            (None, Some((_, name, _))) => (name.clone(), format!("Calling {name}…"), false),
            (None, None) => return None,
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(style::RADIUS))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.accent.opacity(0.07))
                .child(style::avatar(&name, 34., cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .text_ellipsis()
                        .child(title),
                )
                .when(incoming, |card| {
                    card.child(
                        style::chip("home-accept", "Accept", Some(IconName::Phone), Tone::On, cx)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.session
                                    .update(cx, |session, cx| session.accept_ringing(cx))
                            })),
                    )
                    .child(
                        style::chip("home-decline", "Decline", None, Tone::Plain, cx).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.session
                                    .update(cx, |session, cx| session.decline_ringing(cx))
                            }),
                        ),
                    )
                })
                .when(!incoming, |card| {
                    card.child(
                        style::chip("home-cancel", "Cancel", None, Tone::Plain, cx).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.session.update(cx, |session, cx| session.leave(cx))
                            }),
                        ),
                    )
                }),
        )
    }

    fn call_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let room = session.room.clone()?;
        let elapsed = session
            .joined_at
            .map(|at| style::duration(at.elapsed().as_secs()))
            .unwrap_or_default();
        let people: Vec<(String, String, bool)> = session
            .participants
            .iter()
            .map(|participant| {
                (
                    participant.identity.clone(),
                    session.name_for(&participant.identity),
                    session.is_granted(&participant.identity),
                )
            })
            .collect();
        let (connected, microphone, camera, sharing, selecting, canceled) = (
            session.connected,
            session.microphone,
            session.camera,
            session.sharing,
            session.selecting_source,
            session.selection_canceled,
        );
        let remote = session.remote_identity.is_some();
        let headline = if people.is_empty() {
            room.name.clone()
        } else {
            format!(
                "Pairing with {}",
                people
                    .iter()
                    .map(|(_, name, _)| name.split_whitespace().next().unwrap_or(name).to_owned())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let mut roster = div().flex().flex_col().gap(px(4.));
        for (identity, name, granted) in people {
            roster = roster.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(style::avatar(&name, 22., cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_ellipsis()
                            .child(name),
                    )
                    .when(sharing, |row| {
                        let id = identity.clone();
                        row.child(
                            style::chip(
                                SharedString::from(format!("grant-{identity}")),
                                if granted {
                                    "Has control"
                                } else {
                                    "Give control"
                                },
                                Some(IconName::MousePointer2),
                                if granted { Tone::On } else { Tone::Plain },
                                cx,
                            )
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    if granted {
                                        view.session.update(cx, |session, cx| session.revoke(cx));
                                    } else {
                                        let id = id.clone();
                                        view.session.update(cx, |session, cx| {
                                            session.grant_control(id, cx)
                                        });
                                    }
                                },
                            )),
                        )
                    }),
            );
        }

        let controls = div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                style::control(
                    "call-mic",
                    if microphone {
                        IconName::Mic
                    } else {
                        IconName::MicOff
                    },
                    if microphone { Tone::Plain } else { Tone::Off },
                    if microphone { "Mute" } else { "Unmute" },
                    cx,
                )
                .when(connected, |control| {
                    control.on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_microphone(cx))
                    }))
                }),
            )
            .child(
                style::control(
                    "call-camera",
                    if camera {
                        IconName::Video
                    } else {
                        IconName::VideoOff
                    },
                    if camera { Tone::Plain } else { Tone::Off },
                    if camera { "Camera off" } else { "Camera on" },
                    cx,
                )
                .when(connected, |control| {
                    control.on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_camera(cx))
                    }))
                }),
            )
            .child(
                style::chip(
                    "call-share",
                    if selecting {
                        if canceled {
                            "Dismiss chooser"
                        } else {
                            "Cancel"
                        }
                    } else if sharing {
                        "Stop sharing"
                    } else {
                        "Share screen"
                    },
                    Some(if sharing {
                        IconName::ScreenShareOff
                    } else {
                        IconName::ScreenShare
                    }),
                    if sharing { Tone::On } else { Tone::Plain },
                    cx,
                )
                .h(px(28.))
                .when(connected || selecting, |chip| {
                    chip.on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_share(false, cx))
                    }))
                }),
            )
            .when(!sharing && !selecting && connected, |row| {
                row.child(
                    style::chip(
                        "call-window",
                        "Window",
                        Some(IconName::PictureInPicture2),
                        Tone::Plain,
                        cx,
                    )
                    .h(px(28.))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, cx| session.toggle_share(true, cx))
                    })),
                )
            })
            .child(div().flex_1())
            .child(
                style::control("call-faces", IconName::Users, Tone::Plain, "Show faces", cx)
                    .on_click(cx.listener(|_, _, _, cx| cx.defer(Shell::show_faces))),
            )
            .when(remote, |row| {
                row.child(
                    style::control(
                        "call-screen",
                        IconName::Monitor,
                        Tone::Plain,
                        "Show shared screen",
                        cx,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.defer(Shell::show_viewer))),
                )
            })
            .child(
                style::control(
                    "call-leave",
                    IconName::PhoneOff,
                    Tone::Danger,
                    "Leave · Ctrl Shift L",
                    cx,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.session.update(cx, |session, cx| session.leave(cx))
                })),
            );

        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(style::RADIUS))
                .border_1()
                .border_color(theme.success.opacity(0.55))
                .bg(theme.success.opacity(0.06))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().size(px(7.)).rounded_full().bg(if connected {
                            theme.success
                        } else {
                            theme.warning
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.bright)
                                .text_ellipsis()
                                .child(headline),
                        )
                        .child(style::muted_text(elapsed, cx)),
                )
                .when(selecting, |card| {
                    card.child(style::muted_text(
                        if canceled {
                            "Selection canceled. Close the system chooser with Escape."
                        } else {
                            "Pick a screen or window in the system chooser."
                        },
                        cx,
                    ))
                })
                .child(roster)
                .child(controls),
        )
    }

    fn teammates(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let query = self.query(cx);
        let session = self.session.read(cx);
        let own = session.user.as_ref().map(|user| user.id.clone());
        let in_call = session.in_call();
        let busy = session.dialing.is_some();
        let room_of = |id: &str| {
            session
                .room_presence
                .iter()
                .find(|(_, members)| members.iter().any(|member| member == id))
                .and_then(|(room, _)| session.rooms.iter().find(|candidate| &candidate.id == room))
                .map(|room| room.name.clone())
        };
        let mut people: Vec<_> = session
            .teammates
            .iter()
            .filter(|teammate| Some(&teammate.id) != own.as_ref())
            .map(|teammate| {
                let name = full_name(&teammate.first_name, &teammate.last_name);
                (teammate.clone(), name, room_of(&teammate.id))
            })
            .filter(|(_, name, _)| query.is_empty() || name.to_lowercase().contains(&query))
            .collect();
        people.sort_by(|a, b| b.0.is_active.cmp(&a.0.is_active).then(a.1.cmp(&b.1)));
        let available = people
            .iter()
            .filter(|(teammate, ..)| teammate.is_active)
            .count();

        let mut list = div().flex().flex_col().gap(px(2.)).mx(px(-6.));
        if people.is_empty() {
            list = list.child(div().px(px(6.)).py(px(4.)).child(style::muted_text(
                if query.is_empty() {
                    "Invite teammates to your Hopp team to pair with them."
                } else {
                    "No teammates match."
                },
                cx,
            )));
        }
        for (teammate, name, room) in people {
            let status = match (&room, teammate.is_active) {
                (Some(room), _) => format!("In {room}"),
                (None, true) => "Available".to_owned(),
                (None, false) => "Away".to_owned(),
            };
            let target = teammate.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("person-{}", teammate.id)))
                    .group("person")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(6.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|style| style.bg(theme.selection))
                    .child(
                        div()
                            .relative()
                            .child(style::avatar(&name, 30., cx))
                            .child(style::presence_dot(teammate.is_active, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(theme.bright)
                                    .text_ellipsis()
                                    .child(name),
                            )
                            .child(style::muted_text(status, cx).text_ellipsis()),
                    )
                    .child(
                        div()
                            .invisible()
                            .group_hover("person", |style| style.visible())
                            .child(
                                style::chip(
                                    SharedString::from(format!("pair-{}", teammate.id)),
                                    if in_call { "Invite" } else { "Pair" },
                                    Some(IconName::Phone),
                                    Tone::On,
                                    cx,
                                )
                                .when(!busy, |chip| {
                                    chip.on_click(cx.listener(move |view, _, _, cx| {
                                        let target = target.clone();
                                        view.session
                                            .update(cx, |session, cx| session.call(&target, cx));
                                    }))
                                }),
                            ),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(style::section_label("TEAMMATES", cx))
                    .child(style::section_label(format!("{available} available"), cx)),
            )
            .child(list)
    }

    /// Calls and invitations nobody answered, newest first, until you call back or
    /// dismiss them.
    fn missed(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        if session.missed.is_empty() {
            return None;
        }
        let missed = session.missed.clone();
        let busy = session.dialing.is_some() || session.ringing.is_some();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as i64);
        let mut list = div().flex().flex_col().gap(px(2.)).mx(px(-6.));
        for entry in &missed {
            let (back, gone) = (entry.caller_id.clone(), entry.caller_id.clone());
            list = list.child(
                div()
                    .id(SharedString::from(format!("missed-{}", entry.caller_id)))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(6.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|style| style.bg(theme.selection))
                    .child(style::avatar(&entry.name, 30., cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(theme.bright)
                                    .text_ellipsis()
                                    .child(entry.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(theme.danger)
                                    .text_ellipsis()
                                    .child(crate::missed::describe(entry, now)),
                            ),
                    )
                    .child(
                        style::chip(
                            SharedString::from(format!("call-back-{}", entry.caller_id)),
                            "Call back",
                            Some(IconName::Phone),
                            Tone::On,
                            cx,
                        )
                        .when(!busy, |chip| {
                            chip.on_click(cx.listener(move |view, _, _, cx| {
                                view.session
                                    .update(cx, |session, cx| session.call_back(&back, cx));
                            }))
                        }),
                    )
                    .child(
                        style::chip(
                            SharedString::from(format!("dismiss-{}", entry.caller_id)),
                            "",
                            Some(IconName::X),
                            Tone::Plain,
                            cx,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.dismiss_missed(Some(&gone), cx));
                        })),
                    ),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(style::section_label("MISSED", cx))
                        .child(
                            style::section_label("Clear all", cx)
                                .id("missed-clear")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.session
                                        .update(cx, |session, cx| session.dismiss_missed(None, cx));
                                })),
                        ),
                )
                .child(list),
        )
    }

    fn rooms(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let query = self.query(cx);
        let session = self.session.read(cx);
        let current = session.room.as_ref().map(|room| room.id.clone());
        let mut list = div().flex().flex_col().gap(px(2.)).mx(px(-6.));
        let rooms: Vec<_> = session
            .rooms
            .iter()
            .filter(|room| query.is_empty() || room.name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let active = rooms
            .iter()
            .filter(|room| {
                session
                    .room_presence
                    .get(&room.id)
                    .is_some_and(|members| !members.is_empty())
            })
            .count();
        for room in rooms {
            let members: Vec<String> = session
                .room_presence
                .get(&room.id)
                .map(|members| {
                    members
                        .iter()
                        .filter_map(|id| {
                            session.teammates.iter().find(|teammate| &teammate.id == id)
                        })
                        .map(|teammate| full_name(&teammate.first_name, &teammate.last_name))
                        .collect()
                })
                .unwrap_or_default();
            let here = current.as_ref() == Some(&room.id);
            let target = room.clone();
            let mut faces = div().flex().flex_shrink_0().items_center().gap(px(2.));
            for name in members.iter().take(4) {
                faces = faces.child(style::avatar(name, 18., cx));
            }
            list = list.child(
                div()
                    .id(SharedString::from(format!("room-{}", room.id)))
                    .group("room")
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(6.))
                    .py(px(5.))
                    .rounded(px(8.))
                    .hover(|style| style.bg(theme.selection))
                    .child(
                        div()
                            .size(px(30.))
                            .rounded(px(8.))
                            .bg(theme.surface)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(theme.secondary)
                            .child(icon(IconName::Hash).size(px(14.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(theme.bright)
                                    .text_ellipsis()
                                    .child(room.name.clone()),
                            )
                            .child(style::muted_text(
                                if here {
                                    "You're here".to_owned()
                                } else if members.is_empty() {
                                    "Empty".to_owned()
                                } else {
                                    format!("{} here", members.len())
                                },
                                cx,
                            )),
                    )
                    .child(faces)
                    .when(!here, |row| {
                        row.child(
                            div()
                                .invisible()
                                .group_hover("room", |style| style.visible())
                                .child(
                                    style::chip(
                                        SharedString::from(format!("join-{}", room.id)),
                                        "Join",
                                        None,
                                        Tone::Plain,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            let room = target.clone();
                                            view.session.update(cx, |session, cx| {
                                                session.join_room(room, cx)
                                            });
                                        },
                                    )),
                                ),
                        )
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(style::section_label("ROOMS", cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(style::section_label(format!("{active} active"), cx))
                            .child(
                                div()
                                    .id("new-room")
                                    .cursor_pointer()
                                    .text_color(theme.secondary)
                                    .hover(|style| style.text_color(theme.bright))
                                    .child(icon(IconName::Plus).size(px(13.)))
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.room_form = !view.room_form;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .when(self.room_form, |section| {
                section.child(input("room-name", &self.room_name, window, cx))
            })
            .child(list)
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let name = session.display_name();
        let email = session
            .user
            .as_ref()
            .map(|user| user.email.clone())
            .unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .pt(px(10.))
            .border_t_1()
            .border_color(theme.divider())
            .child(style::avatar(&name, 26., cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(12.)).text_ellipsis().child(name))
                    .child(style::muted_text(email, cx).text_ellipsis()),
            )
            .child(style::muted_text("⏎ pair · Super H peek", cx))
    }

    fn sign_in_page(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.omarchy().clone();
        let server = self.session.read(cx).settings.server.clone();
        let host = url::Url::parse(&server)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or(server);
        div()
            .flex_1()
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(14.))
            .px(px(8.))
            .child(style::mark(40., cx))
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.bright)
                    .child("Pair with your team"),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme.secondary.opacity(0.8))
                    .child("Share a screen, hand over the keyboard, and talk it through. Hopp lives in your bar and stays out of your tiles."),
            )
            .child(
                style::chip("browser-sign-in", "Sign in with your browser", Some(IconName::ArrowRight), Tone::On, cx)
                    .h(px(34.))
                    .justify_center()
                    .text_size(px(12.5))
                    .on_click(cx.listener(|view, _, _, cx| view.sign_in(false, cx))),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        div()
                            .id("token-toggle")
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(theme.accent)
                            .child("Use an access token")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.token_form = !view.token_form;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("server-settings")
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(theme.secondary.opacity(0.7))
                            .child(format!("Server: {host}"))
                            .on_click(|_, _, cx| Shell::show_settings(cx)),
                    ),
            )
            .when(self.token_form, |page| {
                page.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(div().flex_1().child(input("token", &self.token, window, cx)))
                        .child(
                            style::chip("token-connect", "Connect", None, Tone::Plain, cx)
                                .h(px(30.))
                                .on_click(cx.listener(|view, _, _, cx| view.sign_in(true, cx))),
                        ),
                )
            })
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let signed_in = self.session.read(cx).user.is_some();
        let header = self.header(cx);
        let errors = self.errors(cx);
        let mut body = div()
            .id("home-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.));
        body = match signed_in {
            false => body.child(self.sign_in_page(window, cx)),
            true => {
                let ringing = self.ringing_card(cx);
                let call = self.call_card(cx);
                let missed = self.missed(cx);
                let teammates = self.teammates(cx);
                let rooms = self.rooms(window, cx);
                body.children(ringing)
                    .children(call)
                    .children(missed)
                    .child(
                        input("search", &self.search, window, cx)
                            .prefix(icon(IconName::Search).size(px(14.))),
                    )
                    .child(teammates)
                    .child(rooms)
            }
        };
        let session = self.session.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(18.))
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .text_size(px(12.5))
            .on_key_down(
                cx.listener(move |view, event: &gpui_kit::KeyDownEvent, _, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if event.keystroke.key == "q"
                        && modifiers.control
                        && !(modifiers.shift || modifiers.alt || modifiers.platform)
                    {
                        view.session.update(cx, |session, cx| session.quit(cx));
                        cx.stop_propagation();
                    } else if super::global_shortcut(&event.keystroke, event.is_held, &session, cx)
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .child(header)
            .child(errors)
            .child(body)
            .when(signed_in, |root| root.child(self.footer(cx)))
    }
}

pub fn open(session: Entity<Session>, cx: &mut gpui_kit::App) -> Option<WindowHandle<HomeView>> {
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(420.), px(640.)),
            cx,
        ))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp".into()),
            ..Default::default()
        }),
        app_id: Some("hopp".into()),
        window_min_size: Some(size(px(340.), px(420.))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| HomeView::new(session, window, cx))
    })
    .ok()
}
