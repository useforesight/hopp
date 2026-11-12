//! Screensharing window with interactive Iced UI rendered via wgpu.
//!
//! This module implements a standalone window for the screensharing controls,
//! using winit for the window, wgpu for the GPU surface, and iced for
//! the interactive widget tree (buttons, text, layout).
//!
//! UI styling is ported from the iced-poc sharing window:
//! - Geist font family (Regular + Medium)
//! - Tailwind color tokens (Slate, Gray, Green, Orange, Red)
//! - Shadow tokens for consistent depth
//! - Pill-shaped control buttons with solid/gradient backgrounds

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant as StdInstant};

use iced::mouse;
use iced::widget::{canvas, column, container, row, shader, stack, text, Space};
use iced::{
    alignment, gradient, Alignment, Background, Border, Color, Length, Padding, Pixels, Point,
    Radians, Rectangle, Shadow, Size, Vector,
};
use iced_core::clipboard::Kind;
use iced_wgpu::graphics::Viewport;
use iced_winit::core::renderer::Style;
use iced_winit::core::time::Instant;
use iced_winit::core::{window, Event, Theme};
use iced_winit::runtime::user_interface::Cache;
use iced_winit::runtime::UserInterface;
use iced_winit::{conversion, Clipboard};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState};
#[cfg(not(target_os = "macos"))]
use winit::window::CursorIcon;
use winit::window::{Window, WindowAttributes, WindowId};

use thiserror::Error;

use super::aspect_ratio::{
    calculate_max_window_size, default_window_size, layout_aspect, min_window_size,
    AspectRatioEnforcer, WindowConstant, FACES_BOTTOM_SHARE, FACES_SIDE_SHARE,
};
use super::camera_window::{face_card, identity_to_id, round_face, should_skip, RoundFace};
use super::drawing_helpers;
use crate::components::call_controls::{
    CallControlsDensity, CallControlsMessage, CallControlsState,
};
use crate::components::dropdown::{dropdown_overlay, dropdown_trigger_button, DropdownItemDef};
use crate::components::fonts::{self as fonts_mod, GEIST_MEDIUM, GEIST_REGULAR};
use crate::components::segmented_control::{
    self as seg_ctrl_mod, SegmentedButton, SegmentedControlAnim,
};
use crate::graphics::graphics_context::click_animation::ClickAnimationRenderer;
use crate::graphics::graphics_context::participant::{
    CursorMode, ParticipantError, ParticipantsManager,
};
use crate::graphics::graphics_window_context::{
    ContextManager, GraphicsWindowContext, GraphicsWindowContextError,
};
use crate::graphics::yuv_renderer::YuvVideoProgram;
use crate::livekit::participant::ParticipantInfo;
use crate::utils::clock;
use crate::utils::geometry::{Extent, Position};
use crate::windows::colors::ColorToken;
use hopp_marks::stamps::{StampInput, StampMessage};
use socket_lib::{CallWindowPrefs, FacesInShared};

pub fn screensharing_window_attributes() -> WindowAttributes {
    let (init_w, init_h) = default_window_size();
    let (min_w, min_h) = min_window_size();
    let attrs = WindowAttributes::default()
        .with_title("Hopp Screensharing")
        .with_inner_size(winit::dpi::LogicalSize::new(init_w, init_h))
        .with_resizable(true)
        .with_min_inner_size(winit::dpi::LogicalSize::new(min_w, min_h));

    #[cfg(target_os = "macos")]
    let attrs = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attrs
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_transparent(true)
    };
    attrs
}

const SCREENSHARE_CALL_CONTROLS_MIN_WIDTH: f32 = 768.0;
const SCREENSHARE_SEGMENTED_CONTROLS_WIDTH: f32 = 132.0;
const SCREENSHARE_SETTINGS_BUTTON_WIDTH: f32 = 44.0;
/// The faces button and the cog, at the right of the header; the same again on the left keeps
/// the drawing controls in the middle.
const SCREENSHARE_HEADER_END_WIDTH: f32 = SCREENSHARE_SETTINGS_BUTTON_WIDTH * 2.0;

/// Available screen area detected at runtime by probing with a temporary window.
/// This replaces hardcoded OS chrome offsets (menubar, taskbar, dock) with
/// actual values from the window manager.
#[derive(Debug)]
struct ScreenArea {
    /// Top-left position of the available area in logical pixels.
    position: Position,
    /// Available dimensions in logical pixels.
    extent: Extent,
}

/// Probe the available screen area by creating a temporary borderless, maximized,
/// invisible window. The window manager will constrain it to the usable area
/// (excluding menubar, dock, taskbar), letting us read back the true offsets.
/// Retries for up to 100ms if the window manager returns zero dimensions.
/// Falls back to monitor size with hardcoded OS chrome offsets on failure.
fn probe_available_screen_area(event_loop: &ActiveEventLoop) -> ScreenArea {
    let start = StdInstant::now();
    let timeout = Duration::from_millis(200);

    let attrs = WindowAttributes::default()
        .with_decorations(false)
        .with_maximized(true)
        .with_transparent(true)
        .with_visible(false);

    // On windows set the decorations in order to align the bottom of the window with the taskbar
    #[cfg(target_os = "windows")]
    let attrs = attrs.with_decorations(true);

    let Some(window) = event_loop.create_window(attrs).ok() else {
        log::warn!("probe_available_screen_area: failed to create probe window, using defaults");
        return default_screen_area_from_hardcoded();
    };

    let mut area = fallback_screen_area_from_monitor(&window);
    let scale = window.scale_factor();

    // On macOS, read monitor origin to detect if WM has placed the window yet.
    #[cfg(target_os = "macos")]
    let monitor_origin_y: f64 = window
        .current_monitor()
        .map(|m| {
            let p: winit::dpi::LogicalPosition<f64> = m.position().to_logical(m.scale_factor());
            p.y
        })
        .unwrap_or(0.0);

    loop {
        let inner: winit::dpi::LogicalSize<f64> = window.inner_size().to_logical(scale);
        let pos: Option<winit::dpi::LogicalPosition<f64>> =
            window.outer_position().ok().map(|p| p.to_logical(scale));

        if let Some(pos) = pos.filter(|_| inner.width > 0.0 && inner.height > 0.0) {
            area = ScreenArea {
                position: Position { x: pos.x, y: pos.y },
                extent: Extent {
                    width: inner.width,
                    height: inner.height,
                },
            };

            // On macOS, position must differ from the monitor origin
            // to confirm the WM accounted for the menu bar.
            #[cfg(target_os = "macos")]
            let settled = (pos.y - monitor_origin_y).abs() >= 1.0;
            #[cfg(not(target_os = "macos"))]
            let settled = true;

            if settled {
                // On windows the probing window returns wrong origin, by
                // locking it to the monitor's origin we ensure that we get the
                // proper dims.
                if let Some(monitor) = window.current_monitor() {
                    let mon_pos: winit::dpi::LogicalPosition<f64> =
                        monitor.position().to_logical(monitor.scale_factor());
                    let dy = mon_pos.y - area.position.y;
                    if dy > 0.0 {
                        area.position.y = mon_pos.y;
                        area.extent.height -= dy;
                    }
                }

                let elapsed = start.elapsed();
                log::info!(
                    "probe_available_screen_area: position=({:.1}, {:.1}), size={:.1}x{:.1} (took {:.1?})",
                    area.position.x,
                    area.position.y,
                    area.extent.width,
                    area.extent.height,
                    elapsed
                );
                break;
            }
        }

        if start.elapsed() >= timeout {
            let elapsed = start.elapsed();
            log::warn!(
                "probe_available_screen_area: area {:?} after {:.1?}",
                area,
                elapsed
            );

            // On macOS, if position never moved from monitor origin,
            // the WM didn't account for the menu bar. Apply 35px offset.
            #[cfg(target_os = "macos")]
            if (area.position.y - monitor_origin_y).abs() < 1.0 {
                area.position.y += 35.0;
                area.extent.height -= 35.0;
                log::info!("probe_available_screen_area: applied 35px macOS menu bar offset");
            }

            break;
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    area
}

/// Build a fallback `ScreenArea` from the monitor attached to the given window,
/// subtracting hardcoded OS chrome heights.
fn fallback_screen_area_from_monitor(window: &Window) -> ScreenArea {
    let Some(monitor) = window.current_monitor() else {
        return default_screen_area_from_hardcoded();
    };
    let logical_size: winit::dpi::LogicalSize<f64> =
        monitor.size().to_logical(monitor.scale_factor());
    let logical_pos: winit::dpi::LogicalPosition<f64> =
        monitor.position().to_logical(monitor.scale_factor());

    let os_chrome_height = if cfg!(target_os = "macos") {
        30.0
    } else if cfg!(target_os = "windows") {
        70.0
    } else {
        0.0
    };

    ScreenArea {
        position: Position {
            x: logical_pos.x,
            y: logical_pos.y + os_chrome_height,
        },
        extent: Extent {
            width: logical_size.width,
            height: logical_size.height - os_chrome_height,
        },
    }
}

/// Last-resort fallback when no monitor info is available at all.
fn default_screen_area_from_hardcoded() -> ScreenArea {
    let os_chrome_height = if cfg!(target_os = "macos") {
        30.0
    } else if cfg!(target_os = "windows") {
        70.0
    } else {
        0.0
    };

    ScreenArea {
        position: Position {
            x: 0.0,
            y: os_chrome_height,
        },
        extent: Extent {
            width: WindowConstant::DEFAULT_WIDTH,
            height: 900.0,
        },
    }
}

const REDRAW_INTERVAL: Duration = Duration::from_millis(1_000 / 10);
/// With faces in the window, their cameras move even when the screen doesn't: as often as the
/// faces window draws them while someone shares.
const FACES_REDRAW_INTERVAL: Duration = Duration::from_millis(1_000 / 15);
const ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(16);

pub enum RedrawCommand {
    ForceRedraw,
    /// Something on screen is animating, so the next frame can't wait for video or the slow timer.
    Animate,
    /// Whether the faces are in the window.
    Faces(bool),
    Stop,
}

fn run_redraw_loop(
    redraw_rx: std::sync::mpsc::Receiver<RedrawCommand>,
    redraw_in_progress: Arc<AtomicBool>,
    request_redraw: impl Fn(),
) {
    let mut idle = REDRAW_INTERVAL;
    let mut interval = idle;
    loop {
        match redraw_rx.recv_timeout(interval) {
            Ok(RedrawCommand::ForceRedraw) => {
                if !redraw_in_progress.load(Ordering::Acquire) {
                    request_redraw();
                }
            }
            Ok(RedrawCommand::Animate) => interval = ANIMATION_FRAME_INTERVAL,
            Ok(RedrawCommand::Faces(shown)) => {
                idle = if shown {
                    FACES_REDRAW_INTERVAL
                } else {
                    REDRAW_INTERVAL
                };
                interval = interval.min(idle);
            }
            Ok(RedrawCommand::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                request_redraw();
                interval = idle;
            }
        }
    }
}

fn spawn_redraw_thread(
    redraw_rx: std::sync::mpsc::Receiver<RedrawCommand>,
    redraw_in_progress: Arc<AtomicBool>,
    window: Arc<Window>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        run_redraw_loop(redraw_rx, redraw_in_progress, move || {
            window.request_redraw()
        })
    })
}
/// Dedicated renderer ID for the screensharing stream in YUV pipeline caches.
const SCREENSHARE_STREAM_ID: u64 = u64::MAX;

const ICON_COG: &[u8] = include_bytes!("../../resources/icons/cog.svg");
const ICON_FACES_POP_OUT: &[u8] = include_bytes!("../../resources/icons/faces-pop-out.svg");
const ICON_FACES_POP_IN: &[u8] = include_bytes!("../../resources/icons/faces-pop-in.svg");
const ICON_PENCIL_SVG: &[u8] = include_bytes!("../../resources/icons/pencil.svg");

/// Icon font codepoints for segmented control (from icons-font).
const ICON_REMOTE_CONTROL: char = '\u{F107}';
const ICON_PEN: char = '\u{F109}';
const ICON_CLICK_POINTER: char = '\u{F108}';
const ICON_PIN: char = '\u{F10B}';

// ── Segmented control buttons ────────────────────────────────────────────────
const SEGMENTED_BUTTONS: &[SegmentedButton] = &[
    SegmentedButton {
        id: "control",
        icon_char: ICON_REMOTE_CONTROL,
        description: Some("Remote control"),
    },
    SegmentedButton {
        id: "draw",
        icon_char: ICON_PEN,
        description: Some("Draw"),
    },
    SegmentedButton {
        id: "stamp",
        icon_char: ICON_PIN,
        description: Some("Stamp"),
    },
    SegmentedButton {
        id: "point",
        icon_char: ICON_CLICK_POINTER,
        description: Some("Click animation"),
    },
];

#[derive(Error, Debug)]
pub enum ScreensharingWindowError {
    #[error("Failed to create window")]
    WindowCreation,
    #[error("Failed to create wgpu surface")]
    SurfaceCreation,
    #[error("No suitable GPU adapter found")]
    AdapterRequest,
    #[error("Failed to request GPU device")]
    DeviceRequest,
}

// ── Iced messages ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ScreensharingMessage {
    CallControls(CallControlsMessage),
    TabSelected(&'static str),
    ToggleDropdown,
    DismissDropdown,
    DropdownItemClicked(usize),
    /// The faces go to a window of their own, or come back into this one.
    ToggleFacesOut,
    /// From something in the window that doesn't do anything here.
    Nothing,
}

// ── Input events to forward to room service ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenShareTab {
    Control,
    Draw,
    Stamp,
    Point,
}

