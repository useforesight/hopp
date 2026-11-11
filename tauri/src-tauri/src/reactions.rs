//! Reactions: confetti, a gong, fireworks… that play on everyone's screen in the call (see
//! `socket_lib::Reaction`). The core sends and receives them; here they're picked, in the
//! tray, and played, on a layer over the whole screen.
//!
//! The layer floats above everything, lets every click through and is hidden from your own
//! screen share. It's made once and only hidden while nothing plays, because making a webview
//! brings Hopp forward.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use socket_lib::{CoreParticipantState, Message, Reaction, ReactionPlay};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::AppData;

pub const LAYER_WINDOW_LABEL: &str = "reactions";
pub const TRAY_WINDOW_LABEL: &str = "reactionsTray";

const TRAY_WIDTH: f64 = 336.0;
const TRAY_HEIGHT: f64 = 404.0;
/// Between the reactions button and the tray.
#[cfg(target_os = "macos")]
const TRAY_GAP: f64 = 14.0;
/// Clicking the reactions button while the tray is open takes the tray's focus, which closes
/// it, just before the click arrives. That click is to close it, not to open it again.
const TRAY_JUST_CLOSED: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct Reactions {
    layer: Mutex<Layer>,
    /// Held while finding or making the layer's window, so that it's only made once.
    making: Mutex<()>,
    /// Everyone else in the call, for an achievement that's for one of them.
    people: Mutex<Vec<Person>>,
    tray_closed: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct Layer {
    /// The page is listening.
    ready: bool,
    /// Plays the page hasn't had, because it wasn't listening yet.
    waiting: Vec<Play>,
    /// Counts the plays, so that the page saying it's done can't hide one only now arriving.
    played: u64,
}

/// A reaction for the layer, with where the call window is on it.
#[derive(Serialize, Clone)]
pub struct Play {
    seq: u64,
    play: ReactionPlay,
    /// The call window, in points from the top left of the layer: x, y, width, height.
    call_window: Option<[f64; 4]>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Person {
    identity: String,
    name: String,
}

/// An identity without its connection's suffix, as reactions name people.
fn person_identity(identity: &str) -> &str {
    identity
        .strip_suffix(":audio")
        .or_else(|| identity.strip_suffix(":video"))
        .or_else(|| identity.strip_suffix(":camera"))
        .unwrap_or(identity)
}

/// Everyone else in the call, once each, from the core's snapshot.
fn others(snapshot: &[CoreParticipantState]) -> Vec<Person> {
    let mut people: Vec<Person> = Vec::new();
    for participant in snapshot {
        if participant.identity == "local" {
            continue;
        }
        let identity = person_identity(&participant.identity);
        if people.iter().any(|known| known.identity == identity) {
            continue;
        }
        people.push(Person {
            identity: identity.to_string(),
            name: participant.name.clone(),
        });
    }
    people
}

pub fn note_people(app: &AppHandle, snapshot: &[CoreParticipantState]) {
    *app.state::<Reactions>()
        .people
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = others(snapshot);
}

/// Make the layer ahead of the first reaction, so that one plays at once.
pub fn prepare(app: &AppHandle) {
    if let Err(e) = make_layer(app) {
        log::error!("reactions: prepare: {e}");
    }
}

/// Play a reaction from the core.
pub fn play(app: &AppHandle, play: ReactionPlay) {
    if let Err(e) = make_layer(app) {
        log::error!("reactions: play: {e}");
        return;
    }
    let for_main_thread = app.clone();
    let result = app.run_on_main_thread(move || {
        let call_window = place_layer(&for_main_thread, play.call_window);
        deliver(&for_main_thread, play, call_window);
    });
    if let Err(e) = result {
        log::error!("reactions: play: {e:?}");
    }
}

/// The call is over: nothing more to play or send.
pub fn call_ended(app: &AppHandle) {
    app.state::<Reactions>()
        .people
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
    if let Some(tray) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        if let Err(e) = tray.hide() {
            log::error!("reactions: call_ended: tray: {e:?}");
        }
    }
    if let Some(layer) = app.get_webview_window(LAYER_WINDOW_LABEL) {
        if let Err(e) = app.emit_to(LAYER_WINDOW_LABEL, "reactions_stop", ()) {
            log::error!("reactions: call_ended: {e:?}");
        }
        if let Err(e) = layer.hide() {
            log::error!("reactions: call_ended: layer: {e:?}");
        }
    }
}

fn make_layer(app: &AppHandle) -> Result<(), String> {
    let reactions = app.state::<Reactions>();
    let _making = reactions
        .making
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if app.get_webview_window(LAYER_WINDOW_LABEL).is_some() {
        return Ok(());
    }
    *reactions
        .layer
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Layer::default();
    #[cfg(target_os = "macos")]
    crate::cards::making_a_webview();
    let window = WebviewWindowBuilder::new(
        app,
        LAYER_WINDOW_LABEL,
        WebviewUrl::App("reactions.html".into()),
    )
    .title("Hopp reactions")
    .inner_size(800.0, 600.0)
    .visible(false)
    .focused(false)
    .focusable(false)
    .transparent(true)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .resizable(false)
    .skip_taskbar(true)
    .content_protected(true)
    .build()
    .map_err(|e| format!("Failed to create the reactions window: {e}"))?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|e| format!("Failed to let clicks through the reactions window: {e}"))
}

