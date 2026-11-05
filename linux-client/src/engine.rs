use std::{
    collections::{HashMap, VecDeque},
    process::{Command as ProcessCommand, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tokio::sync::{Notify, mpsc as async_mpsc, watch};

use crate::{
    api::{ApiClient, RoomInfo, UserInfo},
    convert::{self, ScreenSlot},
    frames::{self, CameraSlots, FrameSlot},
    media::{self, Session},
    native_input::{InputModifiers, NativeInput},
    presence::{self, PresenceEvent},
    protocol::{ControlGrant, InputEvent, canonical_identity},
    screencast::{Capture, CaptureTarget},
    settings::{self, Settings},
    wake,
};

#[derive(Debug, Clone, Default)]
pub struct Participant {
    pub identity: String,
    pub name: String,
    /// They are capturing the call.
    pub capturing: bool,
}

/// How the capture of the current call is going, for the UI and the bar.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct CaptureStatus {
    pub id: String,
    pub model: String,
    /// starting, downloading, loading, ready, idle (model unloaded until someone
    /// speaks) or failed.
    pub state: String,
    /// Where transcription runs: the GPU's name, or "CPU".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Audio waiting to be transcribed.
    pub behind_ms: i64,
    pub lines: u64,
}

impl From<crate::capture::recorder::Status> for CaptureStatus {
    fn from(status: crate::capture::recorder::Status) -> Self {
        use crate::capture::recorder::Phase;
        let (state, percent, message) = match status.phase {
            Phase::Starting => ("starting", None, None),
            Phase::Downloading { percent } => ("downloading", Some(percent), None),
            Phase::Loading => ("loading", None, None),
            Phase::Ready => ("ready", None, None),
            Phase::Idle => ("idle", None, None),
            Phase::Failed { message } => ("failed", None, Some(message)),
        };
        Self {
            id: status.id,
            model: status.model,
            state: state.into(),
            percent,
            message,
            behind_ms: status.behind_ms,
            lines: status.lines,
            device: status.device,
        }
    }
}

pub use crate::protocol::UiRequest;

pub enum Command {
    SignIn {
        server: String,
        token: String,
    },
    SignOut,
    Refresh,
    CreateRoom(String),
    RenameRoom {
        id: String,
        name: String,
    },
    DeleteRoom(String),
    JoinRoom(RoomInfo),
    /// Join a pairing session started from Slack, by its id (`hopp:///join-session`).
    JoinSession(String),
    Leave,
    Share {
        window_only: bool,
    },
    StopShare,
    Microphone(bool),
    Camera(bool),
    GrantControl(String),
    RevokeControl,
    SendInput(InputEvent),
    /// Place, update or remove one of your stamps.
    SendStamp(crate::protocol::StampMessage),
    ShareClipboard,
    SelectAudio {
        input: Option<String>,
        output: Option<String>,
    },
    SelectCamera(Option<String>),
    Shutdown,
    CallPeer {
        user_id: String,
        name: String,
    },
    AcceptCall(String),
    RejectCall(String),
    Activate,
    Ui(UiRequest),
    CopyClipboard(bool),
    InvitePeer {
        user_id: String,
        name: String,
    },
    AcceptInvite {
        inviter_id: String,
        invite_id: String,
    },
    RejectInvite {
        inviter_id: String,
        invite_id: String,
    },
    CancelInvite,
    /// Start capturing the call.
    StartCapture,
    StopCapture,
    /// Mark this moment in the capture, with an optional label.
    Bookmark(Option<String>),
    #[cfg(test)]
    TestDesktopLock(bool),
}

pub enum Event {
    Ui(UiRequest),
    LoggedIn(UserInfo),
    SignedOut,
    Rooms(Vec<RoomInfo>),
    RoomPresence(crate::api::RoomsPresence),
    /// Who each teammate is in a call with.
    CallsPresence(crate::api::CallsPresence),
    Status(String),
    Joined(RoomInfo),
    Connected,
    Disconnected(String),
    Participants(Vec<Participant>),
    RemoteShare {
        identity: String,
        active: bool,
    },
    Sharing {
        active: bool,
        target: Option<CaptureTarget>,
    },
    Microphone(bool),
    Camera(bool),
    /// A speaker or microphone stream broke mid-call; the engine replaces it.
    AudioDeviceFailed {
        input: bool,
        error: String,
    },
    ControlGranted(Option<String>),
    PeerControl {
        enabled: bool,
    },
    Error(String),
    Data {
        sender: String,
        payload: Vec<u8>,
        topic: Option<String>,
    },
    CaptureClosed,
    SelectingSource(bool),
    Drawing {
        sender: String,
        event: InputEvent,
    },
    /// Someone placed, changed or removed a stamp.
    Stamp {
        sender: String,
        message: crate::protocol::StampMessage,
    },
    Pointer {
        sender: String,
        point: crate::protocol::Point,
    },
    Devices {
        inputs: Vec<String>,
        outputs: Vec<String>,
        cameras: Vec<(String, String)>,
    },
    Teammates(Vec<crate::api::Teammate>),
    IncomingCall {
        caller_id: String,
        name: String,
    },
    /// A call or invitation that rang, or should have, and wasn't answered: the caller
    /// gave up, it came while you were busy or offline, or another one pushed it aside.
    Missed {
        caller_id: String,
        name: String,
        invite: bool,
        /// When they tried, in Unix milliseconds.
        at_ms: i64,
    },
    Calling {
        peer_id: String,
        name: String,
    },
    Activate,
    IncomingInvite {
        inviter_id: String,
        invite_id: String,
        name: String,
    },
    Inviting {
        peer_id: String,
        invite_id: String,
        name: String,
    },
    InviteResolved {
        message: String,
    },
    DesktopLocked,
    DesktopUnlocked,
    TransportReconnecting,
    TransportReconnected,
    /// The capture you are running, or `None` when you aren't.
    Capture(Option<CaptureStatus>),
    /// From the recorder thread: its status changed. Engine-internal.
    CaptureChanged,
}

#[derive(Clone)]
pub struct SharedFrames {
    /// Size of the last captured frame handed to the encoder (tests compare it with what
    /// viewers decode).
    #[cfg(test)]
    pub published: Arc<Mutex<Option<[u32; 2]>>>,
    pub remote: FrameSlot,
    pub local_camera: FrameSlot,
    pub cameras: CameraSlots,
}

impl Default for SharedFrames {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            published: Default::default(),
            remote: frames::slot(),
            local_camera: frames::slot(),
            cameras: Default::default(),
        }
    }
}

pub struct Engine {
    pub commands: async_mpsc::Sender<Command>,
    pub events: mpsc::Receiver<Event>,
    pub shared: SharedFrames,
    /// Notified whenever `events` or `shared` has something new for the UI.
    pub wake: Arc<Notify>,
}

enum JoinTarget {
    Room(RoomInfo),
    /// A Slack pairing session: a room reached through the session's own token endpoint.
    Session(RoomInfo),
    Direct {
        payload: serde_json::Value,
        peer: String,
        name: String,
    },
    Invitation {
        inviter: String,
        name: String,
    },
}

struct PendingJoin {
    task: tokio::task::JoinHandle<Result<Option<ConnectedSession>>>,
    cancel: watch::Sender<bool>,
}

struct PendingCapture {
    task: tokio::task::JoinHandle<Result<Capture>>,
    canceled: Arc<AtomicBool>,
    generation: u64,
}

struct ConnectedSession {
    session: Session,
    room: RoomInfo,
    peer: Option<String>,
    rollback: JoinRollback,
}

#[derive(Clone)]
struct JoinRollback {
    api: ApiClient,
    room: Option<String>,
    peer: Option<String>,
    outgoing: Option<async_mpsc::Sender<serde_json::Value>>,
}

impl JoinRollback {
    async fn run(&self) {
        if let Some(room) = &self.room {
            let _ = self.api.leave_room(room).await;
        } else if let (Some(peer), Some(outgoing)) = (&self.peer, &self.outgoing) {
            let _ = outgoing.try_send(presence::call_end(peer));
        }
    }
}

struct PendingInvite {
    peer: String,
    invite_id: String,
}

impl Engine {
    pub fn start(settings: Settings) -> Self {
        let (commands, commands_rx) = async_mpsc::channel(256);
        let wake = Arc::new(Notify::new());
        let (events_tx, events) = wake::channel(&wake);
        let shared = SharedFrames::default();
        let worker_shared = shared.clone();
        std::thread::Builder::new()
            .name("hopp-session".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("session runtime");
                runtime.block_on(async move {
                    let mut worker = Worker::new(settings, worker_shared, events_tx.clone());
                    if let Err(error) = worker.run(commands_rx).await {
                        let _ = events_tx.send(Event::Error(format!("{error:#}")));
                    }
                    worker.leave().await;
                });
            })
            .expect("start native session worker");
        Self {
            commands,
            events,
            shared,
            wake,
        }
    }
}

enum ClipboardResult {
    SendPaste(Vec<u8>),
    ReplyCopy { sender: String, bytes: Vec<u8> },
    ApplyPaste { sender: String },
    WroteLocal,
}

struct ClipboardPacketOut {
    event: InputEvent,
    recipient: String,
    to_host: bool,
}

struct Refreshed {
    rooms: Vec<RoomInfo>,
    teammates: Vec<crate::api::Teammate>,
    presence: crate::api::RoomsPresence,
    calls: crate::api::CallsPresence,
}

/// Reopening an audio device that failed mid-call. Glitches happen (a late period while the
/// machine is busy, a Bluetooth hiccup), so the first retry is quick and unannounced; a
/// device that keeps failing backs off to one cheap attempt every few seconds.
#[derive(Default)]
struct Recovery {
    due: Option<Instant>,
    delay: Duration,
    last_failure: Option<Instant>,
    reported: bool,
}

impl Recovery {
    const FIRST: Duration = Duration::from_millis(250);
    const MAX: Duration = Duration::from_secs(5);
    /// A device that ran this long since its last failure starts over with a quick retry.
    const STABLE: Duration = Duration::from_secs(30);

    fn failed(&mut self) {
        let now = Instant::now();
        let recent = self
            .last_failure
            .is_some_and(|at| now.duration_since(at) < Self::STABLE);
        self.delay = if recent {
            (self.delay * 2).clamp(Self::FIRST, Self::MAX)
        } else {
            Self::FIRST
        };
        if !recent {
            self.reported = false;
        }
        self.last_failure = Some(now);
        self.due = Some(now + self.delay);
    }

    fn due(&self) -> bool {
        self.due.is_some_and(|at| Instant::now() >= at)
    }

    fn wait(&self) -> Option<Duration> {
        self.due
            .map(|at| at.saturating_duration_since(Instant::now()))
    }

    /// Whether to tell the user, once per outage, that the device is still down.
    fn report(&mut self) -> bool {
        !std::mem::replace(&mut self.reported, true)
    }
}

