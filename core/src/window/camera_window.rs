//! Camera window with interactive Iced UI rendered via wgpu.
//!
//! This module implements a standalone window for the camera/video call view,
//! using winit for the window, wgpu for the GPU surface, and iced for
//! the interactive widget tree (buttons, text, layout).
//!
//! UI styling is ported from the iced-poc video call window:
//! - Geist font family (Regular + Medium)
//! - Tailwind color tokens (Slate, Gray, Green, Orange, Red, Lime)
//! - Shadow tokens for consistent depth
//! - Header control buttons via `split_button` (solid colors + optional dropdowns)
//! - Responsive participant grid with name labels and speaking indicators

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use iced::widget::{
    button, column, container, mouse_area, row, shader, stack, svg, text, tooltip, Space,
};
use iced::{
    gradient, Alignment, Background, Border, Color, Length, Padding, Pixels, Radians, Shadow,
    Size as IcedSize,
};
use iced_wgpu::core::mouse;
use iced_wgpu::graphics::Viewport;
use iced_winit::core::renderer::Style;
use iced_winit::core::time::Instant;
use iced_winit::core::{window, Event, Size, Theme};
use iced_winit::runtime::user_interface::Cache;
use iced_winit::runtime::UserInterface;
use iced_winit::{conversion, Clipboard};
use socket_lib::{CallWindowPrefs, FacesLayout, FacesOnTop, PinnedStyle, ScreenCorner};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use thiserror::Error;

use crate::components::call_controls::{
    CallControlsDensity, CallControlsMessage, CallControlsState, ICON_CAPTURE,
};
use crate::components::fonts::{self as fonts_mod, GEIST_MEDIUM, GEIST_REGULAR, ICONS_FONT};
use crate::components::toast::{self, ToastPosition, ToastState};
use crate::graphics::graphics_window_context::{
    ContextManager, GraphicsWindowContext, GraphicsWindowContextError,
};
use crate::graphics::yuv_renderer::YuvVideoProgram;
use crate::livekit::participant::ParticipantInfo;
use crate::livekit::video::VideoBufferManager;
use crate::windows::colors::ColorToken;
use crate::windows::shadows::ShadowToken;
use crate::UserEvent;

/// Initial camera window dimensions (logical pixels).
const CAMERA_WINDOW_WIDTH: f64 = 1035.0;
const CAMERA_WINDOW_HEIGHT: f64 = 555.0;

/// Minimum camera window dimensions.
const CAMERA_WINDOW_MIN_WIDTH: f64 = 100.0;
const CAMERA_WINDOW_MIN_HEIGHT: f64 = 100.0;

use std::sync::mpsc;

/// Target redraw interval: 30 FPS
const REDRAW_INTERVAL: Duration = Duration::from_millis(1_000 / 30);
/// Redraw interval when screensharing is active: 15 FPS
const REDRAW_INTERVAL_SCREENSHARING: Duration = Duration::from_millis(1_000 / 15);

pub enum RedrawCommand {
    Stop,
    Pause,
    Resume,
    SetScreensharingActive(bool),
}

fn spawn_redraw_thread(
    redraw_rx: mpsc::Receiver<RedrawCommand>,
    window: Arc<Window>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut interval = REDRAW_INTERVAL;
        let mut paused = false;
        loop {
            match redraw_rx.recv_timeout(interval) {
                Ok(RedrawCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(RedrawCommand::Pause) => paused = true,
                Ok(RedrawCommand::Resume) => paused = false,
                Ok(RedrawCommand::SetScreensharingActive(active)) => {
                    interval = if active {
                        REDRAW_INTERVAL_SCREENSHARING
                    } else {
                        REDRAW_INTERVAL
                    };
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !paused {
                        window.request_redraw();
                    }
                }
            }
        }
    })
}

const CONTENT_PADDING: f32 = 12.0;

// Grid layout constants
const TILE_SPACING: f32 = 16.0;
const MIN_GRID_PADDING: f32 = 8.0;

// Header height: split_button (20px) + padding top (12)
const HEADER_HEIGHT: f32 = 20.0 + CONTENT_PADDING;

// Window size thresholds for small window styling
const SMALL_WIDTH_THRESHOLD: f32 = 450.0;
const SMALL_HEIGHT_THRESHOLD: f32 = 600.0;
const COMPACT_WIDTH_THRESHOLD: f32 = 300.0;
/// Narrower than this, the controls take less room to fit between the sides of the header.
const REGULAR_CONTROLS_MIN_WIDTH: f32 =
    CallControlsDensity::Regular.total_width() + 2.0 * (HEADER_SIDE_WIDTH + CONTENT_PADDING);
/// Keeps the native traffic lights clear, and balances them on the other side of the controls.
const HEADER_SIDE_WIDTH: f32 = 92.0;
/// Narrower than this, the controls don't fit in the header at all.
const HEADER_CONTROLS_MIN_WIDTH: f32 =
    CallControlsDensity::Compact.total_width() + 2.0 * (HEADER_SIDE_WIDTH + CONTENT_PADDING);
/// The controls' height, and the margin below them. The grid's padding is the margin above.
const CONTROLS_STRIP_HEIGHT: f32 = 26.0 + MIN_GRID_PADDING;
/// Shorter than this, a strip would leave too little video, so the controls show on hover.
const CONTROLS_STRIP_MIN_WINDOW_HEIGHT: f32 = 140.0;
/// Narrower than this, the controls are down to the essentials.
const ICON_CONTROLS_MIN_WIDTH: f32 = CallControlsDensity::Icons.total_width() + 8.0;
const HIDE_NAME_TILE_THRESHOLD: f32 = 150.0;
/// Narrower than this, a tile in another window has no room for a name.
const FACE_CARD_NAME_MIN_WIDTH: f32 = 96.0;

const COMPACT_TOP_INSET: f32 = 24.0;

const ICON_MICROPHONE_OFF: char = '\u{F106}';
const ICON_PIN_ANGLE: char = '\u{F10B}';
const ICON_SCREEN_SHARE: char = '\u{F102}';

/// Pinned as a box of everyone.
const PIN_BOX_WIDTH: f64 = CAMERA_WINDOW_MIN_WIDTH;
const PIN_BOX_HEIGHT: f64 = CAMERA_WINDOW_MIN_HEIGHT;
/// Pinned to show only whoever is speaking: room for their picture and name. Short enough
/// that the controls show on hover, see `CONTROLS_STRIP_MIN_WINDOW_HEIGHT`.
const PIN_SPEAKER_WIDTH: f64 = 200.0;
const PIN_SPEAKER_HEIGHT: f64 = 135.0;
/// Between a pinned window and the edges of the screen.
const PIN_CORNER_MARGIN: f64 = 16.0;

/// Pinned as a strip: a column of round faces, a little bigger while someone speaks.
const STRIP_FACE: f32 = 44.0;
const STRIP_SPEAKER_FACE: f32 = 52.0;
const STRIP_GAP: f32 = 8.0;
/// Room for a face's ring and shadow at the edges of the window.
const STRIP_PADDING: f32 = 8.0;
const STRIP_BUTTON_HEIGHT: f32 = 26.0;
/// Under the faces, for the controls that show on hover.
const STRIP_CONTROLS_HEIGHT: f32 = STRIP_GAP + 2.0 * STRIP_BUTTON_HEIGHT + 6.0;
/// With more people than this, the strip ends with how many more there are.
const STRIP_MAX_FACES: usize = 6;
const STRIP_WIDTH: f32 = STRIP_SPEAKER_FACE + 2.0 * STRIP_PADDING;

/// How long a dragged pinned window waits, still, before it settles into the nearest corner.
const SNAP_DELAY: Duration = Duration::from_millis(250);
/// Someone who takes over the speaker view keeps it at least this long, so it doesn't flick
/// between people talking over each other.
const SPEAKER_HOLD: Duration = Duration::from_millis(1500);

/// Tiles go from square, where the space is tall, to as wide as a camera's picture.
const SQUAREST_TILE: f32 = 1.0;
const WIDEST_TILE: f32 = 16.0 / 9.0;
/// The speaker view's row of everyone else, as a share of the height, and its limits.
const SPEAKER_ROW_SHARE: f32 = 0.22;
const SPEAKER_ROW_MIN_HEIGHT: f32 = 60.0;
const SPEAKER_ROW_MAX_HEIGHT: f32 = 140.0;

const ICON_EYE_ON_SVG: &[u8] = include_bytes!("../../resources/icons/EyeOn.svg");
const ICON_MICROPHONE_SVG: &[u8] = include_bytes!("../../resources/icons/microphone.svg");
const ICON_EYE_OFF_SVG: &[u8] = include_bytes!("../../resources/icons/EyeOff.svg");
const ICON_LAYOUT_GRID_SVG: &[u8] = include_bytes!("../../resources/icons/layout-grid.svg");
const ICON_LAYOUT_SPEAKER_SVG: &[u8] = include_bytes!("../../resources/icons/layout-speaker.svg");

const AVATAR_SIZE: f32 = 130.0;
const AVATAR_RADIUS: f32 = 20.0;
const AVATAR_FONT_SIZE: f32 = 42.0;
const AVATAR_LETTER_SPACING: f32 = -3.0;

#[derive(Error, Debug)]
pub enum CameraWindowError {
    #[error("Failed to create window")]
    WindowCreation,
    #[error("Failed to create wgpu surface")]
    SurfaceCreation,
    #[error("No suitable GPU adapter found")]
    AdapterRequest,
    #[error("Failed to request GPU device")]
    DeviceRequest,
}

#[derive(Debug, Clone)]
pub enum CameraMessage {
    CallControls(CallControlsMessage),
    ToggleSelfVisibility,
    /// Mouse entered or left the local participant tile (for hover-only chrome).
    LocalTileHover(bool),
    PinToCorner,
    /// Bring the shared screen to you, from the sharer's badge.
    ViewScreen,
    /// Pressed on a pinned window's faces, which move it.
    DragWindow,
    /// Between everyone the same size and the speaker big.
    ToggleLayout,
}

struct CameraState {
    // TODO: why do we keep state for this instead of reusing the viewport_size from the CameraWindow?
    viewport_size: IcedSize,
    /// Local camera on/off state, updated from StartCamera/StopCamera handlers.
    /// When true, the local participant tile is hidden (floating control restores it).
    self_hidden: bool,
    /// True while the pointer is over the local tile (show hide-self control).
    local_tile_hovered: bool,
    /// Window narrower than `COMPACT_WIDTH_THRESHOLD` has no pin button, and is taken to be pinned.
    is_compact: bool,
    /// True while the pointer is on the window (`ControlsPlacement::Hover` shows the controls).
    pointer_over_window: bool,
    toast: Option<ToastState>,
    /// How the window looks while it's pinned; None when it's an ordinary window.
    pinned: Option<PinnedStyle>,
    layout: FacesLayout,
    /// Who the speaker view shows, see `SpeakerFocus`.
    speaker: Option<String>,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            viewport_size: IcedSize::new(CAMERA_WINDOW_WIDTH as f32, CAMERA_WINDOW_HEIGHT as f32),
            self_hidden: false,
            local_tile_hovered: false,
            is_compact: false,
            pointer_over_window: false,
            toast: None,
            pinned: None,
            layout: FacesLayout::default(),
            speaker: None,
        }
    }
}