#[derive(Debug)]
pub(crate) enum ScreenShareInputEvent {
    CursorMoved {
        x: f64,
        y: f64,
    },
    MouseClick(crate::room_service::MouseClickData),
    Scroll(crate::room_service::WheelDelta),
    KeyInput(crate::room_service::KeystrokeData),
    DrawStart {
        x: f64,
        y: f64,
        path_id: u64,
    },
    DrawAddPoint {
        x: f64,
        y: f64,
    },
    DrawEnd {
        x: f64,
        y: f64,
    },
    DrawClearAllPaths,
    DrawClearPaths(Vec<u64>),
    ClickAnimation {
        x: f64,
        y: f64,
    },
    Stamp(StampMessage),
    /// You pressed the left button on a stamp you just placed, or let it go.
    StampHold(bool),
    DrawingModeChanged(crate::room_service::DrawingMode),
    /// You picked up the stamp tool. Everyone hears this as drawing, so your ink stays up, but
    /// it's not your last mode: the next screen doesn't open with the pencil because of it.
    StampTabSelected(crate::room_service::DrawingMode),
    AddToClipboard {
        is_copy: bool,
    },
    PasteFromClipboard(Option<String>),
}

impl From<drawing_helpers::StrokeEvent> for ScreenShareInputEvent {
    fn from(event: drawing_helpers::StrokeEvent) -> Self {
        match event {
            drawing_helpers::StrokeEvent::Start { x, y, path_id } => {
                Self::DrawStart { x, y, path_id }
            }
            drawing_helpers::StrokeEvent::AddPoint { x, y } => Self::DrawAddPoint { x, y },
            drawing_helpers::StrokeEvent::End { x, y } => Self::DrawEnd { x, y },
            drawing_helpers::StrokeEvent::ClearPaths(path_ids) => Self::DrawClearPaths(path_ids),
        }
    }
}

// ── Application state for the screensharing UI ─────────────────────────────

struct ScreensharingState {
    active_tab: &'static str,
    /// Animation state for the segmented-control indicator slide.
    tab_anim: Option<SegmentedControlAnim>,
    /// Stream aspect ratio (width/height) — repurposed from img_aspect.
    img_aspect: f64,
    /// Last known stream dimensions for change detection.
    last_stream_width: u32,
    last_stream_height: u32,
    /// The stroke being drawn while the left mouse button is held in draw mode.
    local_stroke: drawing_helpers::LocalStroke,
    drawing_hint: drawing_helpers::DrawingHint,
    /// A note you've started typing over this screen, not yet stamped down: it follows the
    /// pointer, and a click puts it there.
    note: Option<String>,
    /// Where the pointer last was over this screen, which is where a note waits if it leaves.
    note_point: Option<(f64, f64)>,
    /// The press that stamped your note, whose release isn't for anything else.
    note_press: bool,
    /// You're holding your stamp down.
    stamp_held: bool,
    /// The call is captured, so holding a stamp down can hear you.
    capturing: bool,
    /// Last cursor position (percentage 0.0-1.0) inside participant area.
    last_draw_cursor: Option<(f64, f64)>,
    /// Whether the settings dropdown is open.
    dropdown_open: bool,
    /// When true, drawn strokes persist until right-click; otherwise they fade out.
    draw_persist: bool,
    /// Whether the sharer currently allows remote control input.
    remote_control_allowed: bool,
    app_veil_snapshot: crate::room_service::AppVeilSnapshot,
    /// True after the user manually resizes the window; suppresses auto-maximize.
    user_has_resized: bool,
    /// Multi-click detection state.
    last_click_count: u32,
    last_click_button: u32,
    last_click_time: StdInstant,
    last_click_x: f32,
    last_click_y: f32,
    sharer_name: String,
    /// Modifier key-down events buffered until we know whether they precede a
    /// clipboard shortcut (in which case they are discarded) or a regular key
    /// (in which case they are flushed first).
    queued_modifier_events: Vec<crate::room_service::KeystrokeData>,
}

impl Default for ScreensharingState {
    fn default() -> Self {
        Self {
            active_tab: SEGMENTED_BUTTONS[0].id,
            tab_anim: None,
            img_aspect: 16.0 / 9.0,
            last_stream_width: 0,
            last_stream_height: 0,
            local_stroke: Default::default(),
            drawing_hint: Default::default(),
            note: None,
            note_point: None,
            note_press: false,
            stamp_held: false,
            capturing: false,
            last_draw_cursor: None,
            dropdown_open: false,
            draw_persist: true,
            remote_control_allowed: true,
            app_veil_snapshot: Default::default(),
            user_has_resized: false,
            last_click_count: 0,
            last_click_button: 0,
            last_click_time: StdInstant::now(),
            last_click_x: 0.0,
            last_click_y: 0.0,
            sharer_name: "Screen".to_string(),
            queued_modifier_events: Vec::new(),
        }
    }
}

fn project_app_veil_rect(
    rect: &crate::room_service::NormalizedRect,
    bounds: Rectangle,
) -> Rectangle {
    Rectangle {
        x: rect.x * bounds.width,
        y: rect.y * bounds.height,
        width: rect.width * bounds.width,
        height: rect.height * bounds.height,
    }
}

fn app_veil_label_visible(rect: Rectangle) -> bool {
    rect.width >= 160.0 && rect.height >= 40.0
}

struct AppVeilOverlay<'a> {
    snapshot: &'a crate::room_service::AppVeilSnapshot,
}

impl<Message> canvas::Program<Message> for AppVeilOverlay<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        for window in &self.snapshot.windows {
            let rect = project_app_veil_rect(&window.frame, bounds);
            let path = canvas::Path::rounded_rectangle(
                Point::new(rect.x, rect.y),
                Size::new(rect.width, rect.height),
                10.0.into(),
            );
            for fragment in &window.visible_fragments {
                frame.with_clip(project_app_veil_rect(fragment, bounds), |frame| {
                    frame.fill(&path, ColorToken::Slate900.to_color());
                    if app_veil_label_visible(rect) {
                        frame.fill_text(canvas::Text {
                            content: "Hidden with App Veil".to_string(),
                            position: Point::new(
                                rect.x + rect.width / 2.0,
                                rect.y + rect.height / 2.0,
                            ),
                            color: Color::WHITE,
                            size: Pixels(14.0),
                            font: GEIST_MEDIUM,
                            align_x: alignment::Horizontal::Center.into(),
                            align_y: alignment::Vertical::Center,
                            ..Default::default()
                        });
                    }
                });
            }
        }

        vec![frame.into_geometry()]
    }
}

// ── Faces beside their screen ───────────────────────────────────────────────

/// Between their screen and the faces beside it.
const FACES_GAP: f32 = 8.0;
const BUBBLE_SIZE: f32 = 48.0;
const BUBBLE_GAP: f32 = 6.0;
/// In from the corner of their screen.
const BUBBLE_MARGIN: f32 = 10.0;
const MAX_BUBBLES: usize = 5;

/// Where things go in the window, in logical pixels: the picture of their screen, which keeps
/// the stream's shape however the window is shaped, and the faces beside it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ShareLayout {
    picture: Rectangle,
    faces: Option<Rectangle>,
}

fn share_layout(window: Size, stream_aspect: f32, faces: FacesInShared) -> ShareLayout {
    let content = Rectangle {
        x: WindowConstant::PADDING,
        y: WindowConstant::HEADER_HEIGHT,
        width: (window.width - 2.0 * WindowConstant::PADDING).max(1.0),
        height: (window.height - WindowConstant::HEADER_HEIGHT - WindowConstant::PADDING).max(1.0),
    };
    let (area, rail) = match faces {
        FacesInShared::Side => {
            let share = (content.height * FACES_SIDE_SHARE as f32).min(content.width / 2.0);
            (
                Rectangle {
                    width: content.width - share,
                    ..content
                },
                Some(Rectangle {
                    x: content.x + content.width - share + FACES_GAP,
                    width: share - FACES_GAP,
                    ..content
                }),
            )
        }
        FacesInShared::Bottom => {
            let share = (content.width * FACES_BOTTOM_SHARE as f32).min(content.height / 2.0);
            (
                Rectangle {
                    height: content.height - share,
                    ..content
                },
                Some(Rectangle {
                    y: content.y + content.height - share + FACES_GAP,
                    height: share - FACES_GAP,
                    ..content
                }),
            )
        }
        FacesInShared::Bubbles | FacesInShared::Off => (content, None),
    };
    ShareLayout {
        picture: letterbox(area, stream_aspect),
        faces: rail,
    }
}

/// The biggest picture of this shape in the area, in the middle of it.
fn letterbox(area: Rectangle, aspect: f32) -> Rectangle {
    if !(aspect.is_finite() && aspect > 0.0) {
        return area;
    }
    let (width, height) = if area.width / area.height > aspect {
        (area.height * aspect, area.height)
    } else {
        (area.width, area.width / aspect)
    };
    Rectangle {
        x: area.x + (area.width - width) / 2.0,
        y: area.y + (area.height - height) / 2.0,
        width,
        height,
    }
}

/// How many places there are for faces down the side, or along the bottom, and their size.
/// They fill the side's width (or the bottom's height), and get shorter (or narrower) to fit
/// more people, to a point: past that, the last place says how many more there are.
fn faces_tiles(rail: Size, along_bottom: bool, count: usize) -> (usize, Size) {
    let (across, along) = if along_bottom {
        (rail.height, rail.width)
    } else {
        (rail.width, rail.height)
    };
    // Each face's length along the rail, at its best and at its least.
    let (best, least) = if along_bottom {
        (across * 16.0 / 9.0, across)
    } else {
        (across * 3.0 / 4.0, across * 9.0 / 16.0)
    };
    let room = (((along + FACES_GAP) / (least + FACES_GAP)).floor() as usize).max(1);
    let places = count.clamp(1, room);
    let length = ((along - FACES_GAP * (places - 1) as f32) / places as f32).min(best);
    let size = if along_bottom {
        Size::new(length, across)
    } else {
        Size::new(across, length)
    };
    (places, size)
}

/// Who's in the faces beside their screen: everyone else by name, then you while your
/// camera's on.
fn faces_people(
    participants: &HashMap<String, ParticipantInfo>,
) -> Vec<(&String, &ParticipantInfo)> {
    let mut people: Vec<(&String, &ParticipantInfo)> = participants
        .iter()
        .filter(|(identity, _)| identity.as_str() != "local")
        .collect();
    people.sort_by(|a, b| a.1.name().cmp(b.1.name()));
    people.extend(
        participants
            .iter()
            .filter(|(identity, info)| identity.as_str() == "local" && info.camera_active()),
    );
    people
}