struct Worker {
    settings: Settings,
    shared: SharedFrames,
    events: wake::Sender<Event>,
    /// Notified by everything the loop below consumes: room data and remote input,
    /// frames, capture, presence and desktop events.
    wake: Arc<Notify>,
    internal_tx: wake::Sender<Event>,
    internal_rx: mpsc::Receiver<Event>,
    api: Option<ApiClient>,
    session: Option<Session>,
    room: Option<RoomInfo>,
    capture: Option<Capture>,
    capture_frames: ScreenSlot,
    /// A captured frame is being encoded on a blocking thread.
    publishing: Arc<AtomicBool>,
    /// Bumped whenever sharing stops, so an in-flight encode can't publish afterwards.
    share_epoch: Arc<AtomicU64>,
    screen_ramp: Option<tokio::task::JoinHandle<()>>,
    input: Option<NativeInput>,
    grant: ControlGrant,
    remote_host: Option<String>,
    peer_control: bool,
    participants: Vec<Participant>,
    presence: Option<tokio::task::JoinHandle<Result<()>>>,
    presence_stop: Option<watch::Sender<bool>>,
    presence_tx: wake::Sender<PresenceEvent>,
    presence_rx: mpsc::Receiver<PresenceEvent>,
    current_room: Arc<Mutex<Option<String>>>,
    last_refresh: Instant,
    clipboard: crate::clipboard::Assembler,
    remote_clipboard: crate::clipboard::Assembler,
    clipboard_tasks: Vec<tokio::task::JoinHandle<Result<ClipboardResult>>>,
    clipboard_epoch: Arc<AtomicU64>,
    clipboard_out: VecDeque<ClipboardPacketOut>,
    outgoing: Option<async_mpsc::Sender<serde_json::Value>>,
    call_peer: Option<String>,
    teammates: Vec<crate::api::Teammate>,
    pending_join: Option<PendingJoin>,
    pending_call: Option<tokio::task::JoinHandle<()>>,
    pending_call_cancel: Option<watch::Sender<bool>>,
    join_cleanup: Option<tokio::task::JoinHandle<()>>,
    pending_capture: Option<PendingCapture>,
    capture_generation: u64,
    capture_events: wake::Sender<Event>,
    capture_rx: mpsc::Receiver<Event>,
    session_frames: SharedFrames,
    desktop_events: wake::Sender<Event>,
    desktop_rx: mpsc::Receiver<Event>,
    desktop_task: Option<tokio::task::JoinHandle<()>>,
    desktop_locked: bool,
    privacy_locked: Arc<AtomicBool>,
    transport_ready: bool,
    incoming_call: Option<String>,
    incoming_invite: Option<(String, String)>,
    outgoing_invite: Option<PendingInvite>,
    socket_connected: bool,
    user_id: Option<String>,
    refresh_task: Option<tokio::task::JoinHandle<Result<Refreshed>>>,
    speaker: Recovery,
    microphone: Recovery,
    /// Shared with each call's audio paths.
    tap: crate::capture::Tap,
    recording: Option<crate::capture::recorder::Recorder>,
    /// Keyframes of the shared screen with everyone's ink, while capturing.
    moments: Option<crate::capture::moments::Moments>,
    /// Captures finishing their last transcriptions after the call.
    finishing: Vec<tokio::task::JoinHandle<()>>,
}

impl Worker {
    fn new(settings: Settings, shared: SharedFrames, events: wake::Sender<Event>) -> Self {
        let wake = Arc::new(Notify::new());
        let (internal_tx, internal_rx) = wake::channel(&wake);
        let (presence_tx, presence_rx) = wake::channel(&wake);
        let (capture_events, capture_rx) = wake::channel(&wake);
        let (desktop_events, desktop_rx) = wake::channel(&wake);
        Self {
            settings,
            shared,
            events,
            wake,
            internal_tx,
            internal_rx,
            api: None,
            session: None,
            room: None,
            capture: None,
            capture_frames: convert::screen_slot(),
            publishing: Arc::new(AtomicBool::new(false)),
            share_epoch: Arc::new(AtomicU64::new(0)),
            screen_ramp: None,
            input: None,
            grant: ControlGrant::default(),
            remote_host: None,
            peer_control: false,
            participants: vec![],
            presence: None,
            presence_stop: None,
            presence_tx,
            presence_rx,
            current_room: Default::default(),
            last_refresh: Instant::now(),
            clipboard: Default::default(),
            remote_clipboard: Default::default(),
            clipboard_tasks: vec![],
            clipboard_epoch: Arc::new(AtomicU64::new(0)),
            clipboard_out: VecDeque::new(),
            outgoing: None,
            call_peer: None,
            teammates: vec![],
            pending_join: None,
            pending_call: None,
            pending_call_cancel: None,
            join_cleanup: None,
            pending_capture: None,
            capture_generation: 0,
            capture_events,
            capture_rx,
            session_frames: Default::default(),
            desktop_events,
            desktop_rx,
            desktop_task: None,
            desktop_locked: true,
            privacy_locked: Arc::new(AtomicBool::new(true)),
            transport_ready: false,
            incoming_call: None,
            incoming_invite: None,
            outgoing_invite: None,
            socket_connected: false,
            user_id: None,
            refresh_task: None,
            speaker: Recovery::default(),
            microphone: Recovery::default(),
            tap: crate::capture::Tap::default(),
            recording: None,
            moments: None,
            finishing: Vec::new(),
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Background work the loop must poll for, since it doesn't wake the loop itself.
    fn busy(&self) -> bool {
        self.pending_join.is_some()
            || self.pending_capture.is_some()
            || self.refresh_task.is_some()
            || !self.clipboard_tasks.is_empty()
            || !self.clipboard_out.is_empty()
    }

    async fn service(&mut self) {
        while let Ok(event) = self.desktop_rx.try_recv() {
            match event {
                Event::DesktopLocked => {
                    self.desktop_locked = true;
                    self.stop_share().await;
                    self.emit(Event::DesktopLocked);
                }
                Event::DesktopUnlocked => {
                    self.desktop_locked = false;
                    self.emit(Event::DesktopUnlocked);
                }
                event => self.emit(event),
            }
        }
        self.finish_operations().await;
        self.recover_audio().await;
        self.flush_frames();
        self.publish_host_cursor().await;
        let mut handled = 0;
        while let Ok(event) = self.internal_rx.try_recv() {
            self.internal(event).await;
            handled += 1;
            if handled == 256 {
                // Let commands (revoke, leave) in before the rest of a burst.
                self.wake.notify_one();
                break;
            }
        }
        while let Ok(event) = self.capture_rx.try_recv() {
            match event {
                Event::CaptureClosed => self.stop_share().await,
                event => self.emit(event),
            }
        }
        while let Ok(event) = self.presence_rx.try_recv() {
            if let Err(error) = self.presence_event(event).await {
                self.emit(Event::Error(format!("{error:#}")));
            }
        }
        if self.last_refresh.elapsed() > Duration::from_secs(20)
            && self.api.is_some()
            && let Err(error) = self.refresh().await
        {
            self.emit(Event::Error(format!("{error:#}")));
        }
    }

    async fn run(&mut self, mut commands: async_mpsc::Receiver<Command>) -> Result<()> {
        self.refresh_devices();
        // A capture left open by a client that crashed or was killed (one another
        // running instance is still recording keeps its lock and stays open).
        if crate::capture::store::default_path().is_ok_and(|path| path.exists())
            && let Err(error) = crate::capture::store::Store::open_default()
                .and_then(|store| crate::capture::liveness::close_orphans(&store).map(|_| ()))
                .and_then(|()| crate::capture::prune().map(|_| ()))
        {
            crate::logger::line(format!("Capture database: {error:#}"));
        }
        let desktop_events = self.desktop_events.clone();
        let privacy_locked = self.privacy_locked.clone();
        self.desktop_task = Some(tokio::spawn(async move {
            if let Err(error) =
                crate::lock::watch_state(desktop_events.clone(), privacy_locked).await
            {
                let _ = desktop_events
                    .send(Event::Error(format!("Desktop privacy observer: {error:#}")));
            }
        }));
        if let Some(token) = self.settings.token.clone()
            && let Err(error) = self.sign_in(self.settings.server.clone(), token).await
        {
            self.emit(Event::Error(format!("{error:#}")));
        }
        let wake = self.wake.clone();
        loop {
            // Everything this loop consumes wakes it on arrival, so input, frames and room
            // data are handled immediately. The poll only covers background jobs (joins,
            // the chooser, clipboard transfers, refreshes) and throttled annotation redraws;
            // otherwise the session sleeps.
            let poll = if self.busy() {
                Duration::from_millis(25)
            } else {
                [self.speaker.wait(), self.microphone.wait()]
                    .into_iter()
                    .flatten()
                    .fold(Duration::from_secs(1), Duration::min)
            };
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { break; };
                    if matches!(command, Command::Shutdown) { break; }
                    if let Err(error) = self.command(command).await {
                        self.emit(Event::Error(format!("{error:#}")));
                    }
                }
                _ = wake.notified() => self.service().await,
                _ = tokio::time::sleep(poll) => self.service().await,
            }
        }
        self.leave().await;
        if let Some(pending) = self.pending_capture.take() {
            pending.canceled.store(true, Ordering::Release);
            pending.task.abort();
            let _ = pending.task.await;
            self.emit(Event::SelectingSource(false));
        }
        if let Some(cleanup) = self.join_cleanup.take() {
            let _ = cleanup.await;
        }
        if let Some(stop) = self.presence_stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.presence.take() {
            task.abort();
        }
        if let Some(task) = self.desktop_task.take() {
            task.abort();
        }
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
        if let Some(cleanup) = self.join_cleanup.take() {
            let _ = cleanup.await;
        }
        // Let a capture write its last lines before Hopp exits.
        for task in self.finishing.drain(..) {
            let _ = tokio::time::timeout(Duration::from_secs(35), task).await;
        }
        Ok(())
    }

    async fn sign_in(&mut self, server: String, token: String) -> Result<()> {
        if token.trim().is_empty() {
            let address = crate::api::login_url(&server)?;
            if self.settings.server != server {
                self.sign_out().await?;
            }
            self.settings.server = server;
            settings::save(&self.settings)?;
            ProcessCommand::new("xdg-open")
                .arg(address)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            self.emit(Event::Status(
                "Complete sign-in in your browser, then return here.".into(),
            ));
            return Ok(());
        }
        self.emit(Event::Status("Signing in…".into()));
        let api = ApiClient::new(&server, Some(token.trim().into()))?;
        let user = api.user().await?;
        self.leave().await;
        if let Some(cleanup) = self.join_cleanup.take() {
            let _ = cleanup.await;
        }
        if let Some(stop) = self.presence_stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.presence.take() {
            task.abort();
        }
        self.settings.server = server;
        self.settings.token = Some(token.trim().into());
        settings::save(&self.settings)?;
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
        let (presence_tx, presence_rx) = wake::channel(&self.wake);
        self.presence_tx = presence_tx;
        self.presence_rx = presence_rx;
        self.socket_connected = false;
        self.user_id = Some(user.id.clone());
        let (stop, rx) = watch::channel(false);
        self.presence_stop = Some(stop);
        let (outgoing, outgoing_rx) = async_mpsc::channel(32);
        self.outgoing = Some(outgoing);
        self.presence = Some(tokio::spawn(presence::run_presence(
            api.clone(),
            self.current_room.clone(),
            rx,
            self.presence_tx.clone(),
            outgoing_rx,
        )));
        self.api = Some(api);
        self.emit(Event::LoggedIn(user));
        self.emit(Event::Status("Ready to pair".into()));
        self.refresh().await
    }

