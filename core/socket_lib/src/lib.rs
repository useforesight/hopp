use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Platform-specific imports
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(windows)]
use std::net::{TcpListener, TcpStream};

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable, Serialize, Deserialize)]
pub struct Extent {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub enum ScreenShareResolution {
    P1080,
    P1440,
    P4K,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSharePickerMode {
    #[default]
    Screen,
    Window,
}

/// When the camera window floats over other windows.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum FacesOnTop {
    Always,
    #[default]
    WhenPinned,
    Never,
}

/// How the camera window lays out the faces.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum FacesLayout {
    /// Everyone the same size, filling the window.
    #[default]
    Grid,
    /// Whoever is speaking big, the others in a row below.
    Speaker,
}

/// Where the faces go in the shared-screen window.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum FacesInShared {
    /// A column down the right side.
    #[default]
    Side,
    /// A row along the bottom.
    Bottom,
    /// Small round faces over the bottom-right corner of the screen.
    Bubbles,
    /// Not there: the camera window stays separate.
    Off,
}

/// What the camera window becomes when it's pinned to a corner.
#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum PinnedStyle {
    /// A column of round faces.
    #[default]
    Strip,
    /// A small window of everyone.
    Box,
    /// Only whoever is speaking.
    Speaker,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

/// How the call windows behave, from Settings.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(default)]
pub struct CallWindowPrefs {
    pub faces_on_top: FacesOnTop,
    pub layout: FacesLayout,
    pub faces_in_shared: FacesInShared,
    pub pinned_style: PinnedStyle,
    /// When someone starts sharing, their screen opens on the display and Space you're on.
    pub bring_shared_to_you: bool,
    /// The corner the camera window was last pinned to.
    pub pinned_corner: ScreenCorner,
}

impl Default for CallWindowPrefs {
    fn default() -> Self {
        Self {
            faces_on_top: FacesOnTop::default(),
            layout: FacesLayout::default(),
            faces_in_shared: FacesInShared::default(),
            pinned_style: PinnedStyle::default(),
            bring_shared_to_you: true,
            pinned_corner: ScreenCorner::default(),
        }
    }
}

