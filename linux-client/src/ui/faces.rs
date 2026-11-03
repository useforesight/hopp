//! The call window: everyone's face in fixed square tiles, plus call controls.
//!
//! The window is floated and pinned by the Hyprland rules Hopp installs, and its size is
//! derived from the number of people rather than chosen by dragging, so a camera is never
//! stretched by a tile. Cameras are centre-cropped to fill their square.
use gpui_kit::{
    AppContext, Bounds, Context, Corners, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, Size, StatefulInteractiveElement, Styled, Window, canvas,
    div, prelude::FluentBuilder, px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon};

use super::{
    geometry::cover,
    painted::Painted,
    session::{Picture, Session, SessionEvent, initials},
    style::{self, CONTROL, Tone, control},
};

pub use crate::settings::{FaceLayout, FaceSize};

impl FaceSize {
    pub fn pixels(self) -> f32 {
        match self {
            Self::Small => 112.,
            Self::Medium => 200.,
            Self::Large => 320.,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Small => Self::Medium,
            Self::Medium => Self::Large,
            Self::Large => Self::Small,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Small => "S",
            Self::Medium => "M",
            Self::Large => "L",
        }
    }
}

impl FaceLayout {
    fn next(self) -> Self {
        match self {
            Self::Column => Self::Row,
            Self::Row => Self::Bubbles,
            Self::Bubbles => Self::Column,
        }
    }
}

const PAD: f32 = 6.;
const GAP: f32 = 6.;
const CONTROL_GAP: f32 = 4.;
/// Mic, camera, share, capture and leave.
pub(super) const CALL_CONTROLS: usize = 5;

/// The call controls in one row.
pub(super) fn controls_row_width() -> f32 {
    CALL_CONTROLS as f32 * CONTROL + (CALL_CONTROLS - 1) as f32 * CONTROL_GAP
}

/// The call controls as a two-column grid.
fn controls_grid() -> Size<f32> {
    let rows = CALL_CONTROLS.div_ceil(2) as f32;
    Size {
        width: 2. * CONTROL + CONTROL_GAP,
        height: rows * CONTROL + (rows - 1.) * CONTROL_GAP,
    }
}

/// Controls as one row or a two-column grid, whichever the tile width allows.
fn controls_in_one_row(layout: FaceLayout, tile: f32) -> bool {
    layout == FaceLayout::Column && tile >= controls_row_width()
}

fn tile_side(layout: FaceLayout, face: FaceSize) -> f32 {
    if layout == FaceLayout::Bubbles {
        64.
    } else {
        face.pixels()
    }
}

/// Exact window size for `people` faces. Pure, so the arithmetic is tested.
pub fn window_size(people: usize, layout: FaceLayout, face: FaceSize) -> Size<f32> {
    sized(people, layout, tile_side(layout, face))
}

/// The tile side to use: the chosen size, shrunk in steps until the window fits `limit`
/// (a large size with a full call would otherwise run off the screen).
fn fitted_side(people: usize, layout: FaceLayout, face: FaceSize, limit: Size<f32>) -> f32 {
    let mut side = tile_side(layout, face);
    while side > 64. {
        let size = sized(people, layout, side);
        if size.width <= limit.width && size.height <= limit.height {
            break;
        }
        side -= 8.;
    }
    side
}

fn sized(people: usize, layout: FaceLayout, tile: f32) -> Size<f32> {
    let people = people.max(1) as f32;
    let strip = people * tile + (people - 1.) * GAP;
    let grid = controls_grid();
    match layout {
        FaceLayout::Column | FaceLayout::Bubbles => {
            let controls = if controls_in_one_row(layout, tile) {
                CONTROL
            } else {
                grid.height
            };
            Size {
                width: tile.max(grid.width) + 2. * PAD,
                height: strip + GAP + controls + 2. * PAD,
            }
        }
        FaceLayout::Row => Size {
            width: strip + GAP + grid.width + 2. * PAD,
            height: tile.max(grid.height) + 2. * PAD,
        },
    }
}