/// `content`, this size and at this place in the window.
fn placed<'a>(
    content: impl Into<iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer>>,
    rect: Rectangle,
) -> iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> {
    container(
        container(content)
            .width(Length::Fixed(rect.width.max(0.0)))
            .height(Length::Fixed(rect.height.max(0.0))),
    )
    .padding(Padding {
        top: rect.y,
        right: 0.0,
        bottom: 0.0,
        left: rect.x,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// How many more people there are than places for them.
fn more_faces<'a>(
    more: usize,
    size: Size,
) -> iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> {
    let radius = if size.width == size.height {
        size.width / 2.0
    } else {
        6.0
    };
    container(
        text(format!("+{more}"))
            .size(14)
            .font(GEIST_MEDIUM)
            .color(Color::WHITE),
    )
    .center_x(Length::Fixed(size.width))
    .center_y(Length::Fixed(size.height))
    .style(move |_theme: &Theme| container::Style {
        background: Some(Background::Color(ColorToken::Slate700.to_color())),
        border: Border {
            radius: radius.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}

/// The faces, down the side or along the bottom of their screen, or in bubbles over its corner.
fn faces_view<'a>(
    faces_in_shared: FacesInShared,
    layout: ShareLayout,
    participants: &Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    frame_ids: &mut HashMap<String, u64>,
    outer_skip: bool,
) -> iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> {
    let Ok(participants) = participants.read() else {
        return Space::new().into();
    };
    let people = faces_people(&participants);
    if people.is_empty() {
        return Space::new().into();
    }
    let mut skip = |identity: &String, info: &ParticipantInfo| {
        should_skip(identity, &info.camera_buffers(), outer_skip, frame_ids)
    };

    if faces_in_shared == FacesInShared::Bubbles {
        let picture = layout.picture;
        let room = ((picture.width - 2.0 * BUBBLE_MARGIN + BUBBLE_GAP) / (BUBBLE_SIZE + BUBBLE_GAP))
            .floor()
            .max(1.0) as usize;
        let places = people.len().min(room.min(MAX_BUBBLES));
        let shown = if people.len() > places {
            places - 1
        } else {
            places
        };
        let mut bubbles = row![].spacing(BUBBLE_GAP);
        for &(identity, info) in &people[..shown] {
            bubbles = bubbles.push(round_face(RoundFace {
                participant_id: identity_to_id(identity),
                name: info.name(),
                buffers: info.camera_buffers(),
                size: BUBBLE_SIZE,
                speaking: info.is_speaking(),
                muted: info.muted(),
                mirror: identity.as_str() == "local",
                skip_buffer: skip(identity, info),
            }));
        }
        if shown < people.len() {
            bubbles = bubbles.push(more_faces(
                people.len() - shown,
                Size::new(BUBBLE_SIZE, BUBBLE_SIZE),
            ));
        }
        let width = places as f32 * (BUBBLE_SIZE + BUBBLE_GAP) - BUBBLE_GAP;
        return placed(
            bubbles,
            Rectangle {
                x: picture.x + picture.width - BUBBLE_MARGIN - width,
                y: picture.y + BUBBLE_MARGIN,
                width,
                height: BUBBLE_SIZE,
            },
        );
    }

    let Some(rail) = layout.faces else {
        return Space::new().into();
    };
    let along_bottom = faces_in_shared == FacesInShared::Bottom;
    let (places, tile) = faces_tiles(rail.size(), along_bottom, people.len());
    let shown = if people.len() > places {
        places - 1
    } else {
        places
    };
    let mut tiles: Vec<iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer>> =
        Vec::with_capacity(places);
    for &(identity, info) in &people[..shown] {
        tiles.push(face_card(
            identity,
            info,
            (tile.width, tile.height),
            skip(identity, info),
            ScreensharingMessage::Nothing,
        ));
    }
    if shown < people.len() {
        tiles.push(more_faces(people.len() - shown, tile));
    }
    let line: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> = if along_bottom {
        container(iced::widget::Row::with_children(tiles).spacing(FACES_GAP))
            .center_x(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        container(iced::widget::Column::with_children(tiles).spacing(FACES_GAP))
            .center_y(Length::Fill)
            .width(Length::Fill)
            .into()
    };
    placed(line, rail)
}

/// Sends the faces to a window of their own, or brings them back.
fn faces_button<'a>(
    faces_in_shared: FacesInShared,
) -> iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> {
    let (icon, tooltip_text) = match faces_in_shared {
        FacesInShared::Off => (ICON_FACES_POP_IN, "Faces in this window"),
        _ => (ICON_FACES_POP_OUT, "Faces in their own window"),
    };
    let tooltip_content = container(text(tooltip_text).size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        });
    iced::widget::tooltip(
        dropdown_trigger_button(icon, false, ScreensharingMessage::ToggleFacesOut),
        tooltip_content,
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(4)
    .snap_within_viewport(true)
    .into()
}

// ── ScreensharingWindow ─────────────────────────────────────────────────────

pub struct ScreensharingWindow {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
    renderer: iced::Renderer,
    viewport: Viewport,
    cache: Option<Cache>,
    clipboard: Clipboard,
    cursor: mouse::Cursor,
    modifiers: ModifiersState,
    state: ScreensharingState,
    call_controls: CallControlsState,
    call_participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    /// Target size of a programmatic resize in flight (logical pixels).
    programmatic_resize_target: Option<(f64, f64)>,
    /// True when the mouse cursor is inside the participant image area.
    mouse_in_participant_area: bool,
    /// True when participant_in_control names the local participant (use OS cursor in control tab).
    local_participant_in_control: bool,
    screen_area: ScreenArea,
    screen_share_buffer: Arc<crate::livekit::video::VideoBufferManager>,
    participants_manager: ParticipantsManager,
    click_animation_renderer: ClickAnimationRenderer,
    last_rendered_frame_id: u64,
    redraw_in_progress: Arc<AtomicBool>,
    redraw_tx: std::sync::mpsc::Sender<RedrawCommand>,
    redraw_thread: Option<std::thread::JoinHandle<()>>,
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    event_loop_proxy: EventLoopProxy<crate::UserEvent>,
    aspect_ratio_enforcer: AspectRatioEnforcer,
    prefs: CallWindowPrefs,
    /// Where the faces go back to when they come back from a window of their own.
    faces_back: FacesInShared,
    /// The camera frame each face last drew, for the faces in this window.
    face_frame_ids: HashMap<String, u64>,
    /// Shown, until it's hidden or closed.
    open: bool,
    #[cfg(target_os = "macos")]
    ns_cursor_pointer: objc2::rc::Retained<objc2_app_kit::NSCursor>,
    #[cfg(target_os = "macos")]
    ns_cursor_pencil: objc2::rc::Retained<objc2_app_kit::NSCursor>,
    #[cfg(target_os = "macos")]
    ns_cursor_point: objc2::rc::Retained<objc2_app_kit::NSCursor>,
    #[cfg(not(target_os = "macos"))]
    custom_cursor_pointer: winit::window::CustomCursor,
    #[cfg(not(target_os = "macos"))]
    custom_cursor_pencil: winit::window::CustomCursor,
    #[cfg(not(target_os = "macos"))]
    custom_cursor_point: winit::window::CustomCursor,
}

pub struct ScreensharingParticipants {
    pub remote: Vec<(String, String, bool)>,
    pub state: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
}

impl ScreensharingParticipants {
    fn from_shared(
        state: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        sharer_identity: Option<&str>,
    ) -> Self {
        let remote = state
            .read()
            .map(|participants| {
                participants
                    .iter()
                    .filter(|(identity, _)| identity.as_str() != "local")
                    .map(|(identity, info)| {
                        (
                            identity.clone(),
                            info.name().to_string(),
                            sharer_identity == Some(identity.as_str()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { remote, state }
    }
}

pub struct ScreensharingWindowConfig {
    pub screen_share_buffer: Arc<crate::livekit::video::VideoBufferManager>,
    pub participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    pub sharer_identity: Option<String>,
    pub camera_active: bool,
    pub selected_camera_name: Option<String>,
    pub selected_mic_name: Option<String>,
    pub draw_persist: bool,
    pub last_mode: Option<socket_lib::StoredMode>,
    pub redraw_rx: std::sync::mpsc::Receiver<RedrawCommand>,
    pub redraw_tx: std::sync::mpsc::Sender<RedrawCommand>,
    pub event_loop_proxy: EventLoopProxy<crate::UserEvent>,
    pub prefs: CallWindowPrefs,
}

fn build_initial_state(
    participants: &[(String, String, bool)],
    draw_persist: bool,
    last_mode: &Option<socket_lib::StoredMode>,
) -> (ScreensharingState, ParticipantsManager) {
    let sharer_first_name = participants
        .iter()
        .find(|(_, _, is_screensharing)| *is_screensharing)
        .map(|(_, name, _)| name.as_str())
        .and_then(|n| n.split_whitespace().next())
        .unwrap_or("Screen")
        .to_string();

    let (initial_tab, initial_mode) = match last_mode {
        Some(socket_lib::StoredMode::Draw { .. }) => (
            "draw",
            crate::room_service::DrawingMode::Draw(crate::room_service::DrawSettings {
                permanent: draw_persist,
            }),
        ),
        Some(socket_lib::StoredMode::ClickAnimation) => {
            ("point", crate::room_service::DrawingMode::ClickAnimation)
        }
        _ => (
            SEGMENTED_BUTTONS[0].id,
            crate::room_service::DrawingMode::Disabled,
        ),
    };

    let mut participants_manager = ParticipantsManager::new();
    for (identity, name, _) in participants {
        if let Err(e) = participants_manager.add_participant(
            identity.clone(),
            name,
            false,
            crate::room_service::DrawingMode::Any,
        ) {
            log::warn!("build_initial_state: failed to add participant {identity}: {e:?}");
        }
    }
    if let Err(e) = participants_manager.add_participant(
        drawing_helpers::LOCAL_PARTICIPANT_IDENTITY.to_string(),
        drawing_helpers::LOCAL_PARTICIPANT_IDENTITY,
        true,
        crate::room_service::DrawingMode::Disabled,
    ) {
        log::warn!("build_initial_state: failed to add local participant: {e:?}");
    }
    participants_manager
        .set_drawing_mode(drawing_helpers::LOCAL_PARTICIPANT_IDENTITY, initial_mode);

    let mut state = ScreensharingState {
        sharer_name: sharer_first_name,
        draw_persist,
        active_tab: initial_tab,
        ..Default::default()
    };
    if initial_tab == "draw" {
        state.drawing_hint.show_once();
    }
    (state, participants_manager)
}

impl ScreensharingWindow {
    /// Create a new screensharing window with wgpu surface and iced renderer.
    pub fn new(
        context_manager: &ContextManager,
        event_loop: &ActiveEventLoop,
        config: ScreensharingWindowConfig,
    ) -> Result<Self, ScreensharingWindowError> {
        log::info!("ScreensharingWindow::new");

        let ScreensharingWindowConfig {
            screen_share_buffer,
            participants,
            sharer_identity,
            camera_active,
            selected_camera_name,
            selected_mic_name,
            draw_persist,
            last_mode,
            redraw_rx,
            redraw_tx,
            event_loop_proxy,
            prefs,
        } = config;

        let screen_area = probe_available_screen_area(event_loop);

        let window = event_loop
            .create_window(screensharing_window_attributes())
            .map_err(|e| {
                log::error!("ScreensharingWindow: failed to create window: {e:?}");
                ScreensharingWindowError::WindowCreation
            })?;
        let window = Arc::new(window);
        // Bring to front when window is created
        window.focus_window();

        // ── wgpu setup ───────────────────────────────────────────────────
        let surface_info = context_manager
            .create_screensharing_surface(&window)
            .map_err(|e| match e {
                GraphicsWindowContextError::SurfaceCreation => {
                    ScreensharingWindowError::SurfaceCreation
                }
                GraphicsWindowContextError::AdapterRequest => {
                    ScreensharingWindowError::AdapterRequest
                }
                GraphicsWindowContextError::DeviceRequest => {
                    ScreensharingWindowError::DeviceRequest
                }
            })?;
        let device = context_manager.screensharing_context.device.clone();
        let format = surface_info.format;
        let alpha_mode = surface_info.alpha_mode;
        log::info!("ScreensharingWindow: selected alpha_mode: {:?}", alpha_mode);
        let surface = surface_info.surface;
        let physical_size = window.inner_size();

        // ── Iced renderer with Geist fonts ───────────────────────────────
        let wgpu_renderer = iced_wgpu::Renderer::new(
            context_manager.screensharing_context.engine.clone(),
            GEIST_REGULAR,
            Pixels::from(16),
        );

        // Load Geist font data into the global iced font system
        fonts_mod::load_fonts();

        let renderer = iced::Renderer::Primary(wgpu_renderer);

        let viewport = Viewport::with_physical_size(
            Size::new(physical_size.width.max(1), physical_size.height.max(1)),
            window.scale_factor() as f32,
        );
        let mut clipboard = Clipboard::connect(window.clone());
        clipboard.write(Kind::Standard, String::new());

        #[cfg(target_os = "macos")]
        super::vibrancy::apply_macos_vibrancy(&window, 8.0);

        let mut aspect_ratio_enforcer = AspectRatioEnforcer::new(&window);
        aspect_ratio_enforcer
            .set_aspect_ratio(&window, layout_aspect(16.0 / 9.0, prefs.faces_in_shared));

        // Create custom cursors for the participant area.
        #[cfg(target_os = "macos")]
        let (ns_cursor_pointer, ns_cursor_pencil, ns_cursor_point) = {
            let px = (drawing_helpers::CURSOR_LOGICAL_SIZE * 4.0).round() as u32;
            let (pointer_rgba, pw, ph) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_POINTER, px);
            let (pencil_rgba, ew, eh) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_PENCIL, px);
            let (point_rgba, pt_w, pt_h) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_POINT, px);
            let pointer = drawing_helpers::create_macos_cursor(
                &pointer_rgba,
                pw,
                ph,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                3.0,
                2.0,
            );
            let pencil = drawing_helpers::create_macos_cursor(
                &pencil_rgba,
                ew,
                eh,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                2.0,
                29.0,
            );
            let point = drawing_helpers::create_macos_cursor(
                &point_rgba,
                pt_w,
                pt_h,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                drawing_helpers::CURSOR_LOGICAL_SIZE,
                2.0,
                4.0,
            );
            (pointer, pencil, point)
        };

        #[cfg(not(target_os = "macos"))]
        let (custom_cursor_pointer, custom_cursor_pencil, custom_cursor_point) = {
            let point_cursor_hotspot = (2.0, 4.0);
            let px = drawing_helpers::CURSOR_LOGICAL_SIZE as u32;
            let (pointer_rgba, pw, ph) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_POINTER, px);
            let (pencil_rgba, ew, eh) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_PENCIL, px);
            let (point_rgba, pt_w, pt_h) =
                drawing_helpers::rasterize_svg_to_rgba(drawing_helpers::CURSOR_ICON_POINT, px);
            let pointer = event_loop.create_custom_cursor(
                winit::window::CustomCursor::from_rgba(pointer_rgba, pw as u16, ph as u16, 3, 2)
                    .expect("create pointer cursor"),
            );
            let pencil = event_loop.create_custom_cursor(
                winit::window::CustomCursor::from_rgba(pencil_rgba, ew as u16, eh as u16, 2, 29)
                    .expect("create pencil cursor"),
            );
            let point = event_loop.create_custom_cursor(
                winit::window::CustomCursor::from_rgba(
                    point_rgba,
                    pt_w as u16,
                    pt_h as u16,
                    point_cursor_hotspot.0 as u16,
                    point_cursor_hotspot.1 as u16,
                )
                .expect("create point-mode cursor"),
            );
            (pointer, pencil, point)
        };

        let ScreensharingParticipants {
            remote,
            state: call_participants,
        } = ScreensharingParticipants::from_shared(participants, sharer_identity.as_deref());
        let (initial_state, participants_manager) =
            build_initial_state(&remote, draw_persist, &last_mode);
        let call_controls =
            CallControlsState::new(camera_active, selected_camera_name, selected_mic_name);
        let redraw_in_progress = Arc::new(AtomicBool::new(false));
        let redraw_thread = spawn_redraw_thread(
            redraw_rx,
            Arc::clone(&redraw_in_progress),
            Arc::clone(&window),
        );
        let s = Self {
            window,
            surface,
            device,
            format,
            alpha_mode,
            renderer,
            viewport,
            cache: Some(Cache::default()),
            clipboard,
            cursor: mouse::Cursor::Unavailable,
            modifiers: ModifiersState::default(),
            state: initial_state,
            call_controls,
            call_participants,
            screen_area,
            programmatic_resize_target: None,
            mouse_in_participant_area: false,
            local_participant_in_control: false,
            screen_share_buffer,
            participants_manager,
            click_animation_renderer: ClickAnimationRenderer::new(clock::default_clock()),
            last_rendered_frame_id: 0,
            redraw_in_progress,
            redraw_tx,
            redraw_thread: Some(redraw_thread),
            event_loop_proxy,
            aspect_ratio_enforcer,
            prefs,
            faces_back: match prefs.faces_in_shared {
                FacesInShared::Off => FacesInShared::Side,
                faces => faces,
            },
            face_frame_ids: HashMap::new(),
            open: true,
            #[cfg(target_os = "macos")]
            ns_cursor_pointer,
            #[cfg(target_os = "macos")]
            ns_cursor_pencil,
            #[cfg(target_os = "macos")]
            ns_cursor_point,
            #[cfg(not(target_os = "macos"))]
            custom_cursor_pointer,
            #[cfg(not(target_os = "macos"))]
            custom_cursor_pencil,
            #[cfg(not(target_os = "macos"))]
            custom_cursor_point,
        };
        s.update_cursor();
        let _ = s.redraw_tx.send(RedrawCommand::Faces(s.shows_faces()));
        Ok(s)
    }

    /// Recreates the context's render engine and rebuilds this window's renderer on it.
    ///
    /// Called on call end to free the engine's GPU memory (MSAA buffer + texture atlas)
    /// while the window itself is reused across calls. The context's engine clone and this
    /// window's renderer hold the only two Arc clones; resetting both drops the old engine.
    pub fn reset_renderer(&mut self, context: &mut GraphicsWindowContext) {
        context.reset_engine(self.format);
        self.renderer = iced::Renderer::Primary(iced_wgpu::Renderer::new(
            context.engine.clone(),
            GEIST_REGULAR,
            Pixels::from(16),
        ));
    }

    /// The winit `WindowId` for event routing.
    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible().unwrap_or(true)
    }

    /// Where the window is on screen, for reactions to come from.
    pub fn frame(&self) -> Option<[f64; 4]> {
        crate::reactions::window_frame(&self.window)
    }

    /// Shows the window. `bring_to_you` brings it from another screen or Space, if it's there.
    pub fn focus_window(&mut self, bring_to_you: bool) {
        if bring_to_you {
            crate::window_manager::bring_to_you(&self.window);
        }
        crate::window_manager::ensure_on_screen(&self.window);
        self.screen_area = fallback_screen_area_from_monitor(&self.window);
        if self.window.is_visible() == Some(false) {
            self.window.set_visible(true);
        }
        self.open = true;
        self.window.focus_window();
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.window.set_visible(false);
    }

    /// Shown, and not hidden or closed since, wherever it is.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the faces are in this window, rather than one of their own.
    pub fn shows_faces(&self) -> bool {
        self.prefs.faces_in_shared != FacesInShared::Off
    }

    pub fn set_prefs(&mut self, prefs: CallWindowPrefs) {
        let before = self.prefs.faces_in_shared;
        self.prefs = prefs;
        if prefs.faces_in_shared != FacesInShared::Off {
            self.faces_back = prefs.faces_in_shared;
        }
        if prefs.faces_in_shared != before {
            log::info!(
                "ScreensharingWindow: faces {before:?} -> {:?}",
                prefs.faces_in_shared
            );
            self.face_frame_ids.clear();
            let _ = self
                .redraw_tx
                .send(RedrawCommand::Faces(self.shows_faces()));
            self.fit_to_layout(before);
            self.window.request_redraw();
        }
    }

    /// The faces to a window of their own, or back here, remembered for next time.
    fn toggle_faces_out(&mut self) {
        let mut prefs = self.prefs;
        prefs.faces_in_shared = match prefs.faces_in_shared {
            FacesInShared::Off => self.faces_back,
            _ => FacesInShared::Off,
        };
        self.set_prefs(prefs);
        let _ = self
            .event_loop_proxy
            .send_event(crate::UserEvent::CallWindowPrefsChanged(prefs));
    }

    /// The shape the window's content keeps: their screen, and the faces where they go.
    fn layout_aspect(&self) -> f64 {
        layout_aspect(self.state.img_aspect, self.prefs.faces_in_shared)
    }

    fn layout(&self) -> ShareLayout {
        share_layout(
            self.viewport.logical_size(),
            self.state.img_aspect as f32,
            self.prefs.faces_in_shared,
        )
    }

    /// Makes room for the faces where they've gone, keeping their screen the size it was where
    /// it can: the same height with the faces down the side, the same width along the bottom.
    fn fit_to_layout(&mut self, before: FacesInShared) {
        let aspect = self.layout_aspect();
        self.aspect_ratio_enforcer
            .set_aspect_ratio(&self.window, aspect);
        if self.state.last_stream_width == 0 || crate::window_manager::is_full_screen(&self.window)
        {
            // Full screen, it stays full screen, and the screen fits inside.
            return;
        }
        let current: winit::dpi::LogicalSize<f64> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        let content_w = current.width - WindowConstant::SKELETON_W;
        let content_h = current.height - WindowConstant::SKELETON_H;
        let keep_width =
            before == FacesInShared::Bottom || self.prefs.faces_in_shared == FacesInShared::Bottom;
        let (mut width, mut height) = if keep_width {
            (
                current.width,
                content_w / aspect + WindowConstant::SKELETON_H,
            )
        } else {
            (
                content_h * aspect + WindowConstant::SKELETON_W,
                current.height,
            )
        };
        let area = self.screen_area.extent;
        if width > area.width || height > area.height {
            if let Some(fits) = calculate_max_window_size(area, aspect) {
                (width, height) = fits;
            }
        }
        // Grown past the edge of the screen, it comes back in.
        if let Ok(position) = self.window.outer_position() {
            let position: winit::dpi::LogicalPosition<f64> =
                position.to_logical(self.window.scale_factor());
            let right = self.screen_area.position.x + area.width;
            let bottom = self.screen_area.position.y + area.height;
            let x = position
                .x
                .min(right - width)
                .max(self.screen_area.position.x);
            let y = position
                .y
                .min(bottom - height)
                .max(self.screen_area.position.y);
            if x != position.x || y != position.y {
                self.window
                    .set_outer_position(winit::dpi::LogicalPosition::new(x, y));
            }
        }
        self.programmatic_resize_target = Some((width, height));
        let _ = self
            .window
            .request_inner_size(winit::dpi::LogicalSize::new(width, height));
    }

    pub fn add_participant(
        &mut self,
        identity: String,
        name: &str,
        auto_clear: bool,
    ) -> Result<(), ParticipantError> {
        self.participants_manager.add_participant(
            identity,
            name,
            auto_clear,
            crate::room_service::DrawingMode::Disabled,
        )
    }

    pub fn remove_participant(&mut self, identity: &str) {
        self.participants_manager.remove_participant(identity);
    }

    pub fn set_cursor_position(&mut self, identity: &str, position: Option<Position>) {
        let was_animating = self.participants_manager.is_cursor_animating();
        self.participants_manager
            .set_cursor_position(identity, position);
        // Once one frame is drawn the rest follow, but on a still screen the next could be
        // 100 ms away.
        if !was_animating && self.participants_manager.is_cursor_animating() {
            let _ = self.redraw_tx.send(RedrawCommand::ForceRedraw);
        }
    }

    pub fn set_call_people(&mut self, local_identity: Option<&str>, people: &[String]) {
        self.participants_manager
            .set_call_people(local_identity, people);
    }

    /// Returns whether this redraws a path that was just cleared, so the pen hasn't moved here.
    pub fn draw_start(&mut self, identity: &str, point: Position, path_id: u64) -> bool {
        let redrawn = self
            .participants_manager
            .draw_start(identity, point, path_id);
        self.window.request_redraw();
        redrawn
    }

    pub fn draw_add_point(&mut self, identity: &str, point: Position) {
        self.participants_manager.draw_add_point(identity, point);
        self.window.request_redraw();
    }

    pub fn draw_end(&mut self, identity: &str, point: Position) {
        self.participants_manager.draw_end(identity, point);
        self.window.request_redraw();
    }

    pub fn draw_clear_path(&mut self, identity: &str, path_id: u64) {
        self.participants_manager.draw_clear_path(identity, path_id);
        self.window.request_redraw();
    }

    pub fn draw_clear_all_paths(&mut self, identity: &str) {
        self.participants_manager.draw_clear_all_paths(identity);
        self.window.request_redraw();
    }

    pub fn set_drawing_mode(&mut self, identity: &str, mode: crate::room_service::DrawingMode) {
        let cursor_mode = match &mode {
            crate::room_service::DrawingMode::Draw(_) => CursorMode::Pencil,
            crate::room_service::DrawingMode::ClickAnimation => CursorMode::Pointer,
            crate::room_service::DrawingMode::Disabled | crate::room_service::DrawingMode::Any => {
                CursorMode::Normal
            }
        };
        self.participants_manager
            .set_cursor_mode(identity, cursor_mode);
        self.participants_manager.set_drawing_mode(identity, mode);
    }

    pub fn drawing_mode(&self) -> crate::room_service::DrawingMode {
        match self.state.active_tab {
            // Stamping is drawing to everyone else, so your ink stays up.
            "draw" | "stamp" => {
                crate::room_service::DrawingMode::Draw(crate::room_service::DrawSettings {
                    permanent: self.state.draw_persist,
                })
            }
            "point" => crate::room_service::DrawingMode::ClickAnimation,
            _ => crate::room_service::DrawingMode::Disabled,
        }
    }

    pub fn trigger_click_animation(&mut self, position: Position) {
        self.click_animation_renderer
            .enable_click_animation(position);
    }

    /// A stamp message from someone else in the call.
    pub fn receive_stamp(&mut self, sender: &str, message: StampMessage) {
        self.participants_manager.receive_stamp(sender, message);
        self.window.request_redraw();
    }

    /// Does what you asked with your stamps, and returns what everyone else needs to hear.
    fn stamp_input(&mut self, input: StampInput) -> Vec<ScreenShareInputEvent> {
        self.stamp_messages(input)
            .into_iter()
            .map(ScreenShareInputEvent::Stamp)
            .collect()
    }

    /// The same, for what's been heard for a stamp you held down.
    pub fn stamp_messages(&mut self, input: StampInput) -> Vec<StampMessage> {
        let messages = self.participants_manager.stamp_input(input);
        self.window.request_redraw();
        messages
    }

    /// You've a stamp taking words here.
    pub fn is_stamp_typing(&self) -> bool {
        self.participants_manager.is_stamp_typing()
    }

    /// You've a stamp hearing you here.
    pub fn is_stamp_hearing(&self) -> bool {
        self.participants_manager.is_stamp_hearing()
    }

    pub fn set_capturing(&mut self, capturing: bool) {
        self.state.capturing = capturing;
        self.window.request_redraw();
    }

    /// The window's going away: nothing's being typed or held down on it any more.
    pub fn drop_note(&mut self) {
        self.state.stamp_held = false;
        self.forget_note();
    }

    /// Your keys and clicks are for their screen from here on, so a note can't keep them.
    fn forget_note(&mut self) {
        self.state.note = None;
        self.state.note_press = false;
        self.note_changed();
    }

    fn note_changed(&mut self) {
        self.sync_note_ghost();
        self.update_cursor();
    }

    fn sync_note_ghost(&mut self) {
        let note = self.state.note.as_deref().zip(self.state.note_point);
        self.participants_manager
            .set_note_ghost(note.map(|(text, point)| (point, text)));
        self.window.request_redraw();
    }

    /// Puts your typed note down at `point` as a finished stamp.
    fn stamp_note(&mut self, point: (f64, f64)) -> Vec<ScreenShareInputEvent> {
        let Some(text) = self.state.note.take() else {
            return Vec::new();
        };
        self.note_changed();
        self.stamp_input(StampInput::Note {
            x: point.0,
            y: point.1,
            text,
        })
    }

    /// You let go of your stamp, if you were holding it.
    fn release_stamp(&mut self) -> Option<ScreenShareInputEvent> {
        if !std::mem::take(&mut self.state.stamp_held) {
            return None;
        }
        self.window.request_redraw();
        Some(ScreenShareInputEvent::StampHold(false))
    }

    pub fn set_remote_control_allowed(&mut self, allowed: bool) {
        self.state.remote_control_allowed = allowed;
        if allowed && self.state.active_tab == "control" {
            self.forget_note();
        }
    }

    pub fn set_camera_active(&mut self, active: bool, device_name: Option<String>) {
        self.call_controls.set_camera_active(active, device_name);
    }

    pub fn set_selected_mic_name(&mut self, name: Option<String>) {
        self.call_controls.set_selected_mic_name(name);
    }

    pub fn set_app_veil_snapshot(&mut self, snapshot: crate::room_service::AppVeilSnapshot) {
        self.state.app_veil_snapshot = snapshot;
        self.window.request_redraw();
    }

    /// Update the window for a new sharer: refresh the display name and swap
    /// the redraw channel so the newly spawned `process_video_stream` can
    /// drive redraws.
    pub fn update_window_with_new_sharer(
        &mut self,
        screen_share_buffer: Arc<crate::livekit::video::VideoBufferManager>,
        participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        sharer_identity: Option<String>,
        draw_persist: bool,
        last_mode: Option<socket_lib::StoredMode>,
        (new_rx, new_tx): (
            std::sync::mpsc::Receiver<RedrawCommand>,
            std::sync::mpsc::Sender<RedrawCommand>,
        ),
    ) {
        // A fresh buffer Arc is created per room/stream — point the window at it.
        self.screen_share_buffer = screen_share_buffer;
        let ScreensharingParticipants { remote, state } =
            ScreensharingParticipants::from_shared(participants, sharer_identity.as_deref());
        self.call_participants = state;

        let (state, participants_manager) = build_initial_state(&remote, draw_persist, &last_mode);
        let capturing = self.state.capturing;
        self.state = state;
        self.state.capturing = capturing;
        self.participants_manager = participants_manager;

        // Window-level state.
        self.call_controls.dismiss_dropdowns();
        self.local_participant_in_control = false;
        self.programmatic_resize_target = None;
        self.mouse_in_participant_area = false;
        self.last_rendered_frame_id = 0;

        self.face_frame_ids.clear();

        // Respawn redraw thread on the new stream's channel.
        self.stop_redraw_thread();
        self.redraw_thread = Some(spawn_redraw_thread(
            new_rx,
            Arc::clone(&self.redraw_in_progress),
            Arc::clone(&self.window),
        ));
        self.redraw_tx = new_tx;
        let _ = self
            .redraw_tx
            .send(RedrawCommand::Faces(self.shows_faces()));
    }

    /// Update local control ownership and refresh cursor. When true, control tab uses OS cursor.
    pub fn set_local_participant_in_control(&mut self, in_control: bool) {
        self.local_participant_in_control = in_control;
        self.update_cursor();
    }

    /// Compute multi-click count using the same logic as Chromium.
    /// Coordinates are in logical pixels so the distance threshold is
    /// resolution-independent.
    fn get_mouse_click_count(&self, x: f32, y: f32, button: u32) -> u32 {
        const DOUBLE_CLICK_TIME_MS: u64 = 500;
        const DOUBLE_CLICK_RANGE: f32 = 4.0;

        let prev = &self.state;
        if prev.last_click_count == 0 {
            return 1;
        }
        if prev.last_click_time.elapsed().as_millis() as u64 > DOUBLE_CLICK_TIME_MS {
            return 1;
        }
        if (x - prev.last_click_x).abs() > DOUBLE_CLICK_RANGE / 2.0 {
            return 1;
        }
        if (y - prev.last_click_y).abs() > DOUBLE_CLICK_RANGE / 2.0 {
            return 1;
        }
        if prev.last_click_button != button {
            return 1;
        }
        // On macOS and Windows keep counting; elsewhere cap at 3.
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        if prev.last_click_count >= 3 {
            return 1;
        }
        prev.last_click_count + 1
    }

    /// Update window cursor based on active tab, mouse position, and local control ownership.
    ///
    /// - Outside participant area → always the OS default cursor.
    /// - Inside participant area + `draw` tab → pencil cursor.
    /// - Inside participant area + `point` tab → click-pointer cursor (`local-participant-pointer.svg`).
    /// - Inside participant area + `control` tab + local in control → OS default cursor.
    /// - Inside participant area + other cases → hand cursor (`local-participant-cursor.svg`).
    fn update_cursor(&self) {
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSCursor;
            if !self.mouse_in_participant_area {
                NSCursor::arrowCursor().set();
            } else if self.state.note.is_some() {
                NSCursor::crosshairCursor().set();
            } else if self.state.active_tab == "draw" {
                self.ns_cursor_pencil.set();
            } else if self.state.active_tab == "stamp" {
                NSCursor::crosshairCursor().set();
            } else if self.state.active_tab == "point" {
                self.ns_cursor_point.set();
            } else if self.state.active_tab == "control" && self.local_participant_in_control {
                NSCursor::arrowCursor().set();
            } else {
                self.ns_cursor_pointer.set();
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if !self.mouse_in_participant_area {
                self.window
                    .set_cursor(winit::window::Cursor::Icon(CursorIcon::Default));
            } else if self.state.note.is_some() {
                self.window
                    .set_cursor(winit::window::Cursor::Icon(CursorIcon::Crosshair));
            } else if self.state.active_tab == "draw" {
                self.window.set_cursor(winit::window::Cursor::Custom(
                    self.custom_cursor_pencil.clone(),
                ));
            } else if self.state.active_tab == "stamp" {
                self.window
                    .set_cursor(winit::window::Cursor::Icon(CursorIcon::Crosshair));
            } else if self.state.active_tab == "point" {
                self.window.set_cursor(winit::window::Cursor::Custom(
                    self.custom_cursor_point.clone(),
                ));
            } else if self.state.active_tab == "control" && self.local_participant_in_control {
                self.window
                    .set_cursor(winit::window::Cursor::Icon(CursorIcon::Default));
            } else {
                self.window.set_cursor(winit::window::Cursor::Custom(
                    self.custom_cursor_pointer.clone(),
                ));
            }
        }
    }

    /// Bounding rectangle of the participant image area in logical pixels.
    fn participant_image_rect(&self) -> Rectangle {
        self.layout().picture
    }

    /// Ends the local stroke where the cursor was last seen inside the participant area, for when
    /// the cursor can't keep drawing: it left the area or the window lost focus.
    fn end_stroke_at_last_cursor(&mut self) -> Vec<ScreenShareInputEvent> {
        if !self.state.local_stroke.is_active() {
            return Vec::new();
        }
        self.window.request_redraw();
        let Some((x, y)) = self.state.last_draw_cursor else {
            self.state.local_stroke.cancel();
            return Vec::new();
        };
        self.state
            .local_stroke
            .end(
                &mut self.participants_manager,
                Position { x, y },
                self.modifiers.shift_key(),
            )
            .into_iter()
            .map(ScreenShareInputEvent::from)
            .collect()
    }

    /// Handle a winit `WindowEvent` — forward to iced and manage resize / redraw.
    /// Returns input events to be forwarded to room service.
    pub fn handle_window_event(&mut self, event: WindowEvent) -> Vec<ScreenShareInputEvent> {
        // Participant area event gating: update cursor-in-rect state and capture input
        // when inside the participant image area. All events still flow to iced.
        let scale_factor = self.window.scale_factor() as f32;
        let rect = self.participant_image_rect();
        let mut input_events: Vec<ScreenShareInputEvent> = Vec::new();

        let is_redraw = matches!(event, WindowEvent::RedrawRequested);

        match &event {
            WindowEvent::CursorMoved { position, .. } => {
                let logical_x = (position.x / scale_factor as f64) as f32;
                let logical_y = (position.y / scale_factor as f64) as f32;
                let inside = !self.state.dropdown_open
                    && !self.call_controls.has_open_dropdown()
                    && logical_x >= rect.x
                    && logical_x < rect.x + rect.width
                    && logical_y >= rect.y
                    && logical_y < rect.y + rect.height;
                let was_inside = self.mouse_in_participant_area;
                self.mouse_in_participant_area = inside;
                if was_inside != inside {
                    self.update_cursor();
                }
                if inside {
                    let pct_x = ((logical_x - rect.x) / rect.width) as f64;
                    let pct_y = ((logical_y - rect.y) / rect.height) as f64;
                    self.state.last_draw_cursor = Some((pct_x, pct_y));
                    self.state.note_point = Some((pct_x, pct_y));

                    if self.state.local_stroke.is_active() {
                        let stroke_events = self.state.local_stroke.extend(
                            &mut self.participants_manager,
                            Position { x: pct_x, y: pct_y },
                            self.modifiers.shift_key(),
                        );
                        input_events
                            .extend(stroke_events.into_iter().map(ScreenShareInputEvent::from));
                        self.window.request_redraw();
                    } else {
                        input_events
                            .push(ScreenShareInputEvent::CursorMoved { x: pct_x, y: pct_y });
                    }

                    if !was_inside {
                        log::debug!(
                            "ScreensharingWindow: cursor entered participant area at ({:.3}, {:.3})",
                            pct_x,
                            pct_y
                        );
                    }
                } else if !inside && was_inside {
                    input_events.extend(self.end_stroke_at_last_cursor());
                    self.state.last_draw_cursor = None;
                    self.state.queued_modifier_events.clear();
                    log::debug!("ScreensharingWindow: cursor left participant area");
                }
                if self.state.note.is_some() {
                    self.sync_note_ghost();
                }
            }
            // Reset when cursor leaves the window — winit skips the final
            // CursorMoved when the cursor exits quickly.
            WindowEvent::CursorLeft { .. } => {
                if self.mouse_in_participant_area {
                    input_events.extend(self.end_stroke_at_last_cursor());
                    self.state.last_draw_cursor = None;
                    self.state.queued_modifier_events.clear();
                    self.mouse_in_participant_area = false;
                    self.update_cursor();
                    log::debug!("ScreensharingWindow: cursor left participant area (CursorLeft)");
                }
            }
            // Also reset when the window loses focus so stale state doesn't
            // linger while the user interacts with another window.
            WindowEvent::Focused(false) => {
                input_events.extend(self.release_stamp());
                // Its release may go elsewhere now, and the next one here isn't its.
                self.state.note_press = false;
                if self.mouse_in_participant_area {
                    input_events.extend(self.end_stroke_at_last_cursor());
                    self.state.last_draw_cursor = None;
                    self.state.queued_modifier_events.clear();
                    self.mouse_in_participant_area = false;
                    self.update_cursor();
                    log::debug!(
                        "ScreensharingWindow: cursor left participant area (window unfocused)"
                    );
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let left = matches!(button, winit::event::MouseButton::Left);
                let left_up = left && !state.is_pressed();
                if left_up {
                    // Wherever the pointer's got to by now.
                    input_events.extend(self.release_stamp());
                }
                let note_click_over = left_up && std::mem::take(&mut self.state.note_press);
                if self.mouse_in_participant_area && !note_click_over {
                    let (pct_x, pct_y) = match &self.cursor {
                        mouse::Cursor::Available(pos) => (
                            ((pos.x - rect.x) / rect.width) as f64,
                            ((pos.y - rect.y) / rect.height) as f64,
                        ),
                        _ => (0.0, 0.0),
                    };

                    let down = state.is_pressed();

                    if left && down && self.state.note.is_some() {
                        self.state.note_press = true;
                        input_events.extend(self.stamp_note((pct_x, pct_y)));
                    } else if self.state.active_tab == "draw" {
                        match button {
                            winit::event::MouseButton::Left => {
                                let point = Position { x: pct_x, y: pct_y };
                                if down {
                                    let start = self
                                        .state
                                        .local_stroke
                                        .begin(&mut self.participants_manager, point);
                                    input_events.push(start.into());
                                } else {
                                    let stroke_events = self.state.local_stroke.end(
                                        &mut self.participants_manager,
                                        point,
                                        self.modifiers.shift_key(),
                                    );
                                    input_events.extend(
                                        stroke_events.into_iter().map(ScreenShareInputEvent::from),
                                    );
                                }
                                self.window.request_redraw();
                            }
                            winit::event::MouseButton::Right if down => {
                                self.state.local_stroke.cancel();
                                self.participants_manager.draw_clear_all_paths(
                                    drawing_helpers::LOCAL_PARTICIPANT_IDENTITY,
                                );
                                input_events.push(ScreenShareInputEvent::DrawClearAllPaths);
                                self.window.request_redraw();
                            }
                            _ => {}
                        }
                    } else if self.state.active_tab == "stamp" {
                        let input = match button {
                            winit::event::MouseButton::Left if down => {
                                Some(StampInput::Place { x: pct_x, y: pct_y })
                            }
                            winit::event::MouseButton::Right if down => {
                                self.participants_manager.stamp_right_click((pct_x, pct_y))
                            }
                            _ => None,
                        };
                        let pressed = left && down;
                        if let Some(input) = input {
                            input_events.extend(self.stamp_input(input));
                        }
                        if pressed {
                            self.state.stamp_held = true;
                            input_events.push(ScreenShareInputEvent::StampHold(true));
                        }
                    } else if self.state.active_tab == "point" {
                        if matches!(button, winit::event::MouseButton::Left) && down {
                            self.click_animation_renderer.enable_click_animation(
                                crate::utils::geometry::Position { x: pct_x, y: pct_y },
                            );
                            input_events
                                .push(ScreenShareInputEvent::ClickAnimation { x: pct_x, y: pct_y });
                        }
                    } else if self.state.remote_control_allowed {
                        // control mode
                        // Use the Web/MDN MouseEvent.button convention
                        // (the receiving side interprets these values):
                        // 0=left, 1=middle, 2=right, 3=back, 4=forward
                        let button_num = match button {
                            winit::event::MouseButton::Left => 0,
                            winit::event::MouseButton::Middle => 1,
                            winit::event::MouseButton::Right => 2,
                            winit::event::MouseButton::Back => 3,
                            winit::event::MouseButton::Forward => 4,
                            winit::event::MouseButton::Other(n) => *n as u32,
                        };

                        // Compute multi-click count on press using logical
                        // pixel positions (same approach as Chromium).
                        let logical_x = match &self.cursor {
                            mouse::Cursor::Available(pos) => pos.x,
                            _ => 0.0,
                        };
                        let logical_y = match &self.cursor {
                            mouse::Cursor::Available(pos) => pos.y,
                            _ => 0.0,
                        };
                        let clicks = if down {
                            let c = self.get_mouse_click_count(logical_x, logical_y, button_num);
                            self.state.last_click_count = c;
                            self.state.last_click_button = button_num;
                            self.state.last_click_time = StdInstant::now();
                            self.state.last_click_x = logical_x;
                            self.state.last_click_y = logical_y;
                            c
                        } else {
                            self.state.last_click_count
                        };

                        input_events.push(ScreenShareInputEvent::MouseClick(
                            crate::room_service::MouseClickData {
                                x: pct_x,
                                y: pct_y,
                                button: button_num,
                                clicks,
                                down,
                                shift: self.modifiers.shift_key(),
                                meta: self.modifiers.super_key(),
                                ctrl: self.modifiers.control_key(),
                                alt: self.modifiers.alt_key(),
                            },
                        ));
                    }

                    log::debug!(
                        "ScreensharingWindow: [participant_area] mouse button {:?} {:?} at ({:.3}, {:.3})",
                        button,
                        state,
                        pct_x,
                        pct_y
                    );
                } else {
                    log::debug!(
                        "ScreensharingWindow: [outside] mouse {:?} {:?} ignored",
                        button,
                        state
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.mouse_in_participant_area
                    && self.state.active_tab == "control"
                    && self.state.remote_control_allowed
                {
                    // Extract delta_x and delta_y from the scroll delta.
                    // Always convert lines to pixels, it seems to work for now, maybe a better approach is
                    // to also forward the unit type (lines/pixels) to the sharer.
                    const LINE_HEIGHT_PX: f64 = 40.0;
                    let (delta_x, delta_y) = match delta {
                        winit::event::MouseScrollDelta::LineDelta(x, y) => {
                            (*x as f64 * LINE_HEIGHT_PX, *y as f64 * LINE_HEIGHT_PX)
                        }
                        winit::event::MouseScrollDelta::PixelDelta(pos) => (pos.x, pos.y),
                    };

                    input_events.push(ScreenShareInputEvent::Scroll(
                        crate::room_service::WheelDelta {
                            deltaX: delta_x,
                            deltaY: delta_y,
                        },
                    ));

                    log::debug!(
                        "ScreensharingWindow: [participant_area] scroll delta {:?}",
                        delta
                    );
                } else {
                    log::debug!("ScreensharingWindow: [outside] scroll ignored");
                }
            }
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } => {
                // While you control their screen, keys are theirs.
                let controlling = self.mouse_in_participant_area
                    && self.state.active_tab == "control"
                    && self.state.remote_control_allowed;
                if let Some(key) = crate::window_manager::window_key(key_event, self.modifiers)
                    .filter(|_| !controlling)
                {
                    use crate::window_manager::WindowKey;
                    match key {
                        WindowKey::Minimise => self.window.set_minimized(true),
                        WindowKey::Close => {
                            input_events.extend(self.release_stamp());
                            input_events.extend(self.stamp_input(StampInput::Finish));
                            self.drop_note();
                            self.window.set_visible(false);
                        }
                        key => {
                            let _ = self
                                .event_loop_proxy
                                .send_event(crate::UserEvent::WindowKey(key));
                        }
                    }
                    return input_events;
                }

                // Intercept Cmd+Q to gracefully exit instead of hard quit
                #[cfg(target_os = "macos")]
                if key_event.state.is_pressed() && self.modifiers.super_key() {
                    if let Key::Character(ref ch) = key_event.logical_key {
                        if ch.as_ref() == "q" {
                            log::info!("ScreensharingWindow: caught Cmd+Q, requesting exit");
                            let _ = self
                                .event_loop_proxy
                                .send_event(crate::UserEvent::ExitRequested);
                            return input_events;
                        }
                    }
                }

                if self.state.active_tab == "draw"
                    && key_event.state.is_pressed()
                    && drawing_helpers::is_undo_shortcut(&key_event.logical_key, self.modifiers)
                {
                    if let Some(undone) =
                        self.state.local_stroke.undo(&mut self.participants_manager)
                    {
                        input_events.push(undone.into());
                        self.window.request_redraw();
                    }
                }

                // A key on its way into a note, or the release of one, isn't for their screen.
                let note_has_key = self.state.note.is_some();
                if key_event.state.is_pressed() {
                    let input = drawing_helpers::stamp_key(
                        &key_event.logical_key,
                        key_event.text.as_deref(),
                        self.modifiers,
                    );
                    if self.is_stamp_typing() {
                        if let Some(input) = input {
                            input_events.extend(self.stamp_input(input));
                        }
                    } else {
                        let may_start = self.state.last_draw_cursor.is_some()
                            && !(self.state.active_tab == "control"
                                && self.state.remote_control_allowed);
                        match drawing_helpers::note_key(&mut self.state.note, input, may_start) {
                            drawing_helpers::NoteKey::Ignored => {}
                            drawing_helpers::NoteKey::Changed => self.note_changed(),
                            drawing_helpers::NoteKey::Stamp => {
                                if let Some(point) = self.state.note_point {
                                    input_events.extend(self.stamp_note(point));
                                }
                            }
                        }
                    }
                }

                if self.mouse_in_participant_area
                    && self.state.active_tab == "control"
                    && self.state.remote_control_allowed
                    && !note_has_key
                {
                    // Extract key string to match the format expected by keyboard.rs.
                    // We need strings like "Enter", "Tab", "a", "A", etc., not control characters.
                    let mut meta = self.modifiers.super_key();
                    let key_str = match &key_event.logical_key {
                        // For character keys, use the character directly
                        Key::Character(s) => s.to_string(),
                        // For named keys (Enter, Tab, Escape, etc.), use the Debug format
                        // which produces strings like "Enter", "Tab", "ArrowLeft", etc.
                        // Space is a named key but maps to the actual space character in the keymap.
                        Key::Named(winit::keyboard::NamedKey::Space) => " ".to_string(),
                        Key::Named(winit::keyboard::NamedKey::Super) => {
                            meta = true;
                            "Meta".to_string()
                        }
                        Key::Named(named) => format!("{:?}", named),
                        // For dead keys, use the character if available, otherwise "Dead"
                        Key::Dead(ch) => {
                            if let Some(c) = ch {
                                c.to_string()
                            } else {
                                "Dead".to_string()
                            }
                        }
                        Key::Unidentified(_) => "Unidentified".to_string(),
                    };

                    let ctrl_or_cmd = if cfg!(target_os = "macos") {
                        self.modifiers.super_key()
                    } else {
                        self.modifiers.control_key()
                    };

                    if ctrl_or_cmd && key_event.state.is_pressed() {
                        match key_str.as_str() {
                            "c" => {
                                self.state.queued_modifier_events.clear();
                                input_events
                                    .push(ScreenShareInputEvent::AddToClipboard { is_copy: true });
                            }
                            "x" => {
                                self.state.queued_modifier_events.clear();
                                input_events
                                    .push(ScreenShareInputEvent::AddToClipboard { is_copy: false });
                            }
                            "v" => {
                                self.state.queued_modifier_events.clear();
                                let clipboard_text =
                                    self.clipboard.read(Kind::Standard).unwrap_or_default();
                                let data = if clipboard_text.is_empty() {
                                    None
                                } else {
                                    self.clipboard.write(Kind::Standard, String::new());
                                    Some(clipboard_text)
                                };
                                input_events.push(ScreenShareInputEvent::PasteFromClipboard(data));
                            }
                            _ => {
                                let down = key_event.state.is_pressed();
                                for queued in self.state.queued_modifier_events.drain(..) {
                                    input_events.push(ScreenShareInputEvent::KeyInput(queued));
                                }
                                input_events.push(ScreenShareInputEvent::KeyInput(
                                    crate::room_service::KeystrokeData {
                                        key: vec![key_str.clone()],
                                        meta,
                                        ctrl: self.modifiers.control_key(),
                                        shift: self.modifiers.shift_key(),
                                        alt: self.modifiers.alt_key(),
                                        down,
                                    },
                                ));
                            }
                        }
                    } else {
                        let down = key_event.state.is_pressed();
                        let keystroke = crate::room_service::KeystrokeData {
                            key: vec![key_str.clone()],
                            meta,
                            ctrl: self.modifiers.control_key(),
                            shift: self.modifiers.shift_key(),
                            alt: self.modifiers.alt_key(),
                            down,
                        };
                        let is_modifier_down =
                            down && matches!(key_str.as_str(), "Meta" | "Control");
                        if is_modifier_down {
                            self.state.queued_modifier_events.push(keystroke);
                        } else {
                            for queued in self.state.queued_modifier_events.drain(..) {
                                input_events.push(ScreenShareInputEvent::KeyInput(queued));
                            }
                            input_events.push(ScreenShareInputEvent::KeyInput(keystroke));
                        }
                    }

                    log::debug!(
                        "ScreensharingWindow: [participant_area] key {:?} {:?}",
                        key_event.logical_key,
                        key_event.state
                    );
                } else {
                    log::debug!(
                        "ScreensharingWindow: [outside] key {:?} ignored",
                        key_event.logical_key
                    );
                }
            }
            _ => {}
        }

        // Convert winit event to iced event
        // TODO check if we have to throttle this, we might get too many mouse events
        if !is_redraw {
            if let Some(iced_event) = conversion::window_event(
                event.clone(),
                self.window.scale_factor() as f32,
                self.modifiers,
            ) {
                if let Event::Mouse(mouse_event) = iced_event {
                    self.cursor = match mouse_event {
                        iced::mouse::Event::CursorMoved { position } => {
                            mouse::Cursor::Available(position)
                        }
                        iced::mouse::Event::CursorLeft => mouse::Cursor::Unavailable,
                        _ => self.cursor,
                    };
                }

                // Build user interface, process the event, and collect messages
                let mut messages: Vec<ScreensharingMessage> = Vec::new();

                let cache = self.cache.take().unwrap_or_default();
                let mut interface = UserInterface::build(
                    Self::view(
                        (&self.state, &self.call_controls),
                        &self.call_participants,
                        self.viewport.logical_size().width,
                        &self.screen_share_buffer,
                        &self.participants_manager,
                        &self.click_animation_renderer,
                        true,
                        (
                            self.layout(),
                            self.prefs.faces_in_shared,
                            &mut self.face_frame_ids,
                        ),
                    ),
                    self.viewport.logical_size(),
                    cache,
                    &mut self.renderer,
                );

                let iced_event = conversion::window_event(
                    event.clone(),
                    self.window.scale_factor() as f32,
                    self.modifiers,
                );
                if let Some(ev) = iced_event {
                    let (_, statuses) = interface.update(
                        &[ev],
                        self.cursor,
                        &mut self.renderer,
                        &mut self.clipboard,
                        &mut messages,
                    );
                    let _ = statuses;
                }

                self.cache = Some(interface.into_cache());

                // Process collected messages
                for msg in messages {
                    match &msg {
                        ScreensharingMessage::TabSelected(id) => {
                            input_events.extend(self.stamp_input(StampInput::Finish));
                            let tab = match *id {
                                "draw" => {
                                    self.state.drawing_hint.show_once();
                                    ScreenShareTab::Draw
                                }
                                "stamp" => ScreenShareTab::Stamp,
                                "point" => ScreenShareTab::Point,
                                _ => ScreenShareTab::Control,
                            };
                            if matches!(tab, ScreenShareTab::Control)
                                && self.state.remote_control_allowed
                            {
                                self.forget_note();
                            }
                            let stamping = matches!(tab, ScreenShareTab::Stamp);
                            let mode = match tab {
                                // Stamping is drawing to everyone else, so your ink stays up.
                                ScreenShareTab::Draw | ScreenShareTab::Stamp => {
                                    crate::room_service::DrawingMode::Draw(
                                        crate::room_service::DrawSettings {
                                            permanent: self.state.draw_persist,
                                        },
                                    )
                                }
                                ScreenShareTab::Point => {
                                    crate::room_service::DrawingMode::ClickAnimation
                                }
                                ScreenShareTab::Control => {
                                    crate::room_service::DrawingMode::Disabled
                                }
                            };
                            if mode == crate::room_service::DrawingMode::Disabled
                                || mode == crate::room_service::DrawingMode::ClickAnimation
                            {
                                self.participants_manager.draw_clear_all_paths(
                                    drawing_helpers::LOCAL_PARTICIPANT_IDENTITY,
                                );
                            }
                            self.participants_manager.set_drawing_mode(
                                drawing_helpers::LOCAL_PARTICIPANT_IDENTITY,
                                mode.clone(),
                            );
                            self.state.local_stroke.cancel();
                            input_events.push(if stamping {
                                ScreenShareInputEvent::StampTabSelected(mode)
                            } else {
                                ScreenShareInputEvent::DrawingModeChanged(mode)
                            });
                        }
                        ScreensharingMessage::DropdownItemClicked(index) => {
                            let permanent = *index == 1;
                            let mode = crate::room_service::DrawingMode::Draw(
                                crate::room_service::DrawSettings { permanent },
                            );
                            self.participants_manager.set_drawing_mode(
                                drawing_helpers::LOCAL_PARTICIPANT_IDENTITY,
                                mode.clone(),
                            );
                            input_events.push(ScreenShareInputEvent::DrawingModeChanged(mode));
                        }
                        _ => {}
                    }
                    self.update(msg);
                }

                // Tick animation; keep requesting redraws while it runs.
                seg_ctrl_mod::tick_animation(&mut self.state.tab_anim);
            }
        }

        // Handle winit-specific events
        match event {
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width > 0 && new_size.height > 0 {
                    let logical: winit::dpi::LogicalSize<f64> =
                        new_size.to_logical(self.window.scale_factor());

                    if logical.width < SCREENSHARE_CALL_CONTROLS_MIN_WIDTH as f64 {
                        self.call_controls.dismiss_dropdowns();
                    }

                    // Classify this resize event.
                    if let Some((target_w, target_h)) = self.programmatic_resize_target {
                        let matches_target = (logical.width - target_w).abs() < 2.0
                            && (logical.height - target_h).abs() < 2.0;
                        if matches_target {
                            self.programmatic_resize_target = None;
                            log::info!(
                                "ScreensharingWindow: programmatic resize landed at {:.1}x{:.1}",
                                logical.width,
                                logical.height
                            );
                        } else {
                            // Stale/reordered event while programmatic resize in flight — skip.
                            log::info!(
                                "ScreensharingWindow: ignoring stale resize {:.1}x{:.1} (waiting for {:.1}x{:.1})",
                                logical.width, logical.height, target_w, target_h
                            );
                        }
                    } else if self.state.last_stream_width == 0 {
                        // No stream data yet — all resizes are part of window creation.
                        log::info!(
                            "ScreensharingWindow: creation-phase resize to {:.1}x{:.1} (ignored)",
                            logical.width,
                            logical.height
                        );
                    } else {
                        let is_zoom = self.aspect_ratio_enforcer.is_zoomed(&self.window);

                        if is_zoom {
                            log::info!(
                                "ScreensharingWindow: zoom resize to {:.1}x{:.1} (logical)",
                                logical.width,
                                logical.height
                            );
                        } else {
                            self.state.user_has_resized = true;
                            log::info!(
                                "ScreensharingWindow: user resize to {:.1}x{:.1} (logical)",
                                logical.width,
                                logical.height
                            );
                        }

                        if let Some((target_w, target_h)) =
                            self.aspect_ratio_enforcer.correct_aspect_after_resize(
                                &self.window,
                                logical.width,
                                logical.height,
                                self.layout_aspect(),
                                self.screen_area.extent,
                                self.screen_area.position,
                            )
                        {
                            self.programmatic_resize_target = Some((target_w, target_h));
                            let _ = self.window.request_inner_size(winit::dpi::LogicalSize::new(
                                target_w, target_h,
                            ));
                            return vec![];
                        }
                    }

                    // Always reconfigure surface + viewport.
                    let aspect = self.layout_aspect();
                    self.aspect_ratio_enforcer
                        .set_aspect_ratio(&self.window, aspect);
                    self.surface.configure(
                        &self.device,
                        &wgpu::SurfaceConfiguration {
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format: self.format,
                            width: new_size.width,
                            height: new_size.height,
                            present_mode: wgpu::PresentMode::Immediate,
                            alpha_mode: self.alpha_mode,
                            view_formats: vec![],
                            desired_maximum_frame_latency: 2,
                        },
                    );
                    self.viewport = Viewport::with_physical_size(
                        Size::new(new_size.width, new_size.height),
                        self.window.scale_factor() as f32,
                    );
                    self.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw_in_progress.store(true, Ordering::Release);
                let cleared = self.redraw();
                self.redraw_in_progress.store(false, Ordering::Release);
                if !cleared.is_empty() {
                    input_events.push(ScreenShareInputEvent::DrawClearPaths(cleared));
                }
            }
            WindowEvent::CloseRequested => {
                input_events.extend(self.release_stamp());
                input_events.extend(self.stamp_input(StampInput::Finish));
                self.drop_note();
                self.hide();
            }
            _ => {}
        }

        input_events
    }

    fn view<'a>(
        (state, call_controls): (&'a ScreensharingState, &'a CallControlsState),
        call_participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        viewport_width: f32,
        screen_share_buffer: &'a Arc<crate::livekit::video::VideoBufferManager>,
        participants: &'a ParticipantsManager,
        click_animation_renderer: &'a ClickAnimationRenderer,
        skip_buffer: bool,
        (layout, faces_in_shared, face_frame_ids): (
            ShareLayout,
            FacesInShared,
            &mut HashMap<String, u64>,
        ),
    ) -> iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> {
        // ── Name label (left of header, after traffic lights) ───────────
        let name_label = container(
            text(format!("{}'s Screen", state.sharer_name))
                .size(14)
                .color(Color::WHITE)
                .font(GEIST_MEDIUM),
        )
        .padding(Padding {
            top: 2.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        });

        let seg_ctrl = seg_ctrl_mod::segmented_control(
            SEGMENTED_BUTTONS,
            state.active_tab,
            &state.tab_anim,
            ScreensharingMessage::TabSelected,
        );

        // ── Header: keep call controls between the centered drawing controls and settings.
        let traffic_light_spacer = if cfg!(target_os = "macos") { 68.0 } else { 0.0 };
        let show_call_controls = viewport_width >= SCREENSHARE_CALL_CONTROLS_MIN_WIDTH;
        let header_left_padding = if cfg!(target_os = "macos") {
            WindowConstant::PADDING
        } else {
            WindowConstant::HEADER_SIDE_PADDING
        };
        let cog_button = dropdown_trigger_button(
            ICON_COG,
            state.dropdown_open,
            ScreensharingMessage::ToggleDropdown,
        );
        let faces_button = faces_button(faces_in_shared);

        let header_content: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> =
            if show_call_controls {
                let header_ends = row![
                    Space::new().width(Length::Fixed(traffic_light_spacer)),
                    name_label,
                    Space::new().width(Length::Fill),
                    faces_button,
                    cog_button,
                ]
                .align_y(Alignment::Center)
                .width(Length::Fill);
                let center_and_call_controls = row![
                    Space::new().width(Length::Fixed(SCREENSHARE_HEADER_END_WIDTH)),
                    Space::new().width(Length::Fill),
                    seg_ctrl,
                    container(
                        call_controls
                            .view(call_participants, CallControlsDensity::Compact)
                            .map(ScreensharingMessage::CallControls),
                    )
                    .width(Length::Fill)
                    .center_x(Length::Fill),
                    Space::new().width(Length::Fixed(SCREENSHARE_HEADER_END_WIDTH)),
                ]
                .width(Length::Fill)
                .align_y(Alignment::Center);
                stack![center_and_call_controls, header_ends].into()
            } else {
                let header_ends = row![
                    Space::new().width(Length::Fixed(traffic_light_spacer)),
                    name_label,
                    Space::new().width(Length::Fill),
                    faces_button,
                    cog_button,
                ]
                .align_y(Alignment::Center)
                .width(Length::Fill);
                let header_center = container(seg_ctrl)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill);
                stack![header_ends, header_center].into()
            };

        let header = container(header_content)
            .width(Length::Fill)
            .padding(Padding {
                top: 4.0,
                right: WindowConstant::HEADER_SIDE_PADDING,
                bottom: 4.0,
                left: header_left_padding,
            });

        // ── Content area (video stream) ──────────────────────────────────
        // Check if buffer has data by peeking at the latest frame
        let has_data = {
            let frame_lock = screen_share_buffer.latest_frame();
            let buf = frame_lock.lock().unwrap();
            buf.width > 0 && buf.height > 0
        };

        let video_content: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> =
            if has_data {
                shader::<ScreensharingMessage, _>(YuvVideoProgram {
                    participant_id: SCREENSHARE_STREAM_ID,
                    buffer: screen_share_buffer.clone(),
                    corner_radius: 12.0,
                    stretch_to_fill: true,
                    skip_upload: skip_buffer,
                    mirror: false,
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            } else {
                // Show placeholder when no data
                container(text(""))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|_theme: &Theme| container::Style {
                        background: Some(Background::Color(ColorToken::Slate800.to_color())),
                        ..Default::default()
                    })
                    .into()
            };

        let canvas_overlay: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> =
            canvas(drawing_helpers::ParticipantOverlay {
                participants,
                click_animation_renderer: Some(click_animation_renderer),
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let app_veil_overlay: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> =
            if state.app_veil_snapshot.windows.is_empty() {
                Space::new().into()
            } else {
                canvas(AppVeilOverlay {
                    snapshot: &state.app_veil_snapshot,
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            };

        let remote_control_disabled_label: iced::Element<
            'a,
            ScreensharingMessage,
            Theme,
            iced::Renderer,
        > = if !state.remote_control_allowed && state.active_tab == "control" {
            container(
                container(
                    text("Remote control is disabled")
                        .size(12)
                        .font(GEIST_REGULAR)
                        .color(Color::from_rgba(0.0, 0.0, 0.0, 0.9)),
                )
                .padding(Padding {
                    top: 2.0,
                    right: 10.0,
                    bottom: 2.0,
                    left: 10.0,
                })
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Gradient(
                        gradient::Linear::new(Radians(0.0))
                            .add_stop(
                                0.0,
                                Color::from_rgba(249.0 / 255.0, 250.0 / 255.0, 251.0 / 255.0, 0.6),
                            )
                            .add_stop(
                                1.0,
                                Color::from_rgba(153.0 / 255.0, 161.0 / 255.0, 175.0 / 255.0, 0.6),
                            )
                            .into(),
                    )),
                    border: Border {
                        radius: 100.0.into(),
                        color: Color::from_rgba(1.0, 1.0, 1.0, 0.8),
                        width: 1.0,
                    },
                    ..Default::default()
                }),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::End)
            .padding(10.0)
            .into()
        } else {
            Space::new().into()
        };

        let layered_content = stack![
            video_content,
            app_veil_overlay,
            canvas_overlay,
            remote_control_disabled_label,
            if state.note.is_some() || state.active_tab == "stamp" {
                drawing_helpers::stamp_hint_view(drawing_helpers::StampHint {
                    note: state.note.is_some(),
                    typing: participants.is_stamp_typing(),
                    hearing: participants.is_stamp_hearing(),
                    held: state.stamp_held,
                    capturing: state.capturing,
                })
            } else if state.active_tab == "draw" {
                state.drawing_hint.view(false)
            } else {
                Space::new().into()
            }
        ];

        let picture = container(layered_content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(ColorToken::Slate800.to_color())),
                border: Border {
                    color: ColorToken::Slate600.to_color(),
                    width: 1.0,
                    radius: 12.0.into(),
                },
                shadow: Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.20),
                    offset: Vector::new(0.0, 6.0),
                    blur_radius: 6.0,
                },
                ..Default::default()
            })
            .clip(true);

        // Their screen keeps its shape wherever the window's edges are, and the faces go
        // beside it, or over its corner.
        let faces = faces_view(
            faces_in_shared,
            layout,
            call_participants,
            face_frame_ids,
            skip_buffer,
        );
        let main_content = stack![
            column![header, Space::new().height(Length::Fill)]
                .width(Length::Fill)
                .height(Length::Fill),
            placed(picture, layout.picture),
            faces,
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        let base: iced::Element<'a, ScreensharingMessage, Theme, iced::Renderer> =
            container(main_content)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme: &Theme| container::Style {
                    background: if cfg!(target_os = "macos") {
                        Some(Background::Color(Color::from_rgba(0.31, 0.31, 0.45, 0.15)))
                    } else {
                        Some(Background::Color(ColorToken::Zinc900.to_color()))
                    },
                    border: Border {
                        radius: 10.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .clip(true)
                .into();

        let base = if state.dropdown_open {
            let items = [
                DropdownItemDef {
                    label: "Fade Out",
                    icon: ICON_PENCIL_SVG,
                    selected: !state.draw_persist,
                },
                DropdownItemDef {
                    label: "Persist Until Right Click",
                    icon: ICON_PENCIL_SVG,
                    selected: state.draw_persist,
                },
            ];
            let menu = crate::components::dropdown::dropdown_menu(
                &items,
                &[],
                ScreensharingMessage::DropdownItemClicked,
            );
            dropdown_overlay(
                base,
                menu,
                ScreensharingMessage::DismissDropdown,
                WindowConstant::HEADER_HEIGHT,
                WindowConstant::HEADER_SIDE_PADDING,
            )
        } else {
            base
        };

        if viewport_width >= SCREENSHARE_CALL_CONTROLS_MIN_WIDTH {
            let call_controls_slot_width = viewport_width
                - header_left_padding
                - WindowConstant::HEADER_SIDE_PADDING
                - SCREENSHARE_SEGMENTED_CONTROLS_WIDTH
                - SCREENSHARE_HEADER_END_WIDTH * 2.0;
            let call_controls_slot_width = call_controls_slot_width / 2.0;
            let trailing_padding = WindowConstant::HEADER_SIDE_PADDING
                + SCREENSHARE_HEADER_END_WIDTH
                + (call_controls_slot_width - CallControlsDensity::Compact.total_width()).max(0.0)
                    / 2.0;
            call_controls.wrap_dropdown(
                base,
                ScreensharingMessage::CallControls,
                CallControlsDensity::Compact,
                WindowConstant::HEADER_HEIGHT,
                trailing_padding,
            )
        } else {
            base
        }
    }

    /// Handle a screensharing UI message (state update).
    fn update(&mut self, message: ScreensharingMessage) {
        match message {
            ScreensharingMessage::CallControls(message) => {
                if matches!(
                    &message,
                    CallControlsMessage::MicDropdownToggle
                        | CallControlsMessage::CameraDropdownToggle
                ) {
                    self.state.dropdown_open = false;
                }
                self.call_controls
                    .update(message, &self.call_participants, &self.event_loop_proxy);
            }
            ScreensharingMessage::TabSelected(id) => {
                self.state.tab_anim =
                    seg_ctrl_mod::start_animation(SEGMENTED_BUTTONS, self.state.active_tab, id);
                self.state.active_tab = id;
                self.state.queued_modifier_events.clear();
                self.update_cursor();
                log::info!("ScreensharingWindow: tab selected = {}", id);
            }
            ScreensharingMessage::ToggleDropdown => {
                self.call_controls.dismiss_dropdowns();
                self.state.dropdown_open = !self.state.dropdown_open;
                log::info!(
                    "ScreensharingWindow: dropdown toggled = {}",
                    self.state.dropdown_open
                );
            }
            ScreensharingMessage::DismissDropdown => {
                self.state.dropdown_open = false;
            }
            ScreensharingMessage::ToggleFacesOut => self.toggle_faces_out(),
            ScreensharingMessage::Nothing => {}
            ScreensharingMessage::DropdownItemClicked(index) => {
                match index {
                    0 => self.state.draw_persist = false,
                    1 => self.state.draw_persist = true,
                    _ => {}
                }
                log::info!(
                    "ScreensharingWindow: draw_persist = {}",
                    self.state.draw_persist
                );
                self.state.dropdown_open = false;
            }
        }
    }

    /// Perform a full redraw: build UI, draw, present.
    /// Returns path IDs cleared by auto-expire during this frame.
    fn redraw(&mut self) -> Vec<u64> {
        self.redraw_inner()
    }

    fn redraw_inner(&mut self) -> Vec<u64> {
        let current_frame_id = self.screen_share_buffer.current_frame_id();
        let mut skip_buffer = false;

        // Reset the last_rendered_frame_id on the first frame, we do this because we might get
        // stale frame ids in the initial renders from the video buffer manager.
        if current_frame_id > 0
            && self.last_rendered_frame_id > 40
            && current_frame_id < self.last_rendered_frame_id - 40
        {
            log::info!(
                "redraw_inner: stream restart detected (frame_id={current_frame_id}, last_rendered={}), resetting",
                self.last_rendered_frame_id
            );
            self.last_rendered_frame_id = 0;
        }
        if current_frame_id > 0 && current_frame_id <= self.last_rendered_frame_id {
            log::trace!(
                "redraw_inner: dropping redraw {current_frame_id} {}",
                self.last_rendered_frame_id
            );
            skip_buffer = true;
        }

        let (stream_w, stream_h);
        {
            let frame_lock = self.screen_share_buffer.latest_frame();
            let buf = frame_lock.lock().unwrap();
            stream_w = buf.width;
            stream_h = buf.height;

            if stream_w > 0
                && stream_h > 0
                && (stream_w != self.state.last_stream_width
                    || stream_h != self.state.last_stream_height)
            {
                // Stream dimensions changed — update aspect ratio and resize window
                self.state.img_aspect = stream_w as f64 / stream_h as f64;
                self.state.last_stream_width = stream_w;
                self.state.last_stream_height = stream_h;
                // The window's shape, with the faces beside their screen.
                let aspect = self.layout_aspect();

                let (width, height) = if !self.state.user_has_resized {
                    // Auto-maximize to fit monitor
                    calculate_max_window_size(self.screen_area.extent, aspect).unwrap_or_else(
                        || {
                            let content_w =
                                WindowConstant::DEFAULT_WIDTH - WindowConstant::SKELETON_W;
                            let content_h = content_w / aspect;
                            log::warn!(
                                "ScreensharingWindow: calculate_max_window_size returned None, using fallback {:.1}x{:.1}",
                                WindowConstant::DEFAULT_WIDTH,
                                content_h + WindowConstant::SKELETON_H
                            );
                            (
                                WindowConstant::DEFAULT_WIDTH,
                                content_h + WindowConstant::SKELETON_H,
                            )
                        },
                    )
                } else {
                    // User manually resized: keep current width, adjust height for new aspect
                    let current_size: winit::dpi::LogicalSize<f64> = self
                        .window
                        .inner_size()
                        .to_logical(self.window.scale_factor());
                    let content_w = current_size.width - WindowConstant::SKELETON_W;
                    let content_h = content_w / aspect;
                    (current_size.width, content_h + WindowConstant::SKELETON_H)
                };

                self.aspect_ratio_enforcer
                    .set_aspect_ratio(&self.window, aspect);

                // If we're auto-maximizing to the probed screen area, move the window first.
                // Otherwise macOS may clamp the resize request against the *current* monitor.
                let saved_pos = if self.state.user_has_resized {
                    self.window.outer_position().ok()
                } else {
                    None
                };

                if !self.state.user_has_resized {
                    self.window
                        .set_outer_position(winit::dpi::LogicalPosition::new(
                            self.screen_area.position.x,
                            self.screen_area.position.y,
                        ));
                }

                self.programmatic_resize_target = Some((width, height));
                let _ = self
                    .window
                    .request_inner_size(winit::dpi::LogicalSize::new(width, height));

                if let Some(pos) = saved_pos {
                    // Restore position so the window doesn't jump when aspect changes.
                    self.window.set_outer_position(pos);
                }

                log::info!(
                    "ScreensharingWindow: stream dimensions changed to {}x{}, aspect={:.3}, window size={:.1}x{:.1}",
                    stream_w,
                    stream_h,
                    aspect,
                    width,
                    height
                );

                // Don't render at the old size — wait for the Resized event to
                // reconfigure the surface and trigger a redraw at the correct size.
                return Vec::new();
            }
        }
        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(e) => {
                log::error!("ScreensharingWindow::redraw: failed to get texture: {e:?}");
                return vec![];
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.click_animation_renderer.update();
        self.participants_manager.hide_inactive_cursors();

        // Build fresh interface from cache
        let cache = self.cache.take().unwrap_or_default();
        let mut interface = UserInterface::build(
            Self::view(
                (&self.state, &self.call_controls),
                &self.call_participants,
                self.viewport.logical_size().width,
                &self.screen_share_buffer,
                &self.participants_manager,
                &self.click_animation_renderer,
                skip_buffer,
                (
                    self.layout(),
                    self.prefs.faces_in_shared,
                    &mut self.face_frame_ids,
                ),
            ),
            self.viewport.logical_size(),
            cache,
            &mut self.renderer,
        );

        // Send a redraw event to iced
        let _ = interface.update(
            &[Event::Window(
                window::Event::RedrawRequested(Instant::now()),
            )],
            self.cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut Vec::new(),
        );

        // Draw the interface with white text for dark background
        interface.draw(
            &mut self.renderer,
            &Theme::Dark,
            &Style {
                text_color: Color::WHITE,
            },
            self.cursor,
        );
        self.cache = Some(interface.into_cache());

        // Present via wgpu
        let wgpu_renderer = match &mut self.renderer {
            iced::Renderer::Primary(r) => r,
            _ => unreachable!(),
        };
        // On macOS, clear to transparent so the vibrancy effect shows through.
        let clear_color = if cfg!(target_os = "macos") {
            Some(Color::TRANSPARENT)
        } else {
            Some(ColorToken::Zinc900.to_color())
        };
        wgpu_renderer.present(clear_color, output.texture.format(), &view, &self.viewport);

        self.window.pre_present_notify();
        output.present();

        // Keep the redraw loop alive while the segmented-control indicator
        // is animating, so the slide plays smoothly even when no user input
        // events are arriving.
        seg_ctrl_mod::tick_animation(&mut self.state.tab_anim);

        let cleared = self.participants_manager.update_auto_clear();
        if self.participants_manager.is_ink_animating()
            || self.participants_manager.is_cursor_animating()
            || self.participants_manager.is_stamp_animating()
            || self.state.drawing_hint.is_visible()
        {
            let _ = self.redraw_tx.send(RedrawCommand::Animate);
        }

        if !skip_buffer {
            self.last_rendered_frame_id = current_frame_id;
        }

        cleared
    }
}

impl ScreensharingWindow {
    /// Sends the Stop command and drops the redraw thread handle (detach).
    pub fn stop_redraw_thread(&mut self) {
        if self.redraw_thread.take().is_some() {
            if let Err(e) = self.redraw_tx.send(RedrawCommand::Stop) {
                log::error!("ScreensharingWindow::stop_redraw_thread: failed to send Stop: {e:?}");
            }
        }
    }
}

impl Drop for ScreensharingWindow {
    fn drop(&mut self) {
        let _ = self.redraw_tx.send(RedrawCommand::Stop);
        self.redraw_thread.take();
    }
}

#[cfg(test)]
mod redraw_loop_tests {
    use super::*;

    fn spawn_loop() -> (
        std::sync::mpsc::Sender<RedrawCommand>,
        std::sync::mpsc::Receiver<()>,
        std::thread::JoinHandle<()>,
    ) {
        let (redraw_tx, redraw_rx) = std::sync::mpsc::channel();
        let (frame_tx, frame_rx) = std::sync::mpsc::channel();
        let redraw_thread = std::thread::spawn(move || {
            run_redraw_loop(redraw_rx, Arc::new(AtomicBool::new(false)), move || {
                let _ = frame_tx.send(());
            })
        });
        (redraw_tx, frame_rx, redraw_thread)
    }

    #[test]
    fn animating_paces_frames_faster_than_the_idle_timer() {
        const FRAMES: u32 = 10;
        let (redraw_tx, frame_rx, redraw_thread) = spawn_loop();

        let started_at = std::time::Instant::now();
        for _ in 0..FRAMES {
            redraw_tx.send(RedrawCommand::Animate).unwrap();
            frame_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(started_at.elapsed() < REDRAW_INTERVAL * FRAMES / 2);

        redraw_tx.send(RedrawCommand::Stop).unwrap();
        redraw_thread.join().unwrap();
    }

    #[test]
    fn frames_slow_back_down_once_the_animation_stops_asking() {
        let (redraw_tx, frame_rx, redraw_thread) = spawn_loop();

        redraw_tx.send(RedrawCommand::Animate).unwrap();
        frame_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let last_frame_at = std::time::Instant::now();
        frame_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(last_frame_at.elapsed() > ANIMATION_FRAME_INTERVAL * 3);

        redraw_tx.send(RedrawCommand::Stop).unwrap();
        redraw_thread.join().unwrap();
    }
}

#[cfg(test)]
mod app_veil_tests {
    use super::*;

    #[test]
    fn projects_normalized_app_veil_rect_into_canvas_bounds() {
        let rect = crate::room_service::NormalizedRect {
            x: 0.25,
            y: 0.5,
            width: 0.5,
            height: 0.25,
        };

        assert_eq!(
            project_app_veil_rect(
                &rect,
                Rectangle::new(Point::new(10.0, 20.0), Size::new(800.0, 400.0))
            ),
            Rectangle::new(Point::new(200.0, 200.0), Size::new(400.0, 100.0))
        );
    }

    #[test]
    fn app_veil_label_requires_minimum_projected_size() {
        assert!(app_veil_label_visible(Rectangle::new(
            Point::ORIGIN,
            Size::new(160.0, 40.0)
        )));
        assert!(!app_veil_label_visible(Rectangle::new(
            Point::ORIGIN,
            Size::new(159.0, 40.0)
        )));
        assert!(!app_veil_label_visible(Rectangle::new(
            Point::ORIGIN,
            Size::new(160.0, 39.0)
        )));
    }
}

#[cfg(test)]
mod faces_layout_tests {
    use super::*;

    /// A window whose content has this shape, `width` wide.
    fn window_for(width: f64, content_aspect: f64) -> Size {
        let content_w = width - WindowConstant::SKELETON_W;
        let content_h = content_w / content_aspect;
        Size::new(
            width as f32,
            (content_h + WindowConstant::SKELETON_H) as f32,
        )
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.5
    }

    #[test]
    fn in_a_window_of_its_shape_the_screen_fills_its_side_of_the_faces() {
        for stream_aspect in [16.0 / 9.0, 16.0 / 10.0, 4.0 / 3.0, 21.0 / 9.0, 0.5] {
            for faces in [
                FacesInShared::Side,
                FacesInShared::Bottom,
                FacesInShared::Bubbles,
                FacesInShared::Off,
            ] {
                let window = window_for(1400.0, layout_aspect(stream_aspect, faces));
                let layout = share_layout(window, stream_aspect as f32, faces);
                let picture = layout.picture;
                assert!(
                    close(picture.x, WindowConstant::PADDING)
                        && close(picture.y, WindowConstant::HEADER_HEIGHT),
                    "{stream_aspect} {faces:?}: no bars, {picture:?}"
                );
                assert!(close(picture.width / picture.height, stream_aspect as f32));
                let right = window.width - WindowConstant::PADDING;
                let bottom = window.height - WindowConstant::PADDING;
                match (faces, layout.faces) {
                    (FacesInShared::Side, Some(rail)) => {
                        assert!(close(picture.x + picture.width + FACES_GAP, rail.x));
                        assert!(close(rail.x + rail.width, right));
                        assert!(close(picture.y + picture.height, bottom));
                    }
                    (FacesInShared::Bottom, Some(rail)) => {
                        assert!(close(picture.y + picture.height + FACES_GAP, rail.y));
                        assert!(close(rail.y + rail.height, bottom));
                        assert!(close(picture.x + picture.width, right));
                    }
                    (FacesInShared::Bubbles | FacesInShared::Off, None) => {
                        assert!(close(picture.x + picture.width, right));
                        assert!(close(picture.y + picture.height, bottom));
                    }
                    other => panic!("{other:?}"),
                }
            }
        }
    }

    #[test]
    fn full_screen_the_screen_keeps_its_shape_with_bars_either_side() {
        // A 16:10 display full screen, showing a 21:9 screen and a 4:3 one.
        let window = Size::new(1728.0, 1117.0);
        for stream_aspect in [21.0 / 9.0, 4.0 / 3.0] {
            for faces in [
                FacesInShared::Side,
                FacesInShared::Bottom,
                FacesInShared::Off,
            ] {
                let layout = share_layout(window, stream_aspect, faces);
                let picture = layout.picture;
                assert!(close(picture.width / picture.height, stream_aspect));
                assert!(picture.x >= WindowConstant::PADDING - 0.01);
                assert!(picture.y >= WindowConstant::HEADER_HEIGHT - 0.01);
                assert!(picture.x + picture.width <= window.width - WindowConstant::PADDING + 0.01);
                assert!(
                    picture.y + picture.height <= window.height - WindowConstant::PADDING + 0.01
                );
                if let Some(rail) = layout.faces {
                    assert!(rail.intersection(&picture).is_none_or(|i| i.area() < 1.0));
                }
            }
        }
    }

    #[test]
    fn the_faces_fill_their_side_and_get_smaller_for_more_people() {
        let side = Size::new(160.0, 800.0);
        let (places, tile) = faces_tiles(side, false, 2);
        assert_eq!(places, 2);
        assert_eq!(tile, Size::new(160.0, 120.0));

        // Too many at their best: shorter, down to 16:9.
        let (places, tile) = faces_tiles(side, false, 7);
        assert_eq!(places, 7);
        assert!(tile.height < 120.0 && tile.height >= 90.0 - 0.01);
        assert!(7.0 * tile.height + 6.0 * FACES_GAP <= 800.0 + 0.01);

        // Past that, the last place says how many more.
        let (places, tile) = faces_tiles(side, false, 30);
        assert!(places < 30);
        assert!(places as f32 * tile.height + (places - 1) as f32 * FACES_GAP <= 800.0 + 0.01);

        let bottom = Size::new(1200.0, 140.0);
        let (places, tile) = faces_tiles(bottom, true, 3);
        assert_eq!(places, 3);
        assert_eq!(tile.height, 140.0);
        assert!(close(tile.width, 140.0 * 16.0 / 9.0));
        let (places, tile) = faces_tiles(bottom, true, 40);
        assert!(places < 40 && tile.width >= 140.0 - 0.01);
    }

    #[test]
    fn the_faces_are_everyone_else_then_you_with_your_camera_on() {
        let mut participants = HashMap::new();
        participants.insert(
            "b".to_string(),
            ParticipantInfo::new("Zed".into(), false, false),
        );
        participants.insert(
            "a".to_string(),
            ParticipantInfo::new("Ada".into(), false, false),
        );
        participants.insert(
            "local".to_string(),
            ParticipantInfo::new("Me".into(), false, false),
        );
        let names: Vec<&str> = faces_people(&participants)
            .iter()
            .map(|(_, info)| info.name())
            .collect();
        // Your camera's off.
        assert_eq!(names, ["Ada", "Zed"]);
    }
}
