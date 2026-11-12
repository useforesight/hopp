use std::sync::Arc;
use thiserror::Error;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowAttributes, WindowLevel};

struct MonitorRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

pub fn ensure_on_screen(window: &Window) {
    let monitors: Vec<MonitorRect> = window
        .available_monitors()
        .map(|m| {
            let scale = m.scale_factor();
            let pos: LogicalPosition<f64> = m.position().to_logical(scale);
            let size: LogicalSize<f64> = m.size().to_logical(scale);
            MonitorRect {
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect();

    let scale = window.scale_factor();
    let pos: LogicalPosition<f64> = match window.outer_position() {
        Ok(p) => p.to_logical(scale),
        Err(_) => {
            log::warn!("ensure_on_screen: failed to get outer_position");
            return;
        }
    };
    let size: LogicalSize<f64> = window.inner_size().to_logical(scale);

    log::info!(
        "ensure_on_screen: window pos=({:.1}, {:.1}) size={:.1}x{:.1}, {} monitors",
        pos.x,
        pos.y,
        size.width,
        size.height,
        monitors.len()
    );
    for (i, m) in monitors.iter().enumerate() {
        log::info!(
            "ensure_on_screen: monitor[{}] pos=({:.1}, {:.1}) size={:.1}x{:.1}",
            i,
            m.x,
            m.y,
            m.width,
            m.height
        );
    }

    let fits = monitors.iter().any(|m| {
        pos.x >= m.x
            && pos.y >= m.y
            && pos.x + size.width <= m.x + m.width
            && pos.y + size.height <= m.y + m.height
    });

    if fits {
        log::info!("ensure_on_screen: window fits, no move needed");
        return;
    }

    if let Some(m) = monitors.first() {
        log::info!(
            "ensure_on_screen: moving window to monitor[0] pos=({:.1}, {:.1})",
            m.x,
            m.y
        );
        window.set_outer_position(LogicalPosition::new(m.x, m.y));
    } else {
        log::warn!("ensure_on_screen: no monitors found");
    }
}

/// The usual Mac window keys. The call windows belong to a helper with no menu, so nothing
/// would give them these otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKey {
    /// ⌘H: hide every call window. The call carries on.
    HideAll,
    /// ⌘M
    Minimise,
    /// ⌘W: put this window away. The call carries on.
    Close,
    /// ⌘`: the other call window.
    NextWindow,
}

/// Which window key this is, if any. Only on macOS, where these are what people expect.
pub fn window_key(
    event: &winit::event::KeyEvent,
    modifiers: winit::keyboard::ModifiersState,
) -> Option<WindowKey> {
    use winit::keyboard::{Key, KeyCode, PhysicalKey};
    if !cfg!(target_os = "macos")
        || !event.state.is_pressed()
        || !modifiers.super_key()
        || modifiers.control_key()
        || modifiers.alt_key()
    {
        return None;
    }
    if event.physical_key == PhysicalKey::Code(KeyCode::Backquote) {
        return Some(WindowKey::NextWindow);
    }
    if modifiers.shift_key() {
        return None;
    }
    let Key::Character(ch) = &event.logical_key else {
        return None;
    };
    match ch.to_lowercase().as_str() {
        "h" => Some(WindowKey::HideAll),
        "m" => Some(WindowKey::Minimise),
        "w" => Some(WindowKey::Close),
        _ => None,
    }
}

/// Hides every window the helper has, as ⌘H does for an app. Showing any of them again
/// brings them all back.
pub fn hide_all() {
    #[cfg(target_os = "macos")]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
    }
}

/// Brings a window you may have lost back to you, before it's shown. A hidden window, or one
/// left on another Space, comes to the screen with your pointer and the Space you're on. One
/// you can already see stays where you put it.
pub fn bring_to_you(window: &Window) {
    let lost = window.is_visible() == Some(false) || !on_active_space(window);
    log::info!("bring_to_you: lost={lost}");
    if lost {
        move_to_pointer_screen(window);
    }
    #[cfg(target_os = "macos")]
    with_ns_window(window, |ns_window| {
        // Ordered in, it joins the Space you're on rather than taking you to its own.
        ns_window.setCollectionBehavior(
            ns_window.collectionBehavior() | NSWindowCollectionBehavior::MoveToActiveSpace,
        );
    });
}

/// The part of the window's screen that windows can use, clear of the menu bar and the Dock:
/// x, y, width, height in points from the top left of the main screen.
pub fn usable_area(window: &Window) -> Option<[f64; 4]> {
    #[cfg(target_os = "macos")]
    {
        let area = with_ns_window(window, |ns_window| {
            let mtm = objc2::MainThreadMarker::new()?;
            let screen = ns_window.screen()?;
            // Cocoa counts up from the bottom of the main screen; winit counts down from its top.
            let main_height = objc2_app_kit::NSScreen::screens(mtm)
                .firstObject()?
                .frame()
                .size
                .height;
            let visible = screen.visibleFrame();
            Some([
                visible.origin.x,
                main_height - visible.origin.y - visible.size.height,
                visible.size.width,
                visible.size.height,
            ])
        })
        .flatten();
        if area.is_some() {
            return area;
        }
    }
    let monitor = window
        .current_monitor()
        .or_else(|| window.available_monitors().next())?;
    let scale = monitor.scale_factor();
    let pos: LogicalPosition<f64> = monitor.position().to_logical(scale);
    let size: LogicalSize<f64> = monitor.size().to_logical(scale);
    // Roughly where a menu bar or a task bar would be.
    Some([pos.x, pos.y + 32.0, size.width, size.height - 64.0])
}

/// Takes away, or gives back, the window's title-bar buttons and shadow, for a window that
/// is only what it draws.
pub fn set_bare(window: &Window, bare: bool) {
    #[cfg(target_os = "macos")]
    with_ns_window(window, |ns_window| {
        use objc2_app_kit::NSWindowButton;
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = ns_window.standardWindowButton(kind) {
                button.setHidden(bare);
            }
        }
        ns_window.setHasShadow(!bare);
    });
    #[cfg(not(target_os = "macos"))]
    let _ = (window, bare);
}