/// Who the speaker view shows: whoever is talking. Someone who takes over keeps it for at
/// least `SPEAKER_HOLD`, and the last to speak keeps it while nobody talks.
#[derive(Default)]
struct SpeakerFocus {
    identity: Option<String>,
    since: Option<Instant>,
}

impl SpeakerFocus {
    /// `people` in the order they're shown, each with whether they're speaking.
    fn update(&mut self, people: &[(&str, bool)], now: Instant) -> Option<String> {
        let here = |identity: &str| people.iter().find(|(person, _)| *person == identity);
        let current = self.identity.as_deref().and_then(here);
        if current.is_none() {
            self.identity = None;
        }
        let current_speaking = current.is_some_and(|(_, speaking)| *speaking);
        let held = current.is_some()
            && self
                .since
                .is_some_and(|since| now.saturating_duration_since(since) < SPEAKER_HOLD);
        if !current_speaking && !held {
            if let Some((next, _)) = people.iter().find(|(_, speaking)| *speaking) {
                self.identity = Some(next.to_string());
                self.since = Some(now);
            }
        }
        if self.identity.is_none() {
            self.identity = people.first().map(|(person, _)| person.to_string());
            self.since = Some(now);
        }
        self.identity.clone()
    }
}

// ── CameraWindow ────────────────────────────────────────────────────────────

pub struct CameraWindow {
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
    state: CameraState,
    call_controls: CallControlsState,
    participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    event_loop_proxy: EventLoopProxy<UserEvent>,
    redraw_tx: mpsc::Sender<RedrawCommand>,
    redraw_thread: Option<JoinHandle<()>>,
    last_rendered_frame_ids: HashMap<String, u64>,
    screensharing_active: bool,
    resize_timer: Option<Instant>,
    visible: bool,
    /// Whether the window floats over others, see `FacesOnTop`.
    on_top: bool,
    prefs: CallWindowPrefs,
    speaker_focus: SpeakerFocus,
    /// When a pinned window last moved, until it settles into a corner.
    moved_at: Option<Instant>,
    /// How many faces the pinned strip is sized for.
    strip_faces: usize,
}

pub fn camera_window_attributes() -> WindowAttributes {
    let attrs = WindowAttributes::default()
        .with_title("Hopp Camera")
        .with_inner_size(winit::dpi::LogicalSize::new(
            CAMERA_WINDOW_WIDTH,
            CAMERA_WINDOW_HEIGHT,
        ))
        .with_min_inner_size(winit::dpi::LogicalSize::new(
            CAMERA_WINDOW_MIN_WIDTH,
            CAMERA_WINDOW_MIN_HEIGHT,
        ))
        .with_resizable(true);

    #[cfg(target_os = "macos")]
    let attrs = {
        use winit::{platform::macos::WindowAttributesExtMacOS, window::WindowButtons};
        attrs
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_transparent(true)
            .with_enabled_buttons(WindowButtons::MINIMIZE)
    };
    attrs
}

impl CameraWindow {
    /// Create a new camera window with wgpu surface and iced renderer.
    pub fn new(
        context_manager: &ContextManager,
        event_loop: &ActiveEventLoop,
        participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        active_mic_name: Option<String>,
        prefs: CallWindowPrefs,
    ) -> Result<Self, CameraWindowError> {
        log::info!("CameraWindow::new");

        // ── Create winit window ──────────────────────────────────────────
        let window = event_loop
            .create_window(camera_window_attributes())
            .map_err(|e| {
                log::error!("CameraWindow: failed to create window: {e:?}");
                CameraWindowError::WindowCreation
            })?;
        // Bring to front when window is created
        window.focus_window();
        let window = Arc::new(window);

        // ── wgpu setup ───────────────────────────────────────────────────
        let surface_info = context_manager
            .create_camera_surface(&window)
            .map_err(|e| match e {
                GraphicsWindowContextError::SurfaceCreation => CameraWindowError::SurfaceCreation,
                GraphicsWindowContextError::AdapterRequest => CameraWindowError::AdapterRequest,
                GraphicsWindowContextError::DeviceRequest => CameraWindowError::DeviceRequest,
            })?;
        let device = context_manager.camera_context.device.clone();
        let format = surface_info.format;
        let alpha_mode = surface_info.alpha_mode;
        let surface = surface_info.surface;
        let physical_size = window.inner_size();

        let wgpu_renderer = iced_wgpu::Renderer::new(
            context_manager.camera_context.engine.clone(),
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
        let clipboard = Clipboard::connect(window.clone());

        #[cfg(target_os = "macos")]
        {
            super::vibrancy::apply_macos_vibrancy(&window, 8.0);
        }

        let logical = viewport.logical_size();
        let camera_active = participants
            .read()
            .ok()
            .and_then(|p| p.get("local").map(|info| info.camera_active()))
            .unwrap_or(false);
        let state = CameraState {
            viewport_size: IcedSize::new(logical.width, logical.height),
            layout: prefs.layout,
            ..Default::default()
        };
        let call_controls = CallControlsState::new(camera_active, None, active_mic_name);

        let (redraw_tx, redraw_rx) = mpsc::channel();
        let redraw_thread = Some(spawn_redraw_thread(redraw_rx, window.clone()));

        Ok(Self {
            window: window.clone(),
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
            state,
            call_controls,
            participants,
            event_loop_proxy,
            redraw_tx,
            redraw_thread,
            last_rendered_frame_ids: HashMap::new(),
            screensharing_active: false,
            resize_timer: None,
            visible: true,
            on_top: false,
            prefs,
            speaker_focus: SpeakerFocus::default(),
            moved_at: None,
            strip_faces: 0,
        })
    }

    /// Settings for the call windows, from the app.
    pub fn set_prefs(&mut self, prefs: CallWindowPrefs) {
        let restyle = self
            .state
            .pinned
            .is_some_and(|style| style != prefs.pinned_style);
        self.prefs = prefs;
        self.state.layout = prefs.layout;
        if restyle {
            self.pin_to_corner();
        }
        self.sync_compact_constraints();
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

    pub fn focus_window(&self) {
        crate::window_manager::ensure_on_screen(&self.window);
        self.window.focus_window();
    }

    /// Request a redraw of the camera window.
    pub fn request_redraw(&mut self) {
        self.window.request_redraw();
    }

    /// Stop the redraw thread by sending Stop and dropping the handle (detach).
    pub fn stop_redraw_thread(&mut self) {
        if let Err(e) = self.redraw_tx.send(RedrawCommand::Stop) {
            log::error!("CameraWindow::stop_redraw_thread: failed to send Stop: {e:?}");
        }
        self.redraw_thread.take();
    }

    pub fn has_focus(&self) -> bool {
        self.window.has_focus()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Where the window is on screen, for reactions to come from.
    pub fn frame(&self) -> Option<[f64; 4]> {
        crate::reactions::window_frame(&self.window)
    }

    pub fn set_participants(
        &mut self,
        participants: Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    ) {
        self.participants = participants;
    }

    /// Re-reveal after a hide(). Resets transient state to match a fresh window.
    pub fn show(&mut self, active_mic_name: Option<String>, screensharing_active: bool) {
        let logical = self.viewport.logical_size();
        let camera_active = self
            .participants
            .read()
            .ok()
            .and_then(|p| p.get("local").map(|info| info.camera_active()))
            .unwrap_or(false);
        // Still the size and shape it was put away at, so still pinned if it was.
        self.state = CameraState {
            viewport_size: IcedSize::new(logical.width, logical.height),
            pinned: self.state.pinned,
            self_hidden: self.state.pinned.is_some(),
            layout: self.prefs.layout,
            ..Default::default()
        };
        self.call_controls = CallControlsState::new(camera_active, None, active_mic_name);
        self.last_rendered_frame_ids.clear();
        self.screensharing_active = false;
        self.resize_timer = None;
        self.cache = Some(Cache::default());

        let _ = self.redraw_tx.send(RedrawCommand::Resume);
        self.visible = true;
        self.window.set_visible(true);
        self.focus_window();

        self.set_screensharing_active(screensharing_active);
    }

    /// Hide and pause the redraw thread (keeps the wgpu surface alive).
    pub fn hide(&mut self) {
        if let Err(e) = self.redraw_tx.send(RedrawCommand::Pause) {
            log::error!("CameraWindow::hide: failed to send Pause: {e:?}");
        }
        self.visible = false;
        self.window.set_visible(false);
    }

    /// Update the redraw interval based on screensharing state.
    /// When screensharing is active, reduces to 15 FPS to save resources.
    pub fn set_screensharing_active(&mut self, active: bool) {
        if active {
            self.screensharing_active = true;
            if self.resize_timer.is_some() {
                return;
            }
            if let Err(e) = self
                .redraw_tx
                .send(RedrawCommand::SetScreensharingActive(active))
            {
                log::error!("CameraWindow::set_screensharing_active: failed: {e:?}");
            }
        } else {
            self.screensharing_active = false;
            if self.resize_timer.is_some() {
                self.resize_timer = None;
            } else if let Err(e) = self
                .redraw_tx
                .send(RedrawCommand::SetScreensharingActive(active))
            {
                log::error!("CameraWindow::set_screensharing_active: failed: {e:?}");
            }
        }
    }

    /// Update the local camera active state. Call from StartCamera/StopCamera handlers.
    pub fn set_camera_active(&mut self, active: bool, device_name: Option<String>) {
        self.call_controls.set_camera_active(active, device_name);
    }

    pub fn set_selected_mic_name(&mut self, name: Option<String>) {
        self.call_controls.set_selected_mic_name(name);
    }

    /// Handle a winit `WindowEvent` — forward to iced and manage resize / redraw.
    pub fn handle_window_event(&mut self, event: WindowEvent) {
        let is_redraw = matches!(event, WindowEvent::RedrawRequested);

        // Process interactive events (mouse, keyboard, etc.) through the iced pipeline.
        // Skip RedrawRequested — redraw() builds its own UI internally, so processing
        // it here would double-build the widget tree on every frame.
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
                    self.state.pointer_over_window =
                        matches!(self.cursor, mouse::Cursor::Available(_));
                }

                // Build user interface, process the event, and collect messages
                let mut messages: Vec<CameraMessage> = Vec::new();

                let cache = self.cache.take().unwrap_or_default();
                let mut interface = UserInterface::build(
                    Self::view(
                        &self.state,
                        &self.call_controls,
                        &self.participants,
                        true,
                        &mut HashMap::new(),
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
                    self.update(msg);
                }
            }
        }

        // Intercept Cmd+Q to prevent app quit
        #[cfg(target_os = "macos")]
        if let WindowEvent::KeyboardInput {
            event: ref key_event,
            ..
        } = event
        {
            if let Some(key) = crate::window_manager::window_key(key_event, self.modifiers) {
                use crate::window_manager::WindowKey;
                let event = match key {
                    WindowKey::Minimise => {
                        self.window.set_minimized(true);
                        None
                    }
                    WindowKey::Close => Some(crate::UserEvent::CloseCameraWindow),
                    key => Some(crate::UserEvent::WindowKey(key)),
                };
                if let Some(event) = event {
                    let _ = self.event_loop_proxy.send_event(event);
                }
                return;
            }
            if key_event.state.is_pressed() && self.modifiers.super_key() {
                if let winit::keyboard::Key::Character(ref ch) = key_event.logical_key {
                    if ch.as_ref() == "q" {
                        log::info!("CameraWindow: caught Cmd+Q, requesting exit");
                        let _ = self
                            .event_loop_proxy
                            .send_event(crate::UserEvent::ExitRequested);
                        return;
                    }
                }
            }
        }

        // Intercept Cmd+P (macOS) / Ctrl+P (Windows/Linux) to pin to corner
        if let WindowEvent::KeyboardInput {
            event: ref key_event,
            ..
        } = event
        {
            if key_event.state.is_pressed() {
                let is_pin_modifier = if cfg!(target_os = "macos") {
                    self.modifiers.super_key()
                } else {
                    self.modifiers.control_key()
                };
                if is_pin_modifier {
                    if let winit::keyboard::Key::Character(ref ch) = key_event.logical_key {
                        if ch.as_ref() == "p" {
                            if self.state.is_compact {
                                self.unpin();
                            } else {
                                self.pin_to_corner();
                            }
                            return;
                        }
                    }
                }
            }
        }

        // Handle winit-specific events
        match event {
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.state();
            }
            WindowEvent::Moved(_) => {
                if self.state.pinned.is_some() {
                    self.moved_at = Some(Instant::now());
                }
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width > 0 && new_size.height > 0 {
                    self.viewport = Viewport::with_physical_size(
                        Size::new(new_size.width, new_size.height),
                        self.window.scale_factor() as f32,
                    );
                    let logical = self.viewport.logical_size();
                    self.state.viewport_size = IcedSize::new(logical.width, logical.height);
                    // Made bigger by hand, it's an ordinary window again.
                    if logical.width >= COMPACT_WIDTH_THRESHOLD && self.state.pinned.is_some() {
                        self.state.pinned = None;
                    }

                    self.sync_compact_constraints();

                    // Configure surface inline instead of deferring
                    self.surface.configure(
                        &self.device,
                        &wgpu::SurfaceConfiguration {
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                            format: self.format,
                            width: new_size.width,
                            height: new_size.height,
                            present_mode: wgpu::PresentMode::AutoVsync,
                            alpha_mode: self.alpha_mode,
                            view_formats: vec![],
                            desired_maximum_frame_latency: 2,
                        },
                    );

                    if self.resize_timer.is_none() {
                        self.resize_timer = Some(Instant::now() + Duration::from_secs(5));
                        if self.screensharing_active {
                            if let Err(e) = self
                                .redraw_tx
                                .send(RedrawCommand::SetScreensharingActive(false))
                            {
                                log::error!("CameraWindow::handle_window_event: failed to send SetScreensharingActive: {e:?}");
                            }
                        }
                    }

                    self.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(timer) = self.resize_timer {
                    if Instant::now() >= timer {
                        self.resize_timer = None;
                        if self.screensharing_active {
                            if let Err(e) = self
                                .redraw_tx
                                .send(RedrawCommand::SetScreensharingActive(true))
                            {
                                log::error!("CameraWindow::handle_window_event: failed to send SetScreensharingActive: {e:?}");
                            }
                        }
                    }
                }
                self.redraw();
            }
            WindowEvent::CloseRequested => {
                self.hide();
            }
            _ => {}
        }
    }

    // ── View ─────────────────────────────────────────────────────────────

    /// Build the Iced widget tree for the camera window.
    ///
    /// Layout (matching iced-poc main.rs):
    /// - Outer container: macOS near-transparent (vibrancy); otherwise Zinc900
    /// - Header row: traffic-light space + centered controls + balance space
    /// - Responsive participant grid with name labels
    /// - Without room for the controls in the header, they go below the grid (see `controls_layout`)
    fn view<'a>(
        state: &'a CameraState,
        call_controls: &'a CallControlsState,
        participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
        outer_skip: bool,
        last_rendered_frame_ids: &mut HashMap<String, u64>,
    ) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
        if state.pinned == Some(PinnedStyle::Strip) {
            return strip_view(state, participants, outer_skip, last_rendered_frame_ids);
        }
        let (controls_placement, controls_density) = controls_layout(state.viewport_size);
        let controls = call_controls
            .view(participants, controls_density)
            .map(CameraMessage::CallControls);

