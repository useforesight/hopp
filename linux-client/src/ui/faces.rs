//! The call window: everyone's face in fixed tiles, plus call controls.
//!
//! The window is floated and pinned by the Hyprland rules Hopp installs, and its size is
//! derived from the number of people rather than chosen by dragging, so a camera is never
//! stretched by a tile. Tiles are square in a column or row, wide (16:9) in the grid and
//! speaker views, and cameras are centre-cropped to fill them.
use gpui_kit::{
    AppContext, Bounds, Context, Corners, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, Size, StatefulInteractiveElement, Styled, Window, canvas,
    div, prelude::FluentBuilder, px, size,
};
use gpui_omarchy::{ActiveTheme, IconName, icon, with_tooltip};

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
            Self::Row => Self::Grid,
            Self::Grid => Self::Speaker,
            Self::Speaker => Self::Bubbles,
            Self::Bubbles => Self::Column,
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Column => IconName::Rows3,
            Self::Row => IconName::Columns3,
            Self::Grid => IconName::LayoutGrid,
            Self::Speaker => IconName::CircleUser,
            Self::Bubbles => IconName::CircleDot,
        }
    }
}

const PAD: f32 = 6.;
const GAP: f32 = 6.;
const CONTROL_GAP: f32 = 4.;
/// Mic, camera, share, capture, reactions and leave.
pub(super) const CALL_CONTROLS: usize = 6;
/// A wide tile's width and height, as multiples of the chosen size: 16:9.
const WIDE: (f32, f32) = (1.6, 0.9);
/// Everyone else's tiles under the speaker: at most this much of the speaker's height,
/// and never smaller than [`LEAST_TILE`].
const UNDER_SPEAKER: f32 = 0.5;
const LEAST_TILE: f32 = 32.;
/// Narrower tiles show the voice slot alone, as bubbles do, instead of a name.
const NAME_MIN_WIDTH: f32 = 72.;

/// The call controls in one row.
pub(super) fn controls_row_width() -> f32 {
    CALL_CONTROLS as f32 * CONTROL + (CALL_CONTROLS - 1) as f32 * CONTROL_GAP
}

fn controls_row() -> Size<f32> {
    Size {
        width: controls_row_width(),
        height: CONTROL,
    }
}

/// The call controls as a two-column grid.
fn controls_grid() -> Size<f32> {
    let rows = CALL_CONTROLS.div_ceil(2) as f32;
    Size {
        width: 2. * CONTROL + CONTROL_GAP,
        height: rows * CONTROL + (rows - 1.) * CONTROL_GAP,
    }
}

fn tile_side(layout: FaceLayout, face: FaceSize) -> f32 {
    if layout == FaceLayout::Bubbles {
        64.
    } else {
        face.pixels()
    }
}

/// Where something goes in the call window, from the window's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Place {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Place {
    fn centre(self) -> (f32, f32) {
        (self.x + self.width / 2., self.y + self.height / 2.)
    }
}

/// Everything in a call window: its size, each tile in [`faces_in_window`] order, and the
/// call controls.
#[derive(Debug)]
pub(super) struct Places {
    pub window: Size<f32>,
    pub tiles: Vec<Place>,
    pub controls: Place,
}

/// `count` lengths in a line, with gaps between.
fn run(count: f32, length: f32) -> f32 {
    count * length + (count - 1.).max(0.) * GAP
}

