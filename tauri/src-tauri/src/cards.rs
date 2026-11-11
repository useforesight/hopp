//! Cards from agents and scripts (see `hopp_capture::cards`), stacked where notifications
//! appear.
//!
//! Like the call card this window floats above everything without being made key, so a card
//! never takes what you're typing. Clicking one brings Hopp forward, as any click on the Mac
//! does, so answering it hands the focus back to the app you came from. Hidden from your own
//! screen share.

use std::sync::{Mutex, PoisonError};

use hopp_capture::cards::{self, Card, CardRequest, CardState, Stack};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

pub const CARDS_WINDOW_LABEL: &str = "cards";

const WIDTH: f64 = 360.0;
/// Every card is this tall, so the stack's height follows from the count.
const CARD_HEIGHT: f64 = 136.0;
const GAP: f64 = 8.0;
const MORE_HEIGHT: f64 = 22.0;
/// Shown at once; the rest wait behind a "+N more" line.
const SHOWN: usize = 3;

#[derive(Default)]
pub struct Cards {
    stack: Mutex<Stack>,
    /// Held while finding or making the window, so that two cards don't both make it.
    window: Mutex<()>,
}

/// The window's height for `count` cards waiting.
fn height(count: usize) -> f64 {
    let shown = count.min(SHOWN) as f64;
    let more = if count > SHOWN {
        GAP + MORE_HEIGHT
    } else {
        0.0
    };
    (shown * CARD_HEIGHT + (shown - 1.0).max(0.0) * GAP + more).max(CARD_HEIGHT)
}

pub fn post(app: &AppHandle, card: CardRequest) -> Result<(), String> {
    let card = card.tidy();
    if card.title.is_empty() {
        return Err("A card needs a title".into());
    }
    let goes = card.goes_by_itself().then(|| card.id.clone());
    app.state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .post(card);
    if let Some(id) = goes {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(cards::EPHEMERAL).await;
            settle(&app, &id, CardState::Expired);
        });
    }
    changed(app)
}

pub fn cancel(app: &AppHandle, id: &str) {
    settle(app, id, CardState::Cancelled);
}

/// What the app's snapshot says about its cards.
pub fn snapshot(app: &AppHandle) -> Value {
    app.state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .snapshot()
}

fn settle(app: &AppHandle, id: &str, state: CardState) -> bool {
    let settled = app
        .state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .settle(id, state);
    if settled {
        if let Err(e) = changed(app) {
            log::error!("cards: {e}");
        }
    }
    settled
}

/// The cards still waiting, for a window that has just loaded.
#[tauri::command(async)]
pub fn cards_showing(cards: State<Cards>) -> Vec<Card> {
    cards
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .showing()
}

/// Somebody answered a card, or closed it.
#[tauri::command(async)]
pub fn cards_settle(app: AppHandle, id: String, state: CardState) {
    if matches!(
        state,
        CardState::Accepted | CardState::Rejected | CardState::Dismissed
    ) {
        settle(&app, &id, state);
        cards_return_focus(app);
    }
}