/// Whether a mouse button is held down anywhere, as it is while a window is dragged.
pub fn mouse_button_held() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_app_kit::NSEvent::pressedMouseButtons() != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(target_os = "macos")]
fn on_active_space(window: &Window) -> bool {
    with_ns_window(window, |ns_window| ns_window.isOnActiveSpace()).unwrap_or(true)
}

#[cfg(not(target_os = "macos"))]
fn on_active_space(_window: &Window) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn with_ns_window<T>(window: &Window, f: impl FnOnce(&objc2_app_kit::NSWindow) -> T) -> Option<T> {
    let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let ns_view: Option<Retained<NSView>> =
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) };
    ns_view.and_then(|v| v.window()).map(|w| f(&w))
}

/// Centres the window on the screen with the pointer, unless it's on that screen already.
#[cfg(target_os = "macos")]
fn move_to_pointer_screen(window: &Window) {
    let Some(pointer) = ScreenshareFunctions::cursor_position() else {
        return;
    };
    let Some(monitor) = window.available_monitors().find(|m| {
        let scale = m.scale_factor();
        let pos: LogicalPosition<f64> = m.position().to_logical(scale);
        let size: LogicalSize<f64> = m.size().to_logical(scale);
        pointer.x >= pos.x
            && pointer.x < pos.x + size.width
            && pointer.y >= pos.y
            && pointer.y < pos.y + size.height
    }) else {
        return;
    };
    if window.current_monitor().as_ref() == Some(&monitor) {
        return;
    }
    let scale = monitor.scale_factor();
    let pos: LogicalPosition<f64> = monitor.position().to_logical(scale);
    let size: LogicalSize<f64> = monitor.size().to_logical(scale);
    let outer: LogicalSize<f64> = window.outer_size().to_logical(window.scale_factor());
    let x = pos.x + ((size.width - outer.width) / 2.0).max(0.0);
    let y = pos.y + ((size.height - outer.height) / 2.0).max(0.0);
    log::info!("move_to_pointer_screen: to ({x:.1}, {y:.1})");
    window.set_outer_position(LogicalPosition::new(x, y));
}