        // ── Participant grid ─────────────────────────────────────────────
        // Without a header, this much is still kept clear of the traffic lights and the pin button.
        let top_inset = if state.is_compact {
            COMPACT_TOP_INSET
        } else {
            HEADER_HEIGHT
        };
        let strip_height = if controls_placement == ControlsPlacement::Strip {
            CONTROLS_STRIP_HEIGHT
        } else {
            0.0
        };
        let arrangement = if state.pinned == Some(PinnedStyle::Speaker) {
            Arrangement::OnlySpeaker
        } else if state.is_compact || state.layout == FacesLayout::Grid {
            Arrangement::Grid
        } else {
            Arrangement::Speaker
        };
        let video_grid = create_participant_grid(
            GridProps {
                available_size: state.viewport_size,
                self_hidden: state.self_hidden,
                local_tile_hovered: state.local_tile_hovered,
                reserved_height: top_inset + strip_height,
                outer_skip,
                arrangement,
                speaker: state.speaker.as_deref(),
            },
            participants,
            last_rendered_frame_ids,
        );

        // ── Main layout ─────────────────────────────────────────────────
        let top_space = Space::new().height(Length::Fixed(top_inset));
        let mut hover_controls = None;
        let content = match controls_placement {
            ControlsPlacement::Header => {
                // ── Header with centered controls (space for native traffic lights)
                let header = row![
                    Space::new().width(Length::Fixed(HEADER_SIDE_WIDTH)),
                    Space::new().width(Length::Fill),
                    controls,
                    Space::new().width(Length::Fill),
                    Space::new().width(Length::Fixed(HEADER_SIDE_WIDTH)),
                ]
                .padding(Padding {
                    top: CONTENT_PADDING,
                    right: CONTENT_PADDING,
                    bottom: 0.0,
                    left: CONTENT_PADDING,
                })
                .align_y(Alignment::Center);
                column![header, video_grid]
            }
            ControlsPlacement::Strip => {
                let strip = container(controls)
                    .center_x(Length::Fill)
                    .height(Length::Fixed(CONTROLS_STRIP_HEIGHT));
                column![top_space, video_grid, strip]
            }
            ControlsPlacement::Hover => {
                if state.pointer_over_window {
                    hover_controls = Some(controls);
                }
                column![top_space, video_grid]
            }
        }
        .width(Length::Fill)
        .height(Length::Fill);

        // What the controls take from the bottom of the window, for what floats there to avoid.
        let bottom_inset = if hover_controls.is_some() {
            CONTROLS_STRIP_HEIGHT
        } else {
            strip_height
        };

