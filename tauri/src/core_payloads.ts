import { invoke } from "@tauri-apps/api/core";

/**
 * A captured call, from crates/hopp-capture/src/store.rs and browse.rs.
 */
export interface CaptureCall {
  id: string;
  room: string;
  title?: string;
  summary?: string;
  /** Unix time in milliseconds. */
  started_ms: number;
  ended_ms?: number;
  participants: string[];
  model?: string;
  lines: number;
}

export interface CaptureListed extends CaptureCall {
  /** While searching, who said what matched, with the matches in [brackets]. */
  found?: string;
}

export interface CaptureMark {
  by: string;
  /** `circle`, `underline`, `line`, `stroke` or `pin`. */
  shape: string;
  pin?: number;
  text?: string;
  /** Fractions of the screen from the top left: `[left, top, right, bottom]`. */
  bounds: [number, number, number, number];
  colour: string;
}

/** One row of a call's timeline, from crates/hopp-capture/src/timeline.rs. */
export type CaptureRow =
  | { kind: "event"; t_ms: number; text: string }
  | { kind: "bookmark"; t_ms: number; who: string | null; text: string }
  | { kind: "gap"; t_ms: number; text: string }
  | { kind: "said"; t_ms: number; speaker: string; text: string; draft: boolean; also: string | null }
  | {
      kind: "drawing";
      t_ms: number;
      end_ms: number;
      frame: number;
      frames: number;
      width: number;
      height: number;
      marks: CaptureMark[];
      /** The part of the frame to show: `[left, top, width, height]` as fractions. */
      crop: [number, number, number, number];
    }
  | { kind: "screen"; t_ms: number; end_ms: number; count: number; thumbs: [number, number][] };

export interface CaptureOverview {
  duration_ms: number;
  /** `[speaker, t_ms, end_ms]` */
  speech: [string, number, number][];
  /** `[t_ms, first person who drew]` */
  drawings: [number, string][];
  /** `[t_ms, end_ms]` */
  screens: [number, number][];
}

export interface CapturePicture {
  frame: number;
  title: string;
  detail: string;
  width: number;
  height: number;
  marks: CaptureMark[];
}

export interface CaptureModel {
  name: string;
  bytes: number;
  note: string;
  installed: boolean;
  shared: boolean;
  downloading: number | null;
  failed: string | null;
}

export interface CaptureSettings {
  model: string;
  gpu: boolean;
  drafts: boolean;
  auto: boolean;
  keep_days: number | null;
  models: CaptureModel[];
  captures_bytes: number;
  models_bytes: number;
  cli_path: string;
}

export type CaptureChange =
  | { model: string }
  | { gpu: boolean }
  | { drafts: boolean }
  | { auto: boolean }
  | { keep_days: number | null };

export interface CaptureUpdate {
  call: CaptureCall;
  /** The rows from this one on replace those the window has. */
  from: number;
  rows: CaptureRow[];
  overview: CaptureOverview;
  colours: Record<string, string>;
  /** Every picture in the call, when they've changed. */
  pictures?: CapturePicture[];
}

/**
 * Socket lib types are defined in (core/socket_lib/src/lib.rs).
 *
 * At some point we could use:
 * https://github.com/Aleph-Alpha/ts-rs
 *
 * But skipping for now, as it would need quite
 * many changes to the socket code, and tweaking.
 */
export interface WindowFrameMessage {
  origin_x: number;
  origin_y: number;
  width: number;
  height: number;
}

export interface CursorPositionMessage {
  x: number;
  y: number;
}

export interface MouseClickMessage {
  x: number;
  y: number;
  button: number;
  clicks: number;
  shift_key: boolean;
}

export interface ScrollMessage {
  x: number;
  y: number;
}

export interface KeystrokeMessage {
  key: string;
  meta: boolean;
  shift: boolean;
  ctrl: boolean;
  alt: boolean;
  down: boolean;
}

export interface CallStartMessage {
  token: string;
}

export interface SentryMetadata {
  user_id: string;
  app_version: string;
}

export interface DrawingEnabled {
  permanent: boolean;
  stamp: boolean;
}

export interface AudioDevice {
  name: string;
  default: boolean;
}

export interface AudioCaptureMessage {
  device_name: string;
}

export interface CameraDevice {
  name: string;
  id: string;
  default: boolean;
}

export interface CameraStartMessage {
  device_name: string;
}

/** What the `hopp` command is told about the app. Read by `crates/hopp-capture/src/cli.rs`. */
export type AppState = {
  signed_in: boolean;
  user?: { name: string };
  ringing: { name: string; kind: "call" | "invite" } | null;
  dialing: { name: string } | null;
  call: {
    room: string | null;
    participants: { name: string }[];
    /** Unix seconds. */
    started: number;
    microphone: boolean;
    camera: boolean;
    sharing: boolean;
    viewing: string | null;
  } | null;
  teammates: { id: string; name: string; online: boolean; room: string | null }[];
  rooms: { id: string; name: string; members: string[] }[];
};

