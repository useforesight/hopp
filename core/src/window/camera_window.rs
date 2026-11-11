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
const HEADER_SIDE_WIDTH: f32 = 80.0;
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

const COMPACT_TOP_INSET: f32 = 24.0;

const ICON_MICROPHONE_OFF: char = '\u{F106}';
const ICON_PIN_ANGLE: char = '\u{F10B}';

const PIN_CORNER_WIDTH: f64 = CAMERA_WINDOW_MIN_WIDTH;
const PIN_CORNER_HEIGHT: f64 = CAMERA_WINDOW_MIN_HEIGHT;
const PIN_CORNER_MARGIN: f64 = 40.0;

const ICON_EYE_ON_SVG: &[u8] = include_bytes!("../../resources/icons/EyeOn.svg");
const ICON_MICROPHONE_SVG: &[u8] = include_bytes!("../../resources/icons/microphone.svg");
const ICON_EYE_OFF_SVG: &[u8] = include_bytes!("../../resources/icons/EyeOff.svg");

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
        }
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
        use winit::{
            platform::macos::WindowAttributesExtMacOS,
            window::{WindowButtons, WindowLevel},
        };
        attrs
            .with_title_hidden(true)
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_transparent(true)
            .with_window_level(WindowLevel::AlwaysOnTop)
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
        })
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
        self.state = CameraState {
            viewport_size: IcedSize::new(logical.width, logical.height),
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
            WindowEvent::Resized(new_size) => {
                if new_size.width > 0 && new_size.height > 0 {
                    self.viewport = Viewport::with_physical_size(
                        Size::new(new_size.width, new_size.height),
                        self.window.scale_factor() as f32,
                    );
                    let logical = self.viewport.logical_size();
                    self.state.viewport_size = IcedSize::new(logical.width, logical.height);

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
        let video_grid = create_participant_grid(
            state.viewport_size,
            participants,
            state.self_hidden,
            state.local_tile_hovered,
            top_inset + strip_height,
            outer_skip,
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
                container(pin_button())
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
        }
    }

    /// Recompute what follows from the current viewport size.
    fn sync_compact_constraints(&mut self) {
        self.state.is_compact = self.state.viewport_size.width < COMPACT_WIDTH_THRESHOLD;

        // Only the header has the device menus. One left open would come back with the header.
        let (controls_placement, _) = controls_layout(self.state.viewport_size);
        if controls_placement != ControlsPlacement::Header {
            self.call_controls.dismiss_dropdowns();
        }
    }

    /// Resize and reposition the window to the top-right corner of the current monitor,
    /// hiding the local participant tile.
    fn pin_to_corner(&mut self) {
        self.state.self_hidden = true;
        self.state.local_tile_hovered = false;
        // The window is about to move out from under the pointer.
        self.cursor = mouse::Cursor::Unavailable;
        self.state.pointer_over_window = false;

        let monitor = self
            .window
            .current_monitor()
            .or_else(|| self.window.available_monitors().next());

        if let Some(monitor) = monitor {
            let scale = monitor.scale_factor();
            let mon_size: winit::dpi::LogicalSize<f64> = monitor.size().to_logical(scale);
            let mon_pos: winit::dpi::LogicalPosition<f64> = monitor.position().to_logical(scale);

            let x = mon_pos.x + mon_size.width - PIN_CORNER_WIDTH - PIN_CORNER_MARGIN;
            let y = mon_pos.y + PIN_CORNER_MARGIN;

            let _ = self.window.request_inner_size(winit::dpi::LogicalSize::new(
                PIN_CORNER_WIDTH,
                PIN_CORNER_HEIGHT,
            ));
            self.window
                .set_outer_position(winit::dpi::LogicalPosition::new(x, y));
        }
    }

    /// Restore the window to its default size, centered on the current monitor.
    fn unpin(&mut self) {
        self.state.self_hidden = false;

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

    /// Perform a full redraw: build UI, draw, present.
    fn redraw(&mut self) {
        self.sync_compact_constraints();
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
    pub(super) buffers: Arc<VideoBufferManager>,
    pub(super) tile_size: f32,
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
        buffers,
        tile_size,
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
            container(initials_avatar(name, tile_size))
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
            border: Border {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                width: 1.0,
                radius: TILE_RADIUS.into(),
            },
            ..Default::default()
        });

    // Stack: clipped video below, border on top.
    let tile = container(iced::widget::stack![clipped_content, border_frame])
        .width(Length::Fixed(tile_size))
        .height(Length::Fixed(tile_size))
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
fn identity_to_id(identity: &str) -> u64 {
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

/// Create the responsive participant grid.
///
/// Calculates optimal grid layout based on available size, maximizing tile size
/// while ensuring all participants are visible. Matches the iced-poc algorithm.
fn create_participant_grid<'a>(
    available_size: IcedSize,
    participants: &'a Arc<RwLock<HashMap<String, ParticipantInfo>>>,
    self_hidden: bool,
    local_tile_hovered: bool,
    reserved_height: f32,
    outer_skip: bool,
    last_rendered_frame_ids: &mut HashMap<String, u64>,
) -> iced::Element<'a, CameraMessage, Theme, iced::Renderer> {
    let participants_guard = participants.read().unwrap();

    // Sort participants by name for stable ordering
    let mut sorted: Vec<(&String, &ParticipantInfo)> = participants_guard.iter().collect();
    sorted.sort_by(|a, b| a.1.name().cmp(b.1.name()));

    let sorted: Vec<(&String, &ParticipantInfo)> = if self_hidden {
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

    // Subtract the header or the strip of controls from vertical space, then apply grid padding
    let available_width = available_size.width - (MIN_GRID_PADDING * 2.0);
    let available_height = (available_size.height - reserved_height) - (MIN_GRID_PADDING * 2.0);

    // Determine if this is a small window for styling purposes
    let is_small_window = available_size.width < SMALL_WIDTH_THRESHOLD
        || available_size.height < SMALL_HEIGHT_THRESHOLD;

    // Find the optimal grid configuration that maximizes tile size
    // while ensuring ALL participants are visible
    let mut best_tile_size = 0.0_f32;
    let mut best_cols = 1;
    let mut best_rows = participant_count;

    for cols in 1..=participant_count {
        let rows = (participant_count as f32 / cols as f32).ceil() as usize;

        // Calculate max tile size for this configuration
        let max_tile_width = (available_width - (TILE_SPACING * (cols - 1) as f32)) / cols as f32;
        let max_tile_height = (available_height - (TILE_SPACING * (rows - 1) as f32)) / rows as f32;

        // Tile size is constrained by both width and height (1:1 aspect ratio)
        let tile_size = max_tile_width.min(max_tile_height);

        // Always pick the configuration with the largest tile size that fits all participants
        if tile_size > 0.0 && tile_size > best_tile_size {
            best_tile_size = tile_size;
            best_cols = cols;
            best_rows = rows;
        }
    }

    let tile_size = best_tile_size;
    let tiles_per_row = best_cols;
    let num_rows = best_rows;
    let hide_name = tile_size < HIDE_NAME_TILE_THRESHOLD;

    // Calculate actual grid dimensions
    let actual_cols = tiles_per_row.min(participant_count);
    let grid_content_width =
        (tile_size * actual_cols as f32) + (TILE_SPACING * (actual_cols - 1) as f32);
    let grid_content_height =
        (tile_size * num_rows as f32) + (TILE_SPACING * (num_rows - 1) as f32);

    // Calculate dynamic padding to center the grid within the grid area (below header)
    let grid_area_height = available_size.height - reserved_height;
    let h_padding = ((available_size.width - grid_content_width) / 2.0).max(MIN_GRID_PADDING);
    let v_padding = ((grid_area_height - grid_content_height) / 2.0).max(MIN_GRID_PADDING);

    // Create rows of participants
    let mut rows_vec: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> = Vec::new();
    let mut participants_iter = sorted.iter();

    for _ in 0..num_rows {
        let mut row_tiles: Vec<iced::Element<'a, CameraMessage, Theme, iced::Renderer>> =
            Vec::new();

        for _ in 0..tiles_per_row {
            if let Some((identity, info)) = participants_iter.next() {
                let id = identity_to_id(identity);
                let camera_buffers = info.camera_buffers();
                let is_local = identity.as_str() == "local";

                let should_skip = if outer_skip {
                    true
                } else {
                    let current_frame_id = camera_buffers.current_frame_id();
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
                };

                row_tiles.push(participant_card(ParticipantCardProps {
                    participant_id: id,
                    name: info.name(),
                    is_speaking: info.is_speaking(),
                    is_muted: info.muted(),
                    is_capturing: info.capturing(),
                    buffers: camera_buffers,
                    tile_size,
                    is_small_window,
                    is_local,
                    local_tile_hovered,
                    hide_name,
                    skip_buffer: should_skip,
                }));
            }
        }

        if !row_tiles.is_empty() {
            let participant_row = row(row_tiles).spacing(TILE_SPACING);
            rows_vec.push(participant_row.into());
        }
    }

    // Create the grid column
    let grid = column(rows_vec)
        .spacing(TILE_SPACING)
        .align_x(Alignment::Center);

    // Use dynamic padding to center the grid exactly
    container(grid)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(
            Padding::new(0.0)
                .top(v_padding)
                .bottom(v_padding)
                .left(h_padding)
                .right(h_padding),
        )
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
            layout(PIN_CORNER_WIDTH as f32, PIN_CORNER_HEIGHT as f32),
            (ControlsPlacement::Hover, CallControlsDensity::Essentials)
        );
    }
}
