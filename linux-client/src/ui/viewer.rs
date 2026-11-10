//! The shared-screen window.
//!
//! As a normal tiled window it is a *viewport*: the remote screen at a readable scale,
//! cropped to whatever shape the tile has and panned to follow the sharer's cursor.
//! The same view in `peek` mode shows the whole screen fitted, in Hyprland's
//! `special:hopp` workspace, toggled with Super+H.
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui_kit::{
    AppContext, Bounds, ContentMask, Context, Corners, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, ScrollDelta,
    StatefulInteractiveElement, Styled, Window, canvas, div, point, prelude::FluentBuilder, px,
    size,
};
use gpui_omarchy::{ActiveTheme, IconName};

use super::{
    Shell,
    faces::{call_controls, face_tile, faces_for},
    geometry::{Bands, ViewMode, Viewport, dock_bands},
    painted::Painted,
    session::{Session, SessionEvent},
    style::{self, Tone},
};
use crate::protocol::Point;

/// How long a sharer cursor position keeps steering the view.
const CURSOR_FRESH: Duration = Duration::from_secs(3);
/// After a manual pan or minimap jump, following waits this long.
const MANUAL_HOLD: Duration = Duration::from_secs(4);
/// The control pill fades after this much stillness.
const PILL_IDLE: Duration = Duration::from_millis(2600);
const MINIMAP_WIDTH: f32 = 168.;

pub struct ViewerView {
    session: Entity<Session>,
    painted: Painted,
    /// Rasterised ink, cursors and pins for this window.
    sprites: Rc<std::cell::RefCell<super::marks::Sprites>>,
    pub peek: bool,
    viewport: Viewport,
    focus: FocusHandle,
    /// Last laid-out video bounds, written during prepaint.
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    scale_factor: f32,
    hover: Option<Point>,
    drag: Option<(gpui_kit::Point<Pixels>, [f32; 2])>,
    manual_until: Option<Instant>,
    activity: Instant,
    last_frame: Instant,
    /// Whether the view has placed itself once since opening.
    settled: bool,
    /// Faces and controls are in the bands around a fitted screen (reported to the shell
    /// so the floating call window can step aside).
    docked: bool,
}

impl ViewerView {
    pub fn new(
        session: Entity<Session>,
        peek: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        // Frames, pointer moves and the docked faces' cameras.
        cx.subscribe(&session, |_, _, event, cx| {
            if matches!(event, SessionEvent::Screen | SessionEvent::Cameras) {
                cx.notify();
            }
        })
        .detach();
        let focus = cx.focus_handle();
        cx.on_blur(&focus, window, |view, _, cx| {
            view.session
                .update(cx, |session, _| session.release_input());
        })
        .detach();
        focus.focus(window, cx);
        Self {
            session,
            painted: Painted::default(),
            sprites: Default::default(),
            peek,
            viewport: Viewport {
                mode: if peek {
                    ViewMode::Fit
                } else {
                    ViewMode::Follow
                },
                ..Default::default()
            },
            focus,
            bounds: Rc::new(Cell::new(None)),
            scale_factor: window.scale_factor(),
            hover: None,
            drag: None,
            manual_until: None,
            activity: Instant::now(),
            last_frame: Instant::now(),
            settled: false,
            docked: false,
        }
    }

    fn source(&self, cx: &gpui_kit::App) -> Option<[u32; 2]> {
        self.session
            .read(cx)
            .screen
            .as_ref()
            .map(|picture| picture.source)
    }

    fn to_frame(&self, position: gpui_kit::Point<Pixels>, cx: &gpui_kit::App) -> Option<Point> {
        let view = self.bounds.get()?;
        self.viewport
            .frame_point(position, view, self.source(cx)?, self.scale_factor)
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        let was_idle = self.activity.elapsed() >= PILL_IDLE;
        self.activity = Instant::now();
        if was_idle {
            cx.notify();
        }
        // Wake once the pill should fade.
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(PILL_IDLE).await;
            let _ = view.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn set_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        self.viewport.mode = mode;
        self.manual_until = None;
        cx.notify();
    }