/// Only macOS can tell where the pointer is, so elsewhere it stays where it was.
#[cfg(not(target_os = "macos"))]
fn move_to_pointer_screen(_window: &Window) {}

#[cfg(target_os = "macos")]
use winit::platform::macos::WindowExtMacOS;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSView, NSWindowCollectionBehavior};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[cfg(target_os = "windows")]
use winit::platform::windows::WindowExtWindows;

use crate::capture::capturer::{MonitorId, ScreenshareExt, ScreenshareFunctions};
use crate::graphics::graphics_context::GraphicsContext;
use crate::graphics::graphics_window_context::ContextManager;
use crate::utils::geometry::Frame;
use crate::UserEvent;
use crate::{SelectionMode, SelectionOverlayState, ServerError};

// Constants for magic numbers
/// Initial size for the overlay window (width and height in logical pixels)
const OVERLAY_WINDOW_INITIAL_SIZE: f64 = 1.0;

pub(crate) fn get_window_attributes() -> WindowAttributes {
    WindowAttributes::default()
        .with_title("Overlay window")
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_decorations(false)
        .with_transparent(true)
        .with_inner_size(LogicalSize::new(
            OVERLAY_WINDOW_INITIAL_SIZE,
            OVERLAY_WINDOW_INITIAL_SIZE,
        ))
        .with_content_protected(true)
}

/// Returns the logical position where a window should be placed for the given monitor.
fn get_window_position_for_monitor(monitor: &MonitorHandle) -> LogicalPosition<f64> {
    let monitor_position = monitor.position();
    monitor_position.to_logical::<f64>(monitor.scale_factor())
}

#[derive(Error, Debug)]
pub enum WindowManagerError {
    #[error("Failed to create window")]
    WindowCreationError,
    #[error("Monitor not found")]
    MonitorNotFound,
    #[error("Fullscreen error: {0}")]
    FullscreenError(String),
}