        let base_inner = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme: &Theme| {
                let bg = if cfg!(target_os = "macos") {
                    Color::from_rgba(0.0, 0.0, 0.0, 0.05)
                } else {
                    ColorToken::Zinc900.to_color()
                };
                container::Style {
                    background: Some(Background::Color(bg)),
                    border: Border {
                        radius: 10.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .clip(true);

        // Only the header's controls have these menus, see `sync_compact_constraints`.
        let trailing_padding = (state.viewport_size.width - controls_density.total_width()) / 2.0;
        let base = call_controls.wrap_dropdown(
            base_inner.into(),
            CameraMessage::CallControls,
            controls_density,
            HEADER_HEIGHT + 2.0,
            trailing_padding,
        );

        let floating_show_btn = if state.self_hidden {
            Some(
                container(self_visibility_button(true))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(Padding::new(4.0).bottom(4.0 + bottom_inset))
                    .into(),
            )
        } else {
            None
        };

        let toast_position: ToastPosition = if state.viewport_size.width < 500.0 {
            ToastPosition {
                top: None,
                right: Some(15.0),
                bottom: Some(15.0 + bottom_inset),
                left: None,
            }
        } else {
            ToastPosition {
                top: Some(15.0),
                right: Some(15.0),
                bottom: None,
                left: None,
            }
        };

        let mut layers: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> = vec![base];
        if let Some(floating) = floating_show_btn {
            layers.push(floating);
        }
        if let Some(controls) = hover_controls {
            layers.push(
                container(controls)
                    .center_x(Length::Fill)
                    .height(Length::Fill)
                    .align_y(Alignment::End)
                    .padding(Padding::new(0.0).bottom(4.0))
                    .into(),
            );
        }
        if !state.is_compact {
            layers.push(
                container(row![layout_button(state.layout), pin_button()].spacing(6))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::Start)
                    .padding(Padding {
                        top: 8.0,
                        right: 8.0,
                        bottom: 0.0,
                        left: 0.0,
                    })
                    .into(),
            );
        }
        if let Some(toast_el) = toast::toast_view(&state.toast, Some(&toast_position)) {
            layers.push(toast_el);
        }

        stack(layers).into()
    }

    pub fn show_error_toast(&mut self, message: &str) {
        self.state.toast = Some(toast::show_toast(
            message.to_string(),
            3000,
            ToastPosition {
                top: Some(15.0),
                right: Some(15.0),
                bottom: None,
                left: None,
            },
        ));
    }

    /// Handle a camera UI message (state update).
    fn update(&mut self, message: CameraMessage) {
        match message {
            CameraMessage::CallControls(message) => {
                self.call_controls
                    .update(message, &self.participants, &self.event_loop_proxy)
            }
            CameraMessage::ToggleSelfVisibility => {
                self.state.self_hidden = !self.state.self_hidden;
                if self.state.self_hidden {
                    self.state.local_tile_hovered = false;
                }
            }
            CameraMessage::LocalTileHover(hovered) => {
                self.state.local_tile_hovered = hovered;
            }
            CameraMessage::PinToCorner => {
                if self.state.is_compact {
                    self.unpin();
                } else {
                    self.pin_to_corner();
                }
            }
            CameraMessage::ToggleLayout => {
                self.prefs.layout = match self.prefs.layout {
                    FacesLayout::Grid => FacesLayout::Speaker,
                    FacesLayout::Speaker => FacesLayout::Grid,
                };
                self.state.layout = self.prefs.layout;
                self.send_prefs();
            }
            CameraMessage::DragWindow => {
                if let Err(e) = self.window.drag_window() {
                    log::warn!("CameraWindow: couldn't drag the window: {e:?}");
                }
            }
            CameraMessage::ViewScreen => {
                let open = UserEvent::OpenScreenShareWindow {
                    participants: Arc::new(RwLock::new(HashMap::new())),
                    sharer_identity: None,
                    redraw_rx: None,
                    redraw_tx: None,
                };
                if let Err(e) = self.event_loop_proxy.send_event(open) {
                    log::error!("CameraWindow: failed to open the shared screen: {e:?}");
                }
            }
        }
    }

    /// Recompute what follows from the current viewport size.
    fn sync_compact_constraints(&mut self) {
        self.state.is_compact = self.state.viewport_size.width < COMPACT_WIDTH_THRESHOLD;

        // By default the faces float over everything only while pinned. Full size, they're a
        // window like any other, so they don't sit on top of a shared screen.
        let on_top = match self.prefs.faces_on_top {
            FacesOnTop::Always => true,
            FacesOnTop::WhenPinned => self.state.is_compact,
            FacesOnTop::Never => false,
        };
        if self.on_top != on_top {
            self.on_top = on_top;
            self.window.set_window_level(if self.on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        }

        // Only the header has the device menus. One left open would come back with the header.
        let (controls_placement, _) = controls_layout(self.state.viewport_size);
        if controls_placement != ControlsPlacement::Header {
            self.call_controls.dismiss_dropdowns();
        }
    }

    /// Shrinks the window into a corner of its screen, in the style from Settings, hiding
    /// the local participant.
    fn pin_to_corner(&mut self) {
        let style = self.prefs.pinned_style;
        self.state.pinned = Some(style);
        self.state.self_hidden = true;
        self.state.local_tile_hovered = false;
        // The window is about to move out from under the pointer.
        self.cursor = mouse::Cursor::Unavailable;
        self.state.pointer_over_window = false;

        let strip = style == PinnedStyle::Strip;
        self.set_bare(strip);
        // The strip is sized by how many faces it has, not by hand.
        self.window.set_resizable(!strip);
        self.strip_faces = self.count_strip_faces();
        let (width, height) = self.pinned_size();
        self.window
            .set_min_inner_size(Some(winit::dpi::LogicalSize::new(
                width.min(CAMERA_WINDOW_MIN_WIDTH),
                height.min(CAMERA_WINDOW_MIN_HEIGHT),
            )));
        let _ = self
            .window
            .request_inner_size(winit::dpi::LogicalSize::new(width, height));
        self.place_in_corner(self.prefs.pinned_corner, (width, height));
    }

    /// Restore the window to its default size, centered on the current monitor.
    fn unpin(&mut self) {
        self.state.self_hidden = false;
        self.state.pinned = None;
        self.moved_at = None;
        self.set_bare(false);
        self.window.set_resizable(true);
        self.window
            .set_min_inner_size(Some(winit::dpi::LogicalSize::new(
                CAMERA_WINDOW_MIN_WIDTH,
                CAMERA_WINDOW_MIN_HEIGHT,
            )));

        let monitor = self
            .window
            .current_monitor()
            .or_else(|| self.window.available_monitors().next());

        let _ = self.window.request_inner_size(winit::dpi::LogicalSize::new(
            CAMERA_WINDOW_WIDTH,
            CAMERA_WINDOW_HEIGHT,
        ));

        if let Some(monitor) = monitor {
            let scale = monitor.scale_factor();
            let mon_size: winit::dpi::LogicalSize<f64> = monitor.size().to_logical(scale);
            let mon_pos: winit::dpi::LogicalPosition<f64> = monitor.position().to_logical(scale);

            let x = mon_pos.x + (mon_size.width - CAMERA_WINDOW_WIDTH) / 2.0;
            let y = mon_pos.y + (mon_size.height - CAMERA_WINDOW_HEIGHT) / 2.0;

            self.window
                .set_outer_position(winit::dpi::LogicalPosition::new(x, y));
        }
    }

    /// The strip has no frame, shadow or frosted glass: only its faces.
    fn set_bare(&self, bare: bool) {
        crate::window_manager::set_bare(&self.window, bare);
        #[cfg(target_os = "macos")]
        super::vibrancy::set_macos_vibrancy_hidden(&self.window, bare);
    }

    fn pinned_size(&self) -> (f64, f64) {
        match self.state.pinned {
            Some(PinnedStyle::Strip) => strip_size(self.strip_faces),
            Some(PinnedStyle::Speaker) => (PIN_SPEAKER_WIDTH, PIN_SPEAKER_HEIGHT),
            _ => (PIN_BOX_WIDTH, PIN_BOX_HEIGHT),
        }
    }

    fn count_strip_faces(&self) -> usize {
        let participants = self.participants.read().unwrap();
        let (faces, more) = strip_people(&participants, self.state.self_hidden);
        faces.len() + usize::from(more > 0)
    }

    fn place_in_corner(&self, corner: ScreenCorner, size: (f64, f64)) {
        let Some(area) = crate::window_manager::usable_area(&self.window) else {
            return;
        };
        let (x, y) = corner_position(corner, size, area);
        self.window
            .set_outer_position(winit::dpi::LogicalPosition::new(x, y));
    }

    /// The strip grows and shrinks as people come and go, staying in its corner.
    fn sync_strip_size(&mut self) {
        if self.state.pinned != Some(PinnedStyle::Strip) {
            return;
        }
        let faces = self.count_strip_faces();
        if faces == self.strip_faces {
            return;
        }
        self.strip_faces = faces;
        let size = strip_size(faces);
        let _ = self
            .window
            .request_inner_size(winit::dpi::LogicalSize::new(size.0, size.1));
        self.place_in_corner(self.prefs.pinned_corner, size);
    }

    /// A pinned window let go after a drag goes to the nearest corner, and stays in that
    /// corner the next time it's pinned.
    fn settle_in_corner(&mut self) {
        let Some(moved_at) = self.moved_at else {
            return;
        };
        if moved_at.elapsed() < SNAP_DELAY || crate::window_manager::mouse_button_held() {
            return;
        }
        self.moved_at = None;
        if self.state.pinned.is_none() {
            return;
        }
        let Some(area) = crate::window_manager::usable_area(&self.window) else {
            return;
        };
        let Ok(position) = self.window.outer_position() else {
            return;
        };
        let scale = self.window.scale_factor();
        let position: winit::dpi::LogicalPosition<f64> = position.to_logical(scale);
        let size: winit::dpi::LogicalSize<f64> = self.window.outer_size().to_logical(scale);
        let centre = (
            position.x + size.width / 2.0,
            position.y + size.height / 2.0,
        );
        let corner = nearest_corner(centre, area);
        let (x, y) = corner_position(corner, (size.width, size.height), area);
        // Its own move here comes back as another, which finds it already in place.
        if (x - position.x).abs() > 1.0 || (y - position.y).abs() > 1.0 {
            self.window
                .set_outer_position(winit::dpi::LogicalPosition::new(x, y));
        }
        if corner != self.prefs.pinned_corner {
            log::info!("CameraWindow: pinned to {corner:?}");
            self.prefs.pinned_corner = corner;
            self.send_prefs();
        }
    }

    /// For the app to remember what was changed here.
    fn send_prefs(&self) {
        if let Err(e) = self
            .event_loop_proxy
            .send_event(UserEvent::CallWindowPrefsChanged(self.prefs))
        {
            log::error!("CameraWindow: failed to send CallWindowPrefsChanged: {e:?}");
        }
    }

    /// Who the speaker view shows: someone else when there's anyone else.
    fn sync_speaker(&mut self) {
        let participants = self.participants.read().unwrap();
        let mut people: Vec<(&str, bool)> = participants
            .iter()
            .filter(|(identity, _)| identity.as_str() != "local")
            .map(|(identity, info)| (identity.as_str(), info.is_speaking()))
            .collect();
        people.sort_by_key(|(identity, _)| participants[*identity].name());
        if people.is_empty() {
            people.extend(
                participants
                    .get("local")
                    .map(|info| ("local", info.is_speaking())),
            );
        }
        let speaker = self.speaker_focus.update(&people, Instant::now());
        drop(participants);
        self.state.speaker = speaker;
    }

    /// Perform a full redraw: build UI, draw, present.
    fn redraw(&mut self) {
        self.sync_compact_constraints();
        self.sync_speaker();
        self.sync_strip_size();
        self.settle_in_corner();
        toast::tick_toast(&mut self.state.toast);

        let output = match self.surface.get_current_texture() {
            Ok(output) => output,
            Err(e) => {
                log::error!("CameraWindow::redraw: failed to get texture: {e:?}");
                return;
            }
        };
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Build fresh interface from cache
        let cache = self.cache.take().unwrap_or_default();
        let mut interface = UserInterface::build(
            Self::view(
                &self.state,
                &self.call_controls,
                &self.participants,
                false,
                &mut self.last_rendered_frame_ids,
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
        let clear_color = if cfg!(target_os = "macos") {
            Some(Color::TRANSPARENT)
        } else {
            Some(ColorToken::Zinc900.to_color())
        };
        wgpu_renderer.present(clear_color, output.texture.format(), &view, &self.viewport);

        self.window.pre_present_notify();
        output.present();
    }
}

impl Drop for CameraWindow {
    fn drop(&mut self) {
        self.stop_redraw_thread();
    }
}

// ── Styling helper functions (ported from iced-poc main.rs) ─────────────────

/// Truncate a name to a maximum of 16 characters, adding "..." if truncated.
fn truncate_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() > 16 {
        chars[..16].iter().collect::<String>() + "..."
    } else {
        name.to_string()
    }
}

/// Create a name label badge.
///
/// The label never changes with who is speaking; only the slot before the name does
/// (see [`voice_slot`]), so the name never moves. As on Linux.
fn name_label<'a>(
    name: &str,
    is_small_window: bool,
    is_speaking: bool,
    is_muted: bool,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let font_size = if is_small_window { 12.0 } else { 16.0 };
    let padding = if is_small_window {
        Padding::from([4, 10])
    } else {
        Padding::from([6, 16])
    };

    let display_name = truncate_name(name);
    let name_el = text(display_name)
        .size(font_size)
        .color(Color::WHITE)
        .font(GEIST_MEDIUM)
        .align_y(Alignment::Center);
    let slot = voice_slot(name, font_size * 0.85, is_speaking, is_muted);
    let label_row = row![slot, Space::new().width(Length::Fixed(6.0)), name_el]
        .spacing(0.0)
        .align_y(Alignment::Center);

    container(label_row)
        .padding(padding)
        .style(move |_theme: &Theme| {
            // Linear gradient from top to bottom (180deg = PI radians)
            let grad = gradient::Linear::new(Radians(std::f32::consts::PI))
                .add_stop(0.0, ColorToken::Slate950.to_color())
                .add_stop(1.0, ColorToken::Slate900.to_color());
            container::Style {
                background: Some(Background::Gradient(grad.into())),
                border: Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.3), // White with 30% opacity
                    width: 1.0,
                    radius: 20.0.into(),
                },
                shadow: ShadowToken::Xl.to_shadow(),
                ..Default::default()
            }
        })
        .into()
}