/// Back to the app you were in, if a card is what took you away.
#[tauri::command(async)]
pub fn cards_return_focus(app: AppHandle) {
    #[cfg(target_os = "macos")]
    focus::hand_back(&app);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Bring the window up to date with the stack. The main thread does that, one at a time, so
/// what's on screen is always the last change and not whichever thread finished last.
fn changed(app: &AppHandle) -> Result<(), String> {
    let cards = app.state::<Cards>();
    {
        let _window = cards.window.lock().unwrap_or_else(PoisonError::into_inner);
        if app.get_webview_window(CARDS_WINDOW_LABEL).is_none() {
            let empty = cards
                .stack
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .showing()
                .is_empty();
            if empty {
                return Ok(());
            }
            #[cfg(target_os = "macos")]
            focus::making_a_webview();
            // Outlives each card and is only hidden, as the call card's window is.
            WebviewWindowBuilder::new(
                app,
                CARDS_WINDOW_LABEL,
                WebviewUrl::App("cards.html".into()),
            )
            .title("Hopp cards")
            .inner_size(WIDTH, CARD_HEIGHT)
            .visible(false)
            .focused(false)
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            .resizable(false)
            .skip_taskbar(true)
            .accept_first_mouse(true)
            .content_protected(true)
            .build()
            .map_err(|e| format!("Failed to create the cards window: {e}"))?;
        }
    }
    refresh_soon(app).map_err(|e| format!("Failed to show the cards window: {e}"))
}

fn refresh_soon(app: &AppHandle) -> tauri::Result<()> {
    let for_main_thread = app.clone();
    app.run_on_main_thread(move || refresh(&for_main_thread))
}

/// Runs on the main thread.
fn refresh(app: &AppHandle) {
    let Some(window) = app.get_webview_window(CARDS_WINDOW_LABEL) else {
        return;
    };
    let showing = app
        .state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .showing();
    if let Err(e) = app.emit_to(CARDS_WINDOW_LABEL, "cards_changed", &showing) {
        log::error!("cards: failed to emit: {e:?}");
    }
    if showing.is_empty() {
        if let Err(e) = window.hide() {
            log::error!("cards: failed to hide: {e:?}");
        }
        return;
    }

    #[cfg(target_os = "macos")]
    {
        focus::note_whats_in_front();
        place(app, showing.len());
    }

    #[cfg(not(target_os = "macos"))]
    if let Err(e) = window
        .set_size(tauri::LogicalSize::new(WIDTH, height(showing.len())))
        .and_then(|_| window.show())
    {
        log::error!("cards: failed to show: {e:?}");
    }
}

/// The call card has come or gone: the stack goes under it.
#[cfg(target_os = "macos")]
pub fn make_room(app: &AppHandle) {
    if app.get_webview_window(CARDS_WINDOW_LABEL).is_none() {
        return;
    }
    if let Err(e) = refresh_soon(app) {
        log::error!("cards: make_room: {e:?}");
    }
}

/// Top right of the screen it's on, or the cursor's screen when it's only now appearing.
#[cfg(target_os = "macos")]
fn place(app: &AppHandle, count: usize) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

    use crate::call_alert;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::error!("cards: place must run on the main thread");
        return;
    };
    let Some(window) = app.get_webview_window(CARDS_WINDOW_LABEL) else {
        return;
    };
    let ns_window: &NSWindow = match window.ns_window() {
        Ok(ns_window) => unsafe { &*ns_window.cast() },
        Err(e) => {
            log::error!("cards: failed to get NSWindow: {e:?}");
            return;
        }
    };

    call_alert::float(ns_window);
    let screen = ns_window
        .isVisible()
        .then(|| ns_window.screen())
        .flatten()
        .or_else(|| call_alert::screen_under_cursor(main_thread));
    if let Some(screen) = screen {
        let visible = screen.visibleFrame();
        let mut top = visible.origin.y + visible.size.height - call_alert::CALL_ALERT_SCREEN_MARGIN;
        if call_alert::is_showing_on(app, &screen) {
            top -= call_alert::CALL_ALERT_HEIGHT + GAP;
        }
        let window_height = height(count);
        ns_window.setFrame_display(
            NSRect::new(
                NSPoint::new(
                    visible.origin.x + visible.size.width
                        - WIDTH
                        - call_alert::CALL_ALERT_SCREEN_MARGIN,
                    top - window_height,
                ),
                NSSize::new(WIDTH, window_height),
            ),
            true,
        );
    }
    ns_window.orderFrontRegardless();
}

