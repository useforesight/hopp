use hopp::AppData;
use socket_lib::{CameraStartMessage, Message};
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

pub struct CallShortcuts {
    pub mic: String,
    pub camera: String,
    pub screenshare: String,
    pub end_call: String,
}

pub fn register_call_shortcuts(app: &tauri::AppHandle, shortcuts: CallShortcuts) {
    register_call_shortcut(app, "mic", &shortcuts.mic, handle_mic);
    register_call_shortcut(app, "camera", &shortcuts.camera, handle_camera);
    register_call_shortcut(
        app,
        "screenshare",
        &shortcuts.screenshare,
        handle_screenshare,
    );
    register_call_shortcut(app, "end_call", &shortcuts.end_call, handle_end_call);
}

/// An empty accelerator means the user has no shortcut for this action.
fn register_call_shortcut(
    app: &tauri::AppHandle,
    name: &str,
    accelerator: &str,
    handler: fn(&tauri::AppHandle),
) {
    if accelerator.is_empty() {
        return;
    }
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(accelerator, move |app, _sc, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            handler(app);
        })
    {
        log::error!(
            "register_call_shortcuts: failed to register {name} shortcut '{accelerator}': {e}"
        );
    }
}

pub fn unregister_call_shortcuts(app: &tauri::AppHandle) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        log::error!("unregister_call_shortcuts: {e}");
    }
}

pub fn handle_mic(app: &tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::ToggleMic) {
        log::error!("handle_mic shortcut: {e}");
    }
}

pub fn handle_camera(app: &tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if data.is_camera_on {
        if let Err(e) = data.sender.send(Message::StopCamera) {
            log::error!("handle_camera shortcut (stop): {e}");
        }
    } else {
        let device_name = data.app_state.last_used_camera();
        if let Err(e) = data
            .sender
            .send(Message::StartCamera(CameraStartMessage { device_name }))
        {
            log::error!("handle_camera shortcut (start): {e}");
        }
    }
}

pub fn handle_end_call(app: &tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    if let Err(e) = data.send_call_end() {
        log::error!("handle_end_call shortcut: {e}");
    }
}

pub fn handle_screenshare(app: &tauri::AppHandle) {
    let (is_screensharing, sender) = {
        let data = app.state::<Mutex<AppData>>();
        let data = data.lock().unwrap();
        (data.is_screensharing, data.sender.clone())
    };

    let message = if is_screensharing {
        Message::StopScreenshare
    } else {
        Message::GetAvailableContent
    };

    if let Err(e) = sender.send(message) {
        log::error!("handle_screenshare shortcut: {e}");
    }
}