/** What the `hopp` command asks only the main window to do. */
export type AppRequest =
  | { type: "call"; value: string }
  | { type: "join_room"; value: string }
  | { type: "accept" }
  | { type: "decline" }
  | { type: "open" };

export interface CoreParticipantState {
  identity: string;
  name: string;
  connected: boolean;
  muted: boolean;
  has_camera: boolean;
  is_screensharing: boolean;
  /** Capturing (transcribing) the call, as Hopp for Linux does. */
  capturing?: boolean;
  /** LiveKit hears them speaking. */
  is_speaking?: boolean;
}

export type ScreenShareResolution = "P1080" | "P1440" | "P4K";
export type ScreenSharePickerMode = "Screen" | "Window";

export interface AppVeilApplication {
  bundle_id: string;
  enabled: boolean;
}

export interface InstalledApplication {
  bundle_id: string;
  name: string;
  icon_png: number[] | null;
}

export interface UserSettings {
  call_feedback_popup: boolean;
  show_dock_icon_in_call: boolean;
  start_camera_on_call: boolean;
  start_mic_on_call: boolean;
  default_mic: string | null;
  remote_control_enabled: boolean;
  noise_cancellation_enabled: boolean;
  screen_share_resolution: ScreenShareResolution;
  screen_share_picker_mode: ScreenSharePickerMode;
  hopp_server_url: string | null;
  shortcut_toggle_mic: string;
  shortcut_toggle_camera: string;
  shortcut_toggle_screenshare: string;
  shortcut_end_call: string;
  telemetry_enabled: boolean;
  auto_update_enabled: boolean;
  app_veil_applications: AppVeilApplication[];
  auto_away_enabled: boolean;
  auto_away_minutes: number;
}

export type CoreRoleChange = "Sharer" | "Controller" | "None";

export interface CoreRoleEvent {
  role: CoreRoleChange;
}

/**
 * Tauri command map.
 *
 * This is a map of all the commands that can be invoked from the Tauri side.
 * It is used to generate the type-safe invoke function.
 */
/** A notice or a question from an agent or script. */
export type AgentCard = {
  id: string;
  title: string;
  body?: string;
  sender?: string;
  question?: { accept: string; reject: string };
  ephemeral: boolean;
};

export type AgentCardAnswer = "accepted" | "rejected" | "dismissed";

export interface CommandMap {
  stop_sharing: { args: void; return: void };
  get_available_content: { args: void; return: void };

  // Token management
  store_token_cmd: { args: { token: string }; return: void };
  get_stored_token: { args: void; return: string | null };
  delete_stored_token: { args: void; return: void };

  // Sound
  play_sound: { args: { soundName: string }; return: void };
  stop_sound: { args: { soundName: string }; return: void };

  // Core process
  reset_core_process: { args: void; return: void };

  // Logs
  get_logs: { args: void; return: string };

  // UI
  set_deactivate_hiding: { args: { deactivate: boolean }; return: void };
  set_controller_cursor: { args: { enabled: boolean }; return: void };
  minimize_main_window: { args: void; return: void };
  quit_app: { args: void; return: void };

  set_tray_notification: { args: { enabled: boolean }; return: void };
  set_tray_missed_calls: { args: { enabled: boolean }; return: void };

  // Permissions
  open_accessibility_settings: { args: void; return: void };
  open_microphone_settings: { args: void; return: void };
  open_camera_settings: { args: void; return: void };
  open_screenshare_settings: { args: void; return: void };
  trigger_screenshare_permission: { args: void; return: boolean };
  get_control_permission: { args: void; return: boolean };
  get_microphone_permission: { args: void; return: boolean };
  get_screenshare_permission: { args: void; return: boolean };
  get_camera_permission: { args: void; return: boolean };
  skip_tray_notification_selection_window: { args: void; return: void };

  // Preferences
  get_last_used_mic: { args: void; return: string | null };
  set_last_used_mic: { args: { mic: string }; return: void };
  get_last_used_camera: { args: void; return: string | null };
  set_last_used_camera: { args: { camera: string }; return: void };
  get_sharer_draw_persist: { args: void; return: boolean };
  set_sharer_draw_persist: { args: { persist: boolean }; return: void };
  get_controller_draw_persist: { args: void; return: boolean };
  set_controller_draw_persist: { args: { persist: boolean }; return: void };
  get_drawing_hint_shown: { args: void; return: boolean };
  set_drawing_hint_shown: { args: { shown: boolean }; return: void };
  get_drawing_enabled: { args: void; return: boolean };
  set_drawing_enabled: { args: { enabled: boolean; permanent: boolean; stamp: boolean }; return: void };

  // LiveKit
  set_livekit_url: { args: { url: string }; return: void };
  get_livekit_url: { args: void; return: string };

  // Windows
  create_feedback_window: { args: { teamId: string; roomId: string; participantId: string }; return: void };
  create_settings_window: { args: void; return: void };