/// Lays out a call window for `people` at tile size `side`. Pure, so the arithmetic is
/// tested.
pub(super) fn places(people: usize, layout: FaceLayout, side: f32) -> Places {
    let count = people.max(1);
    let row = controls_row();
    let grid = controls_grid();
    match layout {
        FaceLayout::Column | FaceLayout::Bubbles => {
            // The controls fit one row under a wide enough column, else a grid.
            let controls = if layout == FaceLayout::Column && side >= row.width {
                row
            } else {
                grid
            };
            let inner = side.max(controls.width);
            let strip = run(count as f32, side);
            Places {
                window: Size {
                    width: inner + 2. * PAD,
                    height: strip + GAP + controls.height + 2. * PAD,
                },
                tiles: (0..count)
                    .map(|index| Place {
                        x: PAD + (inner - side) / 2.,
                        y: PAD + index as f32 * (side + GAP),
                        width: side,
                        height: side,
                    })
                    .collect(),
                controls: Place {
                    x: PAD + (inner - controls.width) / 2.,
                    y: PAD + strip + GAP,
                    width: controls.width,
                    height: controls.height,
                },
            }
        }
        FaceLayout::Row => {
            let inner = side.max(grid.height);
            let strip = run(count as f32, side);
            Places {
                window: Size {
                    width: strip + GAP + grid.width + 2. * PAD,
                    height: inner + 2. * PAD,
                },
                tiles: (0..count)
                    .map(|index| Place {
                        x: PAD + index as f32 * (side + GAP),
                        y: PAD + (inner - side) / 2.,
                        width: side,
                        height: side,
                    })
                    .collect(),
                controls: Place {
                    x: PAD + strip + GAP,
                    y: PAD + (inner - grid.height) / 2.,
                    width: grid.width,
                    height: grid.height,
                },
            }
        }
        FaceLayout::Grid => {
            let (width, height) = (side * WIDE.0, side * WIDE.1);
            // As square a grid as there can be: 2 side by side, 3 or 4 in two rows…
            let columns = (count as f32).sqrt().ceil() as usize;
            let rows = count.div_ceil(columns);
            let inner = run(columns as f32, width).max(row.width);
            let tiles = (0..count)
                .map(|index| {
                    let (line, column) = (index / columns, index % columns);
                    // A last row that isn't full sits in the middle.
                    let in_line = (count - line * columns).min(columns) as f32;
                    Place {
                        x: PAD + (inner - run(in_line, width)) / 2. + column as f32 * (width + GAP),
                        y: PAD + line as f32 * (height + GAP),
                        width,
                        height,
                    }
                })
                .collect();
            let strip = run(rows as f32, height);
            Places {
                window: Size {
                    width: inner + 2. * PAD,
                    height: strip + GAP + row.height + 2. * PAD,
                },
                tiles,
                controls: Place {
                    x: PAD + (inner - row.width) / 2.,
                    y: PAD + strip + GAP,
                    width: row.width,
                    height: row.height,
                },
            }
        }
        FaceLayout::Speaker => {
            let (width, height) = (side * WIDE.0, side * WIDE.1);
            let others = (count - 1) as f32;
            let room = width.max(row.width);
            let small = if others > 0. {
                ((room - (others - 1.) * GAP) / others)
                    .min(height * UNDER_SPEAKER)
                    .max(LEAST_TILE)
            } else {
                0.
            };
            let inner = room.max(run(others, small));
            let mut tiles = vec![Place {
                x: PAD + (inner - width) / 2.,
                y: PAD,
                width,
                height,
            }];
            let left = PAD + (inner - run(others, small)) / 2.;
            tiles.extend((0..count - 1).map(|index| Place {
                x: left + index as f32 * (small + GAP),
                y: PAD + height + GAP,
                width: small,
                height: small,
            }));
            let below = PAD + height + GAP + if others > 0. { small + GAP } else { 0. };
            Places {
                window: Size {
                    width: inner + 2. * PAD,
                    height: below + row.height + PAD,
                },
                tiles,
                controls: Place {
                    x: PAD + (inner - row.width) / 2.,
                    y: below,
                    width: row.width,
                    height: row.height,
                },
            }
        }
    }
}

/// Exact window size for `people` faces.
pub fn window_size(people: usize, layout: FaceLayout, face: FaceSize) -> Size<f32> {
    places(people, layout, tile_side(layout, face)).window
}

/// The tile side to use: the chosen size, shrunk in steps until the window fits `limit`
/// (a large size with a full call would otherwise run off the screen).
fn fitted_side(people: usize, layout: FaceLayout, face: FaceSize, limit: Size<f32>) -> f32 {
    let mut side = tile_side(layout, face);
    while side > 64. {
        let size = places(people, layout, side).window;
        if size.width <= limit.width && size.height <= limit.height {
            break;
        }
        side -= 8.;
    }
    side
}

/// The middle of tile `index` in a call window of `window` size showing `people`, for
/// placing things beside a tile from outside the window. The window was sized to fit the
/// screen, so the tile size that fits the window is the one it shows.
pub(super) fn tile_centre(
    index: usize,
    people: usize,
    layout: FaceLayout,
    face: FaceSize,
    window: Size<f32>,
) -> Option<(f32, f32)> {
    // Hyprland reports whole pixels.
    let limit = Size {
        width: window.width + 1.,
        height: window.height + 1.,
    };
    let side = fitted_side(people, layout, face, limit);
    places(people, layout, side)
        .tiles
        .get(index)
        .map(|place| place.centre())
}

