// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hopp::sounds::{self, SoundConfig};
use log::LevelFilter;
use socket_lib::{
    AudioCaptureMessage, AudioDevice, CameraDevice, DrawingEnabled, Message, ScreenSharePickerMode,
    ScreenShareResolution, SentryMetadata,
};
use std::sync::mpsc as std_mpsc;
use tauri::Manager;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    path::BaseDirectory,
    Emitter,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use tauri_plugin_log::{Target, TargetKind};

use hopp::{
    app_state::{resolve_microphone, AppState, AppVeilApplication, UserSettings},
    application_catalog::InstalledApplication,
    create_core_process, get_log_level, get_log_path, get_sentry_dsn, permissions, ping_frontend,
    recv_expected_response, setup_start_on_launch, setup_tray_icon, AppData,
};
mod shortcuts;
#[cfg(target_os = "macos")]
use hopp::{disable_app_nap, set_window_corner_radius_and_decorations, CORNER_RADIUS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::{env, sync::Arc};

#[cfg(any(target_os = "windows", target_os = "linux"))]
use tauri::PhysicalPosition;

#[tauri::command(async)]
async fn open_stats_window(app: tauri::AppHandle) {
    log::info!("open_stats_window");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::OpenStatsWindow) {
        log::error!("open_stats_window: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
async fn stop_sharing(app: tauri::AppHandle) {
    log::info!("stop_sharing");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::StopScreenshare) {
        log::error!("stop_sharing: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
async fn get_available_content(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("get_available_content: open core screen selection");

    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();

    let remote_control_enabled = data.app_state.user_settings().remote_control_enabled;
    data.sender
        .send(Message::ControllerCursorEnabled(remote_control_enabled))
        .map_err(|e| {
            log::error!("get_available_content: failed to apply remote control setting: {e:?}");
            "Failed to apply remote control setting".to_string()
        })?;

    data.sender.send(Message::GetAvailableContent).map_err(|e| {
        log::error!("get_available_content: failed to send message: {e:?}");
        "Failed to start screen selection".to_string()
    })
}

#[tauri::command(async)]
fn play_sound(app: tauri::AppHandle, sound_name: String) {
    log::info!("play_sound");
    let tmp_sound_name = sound_name.split("/").last();
    if let Some(tmp_sound_name) = tmp_sound_name {
        log::info!("Playing sound: {}", tmp_sound_name);
    }
    let sounds = hopp::sounds::get_all_sounds();
    let mut sound_path = "".to_string();
    let mut sound_config = SoundConfig::default();
    for sound in sounds {
        if sound.0.contains(&sound_name) {
            let resource_path = app.path().resolve(sound.0, BaseDirectory::Resource);
            if let Err(e) = resource_path {
                log::error!("play_sound: Failed to resolve sound path: {e:?}");
                return;
            }
            sound_path = resource_path.unwrap().to_string_lossy().to_string();
            sound_config = sound.1;
            break;
        }
    }
    if sound_path.is_empty() {
        log::error!("play_sound: Failed to find sound");
        return;
    }

    // Held from this check until the new entry is in, so that two plays at the same
    // time can't both start the sound and leave one that nothing can stop.
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();

    /* A sound that has finished has a closed channel, so its entry goes. */
    data.sound_entries.retain(|entry| {
        entry.name != sound_name || entry.tx.send(sounds::SoundCommand::Ping).is_ok()
    });
    if data
        .sound_entries
        .iter()
        .any(|entry| entry.name == sound_name)
    {
        log::warn!("play_sound: Sound is already playing");
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let res = hopp::sounds::play_sound(sound_path, sound_config, rx);
        if res.is_err() {
            log::error!("play_sound: Failed to play sound: {:?}", res.err());
        }
    });

    data.sound_entries.push(sounds::SoundEntry {
        name: sound_name,
        tx,
    });
}

#[tauri::command(async)]
fn stop_sound(app: tauri::AppHandle, sound_name: String) {
    log::info!("stop_sound");
    let tmp_sound_name = sound_name.split("/").last();
    if let Some(tmp_sound_name) = tmp_sound_name {
        log::info!("Stopping sound: {}", tmp_sound_name);
    }
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.sound_entries.retain(|entry| {
        if entry.name != sound_name {
            return true;
        }
        let _ = entry.tx.send(sounds::SoundCommand::Stop);
        false
    });
    log::debug!("stop_sound: entries left: {}", data.sound_entries.len());
}

#[tauri::command(async)]
fn reset_core_process(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    if let Err(e) = data.send_call_end() {
        log::error!("reset_core_process: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
fn store_token_cmd(app: tauri::AppHandle, token: String) {
    log::info!("store_token_cmd");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_user_jwt(Some(token.clone()));

    if let Err(e) = app.emit("token_changed", token) {
        log::error!("Failed to emit token_changed event: {e:?}");
    }
}

#[tauri::command(async)]
fn get_stored_token(app: tauri::AppHandle) -> Option<String> {
    log::info!("get_stored_token");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let token = data.app_state.user_jwt().clone();
    log::debug!("get_stored_token: {token:?}");
    token
}

#[tauri::command(async)]
fn delete_stored_token(app: tauri::AppHandle) {
    log::info!("Deleting stored token");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_user_jwt(None);

    if let Err(e) = app.emit("token_changed", "".to_string()) {
        log::error!("Failed to emit token_changed event: {e:?}");
    }
}

#[tauri::command(async)]
fn get_logs(_app: tauri::AppHandle) -> String {
    log::info!("get_logs:");
    let log_file = get_log_path();
    if let Some(path) = log_file {
        path.to_string_lossy().to_string()
    } else {
        log::error!("Failed to get log path");
        "".to_string()
    }
}

#[tauri::command(async)]
fn set_deactivate_hiding(app: tauri::AppHandle, deactivate: bool) {
    log::debug!("set_deactivate_hiding: {deactivate}");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let mut deactivate_hiding = data.deactivate_hiding.lock().unwrap();
    *deactivate_hiding = deactivate;
}

#[tauri::command(async)]
fn set_controller_cursor(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_controller_cursor: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::ControllerCursorEnabled(enabled)) {
        log::error!("set_controller_cursor: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
fn open_accessibility_settings(_app: tauri::AppHandle) {
    log::info!("open_accessibility_settings");
    let mut process = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn()
        .expect("Failed to open System Preferences for Accessibility permissions");
    let _ = process.wait();
}

#[tauri::command(async)]
fn open_microphone_settings(_app: tauri::AppHandle) {
    log::info!("open_microphone_settings");
    permissions::request_microphone();
}

#[tauri::command(async)]
fn open_camera_settings(_app: tauri::AppHandle) {
    log::info!("open_camera_settings");
    permissions::request_camera();
}

#[tauri::command(async)]
fn open_screenshare_settings(_app: tauri::AppHandle) {
    log::info!("open_screenshare_settings");
    let mut process = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn()
        .expect("Failed to open System Preferences for Screen Capture permissions");
    let _ = process.wait();
}

#[tauri::command(async)]
async fn trigger_screenshare_permission(_app: tauri::AppHandle) -> bool {
    log::info!("trigger_screenshare_permission");
    permissions::request_screenshare()
}

#[tauri::command(async)]
fn get_control_permission(_app: tauri::AppHandle) -> bool {
    let res = permissions::accessibility();
    log::info!("get_control_permission: {res}");
    res
}

#[tauri::command(async)]
fn get_microphone_permission(_app: tauri::AppHandle) -> bool {
    let res = permissions::microphone();
    log::info!("get_microphone_permission: {res}");
    res
}

#[tauri::command(async)]
fn get_screenshare_permission(_app: tauri::AppHandle) -> bool {
    let res = permissions::screenshare();
    log::info!("get_screenshare_permission: {res}");
    res
}

#[tauri::command(async)]
fn get_camera_permission(_app: tauri::AppHandle) -> bool {
    let res = permissions::camera();
    log::info!("get_camera_permission: {res}");
    res
}

#[tauri::command(async)]
fn skip_tray_notification_selection_window(app: tauri::AppHandle) {
    log::info!("executing skip_tray_notification_selection_window");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_tray_notification(false);
}

#[tauri::command(async)]
fn get_last_used_mic(app: tauri::AppHandle) -> Option<String> {
    log::info!("get_last_used_mic");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let value = data.app_state.last_used_mic();
    log::info!("get_last_used_mic: {value:?}");
    value
}

#[tauri::command(async)]
fn set_last_used_mic(app: tauri::AppHandle, mic: String) {
    log::info!("set_last_used_mic: {mic}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_last_used_mic(mic);
}

#[tauri::command(async)]
fn get_last_used_camera(app: tauri::AppHandle) -> Option<String> {
    log::info!("get_last_used_camera");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let value = data.app_state.last_used_camera();
    log::info!("get_last_used_camera: {value:?}");
    value
}

#[tauri::command(async)]
fn set_last_used_camera(app: tauri::AppHandle, camera: String) {
    log::info!("set_last_used_camera: {camera}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_last_used_camera(camera);
}

#[tauri::command(async)]
fn get_sharer_draw_persist(app: tauri::AppHandle) -> bool {
    log::info!("get_sharer_draw_persist");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let value = data.app_state.sharer_draw_persist();
    log::info!("get_sharer_draw_persist: {value}");
    value
}

#[tauri::command(async)]
fn set_sharer_draw_persist(app: tauri::AppHandle, persist: bool) {
    log::info!("set_sharer_draw_persist: {persist}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_sharer_draw_persist(persist);

    if data.drawing_enabled {
        if let Err(e) = data.sender.send(Message::SharerDrawPersistChanged(persist)) {
            log::error!("set_sharer_draw_persist: failed to send message: {e:?}");
        }
    }
}

#[tauri::command(async)]
fn get_drawing_hint_shown(app: tauri::AppHandle) -> bool {
    log::info!("get_drawing_hint_shown");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let value = data.app_state.drawing_hint_shown();
    log::info!("get_drawing_hint_shown: {value}");
    value
}

#[tauri::command(async)]
fn set_drawing_hint_shown(app: tauri::AppHandle, shown: bool) {
    log::info!("set_drawing_hint_shown: {shown}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_drawing_hint_shown(shown);
}

#[tauri::command(async)]
fn get_drawing_enabled(app: tauri::AppHandle) -> bool {
    log::info!("get_drawing_enabled");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    data.drawing_enabled
}

#[tauri::command(async)]
fn set_drawing_enabled(app: tauri::AppHandle, enabled: bool, permanent: bool, stamp: bool) {
    log::info!("set_drawing_enabled: enabled={enabled} permanent={permanent} stamp={stamp}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();

    // Going from drawing to stamps, it stays enabled but the tool changes.
    if data.drawing_enabled == enabled && (!enabled || data.drawing_stamp == stamp) {
        return;
    }

    if let Err(e) = data
        .sender
        .send(Message::DrawingEnabled(DrawingEnabled { permanent, stamp }))
    {
        log::error!("set_drawing_enabled: failed to send message: {e:?}");
        return;
    }

    data.drawing_enabled = enabled;
    data.drawing_stamp = stamp;
    drop(data);

    if let Some(window) = app.get_webview_window("main") {
        #[cfg(not(target_os = "macos"))]
        let _ = window.set_always_on_top(enabled);
        if enabled {
            #[cfg(target_os = "macos")]
            let _ = window.hide();
            #[cfg(target_os = "windows")]
            let _ = window.minimize();
        }
    }
}

#[tauri::command(async)]
fn quit_app(app: tauri::AppHandle) {
    log::info!("quit_app");
    hopp::mark_core_shutdown_requested();
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    if let Err(e) = data.send_call_end() {
        log::error!("quit_app: failed to send CallEnd: {e:?}");
    }
    drop(data);
    app.exit(0);
}

#[tauri::command(async)]
fn minimize_main_window(app: tauri::AppHandle) {
    log::info!("minimize_main_window");
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.minimize() {
            log::error!("Failed to minimize main window: {e:?}");
        }
    } else {
        log::error!("Main window not found");
    }
}

#[tauri::command(async)]
fn set_livekit_url(app: tauri::AppHandle, url: String) {
    log::info!("set_livekit_url");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    if data.livekit_server_url != url {
        data.livekit_server_url = url.clone();
        if let Err(e) = data.sender.send(Message::LivekitServerUrl(url)) {
            log::error!("set_livekit_url: failed to send message: {e:?}");
        }
    }
}

#[tauri::command(async)]
fn get_livekit_url(app: tauri::AppHandle) -> String {
    log::info!("get_livekit_url");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    data.livekit_server_url.clone()
}

#[tauri::command(async)]
fn set_sentry_metadata(app: tauri::AppHandle, user_id: String, app_version: String) {
    log::info!("set_sentry_metadata");
    sentry_utils::init_metadata(user_id.clone(), app_version.clone());
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::SentryMetadata(SentryMetadata {
        user_id,
        app_version,
    })) {
        log::error!("set_sentry_metadata: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
fn call_started(
    app: tauri::AppHandle,
    audio_token: String,
    video_token: String,
    // Set when rejoining after a core restart, to keep what the user had on or off.
    mic_on: Option<bool>,
    camera_on: Option<bool>,
) -> Result<(), String> {
    log::info!("call_started");
    let data = app.state::<Mutex<AppData>>();
    #[allow(unused_mut)]
    let mut data = data.lock().unwrap();
    let user_settings = data.app_state.user_settings();
    #[cfg(target_os = "macos")]
    {
        if user_settings.show_dock_icon_in_call {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
            data.activation_policy_regular = true;
        }
    }
    // Resolve preferred → last used → system default → first → "".
    let audio_device_name = {
        let last_used = data.app_state.last_used_mic();
        let devices: Vec<AudioDevice> = if let Err(e) = data.sender.send(Message::ListAudioDevices)
        {
            log::error!("call_started: failed to list audio devices: {e:?}");
            vec![]
        } else {
            match recv_expected_response(&data.event_socket, |msg| match msg {
                Message::AudioDeviceList(d) => Ok(d),
                other => Err(other),
            }) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("call_started: failed to receive audio device list: {e:?}");
                    vec![]
                }
            }
        };
        resolve_microphone(
            &devices,
            user_settings.default_mic.as_deref(),
            last_used.as_deref(),
        )
    };
    log::info!("call_started: resolved audio_device_name={audio_device_name:?}");
    // A CallEnded still on its way from the previous call must not end this one.
    data.stale_call_ended_events = data.pending_call_end_acks;
    if let Err(e) = data
        .sender
        .send(Message::CallStart(socket_lib::CallStartMessage {
            audio_token: audio_token.clone(),
            video_token: video_token.clone(),
            audio_device_name,
            start_mic_on_call: Some(mic_on.unwrap_or(user_settings.start_mic_on_call)),
            start_camera_on_call: Some(camera_on.unwrap_or(user_settings.start_camera_on_call)),
            capture_stopped_by_hand: data.capture_stopped_by_hand,
        }))
    {
        log::error!("call_started: failed to send: {e:?}");
        return Err("Failed to send message to hopp_core".to_string());
    }
    let result = match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::CallStartResult(r) => Ok(r),
        other => Err(other),
    }) {
        Ok(result) => result,
        Err(e) => {
            log::error!("call_started: recv failed: {e:?}");
            // Core may still be joining. The UI is about to report a failure, so stop it
            // from ending up in a call nobody can see, with a live mic.
            if let Err(e) = data.send_call_end() {
                log::error!("call_started: failed to send CallEnd: {e:?}");
            }
            Err("Failed to receive message from hopp_core".to_string())
        }
    };
    if result.is_ok() {
        data.call_active = true;
        data.is_camera_on = false;
        data.is_screensharing = false;
        // A core that died mid-call never reported CallEnded, so these may still be set.
        shortcuts::unregister_call_shortcuts(&app);
        shortcuts::register_call_shortcuts(&app, resolved_call_shortcuts(&data.app_state));
    }
    result
}

/// When enabled=true, shows the notification variant of the icon.
/// When enabled=false, shows the default variant.
///
/// NOTE: must NOT be `async`. The macOS implementation manipulates AppKit/CALayer,
/// which has to run on the main thread; an async command would run off-thread and
/// the notification dot would silently never update.
#[tauri::command]
fn set_tray_notification(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_tray_notification: enabled={}", enabled);
    let data = app.state::<std::sync::Mutex<hopp::AppData>>();
    let mut data = data.lock().unwrap();
    if let Some(ref mut tray) = data.tray_state {
        tray.set_notification_enabled(enabled);
    }
}

/// Shows or hides the red missed call badge on the tray icon.
///
/// NOTE: must NOT be `async`, for the same reason as `set_tray_notification`.
#[tauri::command]
fn set_tray_missed_calls(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_tray_missed_calls: enabled={}", enabled);
    let data = app.state::<std::sync::Mutex<hopp::AppData>>();
    let mut data = data.lock().unwrap();
    if let Some(ref mut tray) = data.tray_state {
        tray.set_missed_calls_enabled(enabled);
    }
}

#[tauri::command(async)]
fn get_hopp_server_url(app: tauri::AppHandle) -> Option<String> {
    log::info!("get_hopp_server_url");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let url = data.app_state.user_settings().hopp_server_url;
    log::debug!("get_hopp_server_url: {url:?}");
    url
}

#[tauri::command(async)]
fn set_hopp_server_url(app: tauri::AppHandle, url: Option<String>) {
    log::info!("set_hopp_server_url: {url:?}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.hopp_server_url = url.clone());
    let _ = app.emit("hopp_server_url_changed", &url);
}

#[tauri::command(async)]
async fn create_feedback_window(
    app: tauri::AppHandle,
    team_id: String,
    room_id: String,
    participant_id: String,
) -> Result<(), String> {
    log::info!("create_feedback_window");

    let url = format!(
        "feedback.html?teamId={}&roomId={}&participantId={}",
        team_id, room_id, participant_id
    );
    hopp::create_media_window(
        &app,
        hopp::MediaWindowConfig {
            label: "feedback",
            title: "Call Feedback",
            url: &url,
            width: 500.0,
            height: 420.0,
            resizable: false,
            always_on_top: true,
            content_protected: false,
            maximizable: false,
            minimizable: false,
            decorations: true,
            transparent: false,
            background_color: Some(tauri::webview::Color(0, 0, 0, 0)),
        },
    )
}

/// Hidden from shares: what was said on a call is no one else's to read over your shoulder.
#[tauri::command(async)]
async fn create_captures_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("create_captures_window");
    hopp::create_media_window(
        &app,
        hopp::MediaWindowConfig {
            label: "captures",
            title: "Hopp captures",
            url: "captures.html",
            width: 1180.0,
            height: 820.0,
            resizable: true,
            always_on_top: false,
            content_protected: true,
            maximizable: true,
            minimizable: true,
            decorations: true,
            transparent: false,
            background_color: None,
        },
    )?;
    if let Some(window) = app.get_webview_window("captures") {
        if let Err(e) = window.set_min_size(Some(tauri::LogicalSize::new(360.0, 320.0))) {
            log::warn!("create_captures_window: minimum size: {e}");
        }
    }
    Ok(())
}

#[tauri::command(async)]
fn captures_bookmark(app: tauri::AppHandle) {
    log::info!("captures_bookmark");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::CaptureBookmark(None)) {
        log::error!("captures_bookmark: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
fn captures_stop(app: tauri::AppHandle) {
    log::info!("captures_stop");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.capture_stopped_by_hand = data.call_active;
    if let Err(e) = data.sender.send(Message::CaptureStop) {
        log::error!("captures_stop: failed to send message: {e:?}");
    }
}

#[tauri::command(async)]
async fn create_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    log::info!("create_settings_window");
    hopp::create_media_window(
        &app,
        hopp::MediaWindowConfig {
            label: "settings",
            title: "Settings",
            url: "settings.html",
            width: 800.0,
            height: 840.0,
            resizable: false,
            always_on_top: false,
            content_protected: false,
            maximizable: false,
            minimizable: true,
            decorations: true,
            transparent: false,
            background_color: None,
        },
    )
}

fn resolved_call_shortcuts(app_state: &hopp::app_state::AppState) -> shortcuts::CallShortcuts {
    let mut settings = app_state.user_settings();
    settings.resolve_shortcuts();
    shortcuts::CallShortcuts {
        mic: settings.shortcut_toggle_mic.unwrap_or_default(),
        camera: settings.shortcut_toggle_camera.unwrap_or_default(),
        screenshare: settings.shortcut_toggle_screenshare.unwrap_or_default(),
        end_call: settings.shortcut_end_call.unwrap_or_default(),
    }
}

#[tauri::command(async)]
fn show_call_alert_window(app: tauri::AppHandle) -> Result<(), String> {
    hopp::call_alert::show(&app)
}

/// Publishes the team list and call key for launchers such as Raycast. Only this user can read the
/// file, which is what stops a web page that opens a hopp:// link from starting a call.
#[tauri::command(async)]
fn write_launcher_directory(app: tauri::AppHandle, contents: String) -> Result<(), String> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let temporary_path = app_data_dir.join("launcher.json.tmp");

    let mut open_options = std::fs::OpenOptions::new();
    open_options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open_options, 0o600);

    let mut file = open_options
        .open(&temporary_path)
        .map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut file, contents.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary_path, app_data_dir.join("launcher.json")).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn close_call_alert_window(app: tauri::AppHandle) {
    hopp::call_alert::hide(&app);
}

#[tauri::command(async)]
fn get_user_settings(app: tauri::AppHandle) -> UserSettings {
    log::info!("get_user_settings");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    let mut settings = data.app_state.user_settings();
    settings.resolve_shortcuts();
    settings
}

#[tauri::command(async)]
fn list_installed_applications() -> Result<Vec<InstalledApplication>, String> {
    Ok(hopp::application_catalog::list_installed_applications())
}

#[tauri::command(async)]
fn set_app_veil_applications(
    app: tauri::AppHandle,
    applications: Vec<AppVeilApplication>,
) -> Result<(), String> {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    let enabled_bundle_ids = data.app_state.set_app_veil_applications(applications)?;
    data.sender
        .send(Message::SetAppVeilBundleIds(enabled_bundle_ids))
        .map_err(|error| {
            log::error!("set_app_veil_applications: failed to send settings: {error:?}");
            sentry_utils::simple_event(format!(
                "Failed to send App Veil settings to core: {error}"
            ));
            "App Veil was saved but could not be applied until Hopp restarts".to_string()
        })
}

#[tauri::command(async)]
fn set_shortcut_toggle_mic(app: tauri::AppHandle, accel: String) {
    log::info!("set_shortcut_toggle_mic: {accel}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    // An empty accelerator means disabled. None would bring the default back.
    data.app_state
        .update_user_setting(|s| s.shortcut_toggle_mic = Some(accel));
    if data.call_active {
        shortcuts::unregister_call_shortcuts(&app);
        shortcuts::register_call_shortcuts(&app, resolved_call_shortcuts(&data.app_state));
    }
}

#[tauri::command(async)]
fn set_shortcut_toggle_camera(app: tauri::AppHandle, accel: String) {
    log::info!("set_shortcut_toggle_camera: {accel}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    // An empty accelerator means disabled. None would bring the default back.
    data.app_state
        .update_user_setting(|s| s.shortcut_toggle_camera = Some(accel));
    if data.call_active {
        shortcuts::unregister_call_shortcuts(&app);
        shortcuts::register_call_shortcuts(&app, resolved_call_shortcuts(&data.app_state));
    }
}

#[tauri::command(async)]
fn set_shortcut_toggle_screenshare(app: tauri::AppHandle, accel: String) {
    log::info!("set_shortcut_toggle_screenshare: {accel}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    // An empty accelerator means disabled. None would bring the default back.
    data.app_state
        .update_user_setting(|s| s.shortcut_toggle_screenshare = Some(accel));
    if data.call_active {
        shortcuts::unregister_call_shortcuts(&app);
        shortcuts::register_call_shortcuts(&app, resolved_call_shortcuts(&data.app_state));
    }
}

#[tauri::command(async)]
fn set_shortcut_end_call(app: tauri::AppHandle, accel: String) {
    log::info!("set_shortcut_end_call: {accel}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    // An empty accelerator means disabled. None would bring the default back.
    data.app_state
        .update_user_setting(|s| s.shortcut_end_call = Some(accel));
    if data.call_active {
        shortcuts::unregister_call_shortcuts(&app);
        shortcuts::register_call_shortcuts(&app, resolved_call_shortcuts(&data.app_state));
    }
}

#[tauri::command(async)]
fn set_is_camera_on(app: tauri::AppHandle, value: bool) {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.is_camera_on = value;
}

#[tauri::command(async)]
fn set_is_screensharing(app: tauri::AppHandle, value: bool) {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.is_screensharing = value;
}

#[tauri::command(async)]
fn set_call_feedback_popup(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_call_feedback_popup: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.call_feedback_popup = enabled);
}

#[tauri::command(async)]
fn set_telemetry_enabled(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_telemetry_enabled: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.telemetry_enabled = enabled);
    sentry_utils::set_telemetry_enabled(enabled);
    if let Err(e) = data.sender.send(Message::SetTelemetryEnabled(enabled)) {
        log::error!("set_telemetry_enabled: failed to send: {e:?}");
    }
    let _ = app.emit("telemetry_enabled_changed", enabled);
}

#[tauri::command(async)]
fn set_show_dock_icon_in_call(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_show_dock_icon_in_call: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.show_dock_icon_in_call = enabled);
}

#[tauri::command(async)]
fn set_auto_update_enabled(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_auto_update_enabled: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.auto_update_enabled = enabled);
}

#[tauri::command(async)]
fn set_start_camera_on_call(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_start_camera_on_call: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.start_camera_on_call = enabled);
}

#[tauri::command(async)]
fn set_start_mic_on_call(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_start_mic_on_call: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.start_mic_on_call = enabled);
}

#[tauri::command(async)]
fn set_remote_control_enabled(app: tauri::AppHandle, enabled: bool) {
    log::info!("set_remote_control_enabled: {enabled}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|s| s.remote_control_enabled = enabled);
    drop(data);
    set_controller_cursor(app, enabled);
}

#[tauri::command(async)]
fn mute_mic(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::MuteAudio) {
        log::error!("mute_mic: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn unmute_mic(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::UnmuteAudio) {
        log::error!("unmute_mic: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn toggle_mic(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::ToggleMic) {
        log::error!("toggle_mic: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn set_noise_cancellation(app: tauri::AppHandle, enabled: bool) {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|settings| settings.noise_cancellation_enabled = enabled);
    if let Err(e) = data.sender.send(Message::SetNoiseCancellation(enabled)) {
        log::error!("set_noise_cancellation: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn set_screen_share_resolution(app: tauri::AppHandle, resolution: ScreenShareResolution) {
    log::info!("set_screen_share_resolution: {resolution:?}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|settings| settings.screen_share_resolution = resolution);
    if let Err(e) = data
        .sender
        .send(Message::SetScreenShareResolution(resolution))
    {
        log::error!("set_screen_share_resolution: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn set_screen_share_picker_mode(app: tauri::AppHandle, mode: ScreenSharePickerMode) {
    log::info!("set_screen_share_picker_mode: {mode:?}");
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state
        .update_user_setting(|settings| settings.screen_share_picker_mode = mode);
    if let Err(e) = data.sender.send(Message::SetScreenSharePickerMode(mode)) {
        log::error!("set_screen_share_picker_mode: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn start_camera(app: tauri::AppHandle, device_name: Option<String>) -> Result<(), String> {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data
        .sender
        .send(Message::StartCamera(socket_lib::CameraStartMessage {
            device_name,
        }))
    {
        log::error!("start_camera: failed to send: {e:?}");
        return Err("Failed to send message to hopp_core".to_string());
    }
    match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::StartCameraResult(r) => Ok(r),
        other => Err(other),
    }) {
        Ok(result) => result,
        Err(e) => {
            log::error!("start_camera: recv failed: {e:?}");
            Err("Failed to receive message from hopp_core".to_string())
        }
    }
}

#[tauri::command(async)]
fn stop_camera(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::StopCamera) {
        log::error!("stop_camera: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn open_camera_preview(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::OpenCamera) {
        log::error!("open_camera_preview: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn open_screenshare_viewer(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::OpenScreenShareWindow) {
        log::error!("open_screenshare_viewer: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn close_screenshare_viewer(app: tauri::AppHandle) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::CloseScreenShareWindow) {
        log::error!("close_screenshare_viewer: failed to send: {e:?}");
    }
}

#[tauri::command(async)]
fn list_microphones(app: tauri::AppHandle) -> Vec<AudioDevice> {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::ListAudioDevices) {
        log::error!("list_microphones: failed to send: {e:?}");
        return vec![];
    }
    match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::AudioDeviceList(d) => Ok(d),
        other => Err(other),
    }) {
        Ok(devices) => devices,
        Err(e) => {
            log::error!("list_microphones: recv failed: {e:?}");
            vec![]
        }
    }
}

#[tauri::command(async)]
fn set_default_mic(app: tauri::AppHandle, device_name: Option<String>) -> Result<(), String> {
    let data = app.state::<Mutex<AppData>>();
    let mut data = data.lock().unwrap();
    data.app_state.set_default_mic(device_name)
}

#[tauri::command(async)]
fn get_preferred_microphone(app: tauri::AppHandle) -> String {
    let devices = list_microphones(app.clone());
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    resolve_microphone(
        &devices,
        data.app_state.user_settings().default_mic.as_deref(),
        data.app_state.last_used_mic().as_deref(),
    )
}

#[tauri::command(async)]
fn select_microphone(app: tauri::AppHandle, device_name: String) {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data
        .sender
        .send(Message::StartAudioCapture(AudioCaptureMessage {
            device_name,
        }))
    {
        log::error!("select_microphone: failed to send: {e:?}");
        return;
    }
    match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::StartAudioCaptureResult(r) => Ok(r),
        other => Err(other),
    }) {
        Ok(Err(e)) => log::error!("select_microphone: core failed: {e}"),
        Err(e) => log::error!("select_microphone: no result: {e:?}"),
        Ok(Ok(())) => {}
    }
}

#[tauri::command(async)]
fn list_webcams(app: tauri::AppHandle) -> Vec<CameraDevice> {
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::ListCameras) {
        log::error!("list_webcams: failed to send: {e:?}");
        return vec![];
    }
    match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::CameraList(d) => Ok(d),
        other => Err(other),
    }) {
        Ok(devices) => devices,
        Err(e) => {
            log::error!("list_webcams: recv failed: {e:?}");
            vec![]
        }
    }
}

#[tauri::command(async)]
fn bring_windows_to_front(app: tauri::AppHandle) -> bool {
    log::info!("bring_windows_to_front");
    let data = app.state::<Mutex<AppData>>();
    let data = data.lock().unwrap();
    if let Err(e) = data.sender.send(Message::BringWindowsToFront) {
        log::error!("bring_windows_to_front: failed to send: {e:?}");
        return false;
    }
    match recv_expected_response(&data.event_socket, |msg| match msg {
        Message::BringWindowsToFrontResult(f) => Ok(f),
        other => Err(other),
    }) {
        Ok(focused) => focused,
        Err(e) => {
            log::error!("bring_windows_to_front: recv failed: {e:?}");
            false
        }
    }
}

#[tauri::command(async)]
fn end_call(app: tauri::AppHandle) {
    let state = app.state::<Mutex<AppData>>();
    #[allow(unused_mut)]
    let mut data = state.lock().unwrap();
    if let Err(e) = data.send_call_end() {
        log::error!("end_call: failed to send: {e:?}");
    }
    data.call_active = false;
    data.capture_stopped_by_hand = false;
    data.is_camera_on = false;
    data.is_screensharing = false;
    #[cfg(target_os = "macos")]
    {
        data.sleep_prevention.disable();
        let suppress = data.suppress_hide_on_call_end.clone();
        suppress.store(true, Ordering::Relaxed);
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        data.activation_policy_regular = false;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            suppress.store(false, Ordering::Relaxed);
        });
    }
    shortcuts::unregister_call_shortcuts(&app);
}

#[tauri::command(async)]
fn toggle_call_sleep_prevention(app: tauri::AppHandle, enabled: bool) {
    #[cfg(target_os = "macos")]
    {
        let data = app.state::<Mutex<AppData>>();
        let mut data = data.lock().unwrap();
        if enabled {
            data.sleep_prevention.enable();
        } else {
            data.sleep_prevention.disable();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, enabled);
    }
}

fn forward_core_events(events_rx: std_mpsc::Receiver<Message>, app: tauri::AppHandle) {
    log::info!("forward_core_events: starting event forwarding thread");
    for message in events_rx.iter() {
        match message {
            Message::ParticipantsSnapshot(snapshot) => {
                log::info!(
                    "forward_core_events: participants snapshot ({} participants)",
                    snapshot.len()
                );
                if let Err(e) = app.emit("core_participants_snapshot", &snapshot) {
                    log::error!("forward_core_events: failed to emit participants snapshot: {e:?}");
                }
            }
            Message::RoleChange(event) => {
                log::info!("forward_core_events: role change: {event:?}");
                if let Err(e) = app.emit("core_role_change", &event) {
                    log::error!("forward_core_events: failed to emit role change: {e:?}");
                }
            }
            Message::CameraFailed(error) => {
                log::error!("forward_core_events: camera failed: {error}");
                if let Err(e) = app.emit("core_camera_failed", &error) {
                    log::error!("forward_core_events: failed to emit camera failed: {e:?}");
                }
            }
            Message::CallEnded => {
                log::info!("forward_core_events: call ended");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.pending_call_end_acks = data.pending_call_end_acks.saturating_sub(1);
                if data.stale_call_ended_events > 0 {
                    data.stale_call_ended_events -= 1;
                    log::info!("forward_core_events: ignoring CallEnded from a previous call");
                    continue;
                }
                data.call_active = false;
                data.capture_stopped_by_hand = false;
                data.is_camera_on = false;
                data.is_screensharing = false;
                #[cfg(target_os = "macos")]
                {
                    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    data.sleep_prevention.disable();
                    data.activation_policy_regular = false;
                    let suppress = data.suppress_hide_on_call_end.clone();
                    drop(data);
                    suppress.store(true, Ordering::Relaxed);
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                        suppress.store(false, Ordering::Relaxed);
                    });
                }
                #[cfg(not(target_os = "macos"))]
                drop(data);
                shortcuts::unregister_call_shortcuts(&app);
                if let Err(e) = app.emit("core_call_ended", &()) {
                    log::error!("forward_core_events: failed to emit call ended: {e:?}");
                }
            }
            Message::ControllerDrawPersistChanged(persist) => {
                log::info!("forward_core_events: controller draw persist changed: {persist}");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.app_state.set_controller_draw_persist(persist);
            }
            Message::LastModeChanged(mode) => {
                log::info!("forward_core_events: last mode changed: {mode:?}");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.app_state.set_last_mode(mode);
            }
            Message::RoomConnectionFailed(reason) => {
                log::error!("forward_core_events: room connection failed: {reason}");
                if let Err(e) = app.emit("core_room_connection_failed", &reason) {
                    log::error!(
                        "forward_core_events: failed to emit room connection failed: {e:?}"
                    );
                }
            }
            Message::AppVeilFailed(reason) => {
                log::error!("forward_core_events: app veil failed: {reason}");
                if let Err(e) = app.emit("core_app_veil_failed", &reason) {
                    log::error!("forward_core_events: failed to emit app veil failed: {e:?}");
                }
            }
            Message::QueryPreferredCamera => {
                log::info!("forward_core_events: query preferred camera");
                let data = app.state::<Mutex<AppData>>();
                let data = data.lock().unwrap();
                let preferred = data.app_state.last_used_camera();
                if let Err(e) = data.sender.send(Message::PreferredCamera(preferred)) {
                    log::error!("forward_core_events: failed to send preferred camera: {e:?}");
                }
            }
            Message::ActiveMicChanged(device_name) => {
                log::info!("forward_core_events: active mic changed to: {device_name}");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.app_state.set_last_used_mic(device_name.clone());
                drop(data);
                if let Err(e) = app.emit("core_active_mic_changed", &device_name) {
                    log::error!(
                        "forward_core_events: failed to emit core_active_mic_changed: {e:?}"
                    );
                }
            }
            Message::ActiveCameraChanged(device_name) => {
                log::info!("forward_core_events: active camera changed to: {device_name}");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.app_state.set_last_used_camera(device_name.clone());
                drop(data);
                if let Err(e) = app.emit("core_active_camera_changed", &device_name) {
                    log::error!(
                        "forward_core_events: failed to emit core_active_camera_changed: {e:?}"
                    );
                }
            }
            Message::MicrophoneAudioLevel(level) => {
                if let Err(e) = app.emit("core_mic_audio_level", &level) {
                    log::error!("forward_core_events: failed to emit mic audio level: {e:?}");
                }
            }
            Message::MicrophoneSilent(device_name) => {
                log::error!("forward_core_events: no sound from mic: {device_name}");
                if let Err(e) = app.emit("core_microphone_silent", &device_name) {
                    log::error!("forward_core_events: failed to emit microphone silent: {e:?}");
                }
            }
            Message::MicrophoneRecovered => {
                log::info!("forward_core_events: mic is producing sound again");
                if let Err(e) = app.emit("core_microphone_recovered", &()) {
                    log::error!("forward_core_events: failed to emit microphone recovered: {e:?}");
                }
            }
            Message::StartScreenShareResult(Err(error)) => {
                log::error!("forward_core_events: screen share failed: {error}");
                if let Err(e) = app.emit("core_screenshare_failed", &error) {
                    log::error!("forward_core_events: failed to emit screen share failed: {e:?}");
                }
            }
            Message::StartScreenShareResult(Ok(())) => {
                log::info!("forward_core_events: screen share started");
            }
            Message::DrawingDisabled => {
                log::info!("forward_core_events: drawing disabled");
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                data.drawing_enabled = false;
                drop(data);
                #[cfg(not(target_os = "macos"))]
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_always_on_top(false);
                }
                if let Err(e) = app.emit("core_drawing_disabled", &()) {
                    log::error!("forward_core_events: failed to emit core_drawing_disabled: {e:?}");
                }
            }
            Message::ExitRequested => {
                log::info!("forward_core_events: exit requested from core");
                hopp::mark_core_shutdown_requested();
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                if let Err(e) = data.send_call_end() {
                    log::error!("forward_core_events: failed to send CallEnd: {e:?}");
                }
                drop(data);
                app.exit(0);
            }
            other => {
                log::error!("forward_core_events: unhandled event: {other:?}");
            }
        }
    }
    log::info!("forward_core_events: event forwarding thread exiting");
}

fn main() {
    let _guard = sentry_utils::init_sentry("Tauri backend".to_string(), Some(get_sentry_dsn()));

    /*
     * Flag for disabling hiding the window on focus lost.
     * This is used to prevent the window from hiding when the user is writing feedback.
     */
    let deactivate_hiding = Arc::new(Mutex::new(false));
    let deactivate_hiding_clone = deactivate_hiding.clone();

    /*
     * Flag for disabling hiding the window on focus lost.
     * This is used to prevent the window from hiding when the user uses Raycast/Spotlight
     * to open the app again.
     */
    let reopen_requested = Arc::new(Mutex::new(false));
    #[allow(unused_variables)]
    let reopen_requested_clone = reopen_requested.clone();

    /* This is used to guard against showing the main window if the location is not set. */
    #[allow(unused_variables)]
    let location_set = Arc::new(Mutex::new(false));
    #[allow(unused_variables)]
    let location_set_clone = location_set.clone();
    #[allow(unused_variables)]
    let location_set_setup = location_set.clone();

    /* Flag set during tray icon clicks to suppress spurious activation events. */
    let tray_clicked = Arc::new(AtomicBool::new(false));

    /* Flag to suppress main window hide when activation policy switches to Accessory after a call ends. */
    let suppress_hide_on_call_end = Arc::new(AtomicBool::new(false));
    let suppress_hide_on_call_end_clone = suppress_hide_on_call_end.clone();

    let log_level = get_log_level();
    let mut app = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    if !cfg!(debug_assertions) {
        app = app.plugin(tauri_plugin_single_instance::init(
            move |app, _args, _cwd| {
                log::info!("Reopening the app, single instance handler");
                log::debug!("app {app:?}");
                #[cfg(target_os = "macos")]
                {
                    let location_set = location_set_clone.lock().unwrap();
                    if !*location_set {
                        log::info!("Location not set, don't show the main window");
                        return;
                    }

                    let main_window = app.get_webview_window("main");
                    if let Some(window) = main_window {
                        log::info!("Single instance handler: showing main window");
                        let _ = window.show();
                        let _ = window.set_focus();
                    } else {
                        log::error!("Main window not found");
                    }
                }
            },
        ));
    }
    let log_file_name = if cfg!(debug_assertions) {
        Some("debug".to_string())
    } else {
        None
    };
    let app = app
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_positioner::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([
                    Target::new(TargetKind::LogDir {
                        file_name: log_file_name,
                    }),
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::Webview),
                ])
                .level(LevelFilter::Warn)
                .level_for("hopp", log_level)
                .level_for("sentry_utils", log_level)
                .max_file_size(50 * 1024 * 1024) // We are emptying them on startup
                .build(),
        )
        .setup(move |app| {
            /* Create the app_data_dir if it doesn't exist. */
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir.");
            if !app_data_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&app_data_dir) {
                    log::error!("Failed to create app data dir: {e:?}");
                }
            }

            let tidy_handle = app.handle().clone();
            std::thread::spawn(move || {
                let captures = tidy_handle.state::<hopp::captures::Captures>();
                if let Err(error) = captures.tidy() {
                    log::error!("Tidying captures: {error}");
                }
            });

            let (_core_process, sender, mut event_socket) =
                create_core_process(app.handle()).expect("Failed to create core process");

            let core_events_rx = event_socket.take_events();

            let app_state = AppState::new(&app_data_dir);
            hopp::send_initial_core_settings(&sender, &app_state);
            let data = Mutex::new(AppData::new(
                sender,
                event_socket,
                deactivate_hiding_clone,
                app_state,
                suppress_hide_on_call_end.clone(),
                forward_core_events,
            ));
            app.manage(data);

            // Background thread to forward core events to the frontend
            let event_app_handle = app.handle().clone();
            std::thread::spawn(move || {
                forward_core_events(core_events_rx, event_app_handle);
            });

            std::thread::spawn(|| {
                sentry_utils::upload_latest_crash();
            });

            let quit = MenuItemBuilder::new("Quit")
                .id("quit")
                .accelerator("Cmd+Q")
                .build(app)?;
            let menu = MenuBuilder::new(app).items(&[&quit]).build()?;

            setup_tray_icon(app, &menu, location_set_setup.clone(), tray_clicked.clone())?;

            /* Clear app logs in the beginning of a session. */
            let dir = app.path().app_log_dir();
            if let Err(e) = dir {
                log::warn!("Failed to get app log dir: {e:?}");
            } else {
                let dir = dir.unwrap();
                let log_file = dir.join("hopp.log");
                if log_file.exists() {
                    let unix_seconds = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|elapsed| elapsed.as_secs())
                        .unwrap_or_default();
                    if let Err(e) = hopp::archive_log_file(&log_file, unix_seconds) {
                        log::warn!("Failed to keep previous log file: {e:?}");
                    }
                    if let Err(e) = std::fs::write(&log_file, "") {
                        log::warn!("Failed to clear log file: {e:?}");
                    }
                }
            }

            /*
             * We are sending a ping event to the frontend
             * to keep it alive.
             * TODO: do graceful shutdown on exit
             */
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                ping_frontend(app_handle);
            });

            let first_run = {
                let data = app.state::<Mutex<AppData>>();
                let data = data.lock().unwrap();
                data.app_state.first_run()
            };

            setup_start_on_launch(&app.autolaunch(), first_run);

            /* Set first run to false after checking the start on launch. */
            {
                let data = app.state::<Mutex<AppData>>();
                let mut data = data.lock().unwrap();
                if first_run {
                    data.app_state.set_first_run(false);
                }
            }

            /* Main window configuration on windows */
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                let handle = app.handle();
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.set_shadow(false);
                    let _ = window.set_skip_taskbar(false);
                    /* Place window on the bottom right corner of the active display. */
                    let current_monitor = window.current_monitor();
                    if let Ok(Some(monitor)) = current_monitor {
                        let monitor_size = monitor.size();
                        let monitor_pos = monitor.position();
                        let window_size = window.inner_size().unwrap();
                        let base_offset = 20 * monitor.scale_factor() as u32;
                        let offset_y = (25. * monitor.scale_factor()) as u32 + base_offset;
                        let x = monitor_pos.x
                            + (monitor_size.width - window_size.width - base_offset) as i32;
                        let y = monitor_pos.y
                            + (monitor_size.height - window_size.height - offset_y) as i32;
                        let new_position = PhysicalPosition::new(x as f64, y as f64);
                        let _ = window.set_position(new_position);
                    }
                    let _ = window.set_always_on_top(false);
                    let _ = window.show();
                }
            }

            /* macOS specific setup */
            #[cfg(target_os = "macos")]
            {
                disable_app_nap();
                /* Start as Accessory — switch to Regular during calls or when permission windows are visible */
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);

                /*
                 * First show the notification window which explains that hopp lives in the
                 * menubar. Then show the permissions window if needed.
                 */
                let mut show_dock = false;
                let show_tray_notification_selection = {
                    let data = app.state::<Mutex<AppData>>();
                    let data = data.lock().unwrap();
                    data.app_state.tray_notification()
                };
                if show_tray_notification_selection {
                    let height = 250.;
                    let width = 450.;

                    let notification_window = tauri::WebviewWindowBuilder::new(
                        app,
                        "trayNotification",
                        tauri::WebviewUrl::App("trayNotification.html".into()),
                    )
                    .visible(true)
                    .focused(true)
                    .resizable(false)
                    .hidden_title(true)
                    .always_on_top(true)
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .title("Tray Notification")
                    .inner_size(width, height)
                    .build();
                    if let Err(e) = notification_window {
                        log::error!("Failed to create notification window: {e:?}");
                    } else {
                        let notification_window = notification_window.unwrap();
                        let _ = notification_window.show();
                        let _ = notification_window.set_focus();
                        show_dock = true;
                    }
                }

                if permissions::has_ungranted_permissions() {
                    log::info!("Opening permissions window");
                    let permissions_window = tauri::WebviewWindowBuilder::new(
                        app,
                        "permissions",
                        tauri::WebviewUrl::App("permissions.html".into()),
                    )
                    .visible(false)
                    .focused(true)
                    .resizable(false)
                    .hidden_title(true)
                    .always_on_top(false)
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .title("Permissions Configuration")
                    .inner_size(900., 730.)
                    .transparent(true)
                    .shadow(true)
                    .build();
                    if let Err(e) = permissions_window {
                        log::error!("Failed to create permissions window: {e:?}");
                    } else {
                        let permissions_window = permissions_window.unwrap();
                        show_dock = true;

                        // Apply native styling on macOS
                        #[cfg(target_os = "macos")]
                        {
                            set_window_corner_radius_and_decorations(
                                &permissions_window,
                                CORNER_RADIUS,
                                true,
                            );
                        }

                        /*
                         * Focus the window only if the notification window is not shown.
                         * When the notification window is shown we open the permissions window
                         * when it's closed.
                         */
                        if !show_tray_notification_selection {
                            let _ = permissions_window.show();
                            let _ = permissions_window.set_focus();
                        }
                    }
                }

                // Tackles Alt+Tab activation
                if show_dock {
                    app.set_activation_policy(tauri::ActivationPolicy::Regular);
                }
                {
                    let data = app.state::<Mutex<AppData>>();
                    let mut data = data.lock().unwrap();
                    if show_dock {
                        data.activation_policy_regular = true;
                    }
                    if !cfg!(debug_assertions) {
                        data.activation_observer =
                            Some(hopp::app_activation::AppActivationObserver::new(
                                app.handle().clone(),
                                location_set_setup.clone(),
                                reopen_requested_clone.clone(),
                                tray_clicked.clone(),
                            ));
                    }
                }
            }

            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::Focused(is_focused) = event {
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                if *is_focused && window.label() == "main" {
                    /* Place window on the bottom right corner of the active display. */
                    let current_monitor = window.current_monitor();
                    if let Ok(Some(monitor)) = current_monitor {
                        let monitor_size = monitor.size();
                        let monitor_pos = monitor.position();
                        let window_size = window.inner_size().unwrap();
                        let base_offset = 20 * monitor.scale_factor() as u32;
                        let offset_y = (25. * monitor.scale_factor()) as u32 + base_offset;
                        let x = monitor_pos.x
                            + (monitor_size.width - window_size.width - base_offset) as i32;
                        let y = monitor_pos.y
                            + (monitor_size.height - window_size.height - offset_y) as i32;
                        let new_position = PhysicalPosition::new(x as f64, y as f64);
                        let _ = window.set_position(new_position);
                    }
                }

                // detect click outside of the focused window and hide the app
                let deactivate_hiding = deactivate_hiding.lock().unwrap();
                let reopen_requested = reopen_requested.lock().unwrap();
                if !is_focused
                    && window.label() == "main"
                    && !cfg!(debug_assertions)
                    && !*deactivate_hiding
                    && !*reopen_requested
                    && !suppress_hide_on_call_end_clone.load(Ordering::Relaxed)
                {
                    log::info!("Hiding main window on focus lost: {}", *reopen_requested);

                    #[cfg(target_os = "macos")]
                    window.hide().unwrap();
                }
            }
        })
        .manage(hopp::captures::Captures::default())
        .invoke_handler(tauri::generate_handler![
            create_captures_window,
            captures_bookmark,
            captures_stop,
            hopp::captures::captures_list,
            hopp::captures::captures_open,
            hopp::captures::captures_poll,
            hopp::captures::captures_frame,
            hopp::captures::captures_delete,
            hopp::captures::captures_open_markdown,
            hopp::captures::capture_settings,
            hopp::captures::capture_settings_change,
            hopp::captures::capture_model_download,
            hopp::captures::capture_model_delete,
            hopp::captures::capture_open_folder,
            stop_sharing,
            get_available_content,
            store_token_cmd,
            get_stored_token,
            delete_stored_token,
            play_sound,
            stop_sound,
            reset_core_process,
            get_logs,
            set_deactivate_hiding,
            set_controller_cursor,
            open_accessibility_settings,
            open_microphone_settings,
            open_screenshare_settings,
            trigger_screenshare_permission,
            get_control_permission,
            get_microphone_permission,
            get_screenshare_permission,
            skip_tray_notification_selection_window,
            set_last_used_mic,
            get_last_used_mic,
            set_last_used_camera,
            get_last_used_camera,
            get_sharer_draw_persist,
            set_sharer_draw_persist,
            get_drawing_hint_shown,
            set_drawing_hint_shown,
            get_drawing_enabled,
            set_drawing_enabled,
            minimize_main_window,
            set_livekit_url,
            get_livekit_url,
            get_camera_permission,
            open_camera_settings,
            set_sentry_metadata,
            call_started,
            set_tray_notification,
            set_tray_missed_calls,
            get_hopp_server_url,
            set_hopp_server_url,
            create_feedback_window,
            create_settings_window,
            show_call_alert_window,
            write_launcher_directory,
            close_call_alert_window,
            get_user_settings,
            set_default_mic,
            get_preferred_microphone,
            list_installed_applications,
            set_app_veil_applications,
            set_call_feedback_popup,
            set_telemetry_enabled,
            set_show_dock_icon_in_call,
            set_auto_update_enabled,
            set_start_camera_on_call,
            set_start_mic_on_call,
            set_remote_control_enabled,
            set_shortcut_toggle_mic,
            set_shortcut_toggle_camera,
            set_shortcut_toggle_screenshare,
            set_shortcut_end_call,
            set_is_camera_on,
            set_is_screensharing,
            mute_mic,
            unmute_mic,
            toggle_mic,
            set_noise_cancellation,
            set_screen_share_resolution,
            set_screen_share_picker_mode,
            list_microphones,
            select_microphone,
            list_webcams,
            start_camera,
            stop_camera,
            open_camera_preview,
            open_screenshare_viewer,
            close_screenshare_viewer,
            end_call,
            toggle_call_sleep_prevention,
            bring_windows_to_front,
            open_stats_window,
            quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(move |app_handle, event| match event {
        tauri::RunEvent::ExitRequested { .. } => {
            log::info!("Exit requested");
            hopp::mark_core_shutdown_requested();
            sentry_utils::upload_logs_event("Tauri app quit".to_string());
            sentry_utils::flush(std::time::Duration::from_secs(2));
        }
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { .. },
            ..
        } => {
            log::info!("Close requested for window: {label}");
            if label == "trayNotification" {
                /* Make the permissions window visible in this case. */
                let permissions_window = app_handle.get_webview_window("permissions");
                if let Some(window) = permissions_window {
                    log::info!("Show permissions window");
                    let _ = window.show();
                    let _ = window.set_focus();
                } else {
                    #[cfg(target_os = "macos")]
                    {
                        let _ =
                            app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                        app_handle
                            .state::<Mutex<AppData>>()
                            .lock()
                            .unwrap()
                            .activation_policy_regular = false;
                    }
                }
            } else if label == "permissions" {
                #[cfg(target_os = "macos")]
                {
                    let _ = app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                    app_handle
                        .state::<Mutex<AppData>>()
                        .lock()
                        .unwrap()
                        .activation_policy_regular = false;
                }
            }
        }
        _ => {}
    });
}