    /// Choose what the viewport follows: the sharer's cursor while it is moving, otherwise
    /// nothing (the picture stays put rather than wandering after your own mouse).
    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.scale_factor = window.scale_factor();
        let Some(source) = self.source(cx) else {
            return;
        };
        let Some(view) = self.bounds.get() else {
            // Layout hasn't happened yet; come back once the view has a size.
            window.request_animation_frame();
            return;
        };
        if self.viewport.mode == ViewMode::Fit {
            return;
        }
        let manual = self.manual_until.is_some_and(|until| now < until);
        if !manual {
            let session = self.session.read(cx);
            let focus = session.remote_identity.as_deref().and_then(|identity| {
                let wanted = crate::protocol::canonical_identity(identity);
                session
                    .pointers
                    .iter()
                    .find(|(sender, pointer)| {
                        crate::protocol::canonical_identity(sender) == wanted
                            && pointer.moved.elapsed() < CURSOR_FRESH
                    })
                    .map(|(_, pointer)| pointer.position(now))
            });
            if let Some(focus) = focus {
                if self.settled {
                    self.viewport.follow(focus, view, source, self.scale_factor);
                } else {
                    // Open straight onto the sharer's cursor instead of easing from the middle.
                    self.viewport.jump(
                        [focus.x as f32, focus.y as f32],
                        view,
                        source,
                        self.scale_factor,
                    );
                }
            }
            self.settled = true;
        }
        if self.viewport.step(dt) {
            window.request_animation_frame();
        }
    }

    fn handle_global_keys(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        // Typing into your stamp: Enter finishes it, Escape takes it away.
        if self
            .session
            .update(cx, |session, cx| session.stamp_key(&event.keystroke, cx))
        {
            return true;
        }
        // Ctrl+Z while drawing takes back your last stroke.
        if self
            .session
            .update(cx, |session, cx| session.drawing_key(&event.keystroke, cx))
        {
            return true;
        }
        // Typing over their screen starts a note to stamp down with a click.
        if self.focus.is_focused(window)
            && self
                .session
                .update(cx, |session, cx| session.note_key(&event.keystroke, cx))
        {
            return true;
        }
        if key == "escape" {
            let controlling = self.session.read(cx).peer_control;
            if controlling {
                self.session.update(cx, |session, cx| session.revoke(cx));
            } else if self.peek {
                window.remove_window();
            }
            return true;
        }
        let only_ctrl =
            modifiers.control && !modifiers.shift && !modifiers.alt && !modifiers.platform;
        if only_ctrl && self.focus.is_focused(window) && !event.is_held {
            let handled = self.session.update(cx, |session, _| {
                if !session.peer_control || session.drawing {
                    return false;
                }
                match key {
                    "c" => session.copy_clipboard(true),
                    "x" => session.copy_clipboard(false),
                    "v" => session.paste_clipboard(),
                    _ => return false,
                }
                true
            });
            if handled {
                return true;
            }
        }
        // While controlling, keys belong to the remote machine: Ctrl+Shift+C copies in
        // its terminal, and must not stop the capture (or Ctrl+Shift+L leave the call).
        let controlling = {
            let session = self.session.read(cx);
            session.peer_control && !session.drawing
        };
        if controlling && self.focus.is_focused(window) {
            return false;
        }
        super::global_shortcut(&event.keystroke, event.is_held, &self.session, cx)
    }

    /// Follow / 1:1 / Fit.
    fn mode_switch(&self, vertical: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let mode = self.viewport.mode;
        let segment =
            |id: &'static str, label: &'static str, target: ViewMode, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .h(px(22.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .when(mode == target, |segment| {
                        segment
                            .bg(theme.selection)
                            .text_color(theme.bright)
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(mode != target, |segment| {
                        segment
                            .text_color(theme.secondary)
                            .hover(|style| style.text_color(theme.bright))
                    })
                    .on_click(cx.listener(move |view, _, _, cx| view.set_mode(target, cx)))
                    .child(label)
            };
        let switch = div()
            .flex()
            .flex_shrink_0()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme.border.opacity(0.7))
            .child(segment("mode-follow", "Follow", ViewMode::Follow, cx))
            .child(segment("mode-actual", "1:1", ViewMode::Actual, cx))
            .child(segment("mode-fit", "Fit", ViewMode::Fit, cx));
        if vertical {
            switch.flex_col().w_full()
        } else {
            switch
        }
    }

    /// Keep ink until you right-click or clear it, or let it fade: the Mac app's
    /// "Persist until right click". Shown while drawing.
    fn keep_ink_chip(
        &self,
        id: &'static str,
        labels: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let keep = self.session.read(cx).keep_ink;
        let label = match (labels, keep) {
            (false, _) => "",
            (true, true) => "Keeping ink",
            (true, false) => "Keep ink",
        };
        style::chip(
            id,
            label,
            Some(IconName::Pin),
            if keep { Tone::On } else { Tone::Plain },
            cx,
        )
        .on_click(cx.listener(|view, _, _, cx| {
            view.session
                .update(cx, |session, cx| session.toggle_keep_ink(cx))
        }))
    }

    /// Draw, the control state, and peek. `labels` off gives icon-only buttons.
    fn actions(
        &self,
        labels: bool,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::Stateful<gpui_kit::Div>> {
        let say = |text: &'static str| if labels { text } else { "" };
        let session = self.session.read(cx);
        let (controlling, drawing) = (session.peer_control, session.drawing);
        let draw = style::chip(
            "dock-draw",
            if drawing { say("Drawing") } else { say("Draw") },
            Some(IconName::Pencil),
            if drawing { Tone::On } else { Tone::Plain },
            cx,
        )
        .on_click(cx.listener(|view, _, _, cx| {
            view.session
                .update(cx, |session, cx| session.toggle_drawing(cx))
        }));
        let stamping = session.stamping;
        let stamp = style::chip(
            "dock-stamp",
            if stamping {
                say("Stamping")
            } else {
                say("Stamp")
            },
            Some(IconName::MapPin),
            if stamping { Tone::On } else { Tone::Plain },
            cx,
        )
        .on_click(cx.listener(|view, _, _, cx| {
            view.session
                .update(cx, |session, cx| session.toggle_stamping(cx))
        }));
        let control = if controlling {
            style::chip(
                "dock-control",
                say("Controlling · Esc"),
                Some(IconName::MousePointer2),
                Tone::On,
                cx,
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.session.update(cx, |session, cx| session.revoke(cx))
            }))
        } else {
            style::chip(
                "dock-control",
                say("View only"),
                Some(IconName::Eye),
                Tone::Off,
                cx,
            )
        };
        let peek = style::chip(
            "dock-peek",
            if self.peek {
                say("Back to tiles")
            } else {
                say("Peek")
            },
            Some(if self.peek {
                IconName::Minimize2
            } else {
                IconName::Maximize2
            }),
            Tone::Plain,
            cx,
        )
        .on_click(cx.listener(|_, _, _, cx| cx.defer(Shell::toggle_peek)));
        let keep = drawing.then(|| self.keep_ink_chip("dock-keep", labels, cx));
        [Some(draw), keep, Some(stamp), Some(control), Some(peek)]
            .into_iter()
            .flatten()
            .map(|chip| {
                if labels {
                    chip
                } else {
                    chip.w(px(26.)).px_0().justify_center()
                }
            })
            .collect()
    }

    /// Whose screen and how to view it. The call itself (faces and buttons) is the other
    /// band's card.
    fn dock_panel(&self, bands: Bands, compact: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let sharer = session
            .remote_identity
            .as_deref()
            .map(|identity| session.name_for(identity))
            .unwrap_or_else(|| "Teammate".into());
        let source = session.screen.as_ref().map(|picture| picture.source);
        let title = |cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child(format!("{sharer}’s screen")),
                )
                .when_some(source, |column, [width, height]| {
                    column.child(style::muted_text(format!("{width}×{height}"), cx))
                })
        };
        if !bands.sideways {
            // A bar along the top band.
            let actions = self.actions(!compact, cx);
            return style::frosted(cx)
                .items_center()
                .flex_wrap()
                .justify_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(style::avatar(&sharer, 26., cx))
                        .when(!compact, |who| who.child(title(cx))),
                )
                .child(self.mode_switch(false, cx))
                .child(div().flex().gap(px(6.)).children(actions));
        }
        let narrow = bands.size - 24. < 180.;
        let who = if narrow {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .child(style::avatar(&sharer, 26., cx))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.bright)
                        .child(
                            sharer
                                .split_whitespace()
                                .next()
                                .unwrap_or(&sharer)
                                .to_owned(),
                        ),
                )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(style::avatar(&sharer, 26., cx))
                .child(title(cx))
        };
        let actions = self.actions(!narrow, cx);
        let action_list = if narrow {
            div().flex().gap(px(4.)).children(actions)
        } else {
            div().flex().flex_col().gap(px(6.)).children(actions)
        };
        style::frosted(cx)
            .flex_col()
            .when(narrow, |panel| panel.items_center())
            .gap(px(if narrow { 6. } else { 9. }))
            .p(px(if narrow { 8. } else { 12. }))
            .w(px(if narrow {
                96.
            } else {
                (bands.size - 24.).min(196.)
            }))
            .child(who)
            .child(self.mode_switch(narrow, cx))
            .child(action_list)
    }

    /// `compact` drops labels so the pill fits narrow tiles.
    fn pill(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let say = |full: &'static str| if compact { "" } else { full };
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let sharer = session
            .remote_identity
            .as_deref()
            .map(|identity| session.name_for(identity))
            .unwrap_or_else(|| "Teammate".into());
        let source = session.screen.as_ref().map(|picture| picture.source);
        let stamping = session.stamping;
        let (controlling, drawing, own_drawings) = (
            session.peer_control,
            session.drawing,
            session.has_own_marks(),
        );
        let sharer_key = session
            .remote_identity
            .as_deref()
            .map(crate::protocol::canonical_identity);
        // Everyone else in the call; the sharer already leads the pill.
        let others: Vec<String> = session
            .participants
            .iter()
            .filter(|participant| {
                Some(crate::protocol::canonical_identity(&participant.identity)) != sharer_key
            })
            .map(|participant| session.name_for(&participant.identity))
            .collect();
        let mut faces = div().flex().flex_shrink_0().items_center().gap(px(3.));
        for name in others.iter().take(4) {
            faces = faces.child(style::avatar(name, 20., cx));
        }

        style::pill(cx)
            .id("viewer-pill")
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_move(|_, _, cx| cx.stop_propagation())
            .child(style::avatar(&sharer, 22., cx))
            .when(!compact, |pill| {
                pill.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .flex_col()
                        .pr(px(2.))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.bright)
                                .child(format!("{sharer}’s screen")),
                        )
                        .when_some(source, |column, [width, height]| {
                            column.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(theme.secondary.opacity(0.6))
                                    .child(format!("{width}×{height}")),
                            )
                        }),
                )
            })
            .when(!others.is_empty() && !compact, |pill| pill.child(faces))
            .child(style::separator(cx))
            .child(self.mode_switch(false, cx))
            .child(
                style::chip(
                    "draw",
                    if drawing { say("Drawing") } else { say("Draw") },
                    Some(IconName::Pencil),
                    if drawing { Tone::On } else { Tone::Plain },
                    cx,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.session
                        .update(cx, |session, cx| session.toggle_drawing(cx))
                })),
            )
            .when(drawing, |pill| {
                pill.child(self.keep_ink_chip("keep", !compact, cx))
            })
            .child(
                style::chip(
                    "stamp",
                    if stamping {
                        say("Stamping")
                    } else {
                        say("Stamp")
                    },
                    Some(IconName::MapPin),
                    if stamping { Tone::On } else { Tone::Plain },
                    cx,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.session
                        .update(cx, |session, cx| session.toggle_stamping(cx))
                })),
            )
            .when(own_drawings, |pill| {
                pill.child(
                    style::chip("clear", "Clear", None, Tone::Plain, cx).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.session
                                .update(cx, |session, cx| session.clear_own_marks(cx))
                        },
                    )),
                )
            })
            .child(if controlling {
                style::chip(
                    "control",
                    if compact { "Esc" } else { "Controlling · Esc" },
                    Some(IconName::MousePointer2),
                    Tone::On,
                    cx,
                )
                .on_click(cx.listener(|view, _, _, cx| {
                    view.session.update(cx, |session, cx| session.revoke(cx))
                }))
            } else {
                style::chip(
                    "control",
                    say("View only"),
                    Some(IconName::Eye),
                    Tone::Off,
                    cx,
                )
            })
            .when(controlling, |pill| {
                pill.child(
                    style::chip(
                        "paste",
                        say("Paste"),
                        Some(IconName::ClipboardPaste),
                        Tone::Plain,
                        cx,
                    )
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.session
                            .update(cx, |session, _| session.paste_clipboard())
                    })),
                )
            })
            .child(style::separator(cx))
            .child(
                style::control(
                    "peek",
                    if self.peek {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    },
                    Tone::Plain,
                    if self.peek {
                        "Back to your tiles · Super H"
                    } else {
                        "Whole screen · Super H"
                    },
                    cx,
                )
                .size(px(26.))
                .on_click(cx.listener(|_, _, _, cx| cx.defer(Shell::toggle_peek))),
            )
    }

    fn minimap(
        &self,
        view: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if self.viewport.mode == ViewMode::Fit {
            return None;
        }
        let session = self.session.read(cx);
        let picture = session.screen.clone()?;
        if !self
            .viewport
            .is_cropped(view, picture.source, self.scale_factor)
        {
            return None;
        }
        let theme = cx.omarchy().clone();
        let [left, top, width, height] =
            self.viewport
                .visible(view, picture.source, self.scale_factor);
        let map_height = MINIMAP_WIDTH * picture.source[1] as f32 / picture.source[0].max(1) as f32;
        let (image, accent) = (picture.image, theme.accent);
        let manual = self
            .manual_until
            .is_some_and(|until| Instant::now() < until);
        Some(
            div()
                .id("minimap")
                .absolute()
                .right(px(12.))
                .bottom(px(12.))
                .w(px(MINIMAP_WIDTH))
                .h(px(map_height))
                .rounded(px(7.))
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.inset)
                .shadow_lg()
                .cursor_pointer()
                .child(
                    canvas(
                        |bounds, _, _| bounds,
                        move |_, bounds: Bounds<Pixels>, window, _| {
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                Corners::all(px(6.)),
                                image,
                                0,
                                false,
                            );
                            let rect = Bounds {
                                origin: point(
                                    bounds.origin.x + bounds.size.width * left,
                                    bounds.origin.y + bounds.size.height * top,
                                ),
                                size: size(bounds.size.width * width, bounds.size.height * height),
                            };
                            window.paint_quad(gpui_kit::quad(
                                rect,
                                px(2.),
                                accent.opacity(0.14),
                                px(1.5),
                                accent,
                                Default::default(),
                            ));
                        },
                    )
                    .size_full(),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(6.))
                        .bottom(px(4.))
                        .text_size(px(9.5))
                        .text_color(theme.bright.opacity(0.8))
                        .child(if manual {
                            "Paused · click to jump"
                        } else {
                            "Following"
                        }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, event: &gpui_kit::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let (Some(view_bounds), Some(source)) =
                            (view.bounds.get(), view.source(cx))
                        else {
                            return;
                        };
                        // The minimap sits at a fixed offset from the window's bottom-right.
                        let map_left =
                            view_bounds.origin.x + view_bounds.size.width - px(12. + MINIMAP_WIDTH);
                        let map_top =
                            view_bounds.origin.y + view_bounds.size.height - px(12. + map_height);
                        let x = f32::from(event.position.x - map_left) / MINIMAP_WIDTH;
                        let y = f32::from(event.position.y - map_top) / map_height;
                        view.viewport.jump(
                            [x.clamp(0., 1.), y.clamp(0., 1.)],
                            view_bounds,
                            source,
                            view.scale_factor,
                        );
                        view.manual_until = Some(Instant::now() + MANUAL_HOLD);
                        cx.notify();
                    }),
                ),
        )
    }
}