/// One person in a call surface. Badges are (icon, is it an alert).
pub(super) struct Face {
    pub key: String,
    pub name: String,
    pub picture: Option<Picture>,
    pub badges: Vec<(IconName, bool)>,
}

/// Everyone in the call: the sharer first, you last, so the person you're watching is
/// nearest the screen.
pub(super) fn faces_for(session: &Session) -> Vec<Face> {
    let sharer = session.remote_identity.clone();
    let mut others: Vec<(bool, Face)> = session
        .participants
        .iter()
        .map(|participant| {
            let key = crate::protocol::canonical_identity(&participant.identity).to_owned();
            let sharing = sharer
                .as_deref()
                .is_some_and(|identity| crate::protocol::canonical_identity(identity) == key);
            let mut badges = Vec::new();
            if sharing {
                badges.push((IconName::Monitor, false));
            }
            if session.is_granted(&participant.identity) {
                badges.push((IconName::MousePointer2, false));
            }
            if participant.capturing {
                badges.push((IconName::AudioLines, false));
            }
            let face = Face {
                picture: session.camera_for(&participant.identity).cloned(),
                name: session.name_for(&participant.identity),
                key,
                badges,
            };
            (sharing, face)
        })
        .collect();
    others.sort_by_key(|(sharing, _)| !*sharing);
    let mut badges = Vec::new();
    if !session.microphone {
        badges.push((IconName::MicOff, true));
    }
    if session.sharing {
        badges.push((IconName::Monitor, false));
    }
    if session.capture.is_some() {
        badges.push((IconName::AudioLines, false));
    }
    let you = Face {
        key: "you".into(),
        name: session.display_name(),
        picture: if session.camera {
            session.local_camera.clone()
        } else {
            None
        },
        badges,
    };
    others
        .into_iter()
        .map(|(_, face)| face)
        .chain(std::iter::once(you))
        .collect()
}

/// Mute, camera, share, capture and leave, wired straight to the session.
pub(super) fn call_controls(
    session: &Entity<Session>,
    cx: &gpui_kit::App,
) -> [gpui_kit::Stateful<gpui_kit::Div>; CALL_CONTROLS] {
    let state = session.read(cx);
    let (connected, microphone, camera, sharing, selecting) = (
        state.connected,
        state.microphone,
        state.camera,
        state.sharing,
        state.selecting_source,
    );
    let capture_tip = capture_tooltip(state.capture.as_ref());
    let capturing = state.capture.is_some();
    let on = |action: fn(&mut Session, &mut Context<Session>)| {
        let session = session.clone();
        move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            session.update(cx, action)
        }
    };
    [
        control(
            "mic",
            if microphone {
                IconName::Mic
            } else {
                IconName::MicOff
            },
            if microphone { Tone::Plain } else { Tone::Off },
            if microphone {
                "Mute · Ctrl Shift M"
            } else {
                "Unmute · Ctrl Shift M"
            },
            cx,
        )
        .when(connected, |control| {
            control.on_click(on(|session, cx| session.toggle_microphone(cx)))
        }),
        control(
            "camera",
            if camera {
                IconName::Video
            } else {
                IconName::VideoOff
            },
            if camera { Tone::Plain } else { Tone::Off },
            if camera {
                "Turn camera off"
            } else {
                "Turn camera on"
            },
            cx,
        )
        .when(connected, |control| {
            control.on_click(on(|session, cx| session.toggle_camera(cx)))
        }),
        control(
            "share",
            if sharing || selecting {
                IconName::ScreenShareOff
            } else {
                IconName::ScreenShare
            },
            if sharing { Tone::On } else { Tone::Plain },
            if selecting {
                "Cancel screen selection"
            } else if sharing {
                "Stop sharing · Ctrl Shift S"
            } else {
                "Share a screen · Ctrl Shift S"
            },
            cx,
        )
        .when(connected || selecting, |control| {
            control.on_click(on(|session, cx| session.toggle_share(false, cx)))
        }),
        control(
            "capture",
            IconName::AudioLines,
            if capturing { Tone::On } else { Tone::Plain },
            capture_tip,
            cx,
        )
        .when(connected, |control| {
            control.on_click(on(|session, cx| session.toggle_capture(cx)))
        })
        // Right-click: the transcript so far, and past captures.
        .on_mouse_down(MouseButton::Right, |_, _, cx| {
            cx.stop_propagation();
            super::Shell::show_captures(cx);
        }),
        control("leave", IconName::PhoneOff, Tone::Danger, "Leave call", cx)
            .on_click(on(|session, cx| session.leave(cx))),
    ]
}