impl From<WindowManagerError> for ServerError {
    fn from(err: WindowManagerError) -> Self {
        match err {
            WindowManagerError::WindowCreationError => ServerError::WindowCreationError,
            WindowManagerError::MonitorNotFound => ServerError::WindowCreationError, // Map to WindowCreationError for now
            WindowManagerError::FullscreenError(_) => ServerError::FullscreenError,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ScreenSelectionNavigationDirection {
    Left,
    Right,
    Up,
    Down,
}

struct WindowEntry<'a> {
    window: Arc<Window>,
    monitor_id: MonitorId,
    gfx: GraphicsContext<'a>,
}

pub struct WindowManager<'a> {
    windows: Vec<WindowEntry<'a>>,
    active_monitor_id: Option<MonitorId>,
    event_loop_proxy: EventLoopProxy<UserEvent>,
}

impl<'a> WindowManager<'a> {
    pub fn new(
        event_loop: &ActiveEventLoop,
        context_manager: &ContextManager,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self, WindowManagerError> {
        log::info!("WindowManager::new: creating windows for available monitors");
        let mut windows = Vec::new();

        for monitor in event_loop.available_monitors() {
            let window_entry = Self::create_window_entry(
                event_loop,
                &monitor,
                context_manager,
                event_loop_proxy.clone(),
            )?;
            windows.push(window_entry);
        }

        Ok(Self {
            windows,
            active_monitor_id: None,
            event_loop_proxy,
        })
    }

    fn create_window_entry(
        event_loop: &ActiveEventLoop,
        monitor: &MonitorHandle,
        context_manager: &ContextManager,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> Result<WindowEntry<'a>, WindowManagerError> {
        let attributes = get_window_attributes();
        let window = event_loop
            .create_window(attributes)
            .map_err(|_| WindowManagerError::WindowCreationError)?;

        let window = Arc::new(window);

        #[cfg(target_os = "linux")]
        {
            /* This is needed for getting the system picker for screen sharing. */
            let _ = window.request_inner_size(monitor.size().clone());
        }

        let _ = window.set_cursor_hittest(false);

        #[cfg(target_os = "windows")]
        {
            window.set_skip_taskbar(true);
        }

        #[cfg(target_os = "macos")]
        {
            window.set_has_shadow(false);

            // Needed for the overlay window to follow space changes.
            if let Ok(raw_handle) = window.window_handle() {
                if let RawWindowHandle::AppKit(handle) = raw_handle.as_raw() {
                    let ns_view: Option<Retained<NSView>> =
                        unsafe { Retained::retain(handle.ns_view.as_ptr().cast()) };
                    if let Some(ns_window) = ns_view.and_then(|v| v.window()) {
                        ns_window
                            .setCollectionBehavior(NSWindowCollectionBehavior::CanJoinAllSpaces);
                    }
                }
            }
        }

        let position = get_window_position_for_monitor(monitor);
        window.set_outer_position(position);
        window.set_visible(false);

        let monitor_id = ScreenshareFunctions::get_monitor_id(monitor);

        let gfx = GraphicsContext::new(
            context_manager,
            window.clone(),
            monitor.scale_factor(),
            event_loop_proxy,
        )
        .map_err(|_| WindowManagerError::WindowCreationError)?;

        Ok(WindowEntry {
            window,
            monitor_id,
            gfx,
        })
    }

    pub fn show_window(
        &mut self,
        monitor: &MonitorHandle,
        set_active_window: bool,
    ) -> Result<Arc<Window>, WindowManagerError> {
        let target_id = ScreenshareFunctions::get_monitor_id(monitor);
        log::info!(
            "WindowManager::show_window: looking for window with id {:?}",
            target_id
        );

        // Check existence before the retry loop (avoids borrow issues with iter_mut + retain).
        if !self.windows.iter().any(|e| e.monitor_id == target_id) {
            return Err(WindowManagerError::MonitorNotFound);
        }

        let max_retries = 5;
        let mut last_err = None;
        for retry in 0..max_retries {
            let entry = self
                .windows
                .iter_mut()
                .find(|e| e.monitor_id == target_id)
                .unwrap();
            match set_fullscreen(&entry.window, monitor.clone()) {
                Ok(_) => {
                    last_err = None;
                    break;
                }
                Err(e) => {
                    log::error!(
                        "WindowManager::show_window: error setting fullscreen: {:?}, retry {}/{}",
                        e,
                        retry + 1,
                        max_retries
                    );
                    last_err = Some(e);
                }
            }
        }

        if let Some(e) = last_err {
            log::error!(
                "WindowManager::show_window: failed to set fullscreen after {} retries",
                max_retries
            );
            return Err(WindowManagerError::FullscreenError(e.to_string()));
        }

        let entry = self
            .windows
            .iter_mut()
            .find(|e| e.monitor_id == target_id)
            .unwrap();

        // Reconfigure surface to the actual fullscreen size before first render.
        let fullscreen_size = entry.window.inner_size();
        entry.gfx.resize(fullscreen_size);

        if set_active_window {
            entry
                .gfx
                .add_participant("local".to_string(), "Me ", true)
                .map_err(|_| WindowManagerError::WindowCreationError)?;
            self.active_monitor_id = Some(target_id);
        } else {
            entry.gfx.set_screen_selection(Some(SelectionOverlayState {
                mode: SelectionMode::Screen,
                border_frame: None,
            }));
            let _ = entry.gfx.window().set_cursor_hittest(true);
        }
        entry.window.set_visible(true);

        Ok(entry.window.clone())
    }

    /// Recreates the shared render engine and resets every window's renderer.
    ///
    /// All overlay windows share one Engine Arc. Resetting only the active window's renderer
    /// leaves the other windows holding the old Arc, preventing the Engine (and its MSAA buffer
    /// + texture atlas) from being freed. This method drops all clones at once.
    pub fn reset_engines(&mut self, context_manager: &mut ContextManager) {
        if let Some(format) = self.windows.first().map(|e| e.gfx.surface_format()) {
            context_manager.overlay_context.reset_engine(format);
        }
        for entry in &mut self.windows {
            entry.gfx.reset_renderer(context_manager);
        }
    }

    pub fn active_gfx_mut(&mut self) -> Option<&mut GraphicsContext<'a>> {
        let active_id = self.active_monitor_id.as_ref()?;
        Some(
            &mut self
                .windows
                .iter_mut()
                .find(|entry| &entry.monitor_id == active_id)?
                .gfx,
        )
    }