  // Captures window
  create_captures_window: { args: void; return: void };
  captures_list: { args: { query: string }; return: CaptureListed[] };
  captures_open: { args: { which: string }; return: CaptureUpdate };
  captures_poll: { args: void; return: CaptureUpdate | null };
  captures_frame: { args: { callId: string; number: number; full: boolean }; return: ArrayBuffer };
  captures_delete: { args: { callId: string }; return: void };
  captures_open_markdown: { args: { callId: string }; return: void };
  captures_bookmark: { args: void; return: void };
  captures_stop: { args: void; return: void };
  captures_requested_call: { args: void; return: string | null };
  captures_current: { args: void; return: string | null };
  app_publish_state: { args: { state: AppState }; return: void };
  cards_showing: { args: void; return: AgentCard[] };
  cards_settle: { args: { id: string; state: AgentCardAnswer }; return: void };
  cards_return_focus: { args: void; return: void };
  capture_settings: { args: void; return: CaptureSettings };
  capture_settings_change: { args: { change: CaptureChange }; return: void };
  capture_model_download: { args: { name: string }; return: void };
  capture_model_delete: { args: { name: string }; return: void };
  capture_open_folder: { args: void; return: void };
  capture_open_hooks: { args: void; return: void };
  show_call_alert_window: { args: void; return: void };
  write_launcher_directory: { args: { contents: string }; return: void };
  close_call_alert_window: { args: void; return: void };

  // Sentry
  set_sentry_metadata: { args: { userId: string; appVersion: string }; return: void };

  // Call
  call_started: {
    args: { audioToken: string; videoToken: string; micOn?: boolean; cameraOn?: boolean; roomName?: string };
    return: void;
  };
  end_call: { args: void; return: void };
  toggle_call_sleep_prevention: { args: { enabled: boolean }; return: void };

  // Server
  get_hopp_server_url: { args: void; return: string | null };
  set_hopp_server_url: { args: { url: string | null }; return: void };

  // User settings (Settings window)
  get_user_settings: { args: void; return: UserSettings };
  set_default_mic: { args: { deviceName: string | null }; return: void };
  get_preferred_microphone: { args: void; return: string };
  list_installed_applications: { args: void; return: InstalledApplication[] };
  set_app_veil_applications: { args: { applications: AppVeilApplication[] }; return: void };
  set_call_feedback_popup: { args: { enabled: boolean }; return: void };
  set_telemetry_enabled: { args: { enabled: boolean }; return: void };
  set_show_dock_icon_in_call: { args: { enabled: boolean }; return: void };
  set_auto_update_enabled: { args: { enabled: boolean }; return: void };
  set_auto_away_enabled: { args: { enabled: boolean }; return: void };
  set_auto_away_minutes: { args: { minutes: number }; return: void };
  get_stepped_away: { args: void; return: boolean };
  set_start_camera_on_call: { args: { enabled: boolean }; return: void };
  set_start_mic_on_call: { args: { enabled: boolean }; return: void };
  set_remote_control_enabled: { args: { enabled: boolean }; return: void };
  set_shortcut_toggle_mic: { args: { accel: string }; return: void };
  set_shortcut_toggle_camera: { args: { accel: string }; return: void };
  set_shortcut_toggle_screenshare: { args: { accel: string }; return: void };
  set_shortcut_end_call: { args: { accel: string }; return: void };
  set_is_camera_on: { args: { value: boolean }; return: void };
  set_is_screensharing: { args: { value: boolean }; return: void };

  // Core socket messages — audio
  mute_mic: { args: void; return: void };
  unmute_mic: { args: void; return: void };
  toggle_mic: { args: void; return: void };
  set_noise_cancellation: { args: { enabled: boolean }; return: void };
  set_screen_share_resolution: { args: { resolution: ScreenShareResolution }; return: void };
  set_screen_share_picker_mode: { args: { mode: ScreenSharePickerMode }; return: void };
  list_microphones: { args: void; return: AudioDevice[] };
  select_microphone: { args: { deviceName: string }; return: void };

  // Core socket messages — camera
  list_webcams: { args: void; return: CameraDevice[] };
  start_camera: { args: { deviceName?: string }; return: void };
  stop_camera: { args: void; return: void };
  open_camera_preview: { args: void; return: void };

  // Core socket messages — screenshare viewer
  open_screenshare_viewer: { args: void; return: void };
  close_screenshare_viewer: { args: void; return: void };

  // Core socket messages — window management
  bring_windows_to_front: { args: void; return: boolean };
  open_stats_window: { args: void; return: void };
}

type InvokeArgs<K extends keyof CommandMap> = CommandMap[K]["args"] extends void ? [] : [CommandMap[K]["args"]];

/**
 * Typed invoke wrapper.
 *
 * This is a wrapper around the invoke function that
 * allows us to pass in the arguments and return type
 * of the command.
 */
export function typedInvoke<K extends keyof CommandMap>(
  cmd: K,
  ...args: InvokeArgs<K>
): Promise<CommandMap[K]["return"]> {
  return invoke<CommandMap[K]["return"]>(cmd, args[0] as Record<string, unknown>);
}