impl Render for ViewerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance(window, cx);
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let picture = session.screen.clone();
        let sharer = session
            .remote_identity
            .as_deref()
            .map(|identity| session.name_for(identity))
            .unwrap_or_else(|| "Teammate".into());
        let scene = session.scene(false);
        let controlling = session.peer_control;
        let drawing = session.drawing;
        let fit = self.viewport.mode == ViewMode::Fit;
        // The screen stretched and blurred behind itself, TV-style, in Fit and peek.
        let ambient = if fit { session.ambient.clone() } else { None };
        if let Some(picture) = &picture {
            self.painted.show("screen", &picture.image, window);
        }
        if let Some(ambient) = &ambient {
            self.painted.show("ambient", &ambient.image, window);
        }
        let bands = match (fit, self.bounds.get(), picture.as_ref()) {
            (true, Some(view), Some(picture)) => dock_bands(view, picture.source),
            _ => None,
        };
        let docked = bands.is_some();
        if docked != self.docked {
            self.docked = docked;
            let peek = self.peek;
            cx.defer(move |cx| Shell::set_docked(peek, docked, cx));
        }
        let idle = self.activity.elapsed() >= PILL_IDLE && !self.peek;

        let bounds_cell = self.bounds.clone();
        let viewport = self.viewport;
        let scale_factor = self.scale_factor;

        let mut stage = div()
            .id("viewer")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            // Fit leaves the window transparent around the screen so the blurred copy (and a
            // little of the desktop) shows; cropped modes fill the window themselves.
            .when(!(fit && picture.is_some()), |stage| stage.bg(theme.inset))
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .when(drawing, |stage| stage.cursor_crosshair())
            .when(!drawing && !controlling, |stage| stage.cursor_grab())
            .on_key_down(
                cx.listener(|view, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if view.handle_global_keys(event, window, cx) {
                        cx.stop_propagation();
                        return;
                    }
                    if view
                        .session
                        .update(cx, |session, _| session.remote_key(&event.keystroke, true))
                    {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_key_up(cx.listener(|view, event: &gpui_kit::KeyUpEvent, _, cx| {
                if view
                    .session
                    .update(cx, |session, _| session.remote_key(&event.keystroke, false))
                {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    view.touch(cx);
                    // Dragging with nothing else to do pans the view by hand.
                    if let Some((origin, center)) = view.drag
                        && event.dragging()
                        && let (Some(bounds), Some(source)) = (view.bounds.get(), view.source(cx))
                    {
                        let image = view
                            .viewport
                            .image_bounds(bounds, source, view.scale_factor);
                        let dx =
                            f32::from(event.position.x - origin.x) / f32::from(image.size.width);
                        let dy =
                            f32::from(event.position.y - origin.y) / f32::from(image.size.height);
                        view.viewport.jump(
                            [center[0] - dx, center[1] - dy],
                            bounds,
                            source,
                            view.scale_factor,
                        );
                        view.manual_until = Some(Instant::now() + MANUAL_HOLD);
                        cx.notify();
                        return;
                    }
                    let Some(point) = view.to_frame(event.position, cx) else {
                        view.hover = None;
                        return;
                    };
                    view.hover = Some(point);
                    let (dragging, straight) = (event.dragging(), event.modifiers.shift);
                    view.session.update(cx, |session, cx| {
                        session.pointer_moved(point, dragging, straight, cx)
                    });
                }),
            )
            .on_scroll_wheel(cx.listener(
                |view, event: &gpui_kit::ScrollWheelEvent, window, cx| {
                    view.touch(cx);
                    let (x, y) = match event.delta {
                        ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
                        ScrollDelta::Lines(delta) => (delta.x * 24., delta.y * 24.),
                    };
                    let controlling = view.session.read(cx).peer_control;
                    if controlling && view.focus.is_focused(window) && !event.modifiers.alt {
                        if view.to_frame(event.position, cx).is_some() {
                            view.session.update(cx, |session, _| session.wheel(x, y));
                            cx.stop_propagation();
                        }
                        return;
                    }
                    let (Some(bounds), Some(source)) = (view.bounds.get(), view.source(cx)) else {
                        return;
                    };
                    if event.modifiers.control || event.modifiers.alt {
                        if view.viewport.mode != ViewMode::Fit {
                            view.viewport.mode = ViewMode::Follow;
                            let zoom = view.viewport.zoom * (1. + y / 400.);
                            view.viewport.set_zoom(zoom);
                        }
                    } else {
                        let image = view
                            .viewport
                            .image_bounds(bounds, source, view.scale_factor);
                        let center = view.viewport.center;
                        view.viewport.jump(
                            [
                                center[0] - x / f32::from(image.size.width),
                                center[1] - y / f32::from(image.size.height),
                            ],
                            bounds,
                            source,
                            view.scale_factor,
                        );
                        view.manual_until = Some(Instant::now() + MANUAL_HOLD);
                    }
                    cx.notify();
                },
            ));

        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            stage = stage
                .on_mouse_down(
                    button,
                    cx.listener(|view, event: &gpui_kit::MouseDownEvent, window, cx| {
                        view.focus.focus(window, cx);
                        let Some(point) = view.to_frame(event.position, cx) else {
                            return;
                        };
                        let (controlling, marking) = {
                            let session = view.session.read(cx);
                            (
                                session.peer_control,
                                session.drawing || session.stamping || session.note.is_some(),
                            )
                        };
                        // Dragging pans the view, unless a click means something.
                        if !controlling && !marking && event.button == MouseButton::Left {
                            view.drag = Some((event.position, view.viewport.center));
                            return;
                        }
                        let (button, modifiers, clicks) =
                            (event.button, event.modifiers, event.click_count as u32);
                        view.session.update(cx, |session, cx| {
                            session.pointer_down(point, button, modifiers, clicks, cx)
                        });
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                        view.drag = None;
                        let point = view.to_frame(event.position, cx);
                        let (button, modifiers) = (event.button, event.modifiers);
                        view.session.update(cx, |session, cx| {
                            session.pointer_up(point, button, modifiers, cx)
                        });
                    }),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(|view, event: &gpui_kit::MouseUpEvent, _, cx| {
                        view.drag = None;
                        let point = view.to_frame(event.position, cx);
                        let (button, modifiers) = (event.button, event.modifiers);
                        view.session.update(cx, |session, cx| {
                            session.pointer_up(point, button, modifiers, cx)
                        });
                    }),
                );
        }

        if let Some(picture) = picture {
            let (image, source) = (picture.image, picture.source);
            let sprites = self.sprites.clone();
            stage = stage.child(
                canvas(
                    move |bounds, _, _| {
                        bounds_cell.set(Some(bounds));
                        bounds
                    },
                    move |_, bounds: Bounds<Pixels>, window, cx| {
                        if let Some(ambient) = ambient {
                            // Stretched to the window's shape, not cropped: every band
                            // carries the colours from its own side of the screen.
                            let _ = window.paint_image(
                                bounds,
                                bounds,
                                Corners::default(),
                                ambient.image,
                                0,
                                false,
                            );
                        }
                        let image_bounds = viewport.image_bounds(bounds, source, scale_factor);
                        let _ = window.paint_image(
                            bounds,
                            image_bounds,
                            if fit {
                                Corners::all(px(6.))
                            } else {
                                Corners::default()
                            },
                            image,
                            0,
                            false,
                        );
                        let mut sprites = sprites.borrow_mut();
                        let animating =
                            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                super::marks::paint_scene(
                                    window,
                                    cx,
                                    &mut sprites,
                                    &scene,
                                    image_bounds,
                                    // Bubbles stay on the visible part of the screen.
                                    image_bounds.intersect(&bounds),
                                )
                            });
                        if animating {
                            window.request_animation_frame();
                        }
                    },
                )
                .size_full(),
            );
        } else {
            stage = stage.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .child(style::avatar(&sharer, 44., cx))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.bright)
                            .child(format!("Waiting for {sharer}’s screen…")),
                    ),
            );
        }

        let compact = f32::from(window.viewport_size().width) < 820.;
        let view_bounds = self.bounds.get();
        let minimap = view_bounds.and_then(|bounds| self.minimap(bounds, cx));
        stage = match bands {
            Some(bands) => {
                let panel = self.dock_panel(bands, compact, cx);
                let faces = faces_for(self.session.read(cx));
                // The bars beside a speaker's name move while they talk.
                if faces
                    .iter()
                    .any(|face| face.speaking || face.heard.is_some())
                {
                    window.request_animation_frame();
                }
                let (width, height) = view_bounds
                    .map(|view| (f32::from(view.size.width), f32::from(view.size.height)))
                    .unwrap_or_default();
                let along = if bands.sideways { height } else { width };
                let side = dock_face_side(bands, along, faces.len());
                // The call card: faces filling the band, with the call buttons beside
                // (or under) them.
                let controls = call_controls(&self.session, cx);
                let buttons = if bands.sideways && side >= super::faces::controls_row_width() {
                    div().flex().gap(px(4.)).children(controls)
                } else if side >= 2. * style::CONTROL + 4. {
                    div().grid().grid_cols(2).gap(px(4.)).children(controls)
                } else if bands.sideways {
                    div().flex().flex_col().gap(px(4.)).children(controls)
                } else {
                    div().flex().gap(px(4.)).children(controls)
                };
                let mut tiles = style::frosted(cx).items_center().gap(px(6.)).p(px(8.));
                tiles = if bands.sideways {
                    tiles.flex_col()
                } else {
                    tiles.flex_row()
                };
                for face in faces {
                    tiles =
                        tiles.child(face_tile(&mut self.painted, face, side, false, window, cx));
                }
                let tiles = tiles.child(buttons);
                // Panels swallow pointer input so clicks there never pan or reach the peer.
                let shield = |panel: gpui_kit::Div| {
                    panel
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                        .on_mouse_move(|_, _, cx| cx.stop_propagation())
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                };
                let pad = px(12.);
                let band = px(bands.size);
                if bands.sideways {
                    stage
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(band)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(shield(panel)),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .right_0()
                                .w(band)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(shield(tiles)),
                        )
                } else {
                    stage
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top_0()
                                .h(band)
                                .px(pad)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(shield(panel)),
                        )
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .h(band)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(shield(tiles)),
                        )
                }
            }
            None => {
                let pill = self.pill(compact, cx);
                stage
                    .child(
                        div()
                            .absolute()
                            .top(px(10.))
                            .left_0()
                            .right_0()
                            .flex()
                            .justify_center()
                            .when(idle, |row| row.invisible())
                            .child(pill),
                    )
                    .children(minimap)
            }
        };
        self.painted.finish(window);
        let stamp_hint = {
            let session = self.session.read(cx);
            (session.stamping || session.note.is_some()).then(|| session.stamp_hint())
        };
        stage
            .when_some(stamp_hint, |stage, hint| {
                stage.child(
                    div()
                        .absolute()
                        .bottom(px(12.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(5.))
                                .rounded(px(12.))
                                .bg(cx.omarchy().background.opacity(0.9))
                                .child(style::muted_text(hint, cx)),
                        ),
                )
            })
            .when(controlling && !idle && !docked, |stage| {
                stage.child(
                    div()
                        .absolute()
                        .bottom(px(12.))
                        .left(px(12.))
                        .child(style::muted_text("Esc releases control", cx)),
                )
            })
    }
}

/// Face size in a docked call card: as big as the band allows, shrinking so everyone (and
/// the call buttons) still fits along it.
fn dock_face_side(bands: Bands, along: f32, people: usize) -> f32 {
    let people = people.max(1) as f32;
    // Outer margin and card padding on both sides, and room for the buttons.
    let across = bands.size - 2. * 12. - 2. * 8.;
    let buttons = 2. * style::CONTROL + 4. + 6.;
    let fits = (along - 2. * 12. - 2. * 8. - buttons - 6. * (people - 1.)) / people;
    across.min(fits).clamp(44., 280.)
}

pub fn open(
    session: Entity<Session>,
    peek: bool,
    cx: &mut gpui_kit::App,
) -> Option<gpui_kit::WindowHandle<ViewerView>> {
    let options = gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(800.)),
            cx,
        ))),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(if peek { "Hopp peek" } else { "Hopp screen" }.into()),
            ..Default::default()
        }),
        // Transparent so a little of the desktop shows through the blurred bands in Fit.
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        app_id: Some(if peek { "hopp.peek" } else { "hopp.viewer" }.into()),
        focus: true,
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        cx.new(|cx| ViewerView::new(session, peek, window, cx))
    })
    .ok()
}