    pub fn all_gfx_mut(&mut self) -> Vec<&mut GraphicsContext<'a>> {
        self.windows
            .iter_mut()
            .map(|entry| &mut entry.gfx)
            .collect()
    }

    pub fn active_window_monitor_mismatch(
        &self,
        window_id: winit::window::WindowId,
    ) -> Option<MonitorId> {
        let active_monitor_id = self.active_monitor_id.as_ref()?;
        let entry = self.windows.iter().find(|entry| {
            &entry.monitor_id == active_monitor_id && entry.window.id() == window_id
        })?;
        let current_monitor_id = entry
            .window
            .current_monitor()
            .map(|monitor| ScreenshareFunctions::get_monitor_id(&monitor));

        (current_monitor_id.as_ref() != Some(active_monitor_id)).then(|| active_monitor_id.clone())
    }

    pub fn active_monitor_id(&self) -> Option<MonitorId> {
        self.active_monitor_id.clone()
    }

    pub fn handle_screen_selection_event(
        &mut self,
        window_id: winit::window::WindowId,
        event: &WindowEvent,
        mode: SelectionMode,
    ) -> (bool, Option<SelectionMode>) {
        self.windows
            .iter_mut()
            .find(|entry| entry.window.id() == window_id)
            .map(|entry| entry.gfx.handle_screen_selection_event(event, mode))
            .unwrap_or((false, None))
    }

    pub fn update_screen_selection(&mut self, mode: SelectionMode, hovered_frame: Option<Frame>) {
        for entry in &mut self.windows {
            let border_frame = hovered_frame.and_then(|mut frame| {
                let position: LogicalPosition<f64> = entry
                    .window
                    .outer_position()
                    .ok()?
                    .to_logical(entry.window.scale_factor());
                frame.origin_x -= position.x;
                frame.origin_y -= position.y;
                Some(frame)
            });
            entry
                .gfx
                .set_screen_selection(Some(SelectionOverlayState { mode, border_frame }));
            entry.gfx.trigger_render();
        }
    }

    pub fn hide_active_window(&mut self) {
        if let Some(active_id) = self.active_monitor_id.take() {
            log::info!(
                "WindowManager::hide_active_window: hiding window for monitor {:?}",
                active_id
            );
            if let Some(entry) = self
                .windows
                .iter_mut()
                .find(|entry| entry.monitor_id == active_id)
            {
                entry.gfx.participants_manager_mut().clear();
                #[cfg(target_os = "macos")]
                {
                    // this is needed for the screensharing probing logic to work
                    // if we don't do it the probing window is placed above the menubar
                    entry.window.set_simple_fullscreen(false);
                    // set_simple_fullscreen(false) restores the saved style mask which
                    // includes Miniaturizable, causing the window to appear in the dock's
                    // minimized list. Strip that flag immediately to prevent this.
                    remove_miniaturizable_style(&entry.window);
                }
                entry.window.set_visible(false);
                entry.gfx.resize(winit::dpi::PhysicalSize::new(1, 1));
            }
        }
    }

    pub fn active_window_position(&self) -> Option<LogicalPosition<f64>> {
        let active_id = self.active_monitor_id.as_ref()?;
        let entry = self.windows.iter().find(|e| &e.monitor_id == active_id)?;
        let pos = entry.window.outer_position().ok()?;
        Some(pos.to_logical(entry.window.scale_factor()))
    }

    pub fn is_active_window(&self, window_id: winit::window::WindowId) -> bool {
        self.active_monitor_id.as_ref().is_some_and(|active_id| {
            self.windows
                .iter()
                .find(|entry| &entry.monitor_id == active_id)
                .is_some_and(|entry| entry.window.id() == window_id)
        })
    }

    pub fn monitor_id_for_window(&self, window_id: winit::window::WindowId) -> Option<MonitorId> {
        self.windows
            .iter()
            .find(|entry| entry.window.id() == window_id)
            .map(|entry| entry.monitor_id.clone())
    }

    pub fn resize_window(
        &mut self,
        window_id: winit::window::WindowId,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) {
        if let Some(entry) = self
            .windows
            .iter_mut()
            .find(|entry| entry.window.id() == window_id)
        {
            entry.gfx.resize(new_size);
        }
    }