/// Before someone's name: a dim mic, green bars that move while they speak, or a red
/// crossed-out mic when they're muted. A square of fixed `size`, so nothing beside it
/// moves when it changes.
fn voice_slot<'a>(
    name: &str,
    size: f32,
    is_speaking: bool,
    is_muted: bool,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let content: iced::Element<'a, CameraMessage, Theme, iced::Renderer> = if is_muted {
        text(ICON_MICROPHONE_OFF.to_string())
            .font(ICONS_FONT)
            .size(size)
            .color(ColorToken::Red400.to_color())
            .align_y(Alignment::Center)
            .into()
    } else if is_speaking {
        let seed = name.bytes().fold(0u32, |sum, byte| {
            sum.wrapping_mul(31).wrapping_add(byte as u32)
        }) % 97;
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |since| (since.as_millis() % 1_000_000) as f32 / 1000.0);
        let bar = (size / 5.0).max(2.0);
        let mut bars = row![].spacing(bar).align_y(Alignment::Center);
        for height in bar_heights(seed as f32, seconds) {
            bars = bars.push(
                container(Space::new())
                    .width(Length::Fixed(bar))
                    .height(Length::Fixed(height * size))
                    .style(move |_theme: &Theme| container::Style {
                        background: Some(Background::Color(ColorToken::Green400.to_color())),
                        border: Border {
                            radius: (bar / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        bars.into()
    } else {
        let dim = ColorToken::Slate400.to_color();
        svg(svg::Handle::from_memory(ICON_MICROPHONE_SVG))
            .width(Length::Fixed(size))
            .height(Length::Fixed(size))
            .style(move |_theme: &Theme, _status| svg::Style { color: Some(dim) })
            .into()
    };
    container(content)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center_x(Length::Fixed(size))
        .center_y(Length::Fixed(size))
        .into()
}

/// The three bars' heights as fractions of the slot, 0.2 to 1. LiveKit says who is
/// speaking but not how loudly, so they move steadily while someone speaks. Linux's
/// `faces::bar_heights` is the same, so both apps look alike.
fn bar_heights(seed: f32, seconds: f32) -> [f32; 3] {
    let lift = 0.35 + 0.65 * ((seconds * 7.3 + seed).sin() * (seconds * 2.1 + seed).sin()).abs();
    let mut heights = [0.0; 3];
    for (index, scale) in [0.7, 1.0, 0.55].into_iter().enumerate() {
        let wobble = 1.0 + 0.3 * (seconds * 11.0 + index as f32 * 2.0).sin();
        heights[index] = 0.2 + (lift * scale * wobble).clamp(0.0, 1.0) * 0.8;
    }
    heights
}

fn get_initials(name: &str) -> Vec<String> {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.len() {
        0 => vec!["🤷".to_string()],
        1 => parts[0]
            .chars()
            .next()
            .map(|c| vec![c.to_uppercase().to_string()])
            .unwrap_or_default(),
        _ => {
            let first = parts[0]
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            let last = parts
                .last()
                .unwrap()
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            vec![first, last]
        }
    }
}

fn initials_avatar<'a>(
    name: &str,
    tile_size: f32,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let scale = ((tile_size - 16.0).max(40.0) / AVATAR_SIZE).min(1.0);
    let avatar_size = AVATAR_SIZE * scale;
    let font_size = AVATAR_FONT_SIZE * scale;
    let radius = AVATAR_RADIUS * scale;
    let letter_spacing = AVATAR_LETTER_SPACING * scale;
    let initials = get_initials(name);

    let initials_row: iced::Element<'a, CameraMessage, Theme, iced::Renderer> = {
        let mut r = row![];
        for (i, ch) in initials.iter().enumerate() {
            if i > 0 {
                r = r.push(Space::new().width(Length::Fixed(letter_spacing)));
            }
            r = r.push(
                text(ch.clone())
                    .size(font_size)
                    .color(Color::WHITE)
                    .font(GEIST_MEDIUM),
            );
        }
        r.align_y(Alignment::Center).into()
    };

    let inner = container(initials_row)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Violet800.to_color())),
            border: Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    // Inner shadow approximation: white border overlay
    // iced Shadow has no inset mode; approximate with a semi-transparent white border
    // https://discourse.iced.rs/t/advanced-widget-rendering-styling/896
    let inner_shadow_overlay = container(Space::new())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
                width: 1.0,
                radius: radius.into(),
            },
            ..Default::default()
        });

    container(iced::widget::stack![inner, inner_shadow_overlay])
        .width(Length::Fixed(avatar_size))
        .height(Length::Fixed(avatar_size))
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                radius: radius.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.03),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: 32.0,
            },
            ..Default::default()
        })
        .into()
}

/// Tile corner radius for participant cards (Figma: 6px).
const TILE_RADIUS: f32 = 6.0;

fn self_visibility_button<'a>(
    is_hidden: bool,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let icon_data = if is_hidden {
        ICON_EYE_ON_SVG
    } else {
        ICON_EYE_OFF_SVG
    };
    let tooltip_text = if is_hidden {
        "Show your camera"
    } else {
        "Hide yourself"
    };

    let icon_handle = svg::Handle::from_memory(icon_data);
    let slate300 = ColorToken::Slate300.to_color();
    let icon = svg(icon_handle)
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(move |_theme: &Theme, _status| svg::Style {
            color: Some(slate300),
        });

    let btn = button(
        container(icon)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(38.0))
    .height(Length::Fixed(26.0))
    .on_press(CameraMessage::ToggleSelfVisibility)
    .padding(Padding::from([6.0, 12.0]))
    .style(move |_theme: &Theme, status| {
        let bg_color = match status {
            button::Status::Hovered => ColorToken::Slate600.to_color(),
            button::Status::Pressed => ColorToken::Slate800.to_color(),
            _ => ColorToken::Slate700.to_color(),
        };
        button::Style {
            background: Some(Background::Color(bg_color)),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
                width: 1.0,
                radius: 19.0.into(),
            },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        }
    });

    let tooltip_content = container(text(tooltip_text).size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: ShadowToken::Xs.to_shadow(),
            ..Default::default()
        });

    let tip_pos = if is_hidden {
        tooltip::Position::Left
    } else {
        tooltip::Position::Top
    };

    tooltip(btn, tooltip_content, tip_pos)
        .gap(1)
        .snap_within_viewport(true)
        .into()
}

/// Shows the layout you'd switch to.
fn layout_button<'a>(
    layout: FacesLayout,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let (icon_data, tooltip_text) = match layout {
        FacesLayout::Grid => (ICON_LAYOUT_SPEAKER_SVG, "Speaker view"),
        FacesLayout::Speaker => (ICON_LAYOUT_GRID_SVG, "Everyone the same size"),
    };
    let slate300 = ColorToken::Slate300.to_color();
    let icon = svg(svg::Handle::from_memory(icon_data))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(move |_theme: &Theme, _status| svg::Style {
            color: Some(slate300),
        });

    let btn = button(
        container(icon)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(38.0))
    .height(Length::Fixed(26.0))
    .on_press(CameraMessage::ToggleLayout)
    .padding(Padding::from([6.0, 12.0]))
    .style(move |_theme: &Theme, status| {
        let bg_color = match status {
            button::Status::Hovered => ColorToken::Slate600.to_color(),
            button::Status::Pressed => ColorToken::Slate800.to_color(),
            _ => ColorToken::Slate700.to_color(),
        };
        button::Style {
            background: Some(Background::Color(bg_color)),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
                width: 1.0,
                radius: 19.0.into(),
            },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        }
    });

    let tooltip_content = container(text(tooltip_text).size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: ShadowToken::Xs.to_shadow(),
            ..Default::default()
        });

    tooltip(btn, tooltip_content, tooltip::Position::Bottom)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}

fn pin_button<'a>() -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let shortcut = if cfg!(target_os = "macos") {
        "⌘P"
    } else {
        "Ctrl+P"
    };
    let tooltip_text = format!("Pin to corner ({shortcut})");

    let slate300 = ColorToken::Slate300.to_color();
    let icon = text(ICON_PIN_ANGLE.to_string())
        .font(ICONS_FONT)
        .size(14.0)
        .color(slate300)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);

    let btn = button(
        container(icon)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(38.0))
    .height(Length::Fixed(26.0))
    .on_press(CameraMessage::PinToCorner)
    .padding(Padding::from([6.0, 12.0]))
    .style(move |_theme: &Theme, status| {
        let bg_color = match status {
            button::Status::Hovered => ColorToken::Slate600.to_color(),
            button::Status::Pressed => ColorToken::Slate800.to_color(),
            _ => ColorToken::Slate700.to_color(),
        };
        button::Style {
            background: Some(Background::Color(bg_color)),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
                width: 1.0,
                radius: 19.0.into(),
            },
            text_color: Color::WHITE,
            shadow: Shadow::default(),
            snap: false,
        }
    });

    let tooltip_content = container(text(tooltip_text).size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: ShadowToken::Xs.to_shadow(),
            ..Default::default()
        });

    tooltip(btn, tooltip_content, tooltip::Position::Left)
        .gap(1)
        .snap_within_viewport(true)
        .into()
}

/// Create a participant card tile.
///
struct ParticipantCardProps<'a> {
    pub(super) participant_id: u64,
    pub(super) name: &'a str,
    pub(super) is_speaking: bool,
    pub(super) is_muted: bool,
    pub(super) is_capturing: bool,
    pub(super) is_sharing: bool,
    pub(super) buffers: Arc<VideoBufferManager>,
    pub(super) tile: TileSize,
    pub(super) is_small_window: bool,
    pub(super) is_local: bool,
    pub(super) local_tile_hovered: bool,
    pub(super) hide_name: bool,
    pub(super) skip_buffer: bool,
}