/// One person in a call surface. Badges are (icon, is it an alert).
pub(super) struct Face {
    pub key: String,
    pub name: String,
    pub picture: Option<Picture>,
    pub badges: Vec<(IconName, bool)>,
    /// They're sharing their screen (or you are): a green ring and a "Sharing" badge,
    /// which brings their screen to you.
    pub sharing: bool,
    /// The call hears them speaking: green bars move beside their name.
    pub speaking: bool,
    /// Their microphone is muted: a red mic beside their name.
    pub muted: bool,
    /// Your own microphone level while you're unmuted, 0–1, which the bars follow.
    pub level: Option<f32>,
    /// Your microphone gives no sound and reopening it didn't help.
    pub silent: bool,
    /// They said the wake word: a light runs round their tile.
    pub heard: Option<super::heard::Glow>,
}

/// Everyone in the call: the sharer first, you last, so the person you're watching is
/// nearest the screen.
pub(super) fn faces_for(session: &Session) -> Vec<Face> {
    let sharer = session.remote_identity.clone();
    let now = std::time::Instant::now();
    let heard = |key: &str| {
        session
            .heard
            .as_ref()
            .filter(|heard| heard.key == key)
            .and_then(|heard| heard.glow(now))
    };
    let mut others: Vec<(bool, Face)> = session
        .participants
        .iter()
        .map(|participant| {
            let key = crate::protocol::canonical_identity(&participant.identity).to_owned();
            let sharing = sharer
                .as_deref()
                .is_some_and(|identity| crate::protocol::canonical_identity(identity) == key);
            let mut badges = Vec::new();
            if session.is_granted(&participant.identity) {
                badges.push((IconName::MousePointer2, false));
            }
            if participant.capturing {
                badges.push((IconName::AudioLines, false));
            }
            let face = Face {
                picture: session.camera_for(&participant.identity).cloned(),
                name: session.name_for(&participant.identity),
                speaking: session.speaking.is_speaking(&key, now),
                muted: participant.muted,
                heard: heard(&key),
                key,
                badges,
                sharing,
                level: None,
                silent: false,
            };
            (sharing, face)
        })
        .collect();
    others.sort_by_key(|(sharing, _)| !*sharing);
    let mut badges = Vec::new();
    let silent = session.microphone && session.microphone_silent.is_some();
    // Muted shows beside your name, as everyone's does.
    if silent {
        badges.push((IconName::TriangleAlert, true));
    }
    if session.capture.is_some() {
        badges.push((IconName::AudioLines, false));
    }
    let you = Face {
        key: super::speaking::YOU.into(),
        heard: heard(super::speaking::YOU),
        speaking: session.microphone && session.speaking.is_speaking(super::speaking::YOU, now),
        muted: !session.microphone,
        level: session.microphone.then_some(session.microphone_level),
        silent,
        name: session.display_name(),
        picture: if session.camera {
            session.local_camera.clone()
        } else {
            None
        },
        badges,
        sharing: session.sharing,
    };
    others
        .into_iter()
        .map(|(_, face)| face)
        .chain(std::iter::once(you))
        .collect()
}

/// Everyone as the call window shows them: in the speaker view, whoever spoke last comes
/// first (into the big tile), else whoever leads [`faces_for`] (the sharer, or someone
/// else before you).
pub(super) fn faces_in_window(session: &Session, layout: FaceLayout) -> Vec<Face> {
    let mut faces = faces_for(session);
    if layout == FaceLayout::Speaker
        && let Some(speaker) = session
            .speaking
            .last()
            .and_then(|key| faces.iter().position(|face| face.key == key))
    {
        let face = faces.remove(speaker);
        faces.insert(0, face);
    }
    faces
}