    pub fn resize_active_window(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        let active_id = match self.active_monitor_id.as_ref() {
            Some(id) => id.clone(),
            None => return,
        };
        if let Some(entry) = self.windows.iter_mut().find(|e| e.monitor_id == active_id) {
            entry.gfx.resize(new_size);
        }
    }

    pub fn show_screen_selection(&mut self, event_loop: &ActiveEventLoop) {
        log::info!("show_screen_selection");
        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
        for monitor in monitors.iter() {
            let res = self.show_window(monitor, false);
            log::info!("{:?}", res);
        }

        #[cfg(target_os = "macos")]
        let cursor_monitor = ScreenshareFunctions::cursor_position().and_then(|cursor| {
            monitors
                .iter()
                .find(|monitor| {
                    let scale = monitor.scale_factor();
                    let position = monitor.position().to_logical::<f64>(scale);
                    let size = monitor.size().to_logical::<f64>(scale);

                    cursor.x >= position.x
                        && cursor.x < position.x + size.width
                        && cursor.y >= position.y
                        && cursor.y < position.y + size.height
                })
                .cloned()
        });

        #[cfg(not(target_os = "macos"))]
        let cursor_monitor: Option<MonitorHandle> = None;

        let active_monitor = self.active_monitor_id.as_ref().and_then(|active_id| {
            monitors
                .iter()
                .find(|monitor| ScreenshareFunctions::get_monitor_id(monitor) == *active_id)
                .cloned()
        });

        if let Some(monitor) = cursor_monitor
            .or(active_monitor)
            .or_else(|| event_loop.primary_monitor())
            .or_else(|| monitors.first().cloned())
        {
            if !self.focus_monitor(&monitor) {
                log::warn!("show_screen_selection: failed to focus monitor window");
            }
        }
    }

    pub fn focus_window(&self, window_id: winit::window::WindowId) -> bool {
        if let Some(entry) = self
            .windows
            .iter()
            .find(|entry| entry.window.id() == window_id)
        {
            entry.window.focus_window();
            true
        } else {
            false
        }
    }

    pub fn focus_monitor(&self, monitor: &MonitorHandle) -> bool {
        let monitor_id = ScreenshareFunctions::get_monitor_id(monitor);
        if let Some(entry) = self
            .windows
            .iter()
            .find(|entry| entry.monitor_id == monitor_id)
        {
            entry.window.focus_window();
            for entry in &self.windows {
                entry.gfx.trigger_render();
            }
            true
        } else {
            false
        }
    }

    pub fn focus_window_in_direction(
        &self,
        current_window_id: winit::window::WindowId,
        direction: ScreenSelectionNavigationDirection,
    ) -> bool {
        let mut entries: Vec<_> = self
            .windows
            .iter()
            .filter_map(|entry| {
                let position = entry.window.outer_position().ok()?;
                let logical_position: LogicalPosition<f64> =
                    position.to_logical(entry.window.scale_factor());

                Some((entry.window.id(), logical_position.x, logical_position.y))
            })
            .collect();

        if entries.len() < 2 {
            return false;
        }

        match direction {
            ScreenSelectionNavigationDirection::Left
            | ScreenSelectionNavigationDirection::Right => {
                entries.sort_by(|a, b| a.1.total_cmp(&b.1));
                if entries.windows(2).any(|pair| pair[0].1 == pair[1].1) {
                    return false;
                }
            }
            ScreenSelectionNavigationDirection::Up | ScreenSelectionNavigationDirection::Down => {
                entries.sort_by(|a, b| a.2.total_cmp(&b.2));
                if entries.windows(2).any(|pair| pair[0].2 == pair[1].2) {
                    return false;
                }
            }
        }

        let Some(current_index) = entries
            .iter()
            .position(|(window_id, _, _)| *window_id == current_window_id)
        else {
            return false;
        };

        let target_index = match direction {
            ScreenSelectionNavigationDirection::Left | ScreenSelectionNavigationDirection::Up => {
                if current_index == 0 {
                    entries.len() - 1
                } else {
                    current_index - 1
                }
            }
            ScreenSelectionNavigationDirection::Right
            | ScreenSelectionNavigationDirection::Down => (current_index + 1) % entries.len(),
        };

        let target_window_id = entries[target_index].0;
        let Some(target_entry) = self
            .windows
            .iter()
            .find(|entry| entry.window.id() == target_window_id)
        else {
            return false;
        };

        target_entry.window.focus_window();
        for entry in &self.windows {
            entry.gfx.trigger_render();
        }

        true
    }