/// Hand the page a play, or keep it until the page is listening. Runs on the main thread, as
/// hiding the layer does, so that one can't come between the other's check and its change.
fn deliver(app: &AppHandle, play: ReactionPlay, call_window: Option<[f64; 4]>) {
    let reactions = app.state::<Reactions>();
    let mut layer = reactions
        .layer
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    layer.played += 1;
    let play = Play {
        seq: layer.played,
        play,
        call_window,
    };
    if !layer.ready {
        layer.waiting.push(play);
        return;
    }
    if let Err(e) = app.emit_to(LAYER_WINDOW_LABEL, "reaction_play", &play) {
        log::error!("reactions: failed to emit: {e:?}");
    }
}

/// Over the whole screen with the call window on it, or else the one with the pointer.
/// Returns where the call window is on the layer. Runs on the main thread.
#[cfg(target_os = "macos")]
fn place_layer(app: &AppHandle, call_window: Option<[f64; 4]>) -> Option<[f64; 4]> {
    use objc2_app_kit::{NSScreen, NSWindow};
    use objc2_foundation::{MainThreadMarker, NSRect};

    use crate::call_alert;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::error!("reactions: place_layer must run on the main thread");
        return None;
    };
    let window = app.get_webview_window(LAYER_WINDOW_LABEL)?;
    let ns_window: &NSWindow = match window.ns_window() {
        Ok(ns_window) => unsafe { &*ns_window.cast() },
        Err(e) => {
            log::error!("reactions: failed to get NSWindow: {e:?}");
            return None;
        }
    };
    let screens = NSScreen::screens(main_thread);
    // The core has the call window from the top left of the main screen, the first.
    let main_height = screens.firstObject()?.frame().size.height;
    let from_top = |frame: NSRect| {
        [
            frame.origin.x,
            main_height - frame.origin.y - frame.size.height,
            frame.size.width,
            frame.size.height,
        ]
    };
    let screen = call_window
        .and_then(|[x, y, width, height]| {
            let centre = (x + width / 2.0, y + height / 2.0);
            screens.iter().find(|screen| {
                let [left, top, width, height] = from_top(screen.frame());
                call_alert::rect_contains_point((left, top), (width, height), centre)
            })
        })
        .or_else(|| call_alert::screen_under_cursor(main_thread))?;

    let frame = screen.frame();
    call_alert::float(ns_window);
    ns_window.setIgnoresMouseEvents(true);
    ns_window.setFrame_display(frame, true);
    ns_window.orderFrontRegardless();

    let [left, top, ..] = from_top(frame);
    call_window.map(|[x, y, width, height]| [x - left, y - top, width, height])
}

#[cfg(not(target_os = "macos"))]
fn place_layer(app: &AppHandle, call_window: Option<[f64; 4]>) -> Option<[f64; 4]> {
    let window = app.get_webview_window(LAYER_WINDOW_LABEL)?;
    let monitor = window.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let position = monitor.position().to_logical::<f64>(scale);
    let size = monitor.size().to_logical::<f64>(scale);
    if let Err(e) = window
        .set_position(position)
        .and_then(|_| window.set_size(size))
        .and_then(|_| window.show())
    {
        log::error!("reactions: failed to show the layer: {e:?}");
    }
    call_window.map(|[x, y, width, height]| [x - position.x, y - position.y, width, height])
}

/// The page is listening: these came before it was.
#[tauri::command]
pub fn reactions_ready(reactions: State<Reactions>) -> Vec<Play> {
    let mut layer = reactions
        .layer
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    layer.ready = true;
    std::mem::take(&mut layer.waiting)
}

/// Nothing's playing since play `seq`: the layer can go until the next.
#[tauri::command]
pub fn reactions_idle(app: AppHandle, seq: u64) {
    let for_main_thread = app.clone();
    let result = app.run_on_main_thread(move || {
        let played = for_main_thread
            .state::<Reactions>()
            .layer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .played;
        if played != seq {
            return;
        }
        if let Some(layer) = for_main_thread.get_webview_window(LAYER_WINDOW_LABEL) {
            if let Err(e) = layer.hide() {
                log::error!("reactions: failed to hide the layer: {e:?}");
            }
        }
    });
    if let Err(e) = result {
        log::error!("reactions_idle: {e:?}");
    }
}

/// Send a reaction to the call. The core plays it here too.
#[tauri::command]
pub fn reactions_send(app: AppHandle, reaction: Reaction) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap_or_else(PoisonError::into_inner);
    if !data.call_active {
        return;
    }
    if let Err(e) = data.sender.send(Message::ReactionSend(reaction)) {
        log::error!("reactions_send: {e:?}");
    }
}