impl ScreenShareResolution {
    pub fn extent(self) -> Extent {
        match self {
            ScreenShareResolution::P1080 => Extent {
                width: 1920.0,
                height: 1080.0,
            },
            ScreenShareResolution::P1440 => Extent {
                width: 2560.0,
                height: 1440.0,
            },
            ScreenShareResolution::P4K => Extent {
                width: 4096.0,
                height: 2160.0,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WindowFrameMessage {
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CursorPositionMessage {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MouseClickMessage {
    pub x: f32,
    pub y: f32,
    pub button: u32,
    pub clicks: f32,
    pub shift_key: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScrollMessage {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeystrokeMessage {
    pub key: String,
    pub meta: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub down: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum ContentType {
    Display,
    Window,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Content {
    pub content_type: ContentType,
    pub id: u32,
}

impl fmt::Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.content_type {
            ContentType::Display => write!(f, "Display {}", self.id),
            ContentType::Window => write!(f, "Window {}", self.id),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScreenShareMessage {
    pub content: Content,
    pub resolution: Extent,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CallStartMessage {
    pub audio_token: String,
    pub video_token: String,
    pub audio_device_name: String,
    pub start_mic_on_call: Option<bool>,
    pub start_camera_on_call: Option<bool>,
    /// Rejoining a call whose capture was started or stopped by hand: it stays that way.
    pub capture_by_hand: CaptureByHand,
    /// The room's name, for the capture. A call between two people has none.
    #[serde(default)]
    pub room_name: Option<String>,
}

/// Something you clicked in one of the core's windows that's the app's to do, because the app
/// keeps what it changes: what was asked of the capture, and which tool is in hand.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    ToggleCapture,
    OpenCaptures,
    ToggleDraw,
    ToggleStamp,
    /// The reactions tray, over the button that was clicked.
    OpenReactions,
}

/// What the user last did to this call's capture. The core can restart in a call and the
/// choice has to outlive it, so it's kept in the app and handed back on rejoining.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptureByHand {
    #[default]
    Nothing,
    Started,
    Stopped,
}

/// Why the call server cut the app off from a call it hadn't left.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum CallDropReason {
    /// The same person joined the call somewhere else, a browser or another computer,
    /// which takes their place in it.
    JoinedElsewhere,
    /// Removed from the call, or the call was closed for everyone.
    CallClosed,
    /// The connection went and reconnecting didn't bring it back.
    ConnectionLost,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SentryMetadata {
    pub user_id: String,
    pub app_version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DrawingEnabled {
    pub permanent: bool,
    /// Clicks place stamps instead of drawing.
    #[serde(default)]
    pub stamp: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioCaptureMessage {
    pub device_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CameraDevice {
    pub name: String,
    // Camera index now is not stable, the `stable_id` is not yet released.
    // For now we will use the camera name as the id and hope for a new release and
    // no duplicate names 🤞:
    // https://github.com/l1npengtul/nokhwa/blob/ab2bedfbf13dfed4fabe77dcf725012fcaa4d305/nokhwa-core/src/types.rs#L478-L480
    pub id: String,
    pub default: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CameraStartMessage {
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoreParticipantState {
    pub identity: String,
    pub name: String,
    pub connected: bool,
    pub muted: bool,
    pub has_camera: bool,
    pub is_screensharing: bool,
    /// Capturing (transcribing) the call, as Hopp for Linux does.
    #[serde(default)]
    pub capturing: bool,
    /// LiveKit hears them speaking: sound bars move beside their name.
    #[serde(default)]
    pub is_speaking: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum CoreRoleChange {
    Sharer,
    Controller,
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CoreRoleEvent {
    pub role: CoreRoleChange,
}

/// Represents the user's preferred interaction mode for screen sharing sessions.
/// This is stored persistently and restored when the user joins a new session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum StoredMode {
    /// Remote control mode - mouse and keyboard events are forwarded to the sharer
    RemoteControl,
    /// Click animation mode - clicks are visualized on the shared screen
    ClickAnimation,
    /// Drawing mode - freehand drawing on the shared screen
    Draw {
        /// If true, drawings persist until manually cleared; if false, they auto-expire
        permanent: bool,
    },
}

/// One word of what someone is asking an agent.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WakeWord {
    pub text: String,
    /// It is the wake word, to show lit up.
    pub wake: bool,
}

/// Someone the capture heard say the wake word, and what they're asking.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WakeHeard {
    /// Who: their identity without the suffix of the connection it came from, or
    /// "local" for you, as in the participants snapshot.
    pub identity: String,
    /// Their words from the wake word's line on. Of a long ask, the latest.
    pub words: Vec<WakeWord>,
}

/// Reactions: confetti, a gong, fireworks… that play on everyone's screen in the call. They
/// go out on their own topic, [`REACTION_TOPIC`], so clients that don't know them ignore
/// them, and clients that know them ignore kinds they don't.
pub const REACTION_TOPIC: &str = "hopp_reaction";
/// Packets bigger than this are dropped unread.
pub const REACTION_PACKET_MAX: usize = 2 * 1024;
/// The most characters of a reaction's words.
pub const REACTION_TEXT_MAX: usize = 60;

/// One reaction, as it's sent to the call.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Reaction {
    /// Which: confetti, gong, table_flip, emoji, sound, clap, fireworks, achievement,
    /// disco, disco_end or ship_it.
    pub kind: String,
    /// Which emoji or sound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    /// How big: the tap streak of confetti, the rockets of fireworks.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub level: u8,
    /// The achievement's title, or what shipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The identity an achievement is for. None: everyone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

fn is_zero(level: &u8) -> bool {
    *level == 0
}

impl Reaction {
    /// The reaction in a packet from the call, if it's one this client knows.
    pub fn from_packet(payload: &[u8]) -> Option<Self> {
        if payload.len() > REACTION_PACKET_MAX {
            return None;
        }
        serde_json::from_slice::<Self>(payload).ok()?.tidy()
    }

    /// Within bounds, or None for a kind or choice this client doesn't know.
    pub fn tidy(mut self) -> Option<Self> {
        const KINDS: [&str; 11] = [
            "confetti",
            "gong",
            "table_flip",
            "emoji",
            "sound",
            "clap",
            "fireworks",
            "achievement",
            "disco",
            "disco_end",
            "ship_it",
        ];
        const EMOJI: [&str; 8] = [
            "party", "fire", "applause", "lol", "rocket", "hundred", "love", "shock",
        ];
        const SOUNDS: [&str; 8] = [
            "airhorn",
            "badumtss",
            "sad_trombone",
            "crickets",
            "drumroll",
            "chaching",
            "wooow",
            "applause",
        ];
        if !KINDS.contains(&self.kind.as_str()) {
            return None;
        }
        let choices: &[&str] = match self.kind.as_str() {
            "emoji" => &EMOJI,
            "sound" => &SOUNDS,
            _ => &[],
        };
        if choices.is_empty() {
            self.choice = None;
        } else if !self
            .choice
            .as_deref()
            .is_some_and(|choice| choices.contains(&choice))
        {
            return None;
        }
        self.level = match self.kind.as_str() {
            "confetti" => self.level.min(4),
            "fireworks" => self.level.clamp(1, 10),
            _ => 0,
        };
        let words = matches!(self.kind.as_str(), "achievement" | "ship_it");
        self.text = self
            .text
            .filter(|_| words)
            .map(|text| {
                text.trim()
                    .chars()
                    .take(REACTION_TEXT_MAX)
                    .collect::<String>()
            })
            .map(|text| text.trim_end().to_string())
            .filter(|text| !text.is_empty());
        self.to = self
            .to
            .filter(|to| self.kind == "achievement" && !to.is_empty());
        Some(self)
    }
}

/// A reaction for the app to play, from someone in the call or from you.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ReactionPlay {
    pub reaction: Reaction,
    /// Who sent it: their identity without the connection's suffix.
    pub from: String,
    /// Their name in the call.
    pub name: String,
    /// Their colour in the call, as on their cursor.
    pub colour: [u8; 3],
    /// You sent it.
    pub mine: bool,
    /// Your identity, without the connection's suffix, for an achievement that's for you.
    pub me: String,
    /// Everyone else in the call: identity without the connection's suffix, and name.
    pub people: Vec<(String, String)>,
    /// The call window on screen, in points from the top left of the main screen: x, y,
    /// width, height. None when no call window is showing.
    pub call_window: Option<[f64; 4]>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    GetAvailableContent,
    CallStart(CallStartMessage),
    CallStartResult(Result<(), String>),
    CallEnd,
    StartScreenShare(ScreenShareMessage),
    StartScreenShareResult(Result<(), String>),
    StopScreenshare,
    Ping,
    ControllerCursorEnabled(bool),
    LivekitServerUrl(String),
    SentryMetadata(SentryMetadata),
    DrawingEnabled(DrawingEnabled),
    ListAudioDevices,
    AudioDeviceList(Vec<AudioDevice>),
    StartAudioCapture(AudioCaptureMessage),
    StartAudioCaptureResult(Result<(), String>),
    StopAudioCapture,
    MuteAudio,
    UnmuteAudio,
    ListCameras,
    CameraList(Vec<CameraDevice>),
    StartCamera(CameraStartMessage),
    StartCameraResult(Result<(), String>),
    StopCamera,
    CameraFailed(String),
    OpenCamera,
    OpenScreensharing,
    ToggleMic,
    OpenScreenShareWindow,
    CloseScreenShareWindow,
    OpenStatsWindow,
    BringWindowsToFront,
    BringWindowsToFrontResult(bool),
    // Core → Tauri query, Tauri → Core response
    QueryPreferredCamera,
    PreferredCamera(Option<String>),
    // Core → Tauri event forwarding
    ParticipantsSnapshot(Vec<CoreParticipantState>),
    RoleChange(CoreRoleEvent),
    CallEnded, // When call ends from a participant's side in Camera or Screen sharing window.
    RoomConnectionFailed(String),
    /// The call server cut the app off from the call, which is over here.
    CallDropped(CallDropReason),
    /// App Veil could not be initialized; protected apps may be visible while sharing.
    AppVeilFailed(String),
    ControllerDrawPersistChanged(bool),
    SharerDrawPersistChanged(bool),
    LastModeChanged(StoredMode),
    ActiveMicChanged(String),
    ActiveCameraChanged(String),
    DrawingDisabled,
    ExitRequested,
    SetNoiseCancellation(bool),
    SetScreenShareResolution(ScreenShareResolution),
    SetScreenSharePickerMode(ScreenSharePickerMode),
    SetCallWindowPrefs(CallWindowPrefs),
    /// Core → Tauri: changed from the call windows (the layout button, a pinned window
    /// dragged to another corner), to remember.
    CallWindowPrefsChanged(CallWindowPrefs),
    SetAppVeilBundleIds(Vec<String>),
    SetTelemetryEnabled(bool),
    /// Microphone RMS level in [0.0, 1.0], emitted ~1 Hz from core capturer.
    MicrophoneAudioLevel(f32),
    /// The mic is open but restarting it did not produce any sound.
    MicrophoneSilent(String),
    /// A mic reported through `MicrophoneSilent` is producing sound again.
    MicrophoneRecovered,
    /// Mark this moment in the capture of the current call, with this label if given.
    CaptureBookmark(Option<String>),
    /// Stop capturing the current call. It stays stopped until the next call.
    /// Start capturing this call: nothing else starts one once it was stopped by hand.
    CaptureStart,
    CaptureStop,
    /// The capture recording this call now, if any. Its last words may still be finishing
    /// after this says none.
    CaptureChanged(Option<String>),
    /// The call went off the record with this many milliseconds left, or has more time
    /// now. None: it's back on the record. Sent whether or not this app captures.
    OffRecord(Option<u64>),
    /// Two more minutes off the record, for everyone in the call.
    OffRecordMore,
    /// Back on the record now, for everyone in the call.
    OffRecordEnd,
    /// This app's capture heard someone say the wake word, or more of what they're
    /// asking, a second or two after it's said. None: they stopped talking a few seconds
    /// ago, or the final text didn't have the word after all.
    WakeHeard(Option<WakeHeard>),
    AppAction(AppAction),
    /// Send this reaction to the call. Tauri → core.
    ReactionSend(Reaction),
    /// Play this reaction. Core → Tauri.
    ReactionPlay(Box<ReactionPlay>),
    /// You let others control your screen, or stopped them, from the sharing pill.
    ControllerCursorChanged(bool),
}

impl Message {
    pub fn is_response(&self) -> bool {
        matches!(
            self,
            Message::CallStartResult(_)
                | Message::AudioDeviceList(_)
                | Message::StartAudioCaptureResult(_)
                | Message::CameraList(_)
                | Message::StartCameraResult(_)
                | Message::BringWindowsToFrontResult(_)
                | Message::PreferredCamera(_)
        )
    }
}

// Platform-specific stream type alias
#[cfg(unix)]
type Stream = UnixStream;
#[cfg(windows)]
type Stream = TcpStream;

/// Send half of the socket. Clone-safe via Arc<Mutex<Stream>>.
#[derive(Clone)]
pub struct SocketSender {
    stream: Arc<Mutex<Stream>>,
}

impl SocketSender {
    pub fn send(&self, message: Message) -> Result<(), std::io::Error> {
        let serialized_message = serde_json::to_string(&message)?;
        let serialized_message = serialized_message.as_bytes();
        let size = serialized_message.len();
        let mut message_bytes = size.to_le_bytes().to_vec();
        message_bytes.extend_from_slice(serialized_message);
        let mut stream = self.stream.lock().unwrap();
        stream.write_all(&message_bytes)?;
        Ok(())
    }
}

/// Dual-channel receiver that owns the background reader thread.
/// Dropping this struct shuts down the underlying stream, which causes the
/// reader thread to exit and closes both channels on the remote side.
pub struct EventSocket {
    pub events: mpsc::Receiver<Message>,
    pub responses: mpsc::Receiver<Message>,
    stream: Stream,
}

impl EventSocket {
    fn new(mut read_stream: Stream, shutdown_stream: Stream) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let (response_tx, response_rx) = mpsc::channel();

        log::info!("EventSocket: spawning background reader thread");
        std::thread::spawn(move || {
            let _ = read_stream.set_read_timeout(Some(Duration::from_secs(1)));
            loop {
                match Self::read_message(&mut read_stream) {
                    Ok(message) => {
                        let tx = if message.is_response() {
                            &response_tx
                        } else {
                            &event_tx
                        };
                        if tx.send(message).is_err() {
                            log::info!("Background reader: channel closed, stopping");
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut
                        {
                            continue;
                        }
                        log::error!("Background reader: IO error: {e:?}");
                        break;
                    }
                }
            }
            log::info!("Background reader: thread exiting");
        });

        Self {
            events: event_rx,
            responses: response_rx,
            stream: shutdown_stream,
        }
    }

    fn read_message(stream: &mut Stream) -> Result<Message, std::io::Error> {
        let mut size_buffer = [0u8; std::mem::size_of::<usize>()];
        stream.read_exact(&mut size_buffer)?;
        let message_size = usize::from_le_bytes(size_buffer);

        let mut message_buffer = vec![0u8; message_size];
        stream.read_exact(&mut message_buffer)?;
        let buffer_str =
            String::from_utf8(message_buffer).expect("Failed to convert buffer to string");
        let deserialized_message: Message = serde_json::from_str(&buffer_str)?;
        Ok(deserialized_message)
    }
}

impl EventSocket {
    pub fn take_events(&mut self) -> mpsc::Receiver<Message> {
        let (_tx, dummy_rx) = mpsc::channel();
        std::mem::replace(&mut self.events, dummy_rx)
    }

    pub fn take_responses(&mut self) -> mpsc::Receiver<Message> {
        let (_tx, dummy_rx) = mpsc::channel();
        std::mem::replace(&mut self.responses, dummy_rx)
    }
}

impl Drop for EventSocket {
    fn drop(&mut self) {
        log::info!("EventSocket: shutting down");
        use std::net::Shutdown;
        let _ = self.stream.shutdown(Shutdown::Both);
        log::info!("EventSocket: shutdown complete");
    }
}

fn build_pair(stream: Stream) -> std::io::Result<(SocketSender, EventSocket)> {
    let write_stream = stream.try_clone()?;
    let read_stream = stream.try_clone()?;
    let shutdown_stream = stream;

    let sender = SocketSender {
        stream: Arc::new(Mutex::new(write_stream)),
    };

    let event_socket = EventSocket::new(read_stream, shutdown_stream);

    Ok((sender, event_socket))
}

/// Connect to an existing socket (client side).
pub fn connect(socket_path: &str) -> Result<(SocketSender, EventSocket), std::io::Error> {
    log::info!("Connecting to socket at {socket_path}");
    #[cfg(unix)]
    let stream = {
        let stream = UnixStream::connect(socket_path)?;
        stream.set_read_timeout(None)?;
        stream
    };

    #[cfg(windows)]
    let stream = {
        let port = socket_path_to_port(socket_path);
        let addr = format!("127.0.0.1:{port}");
        let stream = TcpStream::connect(addr)?;
        stream.set_read_timeout(None)?;
        stream
    };

    build_pair(stream)
}

/// Create a socket and wait for a client to connect (server side).
pub fn listen(socket_path: &str) -> Result<(SocketSender, EventSocket), std::io::Error> {
    log::info!("Creating socket at {socket_path}");
    #[cfg(unix)]
    let stream = {
        if Path::new(socket_path).exists() {
            fs::remove_file(socket_path)?;
        }

        let listener = UnixListener::bind(socket_path)?;
        listener.set_nonblocking(true)?;
        let mut stream = None;
        let times = if cfg!(debug_assertions) { 100 } else { 10 };
        for i in 0..times {
            log::info!("Waiting for client {i}/{times}");
            match listener.accept() {
                Ok((s, _)) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
        let stream = stream.ok_or_else(|| {
            std::io::Error::other("Client did not connect after multiple attempts")
        })?;
        stream.set_nonblocking(false)?;
        log::info!("Client connected");
        stream.set_read_timeout(None)?;
        stream
    };

    #[cfg(windows)]
    let stream = {
        if Path::new(socket_path).exists() {
            fs::remove_file(socket_path)?;
        }

        // Get initial port to try
        let mut port = socket_path_to_port(socket_path);
        let mut listener = None;

        // Try to bind, incrementing port if necessary
        for try_num in 0..500 {
            let addr = format!("127.0.0.1:{port}");
            match TcpListener::bind(addr) {
                Ok(l) => {
                    listener = Some(l);
                    break;
                }
                Err(_) => {
                    log::info!("Port {port} is in use, trying next port...");
                    port += 1;
                    if try_num != 0 && try_num % 100 == 0 {
                        port += 100;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }

        let listener = listener.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "Could not find available port after multiple attempts",
            )
        })?;

        // Store just the port number in the file
        fs::write(socket_path, port.to_string())?;

        log::info!("Listening on port {port}, waiting for client");
        listener.set_nonblocking(true)?;
        let mut stream = None;
        let times = if cfg!(debug_assertions) { 100 } else { 50 };
        for i in 0..times {
            log::info!("Waiting for client {i}/{times}");
            match listener.accept() {
                Ok((s, _)) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
        let stream = stream.ok_or_else(|| {
            std::io::Error::other("Client did not connect after multiple attempts")
        })?;
        stream.set_nonblocking(false)?;
        log::info!("Client connected");
        stream.set_read_timeout(None)?;
        stream
    };

    build_pair(stream)
}

#[cfg(windows)]
fn socket_path_to_port(socket_path: &str) -> u16 {
    // First try to read the port from the file
    if let Ok(content) = fs::read_to_string(socket_path) {
        if let Ok(port) = content.trim().parse::<u16>() {
            log::info!("Found port {port} in file {socket_path}");
            port
        } else {
            log::warn!("Could not parse port from file {socket_path}: '{content}'");
            calculate_port_from_hash(socket_path)
        }
    } else {
        log::debug!("Could not read port from file {socket_path}, calculating from hash");
        calculate_port_from_hash(socket_path)
    }
}

#[cfg(windows)]
fn calculate_port_from_hash(socket_path: &str) -> u16 {
    let mut hash: u32 = 5381;
    for byte in socket_path.bytes() {
        hash = ((hash << 5).wrapping_add(hash)).wrapping_add(byte as u32);
    }
    // Use ports in range 49152-65535 (dynamic/private range)
    (hash % 15900 + 49152) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[cfg(unix)]
    fn test_pair() -> ((SocketSender, EventSocket), (SocketSender, EventSocket)) {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("test.sock");
        let socket_path_str = socket_path.to_str().unwrap().to_string();
        let socket_path_str2 = socket_path_str.clone();

        let server_handle = std::thread::spawn(move || listen(&socket_path_str).unwrap());

        // Small delay to let server start listening
        std::thread::sleep(Duration::from_millis(100));

        let client = connect(&socket_path_str2).unwrap();
        let server = server_handle.join().unwrap();
        // Keep tempdir alive by leaking it (tests are short-lived)
        std::mem::forget(dir);
        (server, client)
    }

    #[test]
    fn test_send_recv_event() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        // Send an event-type message from client to server
        client_sender.send(Message::GetAvailableContent).unwrap();

        let msg = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg, Message::GetAvailableContent));
    }

    #[test]
    fn test_send_recv_screen_share_result_event() {
        let ((server_sender, _server_events), (_client_sender, client_events)) = test_pair();

        // Send StartScreenShareResult as an event (not response)
        server_sender
            .send(Message::StartScreenShareResult(Ok(())))
            .unwrap();

        let msg = client_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg, Message::StartScreenShareResult(_)));
    }

    #[test]
    fn test_window_content_round_trip() {
        let serialized = serde_json::to_string(&Content {
            content_type: ContentType::Window,
            id: 42,
        })
        .unwrap();
        let content: Content = serde_json::from_str(&serialized).unwrap();

        assert!(matches!(content.content_type, ContentType::Window));
        assert_eq!(content.id, 42);
    }

    #[test]
    fn test_app_veil_bundle_ids_transport_round_trip() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();
        let bundle_ids = vec![
            "com.apple.notificationcenterui".to_string(),
            "com.example.private".to_string(),
        ];

        client_sender
            .send(Message::SetAppVeilBundleIds(bundle_ids.clone()))
            .unwrap();

        let message = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        match message {
            Message::SetAppVeilBundleIds(received) => assert_eq!(received, bundle_ids),
            _ => panic!("expected app veil bundle IDs"),
        }
    }

    #[test]
    fn test_routing_mixed() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        // Send mix of events and responses from client to server
        client_sender.send(Message::GetAvailableContent).unwrap();
        client_sender.send(Message::Ping).unwrap();
        client_sender
            .send(Message::StartScreenShareResult(Ok(())))
            .unwrap();
        client_sender
            .send(Message::CallStart(CallStartMessage {
                audio_token: "test-audio-token".to_string(),
                video_token: "test-video-token".to_string(),
                audio_device_name: String::new(),
                start_mic_on_call: None,
                start_camera_on_call: None,
                capture_by_hand: CaptureByHand::Nothing,
                room_name: None,
            }))
            .unwrap();
        client_sender.send(Message::CallEnd).unwrap();
        client_sender
            .send(Message::CallStartResult(Ok(())))
            .unwrap();

        // Events: GetAvailableContent, Ping, StartScreenShareResult, CallStart, CallEnd
        let msg1 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg1, Message::GetAvailableContent));
        let msg2 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg2, Message::Ping));
        let msg3 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg3, Message::StartScreenShareResult(_)));
        let msg5 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg5, Message::CallStart(_)));
        let msg6 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg6, Message::CallEnd));

        // Responses: CallStartResult
        let msg7 = server_events
            .responses
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg7, Message::CallStartResult(_)));
    }

    #[test]
    fn test_multiple_messages_ordering() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        let n = 10;
        for _ in 0..n {
            client_sender.send(Message::Ping).unwrap();
        }

        for _ in 0..n {
            let msg = server_events
                .events
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            assert!(matches!(msg, Message::Ping));
        }
    }

    #[test]
    fn test_sender_clone() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        let clone = client_sender.clone();
        client_sender.send(Message::Ping).unwrap();
        clone.send(Message::GetAvailableContent).unwrap();

        let msg1 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        let msg2 = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();

        // Both should arrive (order guaranteed since same underlying stream with mutex)
        assert!(matches!(msg1, Message::Ping));
        assert!(matches!(msg2, Message::GetAvailableContent));
    }

    #[test]
    fn test_response_timeout() {
        let ((_server_sender, server_events), (_client_sender, _client_events)) = test_pair();

        let result = server_events
            .responses
            .recv_timeout(Duration::from_millis(100));
        assert!(result.is_err());
    }

    #[test]
    fn test_disconnect_closes_channels() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        // Drop client sender to trigger disconnect
        drop(client_sender);
        drop(_client_events);

        // Server's event channel should eventually close
        // Keep receiving until we get a disconnect error
        loop {
            match server_events.events.recv_timeout(Duration::from_secs(5)) {
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("Timed out waiting for channel to close")
                }
            }
        }
    }

    #[test]
    fn test_request_response_pattern() {
        let ((_server_sender, server_events), (client_sender, _client_events)) = test_pair();

        // Client sends request
        client_sender.send(Message::Ping).unwrap();

        // Server receives event
        let msg = server_events
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(matches!(msg, Message::Ping));
    }

    #[test]
    fn reactions_from_other_clients_read_as_sent() {
        let examples = [
            r#"{"kind":"emoji","choice":"fire"}"#,
            r#"{"kind":"confetti","level":2}"#,
            r#"{"kind":"fireworks","level":7}"#,
            r#"{"kind":"achievement","text":"Fixed the flaky test","to":"room:r:u"}"#,
            r#"{"kind":"ship_it","text":"v1.4.0"}"#,
            r#"{"kind":"gong"}"#,
        ];
        for example in examples {
            let reaction = Reaction::from_packet(example.as_bytes()).expect(example);
            assert_eq!(serde_json::to_string(&reaction).unwrap(), example);
        }
    }

    #[test]
    fn reactions_this_client_cant_play_are_dropped() {
        for packet in [
            r#"{"kind":"kazoo"}"#,
            r#"{"kind":"emoji","choice":"kazoo"}"#,
            r#"{"kind":"emoji"}"#,
            r#"{"kind":"sound","choice":"fire"}"#,
            r#"{"choice":"fire"}"#,
            r#"{"kind":"gong","level":300}"#,
            "not json",
            "",
        ] {
            assert_eq!(Reaction::from_packet(packet.as_bytes()), None, "{packet}");
        }
        let huge = format!(
            r#"{{"kind":"ship_it","text":"{}"}}"#,
            "x".repeat(REACTION_PACKET_MAX)
        );
        assert_eq!(Reaction::from_packet(huge.as_bytes()), None);
    }

    #[test]
    fn reactions_keep_within_bounds() {
        let read = |packet: &str| Reaction::from_packet(packet.as_bytes()).unwrap();
        assert_eq!(read(r#"{"kind":"confetti","level":9}"#).level, 4);
        assert_eq!(read(r#"{"kind":"fireworks"}"#).level, 1);
        assert_eq!(read(r#"{"kind":"fireworks","level":99}"#).level, 10);
        assert_eq!(read(r#"{"kind":"gong","level":3}"#).level, 0);
        let gong = read(r#"{"kind":"gong","choice":"fire","text":"hi","to":"x","later":1}"#);
        assert_eq!((gong.choice, gong.text, gong.to), (None, None, None));
        let long = read(&format!(
            r#"{{"kind":"ship_it","text":"  {}  "}}"#,
            "é".repeat(80)
        ));
        assert_eq!(long.text.unwrap().chars().count(), REACTION_TEXT_MAX);
        assert_eq!(read(r#"{"kind":"ship_it","text":"   "}"#).text, None);
        assert_eq!(read(r#"{"kind":"achievement","to":""}"#).to, None);
    }
}
