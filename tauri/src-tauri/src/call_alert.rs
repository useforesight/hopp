use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const CALL_ALERT_WINDOW_LABEL: &str = "callAlert";

const CALL_ALERT_WIDTH: f64 = 360.0;
pub(crate) const CALL_ALERT_HEIGHT: f64 = 116.0;
#[cfg(target_os = "macos")]
pub(crate) const CALL_ALERT_SCREEN_MARGIN: f64 = 16.0;
/// NSStatusWindowLevel: above normal and floating windows, including fullscreen apps.
#[cfg(target_os = "macos")]
const CALL_ALERT_WINDOW_LEVEL: isize = 25;

/// Shows the call card without taking focus. The window outlives each alert and is
/// only hidden, because a window that is still closing can't be shown again.
pub fn show(app: &AppHandle) -> Result<(), String> {
    let window = match app.get_webview_window(CALL_ALERT_WINDOW_LABEL) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(
            app,
            CALL_ALERT_WINDOW_LABEL,
            WebviewUrl::App("callAlert.html".into()),
        )
        .title("Hopp call")
        .inner_size(CALL_ALERT_WIDTH, CALL_ALERT_HEIGHT)
        .visible(false)
        .focused(false)
        .transparent(true)
        .decorations(false)
        .shadow(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .accept_first_mouse(true)
        .content_protected(true)
        .build()
        .map_err(|e| format!("Failed to create call alert window: {e}"))?,
    };

    #[cfg(target_os = "macos")]
    {
        let window_for_main_thread = window.clone();
        window
            .run_on_main_thread(move || present_on_cursor_screen(&window_for_main_thread))
            .map_err(|e| format!("Failed to show call alert window: {e}"))?;
        crate::cards::make_room(app);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_always_on_top(true)
            .and_then(|_| window.show())
            .map_err(|e| format!("Failed to show call alert window: {e}"))
    }
}

pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(CALL_ALERT_WINDOW_LABEL) {
        if let Err(e) = window.hide() {
            log::error!("call_alert::hide: {e:?}");
        }
        #[cfg(target_os = "macos")]
        crate::cards::make_room(app);
    }
}

/// Whether the call card is up on this screen. Runs on the main thread.
#[cfg(target_os = "macos")]
pub(crate) fn is_showing_on(app: &AppHandle, screen: &objc2_app_kit::NSScreen) -> bool {
    use objc2_app_kit::NSWindow;

    let Some(window) = app.get_webview_window(CALL_ALERT_WINDOW_LABEL) else {
        return false;
    };
    let Ok(ns_window) = window.ns_window() else {
        return false;
    };
    let ns_window: &NSWindow = unsafe { &*ns_window.cast() };
    ns_window.isVisible()
        && ns_window
            .screen()
            .is_some_and(|own| own.frame() == screen.frame())
}

#[cfg(target_os = "macos")]
pub fn is_cursor_over(app: &AppHandle) -> bool {
    use objc2_app_kit::{NSEvent, NSWindow};

    let Some(window) = app.get_webview_window(CALL_ALERT_WINDOW_LABEL) else {
        return false;
    };
    if !window.is_visible().unwrap_or(false) {
        return false;
    }
    let Ok(ns_window) = window.ns_window() else {
        return false;
    };
    let ns_window: &NSWindow = unsafe { &*ns_window.cast() };
    let frame = ns_window.frame();
    let cursor = NSEvent::mouseLocation();
    rect_contains_point(
        (frame.origin.x, frame.origin.y),
        (frame.size.width, frame.size.height),
        (cursor.x, cursor.y),
    )
}

/// Uses AppKit directly: positions are in native points, which avoids tao's mixed-DPI
/// offsets across monitors, and orderFrontRegardless doesn't make the window key, so
/// an open main window isn't hidden by losing focus.
#[cfg(target_os = "macos")]
fn present_on_cursor_screen(window: &tauri::WebviewWindow) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{MainThreadMarker, NSPoint};

    let Some(main_thread) = MainThreadMarker::new() else {
        log::error!("call_alert: present_on_cursor_screen must run on the main thread");
        return;
    };
    let ns_window: &NSWindow = match window.ns_window() {
        Ok(ns_window) => unsafe { &*ns_window.cast() },
        Err(e) => {
            log::error!("call_alert: failed to get NSWindow: {e:?}");
            return;
        }
    };

    float(ns_window);

    if let Some(screen) = screen_under_cursor(main_thread) {
        let visible_frame = screen.visibleFrame();
        let window_width = ns_window.frame().size.width;
        ns_window.setFrameTopLeftPoint(NSPoint::new(
            visible_frame.origin.x + visible_frame.size.width
                - window_width
                - CALL_ALERT_SCREEN_MARGIN,
            visible_frame.origin.y + visible_frame.size.height - CALL_ALERT_SCREEN_MARGIN,
        ));
    }

    ns_window.orderFrontRegardless();
}

/// Above other windows on every space, including fullscreen apps, and going nowhere when
/// Hopp isn't the active app.
#[cfg(target_os = "macos")]
pub(crate) fn float(ns_window: &objc2_app_kit::NSWindow) {
    use objc2_app_kit::NSWindowCollectionBehavior;

    ns_window.setLevel(CALL_ALERT_WINDOW_LEVEL);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    ns_window.setHidesOnDeactivate(false);
}

#[cfg(target_os = "macos")]
pub(crate) fn screen_under_cursor(
    main_thread: objc2_foundation::MainThreadMarker,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSScreen>> {
    use objc2_app_kit::{NSEvent, NSScreen};

    let cursor = NSEvent::mouseLocation();
    NSScreen::screens(main_thread)
        .iter()
        .find(|screen| {
            let frame = screen.frame();
            rect_contains_point(
                (frame.origin.x, frame.origin.y),
                (frame.size.width, frame.size.height),
                (cursor.x, cursor.y),
            )
        })
        .or_else(|| NSScreen::mainScreen(main_thread))
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn rect_contains_point(origin: (f64, f64), size: (f64, f64), point: (f64, f64)) -> bool {
    point.0 >= origin.0
        && point.0 < origin.0 + size.0
        && point.1 >= origin.1
        && point.1 < origin.1 + size.1
}

#[cfg(test)]
mod tests {
    use super::rect_contains_point;

    #[test]
    fn finds_a_point_on_a_monitor_left_of_the_primary() {
        assert!(rect_contains_point(
            (-1920.0, 0.0),
            (1920.0, 1080.0),
            (-100.0, 500.0)
        ));
    }

    #[test]
    fn a_point_on_the_next_monitors_edge_belongs_to_that_monitor() {
        assert!(!rect_contains_point(
            (0.0, 0.0),
            (1920.0, 1080.0),
            (1920.0, 500.0)
        ));
        assert!(rect_contains_point(
            (1920.0, 0.0),
            (1920.0, 1080.0),
            (1920.0, 500.0)
        ));
    }
}