/// Mute, camera, share, capture, reactions and leave, wired straight to the session.
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
    let reacting = super::Shell::reactions_tray_open(cx);
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
        control(
            "reactions",
            IconName::PartyPopper,
            if reacting { Tone::On } else { Tone::Plain },
            "Reactions: confetti, a gong, fireworks… for everyone",
            cx,
        )
        .when(connected, |control| {
            control.on_click(|_, _, cx| cx.defer(super::Shell::toggle_reactions_tray))
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

/// A face `width` × `height` (or round): camera centre-cropped to fill it, initials when
/// it's off.
pub(super) fn face_tile(
    painted: &mut Painted,
    face: Face,
    (width, height): (f32, f32),
    round: bool,
    window: &mut Window,
    cx: &gpui_kit::App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let Face {
        key,
        name,
        picture,
        badges,
        sharing,
        speaking,
        muted,
        level,
        silent,
        heard,
    } = face;
    let theme = cx.omarchy().clone();
    let side = width.min(height);
    let radius = if round { side / 2. } else { 9. };
    let you = key == super::speaking::YOU;
    let color = style::person_color(&name, cx);
    let mut tile = div()
        .id(gpui_kit::ElementId::Name(format!("tile-{key}").into()))
        .group("tile")
        .relative()
        .w(px(width))
        .h(px(height))
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
    let voice = voice(&key, muted, speaking, level, cx);
    if round || width < NAME_MIN_WIDTH {
        // Bubbles and small tiles have no room for a name: the slot alone, while it has
        // news.
        if speaking || muted {
            tile = tile.child(
                div()
                    .absolute()
                    .bottom(px(4.))
                    .left(px(width / 2. - 9.))
                    .size(px(18.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.background.opacity(0.8))
                    .child(voice),
            );
        }
    } else {
        // The name is always there and never changes colour; only the slot before it
        // does, so it never moves.
        tile = tile.child(
            div()
                .absolute()
                .left(px(5.))
                .bottom(px(5.))
                .max_w(px(width - 10.))
                .h(px(16.))
                .px(px(5.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .gap(px(4.))
                .bg(theme.background.opacity(0.8))
                .text_size(px(10.))
                .text_color(theme.bright)
                .child(voice)
                .child(div().min_w_0().text_ellipsis().child(name)),
        );
        if silent {
            tile = tile.child(silent_notice(height, cx));
        }
    }
    let mut corner = div()
        .absolute()
        .top(px(5.))
        .right(px(5.))
        .flex()
        .gap(px(3.));
    if sharing {
        corner = corner.child(sharing_badge(&key, you, width >= 120., cx));
    }
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
    tile.when(sharing, |tile| tile.border_2().border_color(theme.success))
        .child(corner)
        .when_some(heard, |tile, glow| {
            tile.child(super::heard::light((width, height), radius, glow, cx))
        })
}

/// "Sharing" in green on the sharer's tile (just the screen, on a small one). On theirs it
/// brings their screen to you; on yours it only says so.
fn sharing_badge(
    key: &str,
    you: bool,
    label: bool,
    cx: &gpui_kit::App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.omarchy();
    div()
        .id(gpui_kit::ElementId::Name(format!("sharing-{key}").into()))
        .h(px(18.))
        .px(px(if label { 6. } else { 3.5 }))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(3.))
        .bg(theme.success)
        .text_color(theme.on_accent)
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(icon(IconName::Monitor).size(px(11.)))
        .when(label, |badge| badge.child("Sharing"))
        .when(!you, |badge| {
            with_tooltip(
                badge
                    .cursor_pointer()
                    // A click here must not start a window drag.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.defer(super::Shell::show_viewer)),
                "See their screen",
            )
        })
}

/// Before someone's name: a grey mic, green bars that move while they speak, or a red
/// mic when they're muted. A slot of fixed size, so nothing beside it moves.
fn voice(
    key: &str,
    muted: bool,
    speaking: bool,
    level: Option<f32>,
    cx: &gpui_kit::App,
) -> gpui_kit::Div {
    let theme = cx.omarchy();
    let slot = div()
        .size(px(10.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center();
    if muted {
        return slot.child(
            icon(IconName::MicOff)
                .size(px(10.))
                .text_color(theme.danger),
        );
    }
    if !speaking {
        return slot.child(
            icon(IconName::Mic)
                .size(px(10.))
                .text_color(theme.secondary.opacity(0.7)),
        );
    }
    let seed = key
        .bytes()
        .fold(0u32, |sum, byte| sum.wrapping_mul(31) + byte as u32)
        % 97;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0., |since| (since.as_millis() % 1_000_000) as f32 / 1000.);
    slot.justify_between()
        .children(bar_heights(level, seed as f32, seconds).map(|height| {
            div()
                .w(px(2.))
                .h(px(height))
                .rounded(px(1.))
                .bg(theme.success)
        }))
}

/// The three bars' heights in pixels, 2 to 10: your own follow your microphone level;
/// for others, who send no level, they move steadily while LiveKit hears them.
fn bar_heights(level: Option<f32>, seed: f32, seconds: f32) -> [f32; 3] {
    let lift = match level {
        Some(level) => level.clamp(0., 1.),
        None => 0.35 + 0.65 * ((seconds * 7.3 + seed).sin() * (seconds * 2.1 + seed).sin()).abs(),
    };
    let mut heights = [0.; 3];
    for (index, scale) in [0.7, 1., 0.55].into_iter().enumerate() {
        let wobble = 1. + 0.3 * (seconds * 11. + index as f32 * 2.).sin();
        heights[index] = 2. + (lift * scale * wobble).clamp(0., 1.) * 8.;
    }
    heights
}

/// Over your tile when your microphone gives no sound: what's wrong and a way to pick
/// another, since Hopp has already tried reopening it.
fn silent_notice(side: f32, cx: &gpui_kit::App) -> gpui_kit::Div {
    let theme = cx.omarchy();
    div()
        .absolute()
        .left_0()
        .right_0()
        .top(px(side * 0.22))
        .mx(px(6.))
        .p(px(6.))
        .rounded(px(6.))
        .bg(theme.background.opacity(0.88))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .text_size(px(10.))
        .text_color(theme.bright)
        .child("No sound from your mic")
        .child(
            style::chip(
                "choose-mic",
                "Choose mic",
                Some(IconName::Mic),
                Tone::Plain,
                cx,
            )
            // A click here must not start a window drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| {
                super::Shell::show_settings_at(super::settings::Section::Devices, cx)
            }),
        )
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
        // Settings can change them while the window is open.
        (self.face, self.layout) = (session.settings.face_size, session.settings.face_layout);
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
        let places = places(people, self.layout, side);
        let wanted = places.window;
        if self.applied != Some(wanted) {
            self.applied = Some(wanted);
            window.resize(size(px(wanted.width), px(wanted.height)));
            // Hyprland ignores a mapped floating window resizing itself; ask it directly.
            super::hypr::fit("hopp.faces", wanted.width, wanted.height);
        }

        let faces = faces_in_window(session, self.layout);
        // The bars move while anyone speaks.
        if faces
            .iter()
            .any(|face| face.speaking || face.heard.is_some())
        {
            window.request_animation_frame();
        }
        let round = self.layout == FaceLayout::Bubbles;
        let mut root = div()
            .id("faces")
            .group("faces")
            .relative()
            .size_full()
            // Frosted: the desktop shows through between the tiles (Hyprland blurs it
            // when decoration:blur is on).
            .bg(theme.inset.opacity(0.62))
            .text_color(theme.foreground)
            .font_family(theme.font.clone())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move());
        for (face, place) in faces.into_iter().zip(&places.tiles) {
            root = root.child(div().absolute().left(px(place.x)).top(px(place.y)).child(
                face_tile(
                    &mut self.painted,
                    face,
                    (place.width, place.height),
                    round,
                    window,
                    cx,
                ),
            ));
        }
        self.painted.finish(window);

        let controls = places.controls;
        let controls = div()
            .absolute()
            .left(px(controls.x))
            .top(px(controls.y))
            .w(px(controls.width))
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(CONTROL_GAP))
            // Controls must not start a window drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(call_controls(&self.session, cx));

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
                style::chip("layout", "", Some(self.layout.icon()), Tone::Off, cx)
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

        root.child(controls).child(tweaks)
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

    const LAYOUTS: [FaceLayout; 5] = [
        FaceLayout::Column,
        FaceLayout::Row,
        FaceLayout::Bubbles,
        FaceLayout::Grid,
        FaceLayout::Speaker,
    ];

    #[test]
    fn a_tiles_middle_is_found_from_the_window_size() {
        for layout in LAYOUTS {
            for face in [FaceSize::Small, FaceSize::Medium, FaceSize::Large] {
                for people in 1..7 {
                    let placed = places(people, layout, tile_side(layout, face));
                    // As Hyprland reports it, in whole pixels.
                    let window = Size {
                        width: placed.window.width.round(),
                        height: placed.window.height.round(),
                    };
                    for (index, tile) in placed.tiles.iter().enumerate() {
                        let found = tile_centre(index, people, layout, face, window).unwrap();
                        let wanted = tile.centre();
                        assert!(
                            (found.0 - wanted.0).abs() < 0.01 && (found.1 - wanted.1).abs() < 0.01,
                            "{layout:?} {face:?} {people}: {found:?}, not {wanted:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn everything_is_inside_the_window_and_no_two_things_overlap() {
        for layout in LAYOUTS {
            for side in [64., 112., 200., 320.] {
                for people in 1..10 {
                    let placed = places(people, layout, side);
                    assert_eq!(placed.tiles.len(), people, "{layout:?}");
                    let all: Vec<Place> = placed
                        .tiles
                        .iter()
                        .copied()
                        .chain([placed.controls])
                        .collect();
                    for (index, place) in all.iter().enumerate() {
                        assert!(
                            place.x >= PAD - 0.01
                                && place.y >= PAD - 0.01
                                && place.x + place.width <= placed.window.width - PAD + 0.01
                                && place.y + place.height <= placed.window.height - PAD + 0.01,
                            "{layout:?} {side} {people}: {place:?} outside {:?}",
                            placed.window
                        );
                        for other in &all[index + 1..] {
                            let apart = place.x + place.width <= other.x + 0.01
                                || other.x + other.width <= place.x + 0.01
                                || place.y + place.height <= other.y + 0.01
                                || other.y + other.height <= place.y + 0.01;
                            assert!(apart, "{layout:?} {side} {people}: {place:?} on {other:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_window_grows_by_exactly_one_tile_per_person() {
        let two = window_size(2, FaceLayout::Column, FaceSize::Medium);
        let three = window_size(3, FaceLayout::Column, FaceSize::Medium);
        assert_eq!(two.width, three.width, "width never changes in a column");
        assert_eq!(three.height - two.height, 200. + GAP);
        // 200 wide fits all six controls on one row: 6 + 3×200 + 2×6 + 6 + 28 + 6.
        assert_eq!(three.height, 6. + 600. + 12. + 6. + CONTROL + 6.);
    }

    #[test]
    fn large_is_twice_the_old_large_and_small_columns_use_a_control_grid() {
        let size = window_size(2, FaceLayout::Column, FaceSize::Large);
        assert_eq!(size.width, 320. + 12.);
        assert_eq!(size.height, 12. + 2. * 320. + GAP + GAP + CONTROL);
        let small = window_size(2, FaceLayout::Column, FaceSize::Small);
        // Six controls in two columns: three rows.
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
    fn the_grid_is_wide_tiles_as_square_as_it_can_be() {
        let two = places(2, FaceLayout::Grid, 200.);
        assert_eq!((two.tiles[0].width, two.tiles[0].height), (320., 180.));
        assert_eq!(two.tiles[0].y, two.tiles[1].y, "two side by side");
        let four = places(4, FaceLayout::Grid, 200.);
        assert_eq!(four.window.width, two.window.width, "four in two rows");
        assert_eq!(four.tiles[2].x, four.tiles[0].x);
        // Three: the third in the middle under the first two.
        let three = places(3, FaceLayout::Grid, 200.);
        let middle = three.tiles[2].centre().0;
        assert!((middle - three.window.width / 2.).abs() < 0.01);
        assert_eq!(
            three.controls.width,
            controls_row_width(),
            "controls in a row"
        );
    }

    #[test]
    fn the_speaker_is_big_and_everyone_else_is_in_a_row_below() {
        let placed = places(4, FaceLayout::Speaker, 200.);
        let speaker = placed.tiles[0];
        assert_eq!((speaker.width, speaker.height), (320., 180.));
        for other in &placed.tiles[1..] {
            assert_eq!(other.y, speaker.y + speaker.height + GAP);
            assert!(other.height <= speaker.height * UNDER_SPEAKER);
        }
        // So many people that their row is wider than the speaker: the window widens.
        let crowd = places(14, FaceLayout::Speaker, 112.);
        assert!(crowd.tiles[1].width >= LEAST_TILE);
        assert!(crowd.window.width > 112. * WIDE.0 + 2. * PAD);
        // Alone, there's no row.
        let alone = places(1, FaceLayout::Speaker, 200.);
        assert_eq!(alone.controls.y, PAD + 180. + GAP);
    }

    #[test]
    fn the_bars_stay_in_their_slot_and_follow_your_level() {
        for step in 0..200 {
            let seconds = step as f32 * 0.037;
            for level in [None, Some(0.), Some(0.5), Some(3.)] {
                for height in bar_heights(level, 13., seconds) {
                    assert!((2. ..=10.).contains(&height), "{height}");
                }
            }
        }
        assert_eq!(bar_heights(Some(0.), 0., 1.), [2.; 3], "silent: flat");
        let quiet: f32 = bar_heights(Some(0.2), 0., 1.).iter().sum();
        let loud: f32 = bar_heights(Some(0.9), 0., 1.).iter().sum();
        assert!(loud > quiet);
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
        assert!(places(5, FaceLayout::Column, five).window.height <= screen.height);
    }
}
