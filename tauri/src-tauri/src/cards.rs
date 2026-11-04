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
        #[cfg(target_os = "macos")]
        focus::hand_back(&app);
    }
}

/// Bring the window up to date with the stack.
fn changed(app: &AppHandle) -> Result<(), String> {
    let showing = app
        .state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .showing();
    let window = match app.get_webview_window(CARDS_WINDOW_LABEL) {
        Some(window) => window,
        None if showing.is_empty() => return Ok(()),
        // Outlives each card and is only hidden, as the call card's window is.
        None => WebviewWindowBuilder::new(
            app,
            CARDS_WINDOW_LABEL,
            WebviewUrl::App("cards.html".into()),
        )
        .title("Hopp cards")
        .inner_size(WIDTH, height(showing.len()))
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
        .map_err(|e| format!("Failed to create the cards window: {e}"))?,
    };
    if let Err(e) = app.emit_to(CARDS_WINDOW_LABEL, "cards_changed", &showing) {
        log::error!("cards: failed to emit: {e:?}");
    }
    if showing.is_empty() {
        return window
            .hide()
            .map_err(|e| format!("Failed to hide the cards window: {e}"));
    }

    #[cfg(target_os = "macos")]
    {
        let (for_main_thread, count) = (app.clone(), showing.len());
        window
            .run_on_main_thread(move || place(&for_main_thread, count))
            .map_err(|e| format!("Failed to show the cards window: {e}"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_size(tauri::LogicalSize::new(WIDTH, height(showing.len())))
            .and_then(|_| window.show())
            .map_err(|e| format!("Failed to show the cards window: {e}"))
    }
}

/// The call card has come or gone: the stack goes under it.
#[cfg(target_os = "macos")]
pub fn make_room(app: &AppHandle) {
    let Some(window) = app.get_webview_window(CARDS_WINDOW_LABEL) else {
        return;
    };
    let count = app
        .state::<Cards>()
        .stack
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .showing()
        .len();
    if count == 0 {
        return;
    }
    let for_main_thread = app.clone();
    if let Err(e) = window.run_on_main_thread(move || place(&for_main_thread, count)) {
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

/// Whether the cursor is on the stack, for a click on a card bringing Hopp forward: that click
/// is for the card, and shouldn't open anything else.
#[cfg(target_os = "macos")]
pub fn took_a_click(app: &AppHandle) -> bool {
    use objc2_app_kit::{NSEvent, NSWindow};

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

/// Which app had the focus before a click on a card took it.
#[cfg(target_os = "macos")]
mod focus {
    use std::{
        ptr::NonNull,
        sync::atomic::{AtomicBool, AtomicI32, Ordering},
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

    /// From startup on, keep track of the app in front. Runs on the main thread.
    pub fn watch() {
        let own = std::process::id() as i32;
        let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
            let front = NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .map(|front| front.processIdentifier());
            if let Some(front) = front.filter(|front| *front != own) {
                PREVIOUS.store(front, Ordering::Relaxed);
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

    /// After answering a card: back to the app you were in, if the card is what took you away.
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