    pub fn hide_screen_selection(&mut self) {
        let active_monitor_id = self.active_monitor_id.clone();
        for entry in &mut self.windows {
            entry.gfx.set_screen_selection(None);
            let _ = entry.window.set_cursor_hittest(false);

            if active_monitor_id.as_ref() == Some(&entry.monitor_id) {
                entry.window.set_visible(true);
                entry.gfx.trigger_render();
                continue;
            }

            #[cfg(target_os = "macos")]
            {
                entry.window.set_simple_fullscreen(false);
                remove_miniaturizable_style(&entry.window);
            }
            entry.window.set_visible(false);
            entry.gfx.resize(winit::dpi::PhysicalSize::new(1, 1));
        }
    }

    pub fn update(
        &mut self,
        event_loop: &ActiveEventLoop,
        context_manager: &ContextManager,
    ) -> Result<(), WindowManagerError> {
        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();

        log::info!(
            "WindowManager::update: checking {} monitors",
            monitors.len()
        );

        for monitor in &monitors {
            log::info!("WindowManager::update: monitor {:?}", monitor);
        }
        for entry in &self.windows {
            log::info!(
                "WindowManager::update: window for monitor id {:?} with position {:?}",
                entry.monitor_id,
                entry.window.outer_position(),
            );
        }

        let mut matched_monitor_indices: Vec<usize> = Vec::new();
        let mut active_monitor_id_found = false;

        self.windows.retain(|entry| {
            if let Some(monitor_idx) = monitors
                .iter()
                .position(|m| ScreenshareFunctions::get_monitor_id(m) == entry.monitor_id)
            {
                matched_monitor_indices.push(monitor_idx);
                let monitor = &monitors[monitor_idx];

                // Reposition if the window's actual position doesn't match expected.
                let expected_position = get_window_position_for_monitor(monitor);
                let actual_position: LogicalPosition<f64> = entry
                    .window
                    .outer_position()
                    .unwrap_or_default()
                    .to_logical(monitor.scale_factor());
                if actual_position != expected_position {
                    log::info!(
                        "WindowManager::update: repositioning window for monitor {:?} from {:?} to {:?}",
                        entry.monitor_id,
                        actual_position,
                        expected_position
                    );
                    entry.window.set_outer_position(expected_position);

                    // Re-apply fullscreen if this is the active window and position changed
                    if self.active_monitor_id.as_ref() == Some(&entry.monitor_id) {
                        active_monitor_id_found = true;
                        log::info!(
                            "WindowManager::update: re-applying fullscreen for active window on monitor {:?}",
                            entry.monitor_id
                        );
                        if let Err(e) = set_fullscreen(&entry.window, monitor.clone()) {
                            log::error!(
                                "WindowManager::update: error setting fullscreen: {:?}",
                                e
                            );
                        }
                    }
                } else if self.active_monitor_id.as_ref() == Some(&entry.monitor_id) {
                    active_monitor_id_found = true;
                }

                true
            } else {
                log::info!(
                    "WindowManager::update: removing window for disconnected monitor {:?}",
                    entry.monitor_id
                );
                false
            }
        });

        // Clear active monitor if it was disconnected
        if !active_monitor_id_found && self.active_monitor_id.is_some() {
            log::info!(
                "WindowManager::update: active monitor {:?} disconnected, clearing active monitor",
                self.active_monitor_id
            );
            self.active_monitor_id = None;
        }

        // Add windows for new monitors
        for (idx, monitor) in monitors.iter().enumerate() {
            if !matched_monitor_indices.contains(&idx) {
                log::info!(
                    "WindowManager::update: adding new window for monitor {:?}",
                    ScreenshareFunctions::get_monitor_id(monitor)
                );
                self.windows.push(Self::create_window_entry(
                    event_loop,
                    monitor,
                    context_manager,
                    self.event_loop_proxy.clone(),
                )?);
            }
        }

        log::info!(
            "WindowManager::update: windows list length: {:?}",
            self.windows.len()
        );

        Ok(())
    }
}