/// If the participant has camera buffers, renders GPU-accelerated video via the
/// shader widget. Otherwise, falls back to a solid Slate900 placeholder.
/// - Overlaid name label at bottom-left
/// - Shadow on outer container
/// - Clipped with rounded corners (8px radius)
fn participant_card<'a>(
    props: ParticipantCardProps<'_>,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let ParticipantCardProps {
        participant_id,
        name,
        is_speaking,
        is_muted,
        is_capturing,
        is_sharing,
        buffers,
        tile,
        is_small_window,
        is_local,
        local_tile_hovered,
        hide_name,
        skip_buffer,
    } = props;

    // Adjust padding based on window size
    let overlay_padding = if is_small_window { 8.0 } else { 14.0 };
    let name_owned = name.to_string();

    // Background layer: GPU video if buffer exists and is active, Slate900 fallback otherwise
    let bg_element: iced::Element<'a, CameraMessage, Theme, iced::Renderer> =
        if buffers.is_inactive() {
            container(initials_avatar(name, tile.width.min(tile.height)))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_theme: &Theme| container::Style {
                    background: Some(Background::Color(ColorToken::Slate900.to_color())),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: TILE_RADIUS.into(),
                    },
                    ..Default::default()
                })
                .into()
        } else {
            // Render video when stream is active
            let video_program = YuvVideoProgram {
                participant_id,
                buffer: buffers,
                corner_radius: TILE_RADIUS,
                stretch_to_fill: false,
                skip_upload: skip_buffer,
                mirror: is_local,
            };
            let video_bg: iced::widget::Shader<CameraMessage, _> = shader(video_program)
                .width(Length::Fill)
                .height(Length::Fill);
            video_bg.into()
        };
    // Overlay with name label at bottom-left
    let overlay = container(
        column![
            Space::new().height(Length::Fill),
            name_label(&name_owned, is_small_window, is_speaking, is_muted),
        ]
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(overlay_padding);

    let eye_button_overlay: Option<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> =
        if is_local && local_tile_hovered {
            Some(
                container(self_visibility_button(false))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End)
                    .padding(4.0)
                    .into(),
            )
        } else {
            None
        };

    let mut tile_layers: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> =
        vec![bg_element];
    if !hide_name {
        tile_layers.push(overlay.into());
    } else if is_speaking || is_muted {
        // Too small for a name: the slot alone, while it has something to say.
        let chip = container(voice_slot(&name_owned, 10.0, is_speaking, is_muted))
            .padding(4.0)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.6))),
                border: Border {
                    radius: 100.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        tile_layers.push(
            container(chip)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Start)
                .align_y(Alignment::End)
                .padding(4.0)
                .into(),
        );
    }
    if let Some(eye_btn) = eye_button_overlay {
        tile_layers.push(eye_btn);
    }
    // Shown however small the tile: nobody should be captured without knowing.
    if is_capturing {
        tile_layers.push(
            container(capturing_badge())
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Start)
                .align_y(Alignment::Start)
                .padding(if hide_name { 4.0 } else { overlay_padding })
                .into(),
        );
    }
    // Who is sharing, however small the tile, and a way to their screen.
    if is_sharing {
        tile_layers.push(
            container(sharing_badge(is_local, hide_name))
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::Start)
                .padding(if hide_name { 4.0 } else { overlay_padding })
                .into(),
        );
    }

    // Clipped content: video + name label (+ local hide control), masked to rounded corners.
    let clipped_content = container(stack(tile_layers))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                radius: TILE_RADIUS.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .clip(true);

    // Border overlay drawn ON TOP of video so the shader can't cover it.
    let border_frame = container(Space::new())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme: &Theme| container::Style {
            border: if is_speaking {
                Border {
                    color: ColorToken::Green400.to_color(),
                    width: 2.0,
                    radius: TILE_RADIUS.into(),
                }
            } else if is_sharing {
                Border {
                    color: ColorToken::Emerald500.to_color(),
                    width: 2.0,
                    radius: TILE_RADIUS.into(),
                }
            } else {
                Border {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                    width: 1.0,
                    radius: TILE_RADIUS.into(),
                }
            },
            ..Default::default()
        });

    // Stack: clipped video below, border on top.
    let tile = container(iced::widget::stack![clipped_content, border_frame])
        .width(Length::Fixed(tile.width))
        .height(Length::Fixed(tile.height))
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                radius: TILE_RADIUS.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.1),
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: 7.0,
            },
            ..Default::default()
        });

    if is_local {
        mouse_area(tile)
            .on_enter(CameraMessage::LocalTileHover(true))
            .on_exit(CameraMessage::LocalTileHover(false))
            .into()
    } else {
        tile.into()
    }
}

/// Someone's tile in another window: the same as theirs here, without this window's buttons.
/// `nothing` is that window's message for a tile that doesn't do anything.
pub(crate) fn face_card<'a, M: Clone + 'a>(
    identity: &str,
    info: &ParticipantInfo,
    (width, height): (f32, f32),
    skip_buffer: bool,
    nothing: M,
) -> iced::Element<'a, M, Theme, iced::Renderer> {
    participant_card(ParticipantCardProps {
        participant_id: identity_to_id(identity),
        name: info.name(),
        is_speaking: info.is_speaking(),
        is_muted: info.muted(),
        is_capturing: info.capturing(),
        // You're looking at their screen already.
        is_sharing: false,
        buffers: info.camera_buffers(),
        tile: TileSize { width, height },
        is_small_window: true,
        is_local: identity == "local",
        local_tile_hovered: false,
        hide_name: width < FACE_CARD_NAME_MIN_WIDTH,
        skip_buffer,
    })
    .map(move |_| nothing.clone())
}

/// "Sharing" on the sharer's tile. On someone else's, it's a button to their screen.
fn sharing_badge<'a>(
    is_local: bool,
    icon_only: bool,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let ink = ColorToken::Emerald950.to_color();
    let icon = text(ICON_SCREEN_SHARE.to_string())
        .font(ICONS_FONT)
        .size(12.0)
        .color(ink);
    let content: iced::Element<'a, CameraMessage, Theme, iced::Renderer> = if icon_only {
        icon.into()
    } else {
        row![icon, text("Sharing").font(GEIST_MEDIUM).size(12).color(ink)]
            .spacing(5)
            .align_y(Alignment::Center)
            .into()
    };
    let padding = if icon_only {
        Padding::from([4.0, 6.0])
    } else {
        Padding::from([3.0, 8.0])
    };
    let pill = move |hovered: bool| container::Style {
        background: Some(Background::Color(if hovered {
            ColorToken::Emerald300.to_color()
        } else {
            ColorToken::Emerald400.to_color()
        })),
        border: Border {
            radius: 100.0.into(),
            ..Default::default()
        },
        ..Default::default()
    };

    let (badge, hint): (iced::Element<'a, CameraMessage, Theme, iced::Renderer>, _) = if is_local {
        let badge = container(content)
            .padding(padding)
            .style(move |_theme: &Theme| pill(false));
        (badge.into(), "You're sharing your screen")
    } else {
        let badge = button(content)
            .padding(padding)
            .on_press(CameraMessage::ViewScreen)
            .style(move |_theme: &Theme, status| {
                let style = pill(matches!(
                    status,
                    button::Status::Hovered | button::Status::Pressed
                ));
                button::Style {
                    background: style.background,
                    border: style.border,
                    text_color: ink,
                    shadow: Shadow::default(),
                    snap: false,
                }
            });
        (badge.into(), "Show their screen")
    };

    let hint = container(text(hint).size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: ShadowToken::Xs.to_shadow(),
            ..Default::default()
        });

    tooltip(badge, hint, tooltip::Position::Left)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}

fn capturing_badge<'a>() -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let icon = svg(svg::Handle::from_memory(ICON_CAPTURE))
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(|_theme: &Theme, _status| svg::Style {
            color: Some(Color::WHITE),
        });

    let badge = container(icon)
        .padding(Padding::from([5.0, 7.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Rose500.to_color())),
            border: Border {
                radius: 100.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    let hint = container(text("Capturing this call").size(12).color(Color::WHITE))
        .padding(Padding::from([4.0, 8.0]))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Gray600.to_color())),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
                width: 1.0,
                radius: 6.0.into(),
            },
            shadow: ShadowToken::Xs.to_shadow(),
            ..Default::default()
        });

    tooltip(badge, hint, tooltip::Position::Right)
        .gap(4)
        .snap_within_viewport(true)
        .into()
}

/// Hash an identity string to a u64 for GPU texture keying.
pub(crate) fn identity_to_id(identity: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    identity.hash(&mut hasher);
    hasher.finish()
}

/// Where the call controls go, in a window of this size.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ControlsPlacement {
    /// In the header, between the traffic lights and the pin button.
    Header,
    /// In a strip of their own, below the video.
    Strip,
    /// Over the bottom of the video, while the pointer is on the window.
    Hover,
}

/// Where the call controls go in a window of this size, and how much of them fits there.
fn controls_layout(viewport_size: IcedSize) -> (ControlsPlacement, CallControlsDensity) {
    if viewport_size.width >= REGULAR_CONTROLS_MIN_WIDTH {
        return (ControlsPlacement::Header, CallControlsDensity::Regular);
    }
    if viewport_size.width >= HEADER_CONTROLS_MIN_WIDTH {
        return (ControlsPlacement::Header, CallControlsDensity::Compact);
    }
    let placement = if viewport_size.height >= CONTROLS_STRIP_MIN_WINDOW_HEIGHT {
        ControlsPlacement::Strip
    } else {
        ControlsPlacement::Hover
    };
    let density = if viewport_size.width >= ICON_CONTROLS_MIN_WIDTH {
        CallControlsDensity::Icons
    } else {
        CallControlsDensity::Essentials
    };
    (placement, density)
}

/// A tile's size, in logical points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct TileSize {
    width: f32,
    height: f32,
}

impl TileSize {
    fn area(self) -> f32 {
        self.width * self.height
    }
}

/// The biggest tile that fits in a cell this size: as wide as the cell allows, between
/// square and a camera's 16:9.
fn fit_tile(cell_width: f32, cell_height: f32) -> TileSize {
    if cell_width <= 0.0 || cell_height <= 0.0 {
        return TileSize {
            width: 0.0,
            height: 0.0,
        };
    }
    let aspect = (cell_width / cell_height).clamp(SQUAREST_TILE, WIDEST_TILE);
    let width = cell_width.min(cell_height * aspect);
    TileSize {
        width,
        height: width / aspect,
    }
}

/// Columns, rows and tile size that show `count` people as big as they'll go in this space.
fn grid_layout(count: usize, width: f32, height: f32) -> (usize, usize, TileSize) {
    let mut best = (1, count, fit_tile(0.0, 0.0));
    for cols in 1..=count {
        let rows = count.div_ceil(cols);
        let tile = fit_tile(
            (width - TILE_SPACING * (cols - 1) as f32) / cols as f32,
            (height - TILE_SPACING * (rows - 1) as f32) / rows as f32,
        );
        if tile.area() > best.2.area() {
            best = (cols, rows, tile);
        }
    }
    best
}

/// The speaker's tile, and the tiles in the row of `others` below it.
fn speaker_layout(others: usize, width: f32, height: f32) -> (TileSize, TileSize) {
    if others == 0 {
        return (fit_tile(width, height), fit_tile(0.0, 0.0));
    }
    let row_height =
        (height * SPEAKER_ROW_SHARE).clamp(SPEAKER_ROW_MIN_HEIGHT, SPEAKER_ROW_MAX_HEIGHT);
    let cell_width = (width - TILE_SPACING * (others - 1) as f32) / others as f32;
    let small = fit_tile(cell_width.min(row_height * WIDEST_TILE), row_height);
    let big = fit_tile(width, height - small.height - TILE_SPACING);
    (big, small)
}

/// The strip's window, for this many faces.
fn strip_size(faces: usize) -> (f64, f64) {
    let faces = faces.max(1) as f32;
    let height = 2.0 * STRIP_PADDING
        + faces * STRIP_SPEAKER_FACE
        + (faces - 1.0) * STRIP_GAP
        + STRIP_CONTROLS_HEIGHT;
    (STRIP_WIDTH as f64, height as f64)
}

/// Who the strip shows, in order, and how many more there are. Not you, unless you're alone.
fn strip_people(
    participants: &HashMap<String, ParticipantInfo>,
    self_hidden: bool,
) -> (Vec<(&String, &ParticipantInfo)>, usize) {
    let mut people: Vec<(&String, &ParticipantInfo)> = participants
        .iter()
        .filter(|(identity, _)| !self_hidden || identity.as_str() != "local")
        .collect();
    if people.is_empty() {
        people.extend(participants.iter());
    }
    people.sort_by(|a, b| a.1.name().cmp(b.1.name()));
    if people.len() > STRIP_MAX_FACES {
        // The last place goes to how many more there are.
        let more = people.len() - (STRIP_MAX_FACES - 1);
        people.truncate(STRIP_MAX_FACES - 1);
        (people, more)
    } else {
        (people, 0)
    }
}