/// How a capture is getting on, in a few words. Text normally arrives in batches up to
/// half a minute after it was said, so only more than a minute counts as behind.
pub(super) fn capture_state(capture: &crate::engine::CaptureStatus) -> String {
    let model = &capture.model;
    match capture.state.as_str() {
        "downloading" => format!("downloading {model}, {}%", capture.percent.unwrap_or(0)),
        "starting" | "loading" => format!("loading {model}"),
        "idle" => format!("{model} unloaded until someone speaks"),
        "failed" => format!(
            "transcription failed: {}",
            capture.message.as_deref().unwrap_or("unknown error")
        ),
        _ if capture.behind_ms >= 60_000 => format!("{} s behind", capture.behind_ms / 1000),
        _ => {
            let place = match capture.device.as_deref() {
                Some("CPU") => " on the CPU",
                Some(_) => " on the GPU",
                None => "",
            };
            format!("{} lines · {model}{place}", capture.lines)
        }
    }
}

/// What the capture button says, including how the capture is getting on.
pub(super) fn capture_tooltip(capture: Option<&crate::engine::CaptureStatus>) -> String {
    match capture {
        Some(capture) => format!(
            "Capturing: {} · click to stop, right-click for the transcript",
            capture_state(capture)
        ),
        None => "Capture: transcribe this call to this computer · Super+Alt+C".into(),
    }
}

