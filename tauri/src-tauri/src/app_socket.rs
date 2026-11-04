//! What the `hopp` command asks of the running app, answered over the socket in
//! `hopp_capture::app_socket`. The call controls are the keyboard shortcuts' own; what only
//! the window knows how to do (who to call, which room, the card that's ringing) goes to it.

use std::sync::{Mutex, PoisonError};

use hopp::AppData;
use hopp_capture::app_socket::{AppReply, AppRequest};
use serde_json::Value;
use socket_lib::Message;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::shortcuts;

/// What the main window last published, and the call the captures window should show.
#[derive(Default)]
pub struct AppSocket {
    state: Mutex<Option<Value>>,
    captures_call: Mutex<Option<String>>,
}

/// The main window says what's going on whenever it changes.
#[tauri::command(async)]
pub fn app_publish_state(state: Value, socket: State<AppSocket>) {
    *socket.state.lock().unwrap_or_else(PoisonError::into_inner) = Some(state);
}

/// The call the captures window was opened for, once.
#[tauri::command(async)]
pub fn captures_requested_call(socket: State<AppSocket>) -> Option<String> {
    socket
        .captures_call
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

#[cfg(unix)]
pub fn start(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("hopp-app-socket".into())
        .spawn(move || {
            let served = hopp_capture::app_socket::socket_path().and_then(|path| {
                hopp_capture::app_socket::serve(&path, |request| answer(&app, request))
            });
            if let Err(error) = served {
                log::error!("App socket: {error:#}");
            }
        });
    if let Err(error) = spawned {
        log::error!("App socket: {error}");
    }
}

#[cfg(not(unix))]
pub fn start(_app: AppHandle) {}

fn answer(app: &AppHandle, request: AppRequest) -> AppReply {
    log::info!("app_socket: {request:?}");
    let socket = app.state::<AppSocket>();
    match request {
        AppRequest::Snapshot => {
            let capture = {
                let data = app.state::<Mutex<AppData>>();
                let data = data.lock().unwrap_or_else(PoisonError::into_inner);
                data.capture_id.clone()
            };
            let state = socket.state.lock().unwrap_or_else(PoisonError::into_inner);
            match state.as_ref() {
                Some(state) => {
                    let mut state = state.clone();
                    if let Some(id) = capture {
                        state["capture"] = serde_json::json!({ "id": id });
                    }
                    AppReply::state(state)
                }
                None => AppReply::done(),
            }
        }
        AppRequest::Microphone => in_call(app, shortcuts::handle_mic),
        AppRequest::Camera => in_call(app, shortcuts::handle_camera),
        AppRequest::Leave => in_call(app, shortcuts::handle_end_call),
        AppRequest::Share => in_call(app, shortcuts::handle_screenshare),
        AppRequest::Draw => draw(app),
        AppRequest::CaptureStart => in_call(app, |app| crate::captures_start(app.clone())),
        AppRequest::CaptureStop => {
            crate::captures_stop(app.clone());
            AppReply::done()
        }
        AppRequest::CaptureBookmark(label) => {
            let data = app.state::<Mutex<AppData>>();
            let data = data.lock().unwrap_or_else(PoisonError::into_inner);
            match data.sender.send(Message::CaptureBookmark(label)) {
                Ok(()) => AppReply::done(),
                Err(error) => AppReply::refused(format!("Couldn't reach the call: {error}")),
            }
        }
        AppRequest::CaptureWindow(call) => {
            *socket
                .captures_call
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = call;
            match tauri::async_runtime::block_on(crate::create_captures_window(app.clone())) {
                Ok(()) => {
                    let _ = app.emit_to("captures", "captures_call_requested", ());
                    AppReply::done()
                }
                Err(error) => AppReply::refused(error),
            }
        }
        AppRequest::Settings => {
            match tauri::async_runtime::block_on(crate::create_settings_window(app.clone())) {
                Ok(()) => AppReply::done(),
                Err(error) => AppReply::refused(error),
            }
        }
        AppRequest::Call(_)
        | AppRequest::JoinRoom(_)
        | AppRequest::Accept
        | AppRequest::Decline
        | AppRequest::Open => to_the_window(app, &socket, &request),
    }
}

fn draw(app: &AppHandle) -> AppReply {
    let sharing = {
        let data = app.state::<Mutex<AppData>>();
        let data = data.lock().unwrap_or_else(PoisonError::into_inner);
        data.call_active && data.is_screensharing
    };
    if !sharing {
        return AppReply::refused("You're not sharing your screen");
    }
    if !crate::toggle_pen(app) {
        return AppReply::refused("Couldn't reach the call");
    }
    AppReply::done()
}

fn in_call(app: &AppHandle, action: impl FnOnce(&AppHandle)) -> AppReply {
    let active = {
        let data = app.state::<Mutex<AppData>>();
        let data = data.lock().unwrap_or_else(PoisonError::into_inner);
        data.call_active
    };
    if !active {
        return AppReply::refused("You're not in a call");
    }
    action(app);
    AppReply::done()
}

fn to_the_window(app: &AppHandle, socket: &AppSocket, request: &AppRequest) -> AppReply {
    let ready = socket
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if !ready {
        return AppReply::refused("Hopp is still starting");
    }
    match app.emit_to("main", "app_request", request) {
        Ok(()) => AppReply::done(),
        Err(error) => AppReply::refused(format!("Couldn't reach Hopp's window: {error}")),
    }
}