/// Where a window this size sits in a corner of the usable area (x, y, width, height).
fn corner_position(corner: ScreenCorner, size: (f64, f64), area: [f64; 4]) -> (f64, f64) {
    let [x, y, width, height] = area;
    let left = x + PIN_CORNER_MARGIN;
    let right = x + width - size.0 - PIN_CORNER_MARGIN;
    let top = y + PIN_CORNER_MARGIN;
    let bottom = y + height - size.1 - PIN_CORNER_MARGIN;
    match corner {
        ScreenCorner::TopLeft => (left, top),
        ScreenCorner::TopRight => (right, top),
        ScreenCorner::BottomLeft => (left, bottom),
        ScreenCorner::BottomRight => (right, bottom),
    }
}

/// The corner of the usable area (x, y, width, height) nearest a window centred here.
fn nearest_corner(centre: (f64, f64), area: [f64; 4]) -> ScreenCorner {
    let [x, y, width, height] = area;
    let right = centre.0 > x + width / 2.0;
    let bottom = centre.1 > y + height / 2.0;
    match (right, bottom) {
        (false, false) => ScreenCorner::TopLeft,
        (true, false) => ScreenCorner::TopRight,
        (false, true) => ScreenCorner::BottomLeft,
        (true, true) => ScreenCorner::BottomRight,
    }
}

/// Whether this person's picture hasn't changed since it was last drawn, so it needn't be
/// uploaded again.
pub(crate) fn should_skip(
    identity: &str,
    buffers: &VideoBufferManager,
    outer_skip: bool,
    last_rendered_frame_ids: &mut HashMap<String, u64>,
) -> bool {
    if outer_skip {
        return true;
    }
    let current_frame_id = buffers.current_frame_id();
    let last_id = last_rendered_frame_ids
        .entry(identity.to_string())
        .or_insert(0);

    // Same logic for resetting the frame id as in the screensharing window
    if current_frame_id > 0 && *last_id > 40 && current_frame_id < *last_id - 40 {
        *last_id = 0;
    }

    let skip = current_frame_id > 0 && current_frame_id <= *last_id;
    if !skip {
        *last_id = current_frame_id;
    }
    skip
}

/// How the faces are arranged in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Arrangement {
    /// Everyone the same size.
    Grid,
    /// The speaker big, everyone else in a row below.
    Speaker,
    /// Only the speaker.
    OnlySpeaker,
}

struct GridProps<'s> {
    available_size: IcedSize,
    self_hidden: bool,
    local_tile_hovered: bool,
    /// Taken from the top and bottom by the header or the controls.
    reserved_height: f32,
    outer_skip: bool,
    arrangement: Arrangement,
    speaker: Option<&'s str>,
}

/// Create the responsive participant grid.
///
/// Calculates the layout that shows everyone as big as they'll go, with tiles between square
/// and 16:9 so a wide window is filled. In the speaker arrangements, whoever is speaking is
/// big and everyone else is in a row below, or not shown.
fn create_participant_grid<'a>(
    props: GridProps<'_>,
    participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    last_rendered_frame_ids: &mut HashMap<String, u64>,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let participants_guard = participants.read().unwrap();

    // Sort participants by name for stable ordering
    let mut sorted: Vec<(&String, &ParticipantInfo)> = participants_guard.iter().collect();
    sorted.sort_by(|a, b| a.1.name().cmp(b.1.name()));

    let sorted: Vec<(&String, &ParticipantInfo)> = if props.self_hidden {
        sorted
            .into_iter()
            .filter(|(identity, _)| identity.as_str() != "local")
            .collect()
    } else {
        sorted
    };

    let participant_count = sorted.len();
    if participant_count == 0 {
        return Space::new().width(Length::Fill).height(Length::Fill).into();
    }

    let available_size = props.available_size;
    // Subtract the header or the strip of controls from vertical space, then apply grid padding
    let available_width = available_size.width - (MIN_GRID_PADDING * 2.0);
    let available_height =
        (available_size.height - props.reserved_height) - (MIN_GRID_PADDING * 2.0);

    // Determine if this is a small window for styling purposes
    let is_small_window = available_size.width < SMALL_WIDTH_THRESHOLD
        || available_size.height < SMALL_HEIGHT_THRESHOLD;

    let mut card = |(identity, info): (&String, &ParticipantInfo), tile: TileSize, hide_name| {
        let camera_buffers = info.camera_buffers();
        let skip_buffer = should_skip(
            identity,
            &camera_buffers,
            props.outer_skip,
            last_rendered_frame_ids,
        );
        let is_local = identity.as_str() == "local";
        participant_card(ParticipantCardProps {
            participant_id: identity_to_id(identity),
            name: info.name(),
            is_speaking: info.is_speaking(),
            is_muted: info.muted(),
            is_capturing: info.capturing(),
            is_sharing: info.is_screensharing(),
            buffers: camera_buffers,
            tile,
            is_small_window,
            is_local,
            local_tile_hovered: props.local_tile_hovered,
            hide_name,
            skip_buffer,
        })
    };
    let too_small_for_a_name =
        |tile: TileSize| tile.width.min(tile.height) < HIDE_NAME_TILE_THRESHOLD;

    let speaker_index = props
        .speaker
        .and_then(|speaker| {
            sorted
                .iter()
                .position(|(identity, _)| identity.as_str() == speaker)
        })
        .unwrap_or(0);

    let content: iced::Element<'a, CameraMessage, Theme, iced::Renderer> = match props.arrangement {
        Arrangement::OnlySpeaker => {
            // Pinned this small, the name is what says who it is.
            let tile = fit_tile(available_width, available_height);
            card(sorted[speaker_index], tile, false)
        }
        Arrangement::Speaker if participant_count > 1 => {
            let (big, small) =
                speaker_layout(participant_count - 1, available_width, available_height);
            let speaker = card(sorted[speaker_index], big, too_small_for_a_name(big));
            let others: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> = sorted
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != speaker_index)
                .map(|(_, person)| card(*person, small, too_small_for_a_name(small)))
                .collect();
            column![speaker, row(others).spacing(TILE_SPACING)]
                .spacing(TILE_SPACING)
                .align_x(Alignment::Center)
                .into()
        }
        _ => {
            let (tiles_per_row, num_rows, tile) =
                grid_layout(participant_count, available_width, available_height);
            let hide_name = too_small_for_a_name(tile);

            // Create rows of participants
            let mut rows_vec: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> =
                Vec::new();
            let mut participants_iter = sorted.iter();
            for _ in 0..num_rows {
                let row_tiles: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> =
                    participants_iter
                        .by_ref()
                        .take(tiles_per_row)
                        .map(|person| card(*person, tile, hide_name))
                        .collect();
                if !row_tiles.is_empty() {
                    rows_vec.push(row(row_tiles).spacing(TILE_SPACING).into());
                }
            }
            column(rows_vec)
                .spacing(TILE_SPACING)
                .align_x(Alignment::Center)
                .into()
        }
    };

    // Centred in the space below the header
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(MIN_GRID_PADDING)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// Pinned as a strip: a column of round faces, with mute and unpin on hover.
fn strip_view<'a>(
    state: &'a CameraState,
    participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    outer_skip: bool,
    last_rendered_frame_ids: &mut HashMap<String, u64>,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let participants_guard = participants.read().unwrap();
    let (people, more) = strip_people(&participants_guard, state.self_hidden);

    let mut faces = column![].spacing(STRIP_GAP).align_x(Alignment::Center);
    for (identity, info) in people {
        let buffers = info.camera_buffers();
        let skip_buffer = should_skip(identity, &buffers, outer_skip, last_rendered_frame_ids);
        let speaking = info.is_speaking();
        let size = if speaking {
            STRIP_SPEAKER_FACE
        } else {
            STRIP_FACE
        };
        faces = faces.push(
            container(round_face(RoundFace {
                participant_id: identity_to_id(identity),
                name: info.name(),
                buffers,
                size,
                speaking,
                muted: info.muted(),
                mirror: identity.as_str() == "local",
                skip_buffer,
            }))
            .center_x(Length::Fixed(STRIP_SPEAKER_FACE))
            .center_y(Length::Fixed(STRIP_SPEAKER_FACE)),
        );
    }
    if more > 0 {
        let count = container(
            text(format!("+{more}"))
                .size(14)
                .font(GEIST_MEDIUM)
                .color(Color::WHITE),
        )
        .center_x(Length::Fixed(STRIP_FACE))
        .center_y(Length::Fixed(STRIP_FACE))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Slate700.to_color())),
            border: Border {
                radius: (STRIP_FACE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        faces = faces.push(
            container(count)
                .center_x(Length::Fixed(STRIP_SPEAKER_FACE))
                .center_y(Length::Fixed(STRIP_SPEAKER_FACE)),
        );
    }
    // The faces are the handle that moves the window.
    let faces = mouse_area(faces).on_press(CameraMessage::DragWindow);

    let local_muted = participants_guard
        .get("local")
        .is_some_and(|info| info.muted());
    let controls: iced::Element<'a, CameraMessage, Theme, iced::Renderer> =
        if state.pointer_over_window {
            let mic: iced::Element<'a, CameraMessage, Theme, iced::Renderer> = if local_muted {
                text(ICON_MICROPHONE_OFF.to_string())
                    .font(ICONS_FONT)
                    .size(14.0)
                    .color(ColorToken::Red400.to_color())
                    .into()
            } else {
                let slate300 = ColorToken::Slate300.to_color();
                svg(svg::Handle::from_memory(ICON_MICROPHONE_SVG))
                    .width(Length::Fixed(14.0))
                    .height(Length::Fixed(14.0))
                    .style(move |_theme: &Theme, _status| svg::Style {
                        color: Some(slate300),
                    })
                    .into()
            };
            let pin = text(ICON_PIN_ANGLE.to_string())
                .font(ICONS_FONT)
                .size(14.0)
                .color(ColorToken::Slate300.to_color());
            column![
                strip_button(
                    mic,
                    CameraMessage::CallControls(CallControlsMessage::MicToggle)
                ),
                strip_button(pin.into(), CameraMessage::PinToCorner),
            ]
            .spacing(6)
            .align_x(Alignment::Center)
            .into()
        } else {
            Space::new().into()
        };

    column![faces, controls]
        .spacing(STRIP_GAP)
        .padding(STRIP_PADDING)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A small round button under the strip's faces. No tooltip: one wouldn't fit the window.
fn strip_button<'a>(
    icon: iced::Element<'a, CameraMessage, Theme, iced::Renderer>,
    message: CameraMessage,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    button(
        container(icon)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .width(Length::Fixed(38.0))
    .height(Length::Fixed(STRIP_BUTTON_HEIGHT))
    .padding(0)
    .on_press(message)
    .style(|_theme: &Theme, status| {
        let bg_color = match status {
            button::Status::Hovered => ColorToken::Slate600.to_color(),
            button::Status::Pressed => ColorToken::Slate800.to_color(),
            _ => ColorToken::Slate700.to_color(),
        };
        button::Style {
            background: Some(Background::Color(bg_color)),
            border: Border {
                color: Color::from_rgba(1.0, 1.0, 1.0, 0.3),
                width: 1.0,
                radius: 19.0.into(),
            },
            text_color: Color::WHITE,
            shadow: ShadowToken::Xs.to_shadow(),
            snap: false,
        }
    })
    .into()
}