/// Whether a card is why Hopp just came forward, which shouldn't open anything else. Either
/// that's a click on one, or the cards window has only now been made.
#[cfg(target_os = "macos")]
pub fn explains_activation(app: &AppHandle) -> bool {
    use objc2_app_kit::{NSEvent, NSWindow};

    if focus::just_made_a_webview() {
        focus::CLICKED.store(true, std::sync::atomic::Ordering::Relaxed);
        focus::hand_back(app);
        return true;
    }
    let Some(window) = app.get_webview_window(CARDS_WINDOW_LABEL) else {
        return false;
    };
    let Ok(ns_window) = window.ns_window() else {
        return false;
    };
    let ns_window: &NSWindow = unsafe { &*ns_window.cast() };
    if !ns_window.isVisible() {
        return false;
    }
    let frame = ns_window.frame();
    let cursor = NSEvent::mouseLocation();
    let over = crate::call_alert::rect_contains_point(
        (frame.origin.x, frame.origin.y),
        (frame.size.width, frame.size.height),
        (cursor.x, cursor.y),
    );
    if over {
        focus::CLICKED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    over
}

#[cfg(target_os = "macos")]
pub use focus::watch as watch_focus;

#[cfg(target_os = "macos")]
pub(crate) use focus::making_a_webview;

/// Back to the app you were in, whatever brought Hopp forward.
#[cfg(target_os = "macos")]
pub(crate) fn hand_focus_back(app: &AppHandle) {
    focus::CLICKED.store(true, std::sync::atomic::Ordering::Relaxed);
    focus::hand_back(app);
}

/// Which app had the focus before a click on a card took it.
#[cfg(target_os = "macos")]
mod focus {
    use std::{
        ptr::NonNull,
        sync::{
            atomic::{AtomicBool, AtomicI32, Ordering},
            Mutex, PoisonError,
        },
        time::{Duration, Instant},
    };

    use objc2_app_kit::{
        NSApplicationActivationOptions, NSRunningApplication, NSWorkspace,
        NSWorkspaceDidActivateApplicationNotification,
    };
    use objc2_foundation::NSNotification;
    use tauri::AppHandle;

    /// A click on a card is what brought Hopp forward.
    pub static CLICKED: AtomicBool = AtomicBool::new(false);
    /// The process of the last app active that wasn't Hopp.
    static PREVIOUS: AtomicI32 = AtomicI32::new(0);

    /// wry asks for the app to come forward whenever it makes a webview, whatever the window
    /// was told, and this is when that was.
    static WEBVIEW_MADE: Mutex<Option<Instant>> = Mutex::new(None);
    const WEBVIEW_ACTIVATES_WITHIN: Duration = Duration::from_secs(3);

    pub fn making_a_webview() {
        *WEBVIEW_MADE.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    pub fn just_made_a_webview() -> bool {
        WEBVIEW_MADE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|made| made.elapsed() < WEBVIEW_ACTIVATES_WITHIN)
    }

    /// Remember the app in front, unless that's Hopp. Says whether it wasn't.
    pub fn note_whats_in_front() -> bool {
        let own = std::process::id() as i32;
        let front = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|front| front.processIdentifier());
        match front.filter(|front| *front != own) {
            Some(front) => {
                PREVIOUS.store(front, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// From startup on, keep track of the app in front. Runs on the main thread.
    pub fn watch() {
        note_whats_in_front();
        let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
            if note_whats_in_front() {
                CLICKED.store(false, Ordering::Relaxed);
            }
        });
        // The notification center holds on to this for as long as the app runs.
        let _observer = unsafe {
            NSWorkspace::sharedWorkspace()
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceDidActivateApplicationNotification),
                    None,
                    None,
                    &block,
                )
        };
    }

    /// Back to the app you were in, if a card is what took you away.
    pub fn hand_back(app: &AppHandle) {
        if !CLICKED.swap(false, Ordering::Relaxed) {
            return;
        }
        let previous = PREVIOUS.load(Ordering::Relaxed);
        if previous == 0 {
            return;
        }
        let result = app.run_on_main_thread(move || {
            if let Some(previous) =
                NSRunningApplication::runningApplicationWithProcessIdentifier(previous)
            {
                // `activate` needs macOS 14, and this still does what's wanted from the app
                // that's in front.
                #[allow(deprecated)]
                previous.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        });
        if let Err(e) = result {
            log::error!("cards: hand_back: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_grows_to_three_cards_then_a_more_line() {
        assert_eq!(height(0), CARD_HEIGHT, "never zero-sized");
        assert_eq!(height(1), CARD_HEIGHT);
        assert_eq!(height(2), 2.0 * CARD_HEIGHT + GAP);
        assert_eq!(height(3), 3.0 * CARD_HEIGHT + 2.0 * GAP);
        assert_eq!(height(7), 3.0 * CARD_HEIGHT + 3.0 * GAP + MORE_HEIGHT);
    }
}