    async fn sign_out(&mut self) -> Result<()> {
        self.leave().await;
        if let Some(cleanup) = self.join_cleanup.take() {
            let _ = cleanup.await;
        }
        if let Some(stop) = self.presence_stop.take() {
            let _ = stop.send(true);
        }
        if let Some(task) = self.presence.take() {
            task.abort();
        }
        if let Some(task) = self.refresh_task.take() {
            task.abort();
        }
        self.api = None;
        self.outgoing = None;
        self.socket_connected = false;
        self.user_id = None;
        self.teammates.clear();
        let (sender, receiver) = wake::channel(&self.wake);
        self.presence_tx = sender;
        self.presence_rx = receiver;
        self.settings.token = None;
        settings::save(&self.settings)?;
        self.emit(Event::Rooms(vec![]));
        self.emit(Event::Teammates(vec![]));
        self.emit(Event::SignedOut);
        Ok(())
    }

    async fn refresh(&mut self) -> Result<()> {
        self.last_refresh = Instant::now();
        if self.refresh_task.is_none()
            && let Some(api) = self.api.clone()
        {
            self.refresh_task = Some(tokio::spawn(async move {
                let (rooms, teammates, presence) =
                    tokio::try_join!(api.rooms(), api.teammates(), api.rooms_presence())?;
                // Older servers don't have it; nothing else depends on it.
                let calls = api.calls_presence().await.unwrap_or_default();
                Ok(Refreshed {
                    rooms,
                    teammates,
                    presence,
                    calls,
                })
            }));
        }
        Ok(())
    }

    fn refresh_devices(&self) {
        let devices = media::audio_devices().unwrap_or(media::AudioDevices {
            inputs: vec![],
            outputs: vec![],
        });
        let cameras = media::camera_devices()
            .into_iter()
            .map(|d| (d.id, d.name))
            .collect();
        self.emit(Event::Devices {
            inputs: devices.inputs,
            outputs: devices.outputs,
            cameras,
        });
    }