pub(crate) struct RoundFace<'n> {
    pub(crate) participant_id: u64,
    pub(crate) name: &'n str,
    pub(crate) buffers: Arc<VideoBufferManager>,
    pub(crate) size: f32,
    pub(crate) speaking: bool,
    pub(crate) muted: bool,
    pub(crate) mirror: bool,
    pub(crate) skip_buffer: bool,
}

/// Someone's camera, or their initials, in a circle: green-ringed while they speak, with a
/// red mic while they're muted.
pub(crate) fn round_face<'a, M: 'a>(
    face: RoundFace<'_>,
) -> iced::Element<'a, M, Theme, iced::Renderer> {
    let RoundFace {
        participant_id,
        name,
        buffers,
        size,
        speaking,
        muted,
        mirror,
        skip_buffer,
    } = face;
    let radius = size / 2.0;
    let picture: iced::Element<'a, M, Theme, iced::Renderer> = if buffers.is_inactive() {
        let mut initials = row![].align_y(Alignment::Center);
        for letter in get_initials(name) {
            initials = initials.push(
                text(letter)
                    .size(size * 0.36)
                    .color(Color::WHITE)
                    .font(GEIST_MEDIUM),
            );
        }
        container(initials)
            .center_x(Length::Fixed(size))
            .center_y(Length::Fixed(size))
            .style(move |_theme: &Theme| container::Style {
                background: Some(Background::Color(ColorToken::Violet800.to_color())),
                border: Border {
                    radius: radius.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        shader(YuvVideoProgram {
            participant_id,
            buffer: buffers,
            corner_radius: radius,
            stretch_to_fill: false,
            skip_upload: skip_buffer,
            mirror,
        })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
    };
    let ring = container(Space::new())
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme: &Theme| container::Style {
            border: if speaking {
                Border {
                    color: ColorToken::Green400.to_color(),
                    width: 2.5,
                    radius: radius.into(),
                }
            } else {
                Border {
                    color: Color::from_rgba(1.0, 1.0, 1.0, 0.2),
                    width: 1.0,
                    radius: radius.into(),
                }
            },
            ..Default::default()
        });
    let mut layers: Vec<iced::Element<'a, M, Theme, iced::Renderer>> = vec![picture, ring.into()];
    if muted {
        let dot = container(
            text(ICON_MICROPHONE_OFF.to_string())
                .font(ICONS_FONT)
                .size(9.0)
                .color(Color::WHITE),
        )
        .center_x(Length::Fixed(16.0))
        .center_y(Length::Fixed(16.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(ColorToken::Red500.to_color())),
            border: Border {
                radius: 8.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        layers.push(
            container(dot)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::End)
                .align_y(Alignment::End)
                .into(),
        );
    }
    container(stack(layers))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_theme: &Theme| container::Style {
            border: Border {
                radius: radius.into(),
                ..Default::default()
            },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
                offset: iced::Vector::new(0.0, 3.0),
                blur_radius: 12.0,
            },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speaking_bars_stay_in_their_slot() {
        for step in 0..200 {
            for height in bar_heights(13.0, step as f32 * 0.037) {
                assert!((0.2..=1.0).contains(&height), "{height}");
            }
        }
    }

    fn tile(width: f32, height: f32) -> TileSize {
        TileSize { width, height }
    }

    #[test]
    fn tiles_are_between_square_and_sixteen_by_nine() {
        assert_eq!(fit_tile(1000.0, 300.0), tile(300.0 * WIDEST_TILE, 300.0));
        assert_eq!(fit_tile(300.0, 1000.0), tile(300.0, 300.0));
        assert_eq!(fit_tile(400.0, 300.0), tile(400.0, 300.0));
        assert_eq!(fit_tile(0.0, 300.0).area(), 0.0);
    }

    #[test]
    fn a_wide_window_of_four_gets_wide_tiles_in_two_rows() {
        let (cols, rows, tile) = grid_layout(4, 1019.0, 507.0);
        assert_eq!((cols, rows), (2, 2));
        assert!(tile.width / tile.height > 1.7, "{tile:?}");
        // Wider than the squares they were, and as tall.
        assert!(tile.width > 400.0 && tile.height > 240.0, "{tile:?}");
    }

    #[test]
    fn the_grid_always_fits_the_window() {
        for count in 1..=12 {
            for (width, height) in [
                (1019.0, 507.0),
                (300.0, 900.0),
                (84.0, 60.0),
                (1800.0, 300.0),
            ] {
                let (cols, rows, tile) = grid_layout(count, width, height);
                assert!(cols * rows >= count);
                let used_width = cols as f32 * tile.width + (cols - 1) as f32 * TILE_SPACING;
                let used_height = rows as f32 * tile.height + (rows - 1) as f32 * TILE_SPACING;
                assert!(
                    used_width <= width + 0.01,
                    "{count} in {width}x{height}: {tile:?}"
                );
                assert!(
                    used_height <= height + 0.01,
                    "{count} in {width}x{height}: {tile:?}"
                );
            }
        }
    }

    #[test]
    fn the_speaker_is_big_and_everyone_else_fits_below() {
        for others in 1..=8 {
            let (big, small) = speaker_layout(others, 1019.0, 507.0);
            assert!(big.area() > small.area());
            let row_width = others as f32 * small.width + (others - 1) as f32 * TILE_SPACING;
            assert!(row_width <= 1019.0 + 0.01, "{others}: {small:?}");
            assert!(big.height + TILE_SPACING + small.height <= 507.0 + 0.01);
        }
        let (alone, _) = speaker_layout(0, 1019.0, 507.0);
        assert_eq!(alone, fit_tile(1019.0, 507.0));
    }

    #[test]
    fn the_speaker_view_follows_who_talks_without_flicking() {
        let start = Instant::now();
        let at = |millis| start + Duration::from_millis(millis);
        let mut focus = SpeakerFocus::default();
        // Nobody talking: the first person.
        assert_eq!(
            focus
                .update(&[("ana", false), ("bo", false)], at(0))
                .as_deref(),
            Some("ana")
        );
        // Bo talks, but Ana has only just been put there.
        assert_eq!(
            focus
                .update(&[("ana", false), ("bo", true)], at(500))
                .as_deref(),
            Some("ana")
        );
        assert_eq!(
            focus
                .update(&[("ana", false), ("bo", true)], at(1600))
                .as_deref(),
            Some("bo")
        );
        // Ana cuts in straight away: Bo keeps it for a moment.
        assert_eq!(
            focus
                .update(&[("ana", true), ("bo", true)], at(1700))
                .as_deref(),
            Some("bo")
        );
        // Bo stops, and has had it long enough: Ana.
        assert_eq!(
            focus
                .update(&[("ana", true), ("bo", false)], at(3200))
                .as_deref(),
            Some("ana")
        );
        // Nobody talks: Ana stays.
        assert_eq!(
            focus
                .update(&[("ana", false), ("bo", false)], at(9000))
                .as_deref(),
            Some("ana")
        );
        // Ana leaves.
        assert_eq!(
            focus.update(&[("bo", false)], at(9100)).as_deref(),
            Some("bo")
        );
        assert_eq!(focus.update(&[], at(9200)), None);
    }

    #[test]
    fn a_pinned_window_goes_to_the_nearest_corner() {
        let area = [0.0, 25.0, 1440.0, 800.0];
        let size = (68.0, 200.0);
        assert_eq!(
            corner_position(ScreenCorner::TopLeft, size, area),
            (16.0, 41.0)
        );
        assert_eq!(
            corner_position(ScreenCorner::BottomRight, size, area),
            (1440.0 - 68.0 - 16.0, 25.0 + 800.0 - 200.0 - 16.0)
        );
        assert_eq!(nearest_corner((100.0, 100.0), area), ScreenCorner::TopLeft);
        assert_eq!(
            nearest_corner((1300.0, 120.0), area),
            ScreenCorner::TopRight
        );
        assert_eq!(
            nearest_corner((90.0, 700.0), area),
            ScreenCorner::BottomLeft
        );
        assert_eq!(
            nearest_corner((1300.0, 700.0), area),
            ScreenCorner::BottomRight
        );
        // Already in its corner, it stays.
        for corner in [
            ScreenCorner::TopLeft,
            ScreenCorner::TopRight,
            ScreenCorner::BottomLeft,
            ScreenCorner::BottomRight,
        ] {
            let (x, y) = corner_position(corner, size, area);
            assert_eq!(
                nearest_corner((x + size.0 / 2.0, y + size.1 / 2.0), area),
                corner
            );
        }
    }

    #[test]
    fn the_strip_grows_with_its_faces() {
        let (width, one) = strip_size(1);
        let (_, three) = strip_size(3);
        assert_eq!(width, STRIP_WIDTH as f64);
        assert_eq!(three - one, 2.0 * (STRIP_SPEAKER_FACE + STRIP_GAP) as f64);
        assert_eq!(strip_size(0), strip_size(1));
    }

    fn layout(width: f32, height: f32) -> (ControlsPlacement, CallControlsDensity) {
        controls_layout(IcedSize::new(width, height))
    }

    #[test]
    fn controls_fit_where_they_are_at_every_width() {
        for width in CAMERA_WINDOW_MIN_WIDTH as u32..=1200 {
            let width = width as f32;
            let (placement, density) = layout(width, 600.0);
            let room = match placement {
                ControlsPlacement::Header => width - 2.0 * (HEADER_SIDE_WIDTH + CONTENT_PADDING),
                ControlsPlacement::Strip | ControlsPlacement::Hover => width,
            };
            assert!(
                density.total_width() <= room,
                "{density:?} is {} wide, with {room} at a width of {width}",
                density.total_width()
            );
        }
    }

    #[test]
    fn a_wide_window_keeps_them_in_the_header() {
        assert_eq!(
            layout(CAMERA_WINDOW_WIDTH as f32, CAMERA_WINDOW_HEIGHT as f32),
            (ControlsPlacement::Header, CallControlsDensity::Regular)
        );
        assert_eq!(
            layout(HEADER_CONTROLS_MIN_WIDTH, 600.0),
            (ControlsPlacement::Header, CallControlsDensity::Compact)
        );
    }

    #[test]
    fn a_narrow_window_moves_them_below_the_video() {
        for width in [
            HEADER_CONTROLS_MIN_WIDTH - 1.0,
            314.0,
            283.0,
            ICON_CONTROLS_MIN_WIDTH,
        ] {
            assert_eq!(
                layout(width, 400.0),
                (ControlsPlacement::Strip, CallControlsDensity::Icons),
                "at a width of {width}"
            );
        }
        assert_eq!(
            layout(ICON_CONTROLS_MIN_WIDTH - 1.0, 400.0),
            (ControlsPlacement::Strip, CallControlsDensity::Essentials)
        );
    }

    #[test]
    fn a_pinned_window_keeps_its_video_and_shows_them_on_hover() {
        assert_eq!(
            layout(PIN_BOX_WIDTH as f32, PIN_BOX_HEIGHT as f32),
            (ControlsPlacement::Hover, CallControlsDensity::Essentials)
        );
        let (placement, _) = layout(PIN_SPEAKER_WIDTH as f32, PIN_SPEAKER_HEIGHT as f32);
        assert_eq!(placement, ControlsPlacement::Hover);
    }
}