/// A square (or round) face: camera centre-cropped to fill it, initials when it's off.
pub(super) fn face_tile(
    painted: &mut Painted,
    face: Face,
    side: f32,
    round: bool,
    window: &mut Window,
    cx: &gpui_kit::App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let Face {
        key,
        name,
        picture,
        badges,
    } = face;
    let theme = cx.omarchy().clone();
    let radius = if round { side / 2. } else { 9. };
    let color = style::person_color(&name, cx);
    let mut tile = div()
        .id(gpui_kit::ElementId::Name(format!("tile-{key}").into()))
        .group("tile")
        .relative()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(radius))
        .overflow_hidden()
        .bg(color.opacity(0.18))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(color)
                .text_size(px(side * 0.3))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials(&name)),
        );
    if let Some(picture) = picture {
        painted.show(&key, &picture.image, window);
        let (image, source) = (picture.image, picture.source);
        tile = tile.child(
            canvas(
                |bounds, _, _| bounds,
                move |_, bounds: Bounds<gpui_kit::Pixels>, window, _| {
                    let _ = window.paint_image(
                        bounds,
                        cover(bounds, source),
                        Corners::all(px(radius)),
                        image,
                        0,
                        false,
                    );
                },
            )
            .absolute()
            .inset_0(),
        );
    }
    if !round {
        tile = tile.child(
            div()
                .absolute()
                .left(px(5.))
                .bottom(px(5.))
                .max_w(px(side - 10.))
                .px(px(5.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(theme.background.opacity(0.78))
                .text_color(theme.bright)
                .text_size(px(10.))
                .text_ellipsis()
                .invisible()
                .group_hover("tile", |style| style.visible())
                .child(name),
        );
    }
    let mut corner = div()
        .absolute()
        .top(px(5.))
        .right(px(5.))
        .flex()
        .gap(px(3.));
    for (glyph, alert) in badges {
        corner = corner.child(
            div()
                .size(px(18.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(if alert {
                    theme.danger
                } else {
                    theme.background.opacity(0.8)
                })
                .text_color(if alert { theme.on_accent } else { theme.bright })
                .child(icon(glyph).size(px(11.))),
        );
    }
    tile.child(corner)
}

pub struct FacesView {
    session: Entity<Session>,
    painted: Painted,
    face: FaceSize,
    layout: FaceLayout,
    applied: Option<Size<f32>>,
}

impl FacesView {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&session, |_, _, event, cx| {
            if matches!(event, SessionEvent::Cameras) {
                cx.notify();
            }
        })
        .detach();
        let settings = &session.read(cx).settings;
        let face = settings.face_size;
        let layout = settings.face_layout;
        Self {
            session,
            painted: Painted::default(),
            face,
            layout,
            applied: None,
        }
    }

    pub fn people(session: &Session) -> usize {
        session.participants.len() + 1
    }

    pub fn preferred_size(session: &Session) -> Size<f32> {
        window_size(
            Self::people(session),
            session.settings.face_layout,
            session.settings.face_size,
        )
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        let (face, layout) = (self.face, self.layout);
        self.session.update(cx, |session, _| {
            session.settings.face_size = face;
            session.settings.face_layout = layout;
            let _ = crate::settings::save(&session.settings);
        });
        cx.notify();
    }
}

impl Render for FacesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.omarchy().clone();
        let session = self.session.read(cx);
        let people = Self::people(session);
        // Leave room for the bar and a margin around the window.
        let limit = window
            .display(cx)
            .map(|display| {
                let screen = display.bounds().size;
                Size {
                    width: f32::from(screen.width) - 40.,
                    height: f32::from(screen.height) - 80.,
                }
            })
            .unwrap_or(Size {
                width: f32::INFINITY,
                height: f32::INFINITY,
            });
        let side = fitted_side(people, self.layout, self.face, limit);
        let wanted = sized(people, self.layout, side);
        if self.applied != Some(wanted) {
            self.applied = Some(wanted);
            window.resize(size(px(wanted.width), px(wanted.height)));
            // Hyprland ignores a mapped floating window resizing itself; ask it directly.
            super::hypr::fit("hopp.faces", wanted.width, wanted.height);
        }

        let faces = faces_for(session);
        let round = self.layout == FaceLayout::Bubbles;
        let mut tiles = div().flex().gap(px(GAP));
        tiles = if self.layout == FaceLayout::Row {
            tiles.flex_row()
        } else {
            tiles.flex_col()
        };
        for face in faces {
            tiles = tiles.child(face_tile(&mut self.painted, face, side, round, window, cx));
        }
        self.painted.finish(window);

        let controls = call_controls(&self.session, cx);
        let one_row = controls_in_one_row(self.layout, side);
        let mut grid = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(CONTROL_GAP));
        grid = if one_row {
            grid.w_full()
        } else {
            grid.w(px(2. * CONTROL + CONTROL_GAP))
        };
        let controls = grid.children(controls);

        // Layout and size toggles float over the first tile on hover, so they never
        // change the window's size or shape.
        let tweaks = div()
            .absolute()
            .top(px(PAD + 4.))
            .left(px(PAD + 4.))
            .flex()
            .gap(px(3.))
            .invisible()
            .group_hover("faces", |style| style.visible())
            .child(
                style::chip(
                    "layout",
                    "",
                    Some(match self.layout {
                        FaceLayout::Column => IconName::Rows3,
                        FaceLayout::Row => IconName::Columns3,
                        FaceLayout::Bubbles => IconName::CircleDot,
                    }),
                    Tone::Off,
                    cx,
                )
                .px(px(5.))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.layout = view.layout.next();
                    view.persist(cx);
                })),
            )
            .when(!round, |row| {
                row.child(
                    style::chip("size", self.face.label(), None, Tone::Off, cx)
                        .px(px(6.))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.face = view.face.next();
                            view.persist(cx);
                        })),
                )
            });

        let mut root = div()
            .id("faces")
            .group("faces")
            .relative()
            .size_full()
            .p(px(PAD))
            .flex()
            .gap(px(GAP))
            .items_center()
            // Frosted: the desktop shows through between the tiles (Hyprland blurs it
            // when decoration:blur is on).
            .bg(theme.inset.opacity(0.62))
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move());
        root = if self.layout == FaceLayout::Row {
            root.flex_row()
        } else {
            root.flex_col()
        };
        root.child(tiles)
            .child(
                // Controls must not start a window drag.
                div()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(controls),
            )
            .child(tweaks)
    }
}