#[derive(Error, Debug)]
enum FullscreenError {
    #[error("Failed to get raw window handle")]
    #[cfg(target_os = "macos")]
    GetRawWindowHandleError,
    #[error("Failed to get NSView")]
    #[cfg(target_os = "macos")]
    GetNSViewError,
    #[error("Failed to get NSWindow")]
    #[cfg(target_os = "macos")]
    GetNSWindowError,
    #[error("Failed to get raw window handle")]
    #[cfg(target_os = "macos")]
    FailedToGetRawWindowHandle,
    #[error("Window is not associated with a screen")]
    #[cfg(target_os = "macos")]
    ScreenUnavailable,
    #[error("Failed to match fullscreen size within timeout")]
    FailedToMatchFullscreenSize,
}

#[cfg(target_os = "macos")]
fn remove_miniaturizable_style(window: &winit::window::Window) {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindowStyleMask};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(raw_handle) = window.window_handle() else {
        return;
    };
    if let RawWindowHandle::AppKit(handle) = raw_handle.as_raw() {
        let view = handle.ns_view.as_ptr();
        let ns_view: Option<Retained<NSView>> = unsafe { Retained::retain(view.cast()) };
        if let Some(ns_window) = ns_view.and_then(|v| v.window()) {
            let mask = ns_window.styleMask();
            ns_window.setStyleMask(mask & !NSWindowStyleMask::Miniaturizable);
        }
    }
}

fn set_fullscreen(
    window: &winit::window::Window,
    selected_monitor: MonitorHandle,
) -> Result<(), FullscreenError> {
    log::info!("set_fullscreen: {selected_monitor:?}");
    #[cfg(target_os = "macos")]
    {
        if window.current_monitor().is_none() {
            return Err(FullscreenError::ScreenUnavailable);
        }

        /* WA for putting the window in the right place. */
        window.set_simple_fullscreen(true);

        use objc2::rc::Retained;
        use objc2_app_kit::NSMainMenuWindowLevel;
        use objc2_app_kit::NSView;
        use raw_window_handle::HasWindowHandle;
        use raw_window_handle::RawWindowHandle;

        let raw_handle = window
            .window_handle()
            .map_err(|_| FullscreenError::GetRawWindowHandleError)?;
        if let RawWindowHandle::AppKit(handle) = raw_handle.as_raw() {
            let view = handle.ns_view.as_ptr();
            let ns_view: Option<Retained<NSView>> = unsafe { Retained::retain(view.cast()) };
            if ns_view.is_none() {
                return Err(FullscreenError::GetNSViewError);
            }
            let ns_view = ns_view.unwrap();
            let ns_window = ns_view.window();
            if ns_window.is_none() {
                return Err(FullscreenError::GetNSWindowError);
            }
            let ns_window = ns_window.unwrap();
            /* This is a hack to make the overlay window to appear above the main menu. */
            ns_window.setLevel(NSMainMenuWindowLevel + 1);
        } else {
            return Err(FullscreenError::FailedToGetRawWindowHandle);
        }
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        use winit::window::Fullscreen;

        window.set_fullscreen(Some(Fullscreen::Borderless(Some(selected_monitor.clone()))));
    }
    // Wait for the window to reach fullscreen size before creating the graphics context to avoid scissor rect
    // validation errors in wgpu.
    let expected_size = selected_monitor.size();
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(3);

    loop {
        let current_size = window.inner_size();
        if current_size.width == expected_size.width && current_size.height == expected_size.height
        {
            log::info!(
                "set_fullscreen: window reached fullscreen size {:?}",
                current_size
            );
            break;
        }

        if start.elapsed() > timeout {
            log::error!(
                "set_fullscreen: timeout waiting for fullscreen. Current: {:?}, Expected: {:?}",
                current_size,
                expected_size
            );
            return Err(FullscreenError::FailedToMatchFullscreenSize);
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}