/// Everyone else in the call.
#[tauri::command]
pub fn reactions_people(reactions: State<Reactions>) -> Vec<Person> {
    reactions
        .people
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Close the tray. On Escape, back to where you were, which a click outside it already is.
#[tauri::command]
pub fn reactions_close_tray(app: AppHandle, hand_back: bool) {
    *app.state::<Reactions>()
        .tray_closed
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    if let Some(tray) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        if let Err(e) = tray.hide() {
            log::error!("reactions_close_tray: {e:?}");
        }
    }
    #[cfg(target_os = "macos")]
    if hand_back {
        crate::cards::hand_focus_back(&app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = hand_back;
}

/// The reactions button: the tray, by the pointer, or closed if it's open.
pub fn toggle_tray(app: &AppHandle) {
    let just_closed = app
        .state::<Reactions>()
        .tray_closed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .is_some_and(|closed| closed.elapsed() < TRAY_JUST_CLOSED);
    if let Some(tray) = app.get_webview_window(TRAY_WINDOW_LABEL) {
        if tray.is_visible().unwrap_or(false) {
            reactions_close_tray(app.clone(), false);
            return;
        }
    }
    if just_closed {
        return;
    }
    let tray = match app.get_webview_window(TRAY_WINDOW_LABEL) {
        Some(tray) => tray,
        None => match WebviewWindowBuilder::new(
            app,
            TRAY_WINDOW_LABEL,
            WebviewUrl::App("reactionsTray.html".into()),
        )
        .title("Reactions")
        .inner_size(TRAY_WIDTH, TRAY_HEIGHT)
        .visible(false)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .accept_first_mouse(true)
        .content_protected(true)
        .build()
        {
            Ok(tray) => tray,
            Err(e) => {
                log::error!("reactions: failed to create the tray: {e:?}");
                return;
            }
        },
    };
    if let Err(e) = app.emit_to(TRAY_WINDOW_LABEL, "reactions_tray_opened", ()) {
        log::error!("reactions: tray opened: {e:?}");
    }

    #[cfg(target_os = "macos")]
    {
        let for_main_thread = tray.clone();
        let result = tray.run_on_main_thread(move || present_tray(&for_main_thread));
        if let Err(e) = result {
            log::error!("reactions: failed to show the tray: {e:?}");
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = tray.show().and_then(|_| tray.set_focus()) {
        log::error!("reactions: failed to show the tray: {e:?}");
    }
}

/// Below the pointer, which is on the reactions button, or above it when there's no room
/// below; kept on the screen. Runs on the main thread.
#[cfg(target_os = "macos")]
fn present_tray(tray: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSEvent, NSWindow};
    use objc2_foundation::{MainThreadMarker, NSPoint};

    use crate::call_alert;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::error!("reactions: present_tray must run on the main thread");
        return;
    };
    let ns_window: &NSWindow = match tray.ns_window() {
        Ok(ns_window) => unsafe { &*ns_window.cast() },
        Err(e) => {
            log::error!("reactions: failed to get the tray's NSWindow: {e:?}");
            return;
        }
    };
    call_alert::float(ns_window);
    if let Some(screen) = call_alert::screen_under_cursor(main_thread) {
        let visible = screen.visibleFrame();
        let cursor = NSEvent::mouseLocation();
        let (bottom, top) = (visible.origin.y, visible.origin.y + visible.size.height);
        let below = cursor.y - TRAY_GAP;
        let tray_top = if below - TRAY_HEIGHT >= bottom {
            below
        } else {
            (cursor.y + TRAY_GAP + TRAY_HEIGHT).min(top)
        };
        let left = (cursor.x - TRAY_WIDTH / 2.0).clamp(
            visible.origin.x,
            visible.origin.x + visible.size.width - TRAY_WIDTH,
        );
        ns_window.setFrameTopLeftPoint(NSPoint::new(left, tray_top));
    }
    ns_window.orderFrontRegardless();
    if let Err(e) = tray.set_focus() {
        log::error!("reactions: failed to focus the tray: {e:?}");
    }
}

/// Hopp came forward because the tray opened, which shouldn't open anything else.
pub fn explains_activation(app: &AppHandle) -> bool {
    app.get_webview_window(TRAY_WINDOW_LABEL)
        .is_some_and(|tray| tray.is_visible().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(identity: &str, name: &str) -> CoreParticipantState {
        CoreParticipantState {
            identity: identity.to_string(),
            name: name.to_string(),
            connected: true,
            muted: false,
            has_camera: false,
            is_screensharing: false,
            capturing: false,
            is_speaking: false,
        }
    }

    #[test]
    fn the_people_are_everyone_else_once_each() {
        let people = others(&[
            participant("local", "Me"),
            participant("room:r:alex:audio", "Alex Smith"),
            participant("room:r:sam:video", "Sam"),
            participant("room:r:alex:video", "Alex Smith"),
        ]);
        assert_eq!(
            people,
            vec![
                Person {
                    identity: "room:r:alex".into(),
                    name: "Alex Smith".into()
                },
                Person {
                    identity: "room:r:sam".into(),
                    name: "Sam".into()
                },
            ]
        );
    }
}