pub fn open_options(session: &Session) -> gpui_kit::WindowOptions {
    let preferred = FacesView::preferred_size(session);
    gpui_kit::WindowOptions {
        window_bounds: Some(gpui_kit::WindowBounds::Windowed(Bounds {
            origin: Default::default(),
            size: size(px(preferred.width), px(preferred.height)),
        })),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Hopp call".into()),
            ..Default::default()
        }),
        app_id: Some("hopp.faces".into()),
        window_background: gpui_kit::WindowBackgroundAppearance::Transparent,
        focus: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    }
}

pub fn open(
    session: Entity<Session>,
    cx: &mut gpui_kit::App,
) -> Option<gpui_kit::WindowHandle<FacesView>> {
    let options = open_options(session.read(cx));
    cx.open_window(options, |_, cx| cx.new(|cx| FacesView::new(session, cx)))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_grows_by_exactly_one_tile_per_person() {
        let two = window_size(2, FaceLayout::Column, FaceSize::Medium);
        let three = window_size(3, FaceLayout::Column, FaceSize::Medium);
        assert_eq!(two.width, three.width, "width never changes in a column");
        assert_eq!(three.height - two.height, 200. + GAP);
        // 200 wide fits all five controls on one row: 6 + 3×200 + 2×6 + 6 + 28 + 6.
        assert_eq!(three.height, 6. + 600. + 12. + 6. + CONTROL + 6.);
    }

    #[test]
    fn large_is_twice_the_old_large_and_small_columns_use_a_control_grid() {
        let size = window_size(2, FaceLayout::Column, FaceSize::Large);
        assert_eq!(size.width, 320. + 12.);
        assert_eq!(size.height, 12. + 2. * 320. + GAP + GAP + CONTROL);
        let small = window_size(2, FaceLayout::Column, FaceSize::Small);
        // Five controls in two columns: three rows.
        assert_eq!(
            small.height,
            12. + 2. * 112. + GAP + GAP + 3. * CONTROL + 8.
        );
    }

    #[test]
    fn rows_grow_sideways_and_bubbles_stay_small() {
        let row = window_size(3, FaceLayout::Row, FaceSize::Small);
        assert_eq!(row.height, 112. + 12.);
        assert!(
            controls_grid().height <= 112.,
            "the grid fits beside a small tile"
        );
        assert_eq!(row.width, 3. * 112. + 2. * GAP + GAP + 60. + 12.);
        let bubbles = window_size(4, FaceLayout::Bubbles, FaceSize::Large);
        assert_eq!(bubbles.width, 64. + 12.);
    }

    #[test]
    fn large_tiles_shrink_to_keep_a_full_call_on_screen() {
        let screen = Size {
            width: 3160.,
            height: 1270.,
        };
        let two = fitted_side(2, FaceLayout::Column, FaceSize::Large, screen);
        assert_eq!(two, 320.);
        let five = fitted_side(5, FaceLayout::Column, FaceSize::Large, screen);
        assert!(five < 320.);
        assert!(sized(5, FaceLayout::Column, five).height <= screen.height);
    }
}