    async fn command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::SignIn { server, token } => self.sign_in(server, token).await?,
            Command::SignOut => self.sign_out().await?,
            Command::Refresh => {
                self.refresh_devices();
                self.refresh().await?;
            }
            Command::CreateRoom(name) => {
                let api = self.api.as_ref().context("Sign in first")?;
                api.create_room(name.trim()).await?;
                self.refresh().await?;
            }
            Command::RenameRoom { id, name } => {
                let api = self.api.as_ref().context("Sign in first")?;
                api.rename_room(&id, &name).await?;
                self.refresh().await?;
            }
            Command::DeleteRoom(id) => {
                let api = self.api.as_ref().context("Sign in first")?;
                api.delete_room(&id).await?;
                self.refresh().await?;
            }
            Command::JoinRoom(room) => {
                self.api.as_ref().context("Sign in first")?;
                self.leave().await;
                self.emit(Event::Status(format!("Connecting to {}…", room.name)));
                self.begin_join(JoinTarget::Room(room))?;
            }
            Command::JoinSession(id) => {
                self.api.as_ref().context("Sign in first")?;
                if self.room.as_ref().is_some_and(|room| room.id == id) {
                    return Ok(());
                }
                if self.session.is_some() || self.pending_join.is_some() {
                    bail!("Leave your current call first");
                }
                self.emit(Event::Status("Joining the Slack session…".into()));
                self.begin_join(JoinTarget::Session(RoomInfo {
                    id,
                    name: "Slack session".into(),
                }))?;
            }
            Command::Leave => self.leave().await,
            Command::Share { window_only } => {
                if self.pending_capture.is_some() {
                    self.emit(Event::Status(
                        "Finish or dismiss the existing system chooser before sharing again".into(),
                    ));
                    return Ok(());
                }
                if self.privacy_locked.load(Ordering::Acquire) {
                    bail!("Unlock your desktop before sharing");
                }
                self.session
                    .as_ref()
                    .context("Join a call before sharing")?;
                if !self.transport_ready {
                    bail!("Wait for the call to reconnect before sharing");
                }
                self.stop_share().await;
                if self.privacy_locked.load(Ordering::Acquire) {
                    bail!("Unlock your desktop before sharing");
                }
                self.emit(Event::Status(
                    "Choose a source in the system chooser, or cancel there with Escape".into(),
                ));
                self.emit(Event::SelectingSource(true));
                let (events, receiver) = wake::channel(&self.wake);
                self.capture_events = events.clone();
                self.capture_rx = receiver;
                self.capture_frames = convert::screen_slot();
                let frames = self.capture_frames.clone();
                let canceled = Arc::new(AtomicBool::new(false));
                self.pending_capture = Some(PendingCapture {
                    task: tokio::spawn(Capture::start_cancellable(
                        frames,
                        events,
                        window_only,
                        canceled.clone(),
                    )),
                    canceled,
                    generation: self.capture_generation,
                });
            }
            Command::StopShare => self.stop_share().await,
            Command::Microphone(enabled) => {
                self.microphone = Recovery::default();
                self.session
                    .as_mut()
                    .context("Join a room first")?
                    .set_microphone(enabled)
                    .await?;
                self.emit(Event::Microphone(enabled));
            }
            Command::Camera(enabled) => {
                self.session
                    .as_mut()
                    .context("Join a room first")?
                    .set_camera(enabled, self.session_frames.local_camera.clone())
                    .await?;
                if !enabled && let Ok(mut preview) = self.shared.local_camera.lock() {
                    *preview = None;
                }
                self.emit(Event::Camera(enabled));
            }
            Command::GrantControl(identity) => {
                if !self.transport_ready {
                    bail!("Wait for the call to reconnect before granting control");
                }
                if self.privacy_locked.load(Ordering::Acquire) {
                    bail!("Unlock your desktop before granting control");
                }
                if !self
                    .participants
                    .iter()
                    .any(|p| canonical_identity(&p.identity) == canonical_identity(&identity))
                {
                    bail!("Participant is no longer in this call");
                }
                let target = self
                    .capture
                    .as_ref()
                    .context("Share a monitor before granting control")?
                    .target
                    .clone();
                let output = target
                    .output_name
                    .as_deref()
                    .context("Remote control needs a shared monitor that Hyprland can identify")?;
                self.revoke().await;
                if self.privacy_locked.load(Ordering::Acquire) {
                    bail!("Unlock your desktop before granting control");
                }
                self.input = Some(NativeInput::connect(output)?);
                self.grant.grant(identity.clone());
                self.broadcast(
                    InputEvent::RemoteControlEnabled(crate::protocol::Enabled { enabled: true }),
                    vec![identity.clone()],
                )
                .await?;
                self.session
                    .as_ref()
                    .context("Call ended")?
                    .send_data(
                        identity.as_bytes().to_vec(),
                        Some("participant_in_control".into()),
                        vec![],
                    )
                    .await?;
                let given = self.participant_name(&identity);
                self.capture_event(|own| format!("{own} gave {given} control"));
                self.emit(Event::ControlGranted(Some(identity)));
            }
            Command::RevokeControl => {
                self.revoke().await;
                if self.peer_control {
                    self.broadcast(
                        InputEvent::RemoteControlEnabled(crate::protocol::Enabled {
                            enabled: false,
                        }),
                        self.remote_host.clone().into_iter().collect(),
                    )
                    .await
                    .ok();
                    self.peer_control = false;
                    self.emit(Event::PeerControl { enabled: false });
                }
            }
            Command::SendInput(event) => {
                self.own_drawing(&event);
                self.send_input(event).await?
            }
            Command::SendStamp(message) => self.send_stamp(message).await?,
            Command::ShareClipboard => {
                if !self.peer_control || self.privacy_locked.load(Ordering::Acquire) {
                    bail!("Your partner must grant control before you send clipboard text");
                }
                if self.clipboard_tasks.len() >= 4 {
                    bail!("A clipboard transfer is already in progress");
                }
                self.clipboard_tasks.push(tokio::spawn(async {
                    Ok(ClipboardResult::SendPaste(
                        tokio::task::spawn_blocking(crate::clipboard::read).await??,
                    ))
                }));
            }
            Command::CopyClipboard(is_copy) => {
                self.send_input(InputEvent::AddToClipboard(crate::protocol::ClipboardCopy {
                    is_copy,
                }))
                .await?
            }
            Command::SelectAudio { input, output } => {
                if let Some(session) = self.session.as_mut() {
                    session
                        .select_audio_devices(input.clone(), output.clone())
                        .await?;
                }
                self.settings.input_device = input;
                self.settings.output_device = output;
                settings::save(&self.settings)?;
            }
            Command::SelectCamera(device) => {
                if let Some(session) = self.session.as_mut() {
                    session.select_camera_device(device.clone())?;
                }
                self.settings.camera_device = device;
                settings::save(&self.settings)?;
            }
            Command::CallPeer { user_id, name } => {
                if !self.socket_connected {
                    bail!("Hopp is reconnecting; try the call again when connected");
                }
                self.leave().await;
                let cleanup = self.join_cleanup.take();
                let outgoing = self
                    .outgoing
                    .clone()
                    .context("Hopp connection unavailable")?;
                let events = self.internal_tx.clone();
                let callee = user_id.clone();
                let (cancel, canceled) = watch::channel(false);
                self.pending_call_cancel = Some(cancel);
                self.pending_call = Some(tokio::spawn(async move {
                    if let Some(cleanup) = cleanup {
                        let _ = cleanup.await;
                    }
                    // Hold the watch read guard until the request is queued. A
                    // concurrent cancel cannot queue hangup before this request.
                    let canceled = canceled.borrow();
                    if !*canceled && outgoing.try_send(presence::call_request(&callee)).is_err() {
                        let _ =
                            events.send(Event::Error("Hopp connection is busy or ended".into()));
                        let _ = events.send(Event::Disconnected("Call could not be placed".into()));
                    }
                }));
                self.call_peer = Some(user_id.clone());
                self.emit(Event::Calling {
                    peer_id: user_id,
                    name,
                });
            }
            Command::AcceptCall(caller) => {
                if self.incoming_call.as_deref() != Some(caller.as_str()) {
                    bail!("That incoming call is no longer available");
                }
                self.send_socket(presence::call_accept(&caller)).await?;
                self.incoming_call = None;
                self.call_peer = Some(caller.clone());
            }
            Command::RejectCall(caller) => {
                if self.incoming_call.as_deref() == Some(caller.as_str()) {
                    self.incoming_call = None;
                    self.send_socket(presence::call_reject(&caller, "rejected"))
                        .await?;
                    self.emit(Event::Disconnected("Call declined".into()));
                }
            }
            Command::InvitePeer { user_id, name } => {
                self.session
                    .as_ref()
                    .context("Join a call before inviting a teammate")?;
                self.cancel_invite().await;
                let invite_id = format!(
                    "omarchy-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_nanos()
                );
                self.send_socket(presence::invite_request(&user_id, &invite_id))
                    .await?;
                self.outgoing_invite = Some(PendingInvite {
                    peer: user_id.clone(),
                    invite_id: invite_id.clone(),
                });
                self.emit(Event::Inviting {
                    peer_id: user_id,
                    invite_id,
                    name,
                });
            }
            Command::AcceptInvite {
                inviter_id,
                invite_id,
            } => {
                if self.incoming_invite.as_ref() != Some(&(inviter_id.clone(), invite_id.clone())) {
                    bail!("That invitation is no longer available");
                }
                self.send_socket(presence::invite_accept(&inviter_id, &invite_id))
                    .await?;
                self.incoming_invite = None;
                self.emit(Event::InviteResolved {
                    message: "Joining your teammate's call".into(),
                });
                self.leave().await;
                let name = self.peer_name(&inviter_id);
                self.begin_join(JoinTarget::Invitation {
                    inviter: inviter_id,
                    name,
                })?;
            }
            Command::RejectInvite {
                inviter_id,
                invite_id,
            } => {
                if self.incoming_invite.as_ref() == Some(&(inviter_id.clone(), invite_id.clone())) {
                    self.send_socket(presence::invite_reject(&inviter_id, &invite_id, "rejected"))
                        .await?;
                    self.incoming_invite = None;
                    self.emit(Event::InviteResolved {
                        message: "Invitation declined".into(),
                    });
                }
            }
            Command::CancelInvite => self.cancel_invite().await,
            #[cfg(test)]
            Command::TestDesktopLock(locked) => {
                self.privacy_locked.store(locked, Ordering::Release);
                let _ = self.desktop_events.send(if locked {
                    Event::DesktopLocked
                } else {
                    Event::DesktopUnlocked
                });
            }
            Command::Activate => self.emit(Event::Activate),
            Command::Ui(request) => self.emit(Event::Ui(request)),
            Command::StartCapture => self.start_capture().await?,
            Command::StopCapture => self.stop_capture(true).await,
            Command::Bookmark(label) => {
                let own = self.session.as_ref().map(Session::own_name);
                match self.tap.feed() {
                    Some(feed) => {
                        let label = label.unwrap_or_else(|| "Bookmark".into());
                        if let Some(moments) = self.moments.as_mut() {
                            moments.bookmark(own.clone(), &label);
                        }
                        feed.bookmark(own, label);
                        self.emit(Event::Status("Bookmarked".into()));
                    }
                    None => bail!("Nothing is being captured"),
                }
            }
            Command::Shutdown => {}
        }
        Ok(())
    }

    async fn broadcast(&self, event: InputEvent, destinations: Vec<String>) -> Result<()> {
        if !event.valid() {
            bail!("Invalid input event");
        }
        self.session
            .as_ref()
            .context("Call ended")?
            .send_data(
                serde_json::to_vec(&event)?,
                event.topic().map(str::to_owned),
                destinations,
            )
            .await
    }

    fn reset_session_events(&mut self) {
        let (sender, receiver) = wake::channel(&self.wake);
        self.internal_tx = sender;
        self.internal_rx = receiver;
        self.session_frames = SharedFrames::default();
    }

    fn cancel_join(&mut self) {
        if let Some(pending) = self.pending_join.take() {
            let _ = pending.cancel.send(true);
            let preceding = self.join_cleanup.take();
            self.join_cleanup = Some(tokio::spawn(async move {
                if let Some(preceding) = preceding {
                    let _ = preceding.await;
                }
                if let Ok(Ok(Some(mut connected))) = pending.task.await {
                    connected.session.disconnect().await;
                    connected.rollback.run().await;
                }
            }));
        }
    }

    fn begin_join(&mut self, target: JoinTarget) -> Result<()> {
        let api = self.api.clone().context("Sign in first")?;
        self.cancel_join();
        self.reset_session_events();
        let cleanup = self.join_cleanup.take();
        let events = self.internal_tx.clone();
        let frames = self.session_frames.clone();
        let settings = self.settings.clone();
        let outgoing = self.outgoing.clone();
        let tap = self.tap.clone();
        let (cancel, mut canceled) = watch::channel(false);
        let task = tokio::spawn(async move {
            // A room-leave endpoint removes global user presence. Serialize old
            // cleanup before obtaining new tokens so it cannot erase a new call.
            if let Some(cleanup) = cleanup {
                let _ = cleanup.await;
            }
            if *canceled.borrow() {
                if let JoinTarget::Direct { peer, .. } = &target
                    && let Some(outgoing) = outgoing.as_ref()
                {
                    let _ = outgoing.send(presence::call_end(peer)).await;
                }
                return Ok(None);
            }
            let mut rollback = JoinRollback {
                api: api.clone(),
                room: match &target {
                    JoinTarget::Room(room) | JoinTarget::Session(room) => Some(room.id.clone()),
                    _ => None,
                },
                peer: match &target {
                    JoinTarget::Direct { peer, .. } => Some(peer.clone()),
                    JoinTarget::Invitation { inviter, .. } => Some(inviter.clone()),
                    _ => None,
                },
                outgoing,
            };
            // Token endpoints mutate backend presence. Finish their bounded
            // response before rollback, even after cancellation, so a delayed
            // join cannot recreate presence after its leave has completed.
            let fetched = async {
                let (tokens, room, peer) = match target {
                    JoinTarget::Room(room) => (api.join_room(&room.id).await?, room, None),
                    JoinTarget::Session(room) => (api.join_session(&room.id).await?, room, None),
                    JoinTarget::Direct {
                        payload,
                        peer,
                        name,
                    } => {
                        let tokens =
                            crate::api::RoomTokens::from_payload(payload, api.server_url().await?)?;
                        (
                            tokens,
                            RoomInfo {
                                id: format!("call:{peer}"),
                                name,
                            },
                            Some(peer),
                        )
                    }
                    JoinTarget::Invitation { inviter, name } => {
                        let joined = api.join_call(&inviter).await?;
                        rollback.room = joined.room.as_ref().map(|room| room.id.clone());
                        let peer = joined.room.is_none().then(|| inviter.clone());
                        let room = joined.room.unwrap_or(RoomInfo {
                            id: format!("call:{inviter}"),
                            name,
                        });
                        (joined.tokens, room, peer)
                    }
                };
                Ok::<_, anyhow::Error>((tokens, room, peer))
            }
            .await;
            let (tokens, room, peer) = match fetched {
                Ok(fetched) => fetched,
                Err(error) => {
                    rollback.run().await;
                    return Err(error);
                }
            };
            if *canceled.borrow() {
                rollback.run().await;
                return Ok(None);
            }
            let result = {
                let work = async {
                    let mut session = Session::connect(
                        &tokens.livekit_url,
                        &tokens.audio_token,
                        &tokens.video_token,
                        events.clone(),
                        frames.remote.clone(),
                        frames.cameras.clone(),
                        tap,
                    )
                    .await?;
                    // Device preferences are optional. Keeping ownership of a
                    // successful connection avoids leaking a room on device failure.
                    if let Err(error) = session
                        .select_audio_devices(settings.input_device, settings.output_device)
                        .await
                    {
                        let _ = events.send(Event::Error(format!("Audio settings: {error:#}")));
                    }
                    if let Err(error) = session.select_camera_device(settings.camera_device) {
                        let _ = events.send(Event::Error(format!("Camera settings: {error:#}")));
                    }
                    Ok::<_, anyhow::Error>((session, room, peer))
                };
                tokio::pin!(work);
                tokio::select! {
                    biased;
                    _=canceled.changed()=>None,
                    result=&mut work=>Some(result),
                }
            };
            match result {
                Some(Ok((session, room, peer))) => Ok(Some(ConnectedSession {
                    session,
                    room,
                    peer,
                    rollback,
                })),
                Some(Err(error)) => {
                    rollback.run().await;
                    Err(error)
                }
                None => {
                    rollback.run().await;
                    Ok(None)
                }
            }
        });
        self.pending_join = Some(PendingJoin { task, cancel });
        Ok(())
    }

    async fn finish_operations(&mut self) {
        self.finish_clipboard().await;

        if self
            .pending_join
            .as_ref()
            .is_some_and(|pending| pending.task.is_finished())
            && let Some(pending) = self.pending_join.take()
        {
            match pending.task.await {
                Ok(Ok(Some(connected))) => {
                    if let Ok(mut current) = self.current_room.lock() {
                        *current = Some(connected.session.room_name());
                    }
                    self.transport_ready = true;
                    self.session = Some(connected.session);
                    // Publish your microphone track muted straight away, so other Hopp
                    // apps show you as muted rather than as unmuted with no track.
                    if let Some(session) = self.session.as_mut()
                        && let Err(error) = session.publish_muted_microphone().await
                    {
                        crate::logger::line(format!("publishing muted microphone: {error:#}"));
                    }
                    self.room = Some(connected.room.clone());
                    self.call_peer = connected.peer;
                    self.emit(Event::Joined(connected.room));
                    self.emit(Event::Status("Connected".into()));
                    self.auto_capture().await;
                }
                Ok(Ok(None)) => {}
                result => {
                    // Tell them, or they'd be left waiting in a call on their own (the
                    // Mac does the same when a call fails to start).
                    if let Some(peer) = self.call_peer.take() {
                        let _ = self.send_socket(presence::call_end(&peer)).await;
                    }
                    self.reset_session_events();
                    let message = match result {
                        Ok(Err(error)) => format!("{error:#}"),
                        _ => "Native connection task ended".into(),
                    };
                    self.emit(Event::Error(message));
                    self.emit(Event::Disconnected("Could not connect".into()));
                }
            }
        }
        if self
            .pending_capture
            .as_ref()
            .is_some_and(|pending| pending.task.is_finished())
            && let Some(pending) = self.pending_capture.take()
        {
            let canceled = pending.canceled.load(Ordering::Acquire)
                || pending.generation != self.capture_generation;
            self.emit(Event::SelectingSource(false));
            match pending.task.await {
                Ok(Ok(capture))
                    if !canceled
                        && self.session.is_some()
                        && !self.privacy_locked.load(Ordering::Acquire)
                        && self.transport_ready =>
                {
                    if let Some(session) = &self.session {
                        session.set_sharing(true);
                        if let Some(ramp) =
                            self.screen_ramp.replace(session.follow_screen_bandwidth())
                        {
                            ramp.abort();
                        }
                    }
                    let target = capture.target.clone();
                    self.capture = Some(capture);
                    if let Some(moments) = self.moments.as_mut() {
                        moments.screen_changed(Some(&target));
                    }
                    let screen = target.output_name.clone();
                    self.capture_event(|own| match screen {
                        Some(screen) => format!("{own} started sharing {screen}"),
                        None => format!("{own} started sharing a window"),
                    });
                    self.emit(Event::Sharing {
                        active: true,
                        target: Some(target),
                    });
                    self.emit(Event::Status("Sharing your screen".into()));
                }
                Ok(Ok(mut capture)) => {
                    // Late consent belongs to the canceled selection, never
                    // to a newer call. Its private frame/event queues are orphaned.
                    tokio::spawn(async move {
                        capture.stop().await;
                    });
                    self.emit(Event::Status("Screen selection canceled".into()));
                }
                Ok(Err(_)) | Err(_) if canceled => {
                    self.emit(Event::Status("Screen selection canceled".into()));
                }
                Ok(Err(error))
                    if error.downcast_ref::<ashpd::Error>().is_some_and(|error| {
                        matches!(
                            error,
                            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
                        )
                    }) =>
                {
                    self.emit(Event::Status("Screen selection canceled".into()));
                }
                Ok(Err(error)) => {
                    self.emit(Event::Error(format!(
                        "Screen sharing could not start: {error:#}. Choose a source again to retry."
                    )));
                    self.emit(Event::Sharing {
                        active: false,
                        target: None,
                    });
                }
                Err(error) if error.is_cancelled() => {
                    self.emit(Event::Status("Screen selection canceled".into()));
                }
                Err(_) => self.emit(Event::Error(
                    "Screen selection ended unexpectedly. Choose a source again to retry.".into(),
                )),
            }
        }
        if self
            .refresh_task
            .as_ref()
            .is_some_and(|task| task.is_finished())
            && let Some(task) = self.refresh_task.take()
        {
            match task.await {
                Ok(Ok(refreshed)) => {
                    self.teammates = refreshed.teammates.clone();
                    self.emit(Event::Rooms(refreshed.rooms));
                    self.emit(Event::Teammates(refreshed.teammates));
                    self.emit(Event::RoomPresence(refreshed.presence));
                    self.emit(Event::CallsPresence(refreshed.calls));
                }
                Ok(Err(error)) => self.emit(Event::Error(format!("{error:#}"))),
                _ => {}
            }
        }
    }

    async fn cancel_invite(&mut self) {
        if let Some(invite) = self.outgoing_invite.take() {
            let _ = self
                .send_socket(presence::invite_cancel(&invite.peer, &invite.invite_id))
                .await;
            self.emit(Event::InviteResolved {
                message: "Invitation canceled".into(),
            });
        }
    }

    /// While sharing a monitor, tell viewers where the pointer is (normalized to the
    /// shared output), as upstream Hopp hosts do. Viewers use it to follow along.
    async fn publish_host_cursor(&mut self) {
        let Some(capture) = self.capture.as_ref() else {
            return;
        };
        let Some(point) = capture.cursor.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        if !self.transport_ready || self.privacy_locked.load(Ordering::Acquire) {
            return;
        }
        let _ = self
            .broadcast(InputEvent::MouseMove(point), Vec::new())
            .await;
    }

    /// One of your stamps, to everyone in the call. A new one also goes out as a click
    /// animation, which clients that don't know stamps still show.
    async fn send_stamp(&mut self, message: crate::protocol::StampMessage) -> Result<()> {
        if !message.valid() || !self.transport_ready || self.privacy_locked.load(Ordering::Acquire)
        {
            return Ok(());
        }
        if let Some(session) = &self.session {
            let (identity, name) = (session.own_identity(), session.own_name());
            if let Some(moments) = self.moments.as_mut() {
                moments.stamp(&identity, &name, &message);
            }
        }
        let session = self.session.as_ref().context("Call ended")?;
        session
            .send_data(
                serde_json::to_vec(&message)?,
                Some(crate::protocol::STAMP_TOPIC.into()),
                vec![],
            )
            .await?;
        if let crate::protocol::StampMessage::StampPlace { x, y, .. } = message {
            self.broadcast(
                InputEvent::ClickAnimation(crate::protocol::Point { x, y }),
                vec![],
            )
            .await?;
        }
        Ok(())
    }

    /// Your own ink goes into the capture's frames as well as to the call.
    fn own_drawing(&mut self, event: &InputEvent) {
        if self.moments.is_none() || self.remote_host.is_none() && self.capture.is_none() {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let (identity, name) = (session.own_identity(), session.own_name());
        if let Some(moments) = self.moments.as_mut() {
            moments.draw(&identity, &name, event);
        }
    }

    async fn send_input(&self, event: InputEvent) -> Result<()> {
        let drawing = matches!(
            event,
            InputEvent::DrawStart(_)
                | InputEvent::DrawAddPoint(_)
                | InputEvent::DrawEnd(_)
                | InputEvent::DrawClearAllPaths
                | InputEvent::DrawClearPath { .. }
                | InputEvent::DrawingMode(_)
                | InputEvent::ClickAnimation(_)
        );
        if !self.peer_control && !drawing && !matches!(event, InputEvent::MouseMove(_)) {
            return Ok(());
        }
        if !self.transport_ready || self.privacy_locked.load(Ordering::Acquire) {
            return Ok(());
        }
        let destinations = self.remote_host.clone().into_iter().collect();
        self.broadcast(event, destinations).await
    }

    /// The open microphone stopped (device error or its processing ended): close it and
    /// reopen it shortly. The user's mute state is unchanged; they're still unmuted.
    async fn microphone_lost(&mut self) {
        let Some(session) = self
            .session
            .as_mut()
            .filter(|session| session.microphone_failed())
        else {
            return;
        };
        if let Err(error) = session.set_microphone(false).await {
            crate::logger::line(format!("closing failed microphone: {error:#}"));
        }
        self.microphone.failed();
    }

    /// Retry audio devices whose reopen is due.
    async fn recover_audio(&mut self) {
        let Some(session) = self.session.as_mut() else {
            self.speaker = Recovery::default();
            self.microphone = Recovery::default();
            return;
        };
        if self.speaker.due() {
            match session.reopen_speaker() {
                Ok(()) => {
                    self.speaker.due = None;
                    crate::logger::line("speaker reopened");
                }
                Err(error) => {
                    self.speaker.failed();
                    if self.speaker.report() {
                        let _ = self.events.send(Event::Error(format!(
                            "Speaker unavailable, still retrying: {error:#}"
                        )));
                    }
                }
            }
        }
        if self.microphone.due() {
            match session.set_microphone(true).await {
                Ok(()) => {
                    self.microphone.due = None;
                    crate::logger::line("microphone reopened");
                }
                Err(error) => {
                    self.microphone.failed();
                    if self.microphone.report() {
                        let _ = self.events.send(Event::Error(format!(
                            "Microphone unavailable, still retrying: {error:#}"
                        )));
                    }
                }
            }
        }
    }

    async fn internal(&mut self, event: Event) {
        match event {
            Event::AudioDeviceFailed {
                input: false,
                error,
            } => {
                // Ignore a report from a stream that has already been replaced.
                let Some(session) = self.session.as_mut().filter(|s| s.speaker_failed()) else {
                    return;
                };
                session.close_speaker();
                crate::logger::line(format!("speaker failed, reopening: {error}"));
                self.speaker.failed();
            }
            Event::AudioDeviceFailed { input: true, error } => {
                crate::logger::line(format!("microphone failed, reopening: {error}"));
                self.microphone_lost().await;
            }
            Event::Microphone(false) => self.microphone_lost().await,
            Event::Camera(false) => {
                if !self.session.as_ref().is_some_and(Session::camera_failed) {
                    return;
                }
                if let Some(session) = self.session.as_mut()
                    && let Err(error) = session
                        .set_camera(false, self.session_frames.local_camera.clone())
                        .await
                {
                    self.emit(Event::Error(format!("Stopping camera: {error:#}")));
                }
                if let Ok(mut preview) = self.shared.local_camera.lock() {
                    *preview = None;
                }
                self.emit(Event::Camera(false));
            }
            Event::Data {
                sender,
                payload,
                topic,
            } => {
                if let Err(error) = self.receive_data(&sender, &payload, topic.as_deref()).await {
                    self.emit(Event::Error(format!("Remote input: {error:#}")));
                }
            }
            Event::CaptureClosed => self.stop_share().await,
            Event::TransportReconnecting => {
                self.transport_ready = false;
                self.peer_control = false;
                self.stop_share().await;
                self.emit(Event::PeerControl { enabled: false });
                self.emit(Event::TransportReconnecting);
            }
            Event::TransportReconnected => {
                self.transport_ready = true;
                self.emit(Event::TransportReconnected);
            }
            Event::Participants(participants) => {
                if let Some(feed) = self.tap.feed() {
                    let names = |list: &[Participant]| {
                        list.iter()
                            .map(|p| (canonical_identity(&p.identity).to_owned(), p.name.clone()))
                            .collect::<HashMap<_, _>>()
                    };
                    let (before, after) = (names(&self.participants), names(&participants));
                    for (identity, name) in &after {
                        if !before.contains_key(identity) {
                            feed.event(format!("{name} joined"));
                        }
                    }
                    for (identity, name) in &before {
                        if !after.contains_key(identity) {
                            feed.event(format!("{name} left"));
                        }
                    }
                    feed.participants(self.capture_names(&participants));
                }
                self.participants = participants.clone();
                // A departed controller loses its grant immediately.
                if self.grant.active()
                    && !participants
                        .iter()
                        .any(|p| self.grant.allows(&p.identity, self.capture.is_some()))
                {
                    self.revoke().await;
                }
                self.emit(Event::Participants(participants));
            }
            Event::RemoteShare { identity, active } => {
                let audio_identity = format!("{}:audio", canonical_identity(&identity));
                let sharer = self.participant_name(&identity);
                let was = self.remote_host.as_deref() == Some(audio_identity.as_str());
                if active && !was {
                    self.capture_event(|_| format!("{sharer} started sharing their screen"));
                } else if !active && was {
                    self.capture_event(|_| format!("{sharer} stopped sharing their screen"));
                }
                if active {
                    if self.remote_host.as_deref() != Some(audio_identity.as_str()) {
                        if let Ok(mut preview) = self.shared.remote.lock() {
                            *preview = None;
                        }
                        self.peer_control = false;
                        self.clipboard_epoch.fetch_add(1, Ordering::AcqRel);
                        self.clipboard_out.clear();
                        self.remote_clipboard.clear();
                        for task in self.clipboard_tasks.drain(..) {
                            task.abort();
                        }
                        self.emit(Event::PeerControl { enabled: false });
                    }
                    self.remote_host = Some(audio_identity.clone());
                    if let Some(moments) = self.moments.as_mut() {
                        moments.screen_changed(None);
                    }
                } else if self.remote_host.as_deref() == Some(audio_identity.as_str()) {
                    if let Ok(mut preview) = self.shared.remote.lock() {
                        *preview = None;
                    }
                    self.remote_host = None;
                    if let Some(moments) = self.moments.as_mut() {
                        moments.screen_changed(None);
                    }
                    self.peer_control = false;
                    self.clipboard_epoch.fetch_add(1, Ordering::AcqRel);
                    self.clipboard_out.clear();
                    self.remote_clipboard.clear();
                    for task in self.clipboard_tasks.drain(..) {
                        task.abort();
                    }
                    self.emit(Event::PeerControl { enabled: false });
                }
                self.emit(Event::RemoteShare { identity, active });
            }
            Event::Disconnected(reason) => {
                self.leave().await;
                self.emit(Event::Disconnected(reason));
            }
            Event::CaptureChanged => {
                let status = self
                    .recording
                    .as_ref()
                    .map(|recording| CaptureStatus::from(recording.status()));
                self.emit(Event::Capture(status));
            }
            event => self.emit(event),
        }
    }

    async fn receive_data(
        &mut self,
        sender: &str,
        payload: &[u8],
        topic: Option<&str>,
    ) -> Result<()> {
        if payload.len() > 512 * 1024
            || sender.is_empty()
            || !self
                .participants
                .iter()
                .any(|p| canonical_identity(&p.identity) == canonical_identity(sender))
        {
            return Ok(());
        }
        if topic == Some("participant_in_control") {
            if self
                .remote_host
                .as_ref()
                .is_some_and(|host| canonical_identity(host) == canonical_identity(sender))
                && let Some(session) = &self.session
            {
                self.peer_control = std::str::from_utf8(payload).is_ok_and(|id| {
                    canonical_identity(id) == canonical_identity(&session.own_identity())
                });
                self.emit(Event::PeerControl {
                    enabled: self.peer_control,
                });
            }
            return Ok(());
        }
        if topic == Some(crate::protocol::STAMP_TOPIC) {
            if let Ok(message) = serde_json::from_slice::<crate::protocol::StampMessage>(payload)
                && message.valid()
                && self.transport_ready
                && !self.privacy_locked.load(Ordering::Acquire)
            {
                if self.moments.is_some() {
                    let name = self.participant_name(sender);
                    if let Some(moments) = self.moments.as_mut() {
                        moments.stamp(sender, &name, &message);
                    }
                }
                self.emit(Event::Stamp {
                    sender: sender.into(),
                    message,
                });
            }
            return Ok(());
        }
        let Ok(event) = serde_json::from_slice::<InputEvent>(payload) else {
            return Ok(());
        };
        if !event.valid() {
            return Ok(());
        }
        if let InputEvent::RemoteControlEnabled(enabled) = &event {
            // A controller may relinquish its own grant. Only the host can
            // issue one; a peer cannot grant itself control with this message.
            if !enabled.enabled && self.grant.allows(sender, true) {
                self.revoke().await;
            }
            if self
                .remote_host
                .as_ref()
                .is_some_and(|host| canonical_identity(host) == canonical_identity(sender))
            {
                self.peer_control = enabled.enabled;
                self.emit(Event::PeerControl {
                    enabled: self.peer_control,
                });
            }
            return Ok(());
        }
        if let InputEvent::ClipboardData(data) = &event {
            let from_host = self
                .remote_host
                .as_ref()
                .is_some_and(|host| canonical_identity(host) == canonical_identity(sender));
            let addressed = self.session.as_ref().is_some_and(|session| {
                canonical_identity(&data.requester_sid)
                    == canonical_identity(&session.own_identity())
            });
            if self.peer_control
                && from_host
                && addressed
                && !self.privacy_locked.load(Ordering::Acquire)
                && let Some(packet) = data.data.clone()
                && let Some(bytes) = self.remote_clipboard.push(packet)?
            {
                if self.clipboard_tasks.len() >= 4 {
                    bail!("A clipboard transfer is already in progress");
                }
                let epoch = self.clipboard_epoch.clone();
                let expected = epoch.load(Ordering::Acquire);
                let locked = self.privacy_locked.clone();
                self.clipboard_tasks.push(tokio::spawn(async move {
                    tokio::task::spawn_blocking(move || {
                        if locked.load(Ordering::Acquire)
                            || epoch.load(Ordering::Acquire) != expected
                        {
                            bail!("Clipboard transfer was revoked");
                        }
                        crate::clipboard::write(&bytes)
                    })
                    .await??;
                    Ok(ClipboardResult::WroteLocal)
                }));
            }
            return Ok(());
        }
        if self.privacy_locked.load(Ordering::Acquire) || !self.transport_ready {
            return Ok(());
        }
        if matches!(
            event,
            InputEvent::DrawStart(_)
                | InputEvent::DrawAddPoint(_)
                | InputEvent::DrawEnd(_)
                | InputEvent::DrawClearAllPaths
                | InputEvent::DrawClearPath { .. }
                | InputEvent::DrawingMode(_)
                | InputEvent::ClickAnimation(_)
        ) {
            if self.moments.is_some() {
                let name = self.participant_name(sender);
                if let Some(moments) = self.moments.as_mut() {
                    moments.draw(sender, &name, &event);
                }
            }
            self.emit(Event::Drawing {
                sender: sender.into(),
                event,
            });
            return Ok(());
        }
        if let InputEvent::MouseMove(point) = &event {
            self.emit(Event::Pointer {
                sender: sender.into(),
                point: *point,
            });
            if !self.grant.allows(sender, self.capture.is_some()) {
                return Ok(());
            }
        }
        if self.privacy_locked.load(Ordering::Acquire)
            || !self.transport_ready
            || !self.grant.allows(sender, self.capture.is_some())
        {
            return Ok(());
        }
        let Some(input) = self.input.as_mut() else {
            return Ok(());
        };
        match event {
            InputEvent::MouseMove(point) => {
                input.move_to(point.x, point.y)?;
            }
            InputEvent::MouseClick(click) => {
                input.move_to(click.x, click.y)?;
                input.button(
                    match click.button {
                        0 => 272,
                        1 => 274,
                        2 => 273,
                        3 => 275,
                        4 => 276,
                        _ => return Ok(()),
                    },
                    click.down,
                )?;
            }
            InputEvent::WheelEvent(wheel) => input.scroll(wheel.x, wheel.y)?,
            InputEvent::Keystroke(key) => input.key(
                &key.key[0],
                key.down,
                InputModifiers {
                    shift: key.shift,
                    ctrl: key.ctrl,
                    alt: key.alt,
                    meta: key.meta,
                },
            )?,
            InputEvent::AddToClipboard(copy) => {
                if self.clipboard_tasks.len() >= 4 {
                    bail!("A clipboard transfer is already in progress");
                }
                let key = if copy.is_copy { "c" } else { "x" };
                input.key(
                    key,
                    true,
                    InputModifiers {
                        ctrl: true,
                        ..Default::default()
                    },
                )?;
                input.key(key, false, InputModifiers::default())?;
                let sender = sender.to_owned();
                let locked = self.privacy_locked.clone();
                self.clipboard_tasks.push(tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    if locked.load(Ordering::Acquire) {
                        bail!("Desktop locked; clipboard copy stopped");
                    }
                    let bytes = tokio::task::spawn_blocking(crate::clipboard::read).await??;
                    Ok(ClipboardResult::ReplyCopy { sender, bytes })
                }));
            }
            InputEvent::PasteFromClipboard(paste) => {
                if let Some(packet) = paste.data {
                    if let Some(bytes) = self.clipboard.push(packet)? {
                        if self.clipboard_tasks.len() >= 4 {
                            bail!("A clipboard transfer is already in progress");
                        }
                        let sender = sender.to_owned();
                        let locked = self.privacy_locked.clone();
                        let epoch = self.clipboard_epoch.clone();
                        let expected = epoch.load(Ordering::Acquire);
                        self.clipboard_tasks.push(tokio::spawn(async move {
                            if locked.load(Ordering::Acquire) {
                                bail!("Desktop locked; clipboard paste stopped");
                            }
                            tokio::task::spawn_blocking(move || {
                                if locked.load(Ordering::Acquire)
                                    || epoch.load(Ordering::Acquire) != expected
                                {
                                    bail!("Clipboard transfer was revoked");
                                }
                                crate::clipboard::write(&bytes)
                            })
                            .await??;
                            Ok(ClipboardResult::ApplyPaste { sender })
                        }));
                    }
                } else {
                    input.key(
                        "v",
                        true,
                        InputModifiers {
                            ctrl: true,
                            ..Default::default()
                        },
                    )?;
                    input.key("v", false, InputModifiers::default())?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn finish_clipboard(&mut self) {
        let mut index = 0;
        while index < self.clipboard_tasks.len() {
            if !self.clipboard_tasks[index].is_finished() {
                index += 1;
                continue;
            }
            let task = self.clipboard_tasks.swap_remove(index);
            let operation = match task.await {
                Ok(Ok(operation)) => operation,
                Ok(Err(error)) => {
                    self.emit(Event::Error(format!("Clipboard: {error:#}")));
                    continue;
                }
                Err(error) if error.is_cancelled() => continue,
                Err(_) => {
                    self.emit(Event::Error("Clipboard transfer stopped".into()));
                    continue;
                }
            };
            if self.privacy_locked.load(Ordering::Acquire) || !self.transport_ready {
                continue;
            }
            match operation {
                ClipboardResult::SendPaste(bytes) if self.peer_control => {
                    if let Some(host) = self.remote_host.clone() {
                        self.queue_clipboard(&bytes, &host, true);
                    }
                }
                ClipboardResult::ReplyCopy { sender, bytes }
                    if self.grant.allows(&sender, self.capture.is_some()) =>
                {
                    self.queue_clipboard(&bytes, &sender, false);
                }
                ClipboardResult::ApplyPaste { sender }
                    if self.grant.allows(&sender, self.capture.is_some()) =>
                {
                    if let Some(input) = self.input.as_mut() {
                        let result = input
                            .key(
                                "v",
                                true,
                                InputModifiers {
                                    ctrl: true,
                                    ..Default::default()
                                },
                            )
                            .and_then(|()| input.key("v", false, InputModifiers::default()));
                        match result {
                            Ok(()) => {
                                self.emit(Event::Status("Pasted to the shared screen".into()))
                            }
                            Err(error) => self.emit(Event::Error(format!("Clipboard: {error:#}"))),
                        }
                    }
                }
                ClipboardResult::WroteLocal if self.peer_control => {
                    self.emit(Event::Status("Copied from the shared screen".into()));
                }
                _ => {}
            }
        }
        // A large paste must yield to local revoke, leave and lock processing.
        for _ in 0..8 {
            let Some(packet) = self.clipboard_out.pop_front() else {
                break;
            };
            let authorized = if packet.to_host {
                self.peer_control
                    && self.remote_host.as_ref().is_some_and(|host| {
                        canonical_identity(host) == canonical_identity(&packet.recipient)
                    })
            } else {
                self.grant.allows(&packet.recipient, self.capture.is_some())
            };
            if !authorized || self.privacy_locked.load(Ordering::Acquire) || !self.transport_ready {
                continue;
            }
            if let Err(error) = self.broadcast(packet.event, vec![packet.recipient]).await {
                self.clipboard_out.clear();
                self.emit(Event::Error(format!("Clipboard: {error:#}")));
                break;
            }
        }
    }

    fn queue_clipboard(&mut self, bytes: &[u8], recipient: &str, to_host: bool) {
        if self.clipboard_out.len() > 256 {
            self.emit(Event::Error(
                "A clipboard transfer is already in progress".into(),
            ));
            return;
        }
        match crate::clipboard::packets(bytes) {
            Ok(packets) => {
                for packet in packets {
                    let event = if to_host {
                        InputEvent::PasteFromClipboard(crate::protocol::ClipboardPaste {
                            data: Some(packet),
                        })
                    } else {
                        InputEvent::ClipboardData(crate::protocol::ClipboardData {
                            requester_sid: recipient.into(),
                            data: Some(packet),
                        })
                    };
                    self.clipboard_out.push_back(ClipboardPacketOut {
                        event,
                        recipient: recipient.into(),
                        to_host,
                    });
                }
            }
            Err(error) => self.emit(Event::Error(format!("Clipboard: {error:#}"))),
        }
    }

    fn flush_frames(&mut self) {
        // Encode on a blocking thread: conversion and scaling take milliseconds at 4K/5K
        // and must not hold up input. While one frame encodes, newer ones replace each
        // other in the slot and the latest goes next.
        if !self.publishing.load(Ordering::Acquire)
            && let Some(frame) = self
                .capture_frames
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
            && self.capture.is_some()
            && self.transport_ready
            && !self.privacy_locked.load(Ordering::Acquire)
            && let Some(session) = &self.session
        {
            let screen = session.screen_publisher();
            let (busy, privacy, epoch) = (
                self.publishing.clone(),
                self.privacy_locked.clone(),
                self.share_epoch.clone(),
            );
            let generation = epoch.load(Ordering::Acquire);
            let events = self.internal_tx.clone();
            #[cfg(test)]
            if let Ok(mut published) = self.shared.published.lock() {
                *published = Some([frame.width, frame.height]);
            }
            // A copy for the capture's keyframes, converted on the encoder's thread.
            let sampler = self
                .moments
                .as_mut()
                .filter(|moments| moments.wants_sample())
                .map(|moments| {
                    moments.sampling();
                    moments.sampler()
                });
            busy.store(true, Ordering::Release);
            tokio::task::spawn_blocking(move || {
                let mut frame = frame;
                if let Some(sampler) = sampler
                    && !privacy.load(Ordering::Acquire)
                    && let Some(copy) = convert::display_bgra(&mut frame.buffer, Some(1440))
                {
                    sampler.sample(copy);
                }
                // Re-check: the desktop may have locked or sharing stopped meanwhile.
                if !privacy.load(Ordering::Acquire)
                    && epoch.load(Ordering::Acquire) == generation
                    && let Err(error) = screen.publish(frame)
                {
                    let _ = events.send(Event::Error(format!("Screen sharing: {error:#}")));
                }
                busy.store(false, Ordering::Release);
                events.wake();
            });
        }
        let mut fresh = false;
        for (index, (source, destination)) in [
            (&self.session_frames.remote, &self.shared.remote),
            (&self.session_frames.local_camera, &self.shared.local_camera),
        ]
        .into_iter()
        .enumerate()
        {
            let frame = source.lock().ok().and_then(|mut slot| slot.take());
            // The remote screen, copied twice a second for the capture's keyframes.
            if index == 0
                && let Some(frame) = &frame
                && let Some(moments) = self.moments.as_mut()
                && moments.wants_sample()
                && !self.privacy_locked.load(Ordering::Acquire)
            {
                moments.sample(frame.clone());
            }
            if let Some(frame) = frame
                && let Ok(mut preview) = destination.lock()
            {
                *preview = Some(frame);
                fresh = true;
            }
        }
        if let (Ok(cameras), Ok(mut displayed)) = (
            self.session_frames.cameras.lock(),
            self.shared.cameras.lock(),
        ) {
            // Remote cameras decode straight into these shared slots.
            fresh |= cameras
                .values()
                .any(|slot| slot.lock().is_ok_and(|frame| frame.is_some()));
            *displayed = cameras.clone();
        }
        if fresh {
            self.events.wake();
        }
    }

    async fn revoke(&mut self) {
        if self.grant.active() {
            self.capture_event(|own| format!("{own} took back control"));
        }
        self.grant.revoke();
        self.clipboard_epoch.fetch_add(1, Ordering::AcqRel);
        self.clipboard_out.clear();
        self.clipboard.clear();
        self.remote_clipboard.clear();
        for task in self.clipboard_tasks.drain(..) {
            task.abort();
        }
        if let Some(mut input) = self.input.take() {
            let _ = input.release_all();
        }
        self.emit(Event::ControlGranted(None));
        if let Some(session) = &self.session {
            let event =
                InputEvent::RemoteControlEnabled(crate::protocol::Enabled { enabled: false });
            if let Ok(payload) = serde_json::to_vec(&event) {
                let _ = session
                    .send_data(payload, Some("remote_control_enabled".into()), vec![])
                    .await;
            }
        }
    }

    async fn stop_share(&mut self) {
        if self.capture.is_some() {
            self.capture_event(|own| format!("{own} stopped sharing"));
        }
        self.share_epoch.fetch_add(1, Ordering::AcqRel);
        if let Some(ramp) = self.screen_ramp.take() {
            ramp.abort();
        }
        if let Some(session) = &self.session {
            session.set_sharing(false);
        }
        self.emit(Event::Sharing {
            active: false,
            target: None,
        });

        self.capture_generation = self.capture_generation.wrapping_add(1);
        if let Some(pending) = &self.pending_capture
            && !pending.canceled.swap(true, Ordering::AcqRel)
        {
            self.emit(Event::Status(
                "Selection canceled. Dismiss the system chooser with Escape or its Cancel button."
                    .into(),
            ));
        }
        let (sender, receiver) = wake::channel(&self.wake);
        self.capture_events = sender;
        self.capture_rx = receiver;
        self.capture_frames = convert::screen_slot();
        self.revoke().await;
        if let Some(moments) = self.moments.as_mut() {
            moments.screen_changed(None);
        }
        if let Some(mut capture) = self.capture.take() {
            capture.stop().await;
        }
        if let Some(session) = &self.session {
            session.set_sharing(false);
        }
        if let Ok(mut frame) = self.capture_frames.lock() {
            *frame = None;
        }
        self.emit(Event::Sharing {
            active: false,
            target: None,
        });
    }

    async fn leave(&mut self) {
        // Leaving removes us from the call, attribute and all; no need to wait on it.
        self.stop_capture(false).await;
        let was_joining = self.pending_join.is_some();
        if let Some(cancel) = self.pending_call_cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(task) = self.pending_call.take() {
            let preceding = self.join_cleanup.take();
            self.join_cleanup = Some(tokio::spawn(async move {
                if let Some(preceding) = preceding {
                    let _ = preceding.await;
                }
                let _ = task.await;
            }));
        }
        self.cancel_join();
        self.cancel_invite().await;
        self.incoming_call = None;
        self.incoming_invite = None;
        if let Ok(mut room) = self.current_room.lock() {
            *room = None;
        }
        self.stop_share().await;
        let session = self.session.take();
        let peer = self.call_peer.take().filter(|_| !was_joining);
        let room = self.room.take();
        self.reset_session_events();
        if session.is_some() || peer.is_some() || room.is_some() {
            let preceding = self.join_cleanup.take();
            let api = self.api.clone();
            let outgoing = self.outgoing.clone();
            self.join_cleanup = Some(tokio::spawn(async move {
                if let Some(preceding) = preceding {
                    let _ = preceding.await;
                }
                if let Some(mut session) = session {
                    session.disconnect().await;
                }
                if let Some(api) = api {
                    JoinRollback {
                        api,
                        room: room
                            .filter(|room| !room.id.starts_with("call:"))
                            .map(|room| room.id),
                        peer,
                        outgoing,
                    }
                    .run()
                    .await;
                }
            }));
        }
        self.transport_ready = false;
        self.remote_host = None;
        self.peer_control = false;
        self.participants.clear();
        for slot in [&self.shared.remote, &self.shared.local_camera] {
            if let Ok(mut frame) = slot.lock() {
                *frame = None;
            }
        }
        if let Ok(mut cameras) = self.shared.cameras.lock() {
            cameras.clear();
        }
        self.emit(Event::Microphone(false));
        self.emit(Event::Camera(false));
        self.emit(Event::PeerControl { enabled: false });
        self.emit(Event::Disconnected("Call ended".into()));
    }

    /// A participant's display name from any of their identities.
    fn participant_name(&self, identity: &str) -> String {
        let wanted = canonical_identity(identity);
        self.participants
            .iter()
            .find(|p| canonical_identity(&p.identity) == wanted)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Someone".into())
    }

    /// Everyone in the call, you first, for the capture's participant list.
    fn capture_names(&self, participants: &[Participant]) -> Vec<String> {
        self.session
            .as_ref()
            .map(Session::own_name)
            .into_iter()
            .chain(participants.iter().map(|p| p.name.clone()))
            .collect()
    }

    /// Note something that happened in the capture, if one is running. `text` gets
    /// your own name.
    fn capture_event(&self, text: impl FnOnce(String) -> String) {
        if let Some(feed) = self.tap.feed() {
            let own = self
                .session
                .as_ref()
                .map(Session::own_name)
                .unwrap_or_else(|| "You".into());
            feed.event(text(own));
        }
    }

    async fn start_capture(&mut self) -> Result<()> {
        use crate::capture::{config, models, recorder, store};
        if self.recording.is_some() {
            return Ok(());
        }
        let session = self.session.as_ref().context("Join a call to capture it")?;
        let chosen = config::load();
        let model = models::find(&chosen.model).or_else(|_| models::find(models::DEFAULT))?;
        // A call between two people has no room: its capture is named for who was there.
        let room = match self.room.as_ref() {
            Some(room) if room.id.starts_with("call:") => String::new(),
            Some(room) if !room.name.is_empty() => room.name.clone(),
            _ => session.room_name(),
        };
        let changed = self.internal_tx.clone();
        let recorder = recorder::Recorder::start(recorder::Options {
            room,
            model,
            backlog_ms: recorder::LIVE_BACKLOG_MS,
            drain_all: false,
            drafts: chosen.drafts,
            gpu: chosen.gpu,
            store: store::default_path()?,
            changed: Arc::new(move || {
                let _ = changed.send(Event::CaptureChanged);
            }),
        })?;
        // Lets other Hopp for Linux clients show that the call is being captured. Best
        // effort: the capture goes ahead either way.
        match tokio::time::timeout(Duration::from_secs(5), session.set_capturing(true)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::logger::line(format!("{error:#}")),
            Err(_) => crate::logger::line("Telling the call about the capture timed out"),
        }
        let feed = recorder.feed();
        feed.participants(self.capture_names(&self.participants));
        if self.capture.is_some() {
            self.capture_event(|own| format!("{own} is sharing their screen"));
        }
        if let Some(host) = self.remote_host.clone() {
            let sharer = self.participant_name(&host);
            feed.event(format!("{sharer} is sharing their screen"));
        }
        match crate::capture::moments::Moments::start(&recorder.id(), feed.clone()) {
            Ok(mut moments) => {
                // Already sharing: Hopp's own windows on it don't count as changes.
                if let Some(capture) = &self.capture {
                    moments.screen_changed(Some(&capture.target));
                }
                self.moments = Some(moments);
            }
            Err(error) => crate::logger::line(format!("Capture: no keyframes: {error:#}")),
        }
        self.tap.set(Some(feed));
        let id = recorder.id();
        crate::capture::hooks::run(
            "capture-started",
            crate::capture::hooks::payload("capture-started", &id),
        );
        self.emit(Event::Capture(Some(recorder.status().into())));
        self.recording = Some(recorder);
        self.emit(Event::Status("Capturing this call to this computer".into()));
        Ok(())
    }

    /// Every call is captured from the moment it connects, unless `auto` is off in
    /// `capture.json`. A capture stopped by hand stays stopped for the rest of the call.
    async fn auto_capture(&mut self) {
        if !crate::capture::config::load().auto {
            return;
        }
        if let Err(error) = self.start_capture().await {
            crate::logger::line(format!("Capture: didn't start automatically: {error:#}"));
            self.emit(Event::Error(format!(
                "Couldn't start capturing this call: {error:#}"
            )));
        }
    }

    async fn stop_capture(&mut self, tell_others: bool) {
        let Some(recorder) = self.recording.take() else {
            return;
        };
        crate::logger::line(format!(
            "Capture {}: stopping{}",
            recorder.id(),
            if tell_others {
                ""
            } else {
                " (leaving the call)"
            }
        ));
        self.tap.set(None);
        if tell_others && let Some(session) = &self.session {
            match tokio::time::timeout(Duration::from_secs(3), session.set_capturing(false)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::logger::line(format!("{error:#}")),
                Err(_) => crate::logger::line("Telling the call the capture stopped timed out"),
            }
        }
        self.emit(Event::Capture(None));
        let id = recorder.id();
        let moments = self.moments.take();
        self.finishing.retain(|task| !task.is_finished());
        self.finishing.push(tokio::task::spawn_blocking(move || {
            // Its last frames go into the transcript before the recorder finishes.
            if let Some(moments) = moments {
                moments.finish();
            }
            if let Err(error) = recorder.finish() {
                crate::logger::line(format!("Capture {id} did not finish cleanly: {error:#}"));
            }
            crate::capture::hooks::run(
                "capture-complete",
                crate::capture::hooks::payload("capture-complete", &id),
            );
            if let Err(error) = crate::capture::prune() {
                crate::logger::line(format!("Deleting old captures: {error:#}"));
            }
        }));
    }

    async fn send_socket(&self, message: serde_json::Value) -> Result<()> {
        if !self.socket_connected {
            bail!("Hopp is reconnecting; try again when connected");
        }
        self.outgoing
            .as_ref()
            .context("Hopp connection unavailable")?
            .try_send(message)
            .context("Hopp connection is busy or ended")
    }

    /// Report a call or invitation from `caller` that wasn't answered. `initiated_at` is
    /// when they tried (seconds or milliseconds, as the server sends it), else now.
    fn missed(&self, caller: &str, invite: bool, initiated_at: Option<i64>) {
        let at_ms = match initiated_at {
            Some(at) if at > 100_000_000_000 => at,
            Some(at) => at * 1000,
            None => crate::capture::store::now_ms(),
        };
        self.emit(Event::Missed {
            caller_id: caller.to_owned(),
            name: self.peer_name(caller),
            invite,
            at_ms,
        });
    }

    fn peer_name(&self, id: &str) -> String {
        self.teammates
            .iter()
            .find(|t| t.id == id)
            .map(|t| {
                let name = format!("{} {}", t.first_name, t.last_name)
                    .trim()
                    .to_owned();
                if name.is_empty() {
                    t.email.clone()
                } else {
                    name
                }
            })
            .unwrap_or_else(|| "Your teammate".into())
    }

    async fn presence_event(&mut self, event: PresenceEvent) -> Result<()> {
        match event {
            PresenceEvent::Connected => {
                self.socket_connected = true;
                self.refresh().await?;
            }
            PresenceEvent::Disconnected => {
                self.socket_connected = false;
                self.emit(Event::Status("Hopp is reconnecting…".into()));
            }
            PresenceEvent::Changed => self.refresh().await?,
            PresenceEvent::Message { kind, payload } => match kind.as_str() {
                "teammate_online" => self.refresh().await?,
                "incoming_call" => {
                    let incoming: presence::IncomingCall = serde_json::from_value(payload)?;
                    if !presence::recent(incoming.initiated_at) {
                        // Rang while Hopp was offline: too late to answer, not to call back.
                        self.missed(&incoming.caller_id, false, incoming.initiated_at);
                        return Ok(());
                    }
                    if self.session.is_some()
                        || self.pending_join.is_some()
                        || self.call_peer.is_some()
                        || self.incoming_call.is_some()
                    {
                        self.send_socket(presence::call_reject(&incoming.caller_id, "in-call"))
                            .await?;
                        // Not the person you're already with, who is only reconnecting.
                        if self.call_peer.as_deref() != Some(incoming.caller_id.as_str())
                            && self.incoming_call.as_deref() != Some(incoming.caller_id.as_str())
                        {
                            self.missed(&incoming.caller_id, false, None);
                        }
                    } else {
                        self.incoming_call = Some(incoming.caller_id.clone());
                        self.emit(Event::IncomingCall {
                            caller_id: incoming.caller_id.clone(),
                            name: self.peer_name(&incoming.caller_id),
                        });
                    }
                }
                "call_tokens" => {
                    let peer = payload
                        .get("participant")
                        .and_then(|v| v.as_str())
                        .context("Call tokens have no participant")?
                        .to_owned();
                    if self.call_peer.as_deref() == Some(peer.as_str())
                        && self.session.is_none()
                        && self.pending_join.is_none()
                    {
                        let name = self.peer_name(&peer);
                        self.emit(Event::Status(format!("Connecting to {name}…")));
                        self.begin_join(JoinTarget::Direct {
                            payload,
                            peer,
                            name,
                        })?;
                    }
                }
                "call_end" => {
                    let peer = payload.get("participant_id").and_then(|v| v.as_str());
                    if let Some(caller) =
                        peer.filter(|peer| Some(*peer) == self.incoming_call.as_deref())
                    {
                        // They gave up before you answered.
                        let caller = caller.to_owned();
                        self.incoming_call = None;
                        self.missed(&caller, false, None);
                    } else if peer.is_some() && peer == self.call_peer.as_deref() {
                        // The peer already ended its half; avoid sending an echo.
                        self.call_peer = None;
                        self.leave().await;
                    }
                }
                "call_reject" => {
                    // Forwarded rejection carries our caller ID; backend-generated
                    // busy/trial rejection carries the callee ID instead.
                    let id = payload.get("caller_id").and_then(|v| v.as_str());
                    if self.call_peer.is_some()
                        && self.session.is_none()
                        && self.pending_join.is_none()
                        && (id == self.user_id.as_deref() || id == self.call_peer.as_deref())
                    {
                        self.call_peer = None;
                        self.leave().await;
                        self.emit(Event::Disconnected("Call declined".into()));
                    }
                }
                "callee_offline" => {
                    let peer = payload.get("callee_id").and_then(|v| v.as_str());
                    if self
                        .outgoing_invite
                        .as_ref()
                        .is_some_and(|invite| Some(invite.peer.as_str()) == peer)
                    {
                        self.outgoing_invite = None;
                        self.emit(Event::InviteResolved {
                            message: "Teammate is offline".into(),
                        });
                    } else if peer.is_some()
                        && peer == self.call_peer.as_deref()
                        && self.session.is_none()
                    {
                        self.call_peer = None;
                        self.leave().await;
                        self.emit(Event::Disconnected("Teammate is offline".into()));
                    }
                }
                "incoming_invite" => {
                    let invite: presence::IncomingInvite = serde_json::from_value(payload)?;
                    if invite.invite_id.is_empty() || invite.inviter_id.is_empty() {
                        return Ok(());
                    }
                    if !presence::recent(invite.initiated_at) {
                        self.missed(&invite.inviter_id, true, invite.initiated_at);
                        return Ok(());
                    }
                    if let Some((inviter, id)) = self.incoming_invite.take() {
                        let _ = self
                            .send_socket(presence::invite_reject(&inviter, &id, "rejected"))
                            .await;
                        // A newer invitation pushed theirs aside.
                        if inviter != invite.inviter_id {
                            self.missed(&inviter, true, None);
                        }
                    }
                    self.incoming_invite =
                        Some((invite.inviter_id.clone(), invite.invite_id.clone()));
                    self.emit(Event::IncomingInvite {
                        name: self.peer_name(&invite.inviter_id),
                        inviter_id: invite.inviter_id,
                        invite_id: invite.invite_id,
                    });
                }
                "invite_accept" | "invite_reject" => {
                    let id = payload.get("invite_id").and_then(|v| v.as_str());
                    let peer = payload.get("invitee_id").and_then(|v| v.as_str());
                    if self.outgoing_invite.as_ref().is_some_and(|invite| {
                        Some(invite.invite_id.as_str()) == id && Some(invite.peer.as_str()) == peer
                    }) {
                        self.outgoing_invite = None;
                        self.emit(Event::InviteResolved {
                            message: if kind == "invite_accept" {
                                "Teammate accepted"
                            } else {
                                "Invitation declined"
                            }
                            .into(),
                        });
                    }
                }
                "invite_cancel" => {
                    let inviter = payload.get("inviter_id").and_then(|v| v.as_str());
                    let id = payload.get("invite_id").and_then(|v| v.as_str());
                    if self
                        .incoming_invite
                        .as_ref()
                        .is_some_and(|(pending_inviter, pending_id)| {
                            Some(pending_inviter.as_str()) == inviter
                                && Some(pending_id.as_str()) == id
                        })
                    {
                        // They gave up before you answered.
                        if let Some((inviter, _)) = self.incoming_invite.take() {
                            self.missed(&inviter, true, None);
                        }
                    }
                }
                "error" => {
                    if self.session.is_none()
                        && self.pending_join.is_none()
                        && self.call_peer.is_some()
                    {
                        self.leave().await;
                    }
                    if self.outgoing_invite.take().is_some() {
                        self.emit(Event::InviteResolved {
                            message: "Invitation could not be sent".into(),
                        });
                    }
                    self.emit(Event::Error(
                        "Hopp could not complete the call request".into(),
                    ));
                }
                _ => {}
            },
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
