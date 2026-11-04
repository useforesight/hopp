//! Window-independent client state. Every Hopp window reads this one model, so
//! closing a window never stops the call, and frames are decoded exactly once.
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{Context, EventEmitter, Modifiers, MouseButton, RenderImage};

use super::tones::{Tone, Tones};

use crate::{
    api::{RoomInfo, RoomsPresence, Teammate, UserInfo},
    engine::{Command, Engine, Event, Participant},
    frames::FrameSlot,
    protocol::{
        DrawSettings, DrawingMode, InputEvent, Keystroke as RemoteKey, MouseClick, PathPoint,
        Point, StampMessage, Wheel, canonical_identity,
    },
    settings::Settings,
};

/// How often your stamp looks for words you've said.
const STAMP_LISTEN_EVERY: Duration = Duration::from_millis(300);
/// A stamp you've spoken into finishes itself once you've been quiet this long.
const STAMP_SAID_DONE: Duration = Duration::from_secs(3);

/// The words going into your stamp.
#[derive(Default)]
struct StampWords {
    typed: String,
    /// While the call is captured, what you say after placing it.
    heard: Option<Heard>,
}

/// Your words, from the capture's transcript as it's written.
struct Heard {
    store: crate::capture::store::Store,
    call: String,
    /// The end of your speaker id: `:` and your user id.
    me: String,
    /// Lines up to this id were said before the stamp.
    after_id: i64,
    /// Changes up to this revision have been read.
    rev: i64,
    /// Your lines since, by id: (final, text).
    lines: BTreeMap<i64, (bool, String)>,
    changed: Instant,
}

/// Cut `text` to at most `max` bytes, on a character boundary.
fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// A decoded frame ready for painting. `generation` changes with every frame
/// so windows know when to release the previous texture.
#[derive(Clone)]
pub struct Picture {
    pub image: Arc<RenderImage>,
    pub source: [u32; 2],
}

#[derive(Clone)]
pub struct Stroke {
    /// Empty for strokes drawn locally.
    pub sender: String,
    pub id: u64,
    pub points: Vec<Point>,
    /// When the stroke was finished.
    pub ended: Option<Instant>,
    /// When it started fading out: its owner cleared or undid it, or your ink was left
    /// alone for [`super::marks::INK_HOLD`].
    pub fading: Option<Instant>,
}

impl Stroke {
    /// Faded out completely by `now`.
    pub fn faded(&self, now: Instant) -> bool {
        self.fading
            .is_some_and(|start| now >= start + super::marks::INK_FADE)
    }
}

/// How often a straight line being dragged out is sent to others. There is no message
/// for changing a stroke, so each update clears it and starts it again; the exact end is
/// always sent when the stroke ends. The Mac app's rate.
const STRAIGHT_EVERY: Duration = Duration::from_millis(33);

/// A ripple where someone clicked or dropped a pin.
#[derive(Clone, Debug)]
pub struct Ripple {
    pub sender: String,
    pub point: Point,
    pub at: Instant,
}

#[derive(Clone, Default)]
pub struct Devices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub cameras: Vec<(String, String)>,
}

/// Something worth changing the window arrangement for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    CallStarted,
    CallEnded,
    RemoteShareStarted,
    RemoteShareEnded,
    RingingChanged,
    Activate,
    SignedIn,
    SignedOut,
    Request(crate::protocol::UiRequest),
    SharingChanged,
    /// A card was posted, answered or went: the card stack may change size.
    CardsChanged,
    /// Drawing started or stopped: the draw pad over your own share comes or goes.
    DrawingChanged,
    /// A new shared-screen frame or sharer pointer position; only the viewer redraws.
    Screen,
    /// Something to show over your shared screen appeared, or the last of it went: the
    /// marks overlay opens or closes.
    MarksChanged,
    /// New camera frames; only windows showing faces redraw.
    Cameras,
}

#[derive(Clone)]
pub enum Ringing {
    Call {
        caller_id: String,
        name: String,
    },
    Invite {
        inviter_id: String,
        invite_id: String,
        name: String,
    },
}

impl Ringing {
    pub fn name(&self) -> &str {
        match self {
            Self::Call { name, .. } | Self::Invite { name, .. } => name,
        }
    }
}

pub struct Session {
    engine: Engine,
    pub settings: Settings,
    pub user: Option<UserInfo>,
    pub rooms: Vec<RoomInfo>,
    pub room_presence: RoomsPresence,
    pub room: Option<RoomInfo>,
    pub participants: Vec<Participant>,
    pub teammates: Vec<Teammate>,
    pub ringing: Option<Ringing>,
    /// Outgoing call or invitation: (peer id, name, invite id).
    pub dialing: Option<(String, String, Option<String>)>,
    pub connected: bool,
    pub joined_at: Option<Instant>,
    pub desktop_locked: bool,
    pub status: String,
    pub errors: Vec<String>,
    pub sharing: bool,
    pub selecting_source: bool,
    pub selection_canceled: bool,
    pub capture_name: Option<String>,
    /// The monitor you're sharing, for the overlay that shows others' marks on it.
    pub share_target: Option<crate::screencast::CaptureTarget>,
    /// The capture you are running, if any.
    pub capture: Option<crate::engine::CaptureStatus>,
    /// Where transcription last ran ("CPU" or the GPU's name), for Settings.
    pub capture_device: Option<String>,
    pub microphone: bool,
    pub camera: bool,
    pub grant: Option<String>,
    pub peer_control: bool,
    pub remote_identity: Option<String>,
    pub devices: Option<Devices>,
    pub drawing: bool,
    /// The stamp tool is on: a click drops a stamp.
    pub stamping: bool,
    /// Your stamp still taking words.
    pub active_stamp: Option<u64>,
    /// What goes into it: typed, and heard while the call is captured.
    stamp_words: StampWords,
    next_stamp: u64,
    pub paths: Vec<Stroke>,
    pub pointers: HashMap<String, super::marks::Pointer>,
    /// Who is drawing (canonical identities): their cursor is a pencil.
    pens: HashSet<String>,
    /// Stamps placed in this call so far, to number the next.
    stamp_count: u32,
    pub ripples: Vec<Ripple>,
    pub stamps: Vec<super::marks::Stamp>,
    pub screen: Option<Picture>,
    /// Small, blurred, translucent copy of `screen` for the viewer's empty bands.
    pub ambient: Option<Picture>,
    ambient_at: Instant,
    pub local_camera: Option<Picture>,
    pub cameras: HashMap<String, Picture>,
    drawing_path: Option<usize>,
    /// When the straight line you're dragging out (Shift held) was last sent to others.
    straight_sent: Option<Instant>,
    /// The latest end of that line, held back by [`STRAIGHT_EVERY`]; it's sent before
    /// freehand ink carries on from it.
    straight_held: Option<Point>,
    /// Your last change to your ink: a stroke, a point, an undo. All of it fades
    /// [`super::marks::INK_HOLD`] after this, unless you keep it.
    ink_changed: Instant,
    /// Keep your ink until you right-click or clear it, instead of letting it fade (the
    /// Mac app's "Persist until right click").
    pub keep_ink: bool,
    incoming_paths: HashMap<String, usize>,
    /// What `MarksChanged` last said: whether anything shows over your shared screen.
    marks_announced: bool,
    next_path: u64,
    forwarded_keys: HashMap<String, RemoteKey>,
    forwarded_buttons: HashMap<MouseButton, Point>,
    last_move: Instant,
    tones: Tones,
    /// Notices and questions from agents and scripts, newest last (see `cards`).
    pub cards: Vec<Card>,
}

/// A card and where it has got to.
#[derive(Clone, Debug)]
pub struct Card {
    pub request: crate::cards::CardRequest,
    pub state: crate::cards::CardState,
    pub posted: Instant,
}

/// Cards remembered after they are answered, so an asker that looks a little late still
/// finds the answer in the snapshot.
const CARDS_KEPT: usize = 50;

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    /// Notified whenever the engine has events or frames for [`Session::tick`].
    pub fn wake(&self) -> std::sync::Arc<tokio::sync::Notify> {
        self.engine.wake.clone()
    }

    pub fn new(engine: Engine, settings: Settings) -> Self {
        Self {
            engine,
            settings,
            user: None,
            rooms: Vec::new(),
            room_presence: Default::default(),
            room: None,
            participants: Vec::new(),
            teammates: Vec::new(),
            ringing: None,
            dialing: None,
            connected: false,
            joined_at: None,
            desktop_locked: false,
            status: "Ready when you are".into(),
            errors: Vec::new(),
            sharing: false,
            selecting_source: false,
            selection_canceled: false,
            capture_name: None,
            share_target: None,
            capture: None,
            capture_device: None,
            microphone: false,
            camera: false,
            grant: None,
            peer_control: false,
            remote_identity: None,
            devices: None,
            drawing: false,
            paths: Vec::new(),
            pointers: HashMap::new(),
            ripples: Vec::new(),
            stamps: Vec::new(),
            stamping: false,
            active_stamp: None,
            stamp_words: StampWords::default(),
            next_stamp: 0,
            screen: None,
            ambient: None,
            ambient_at: Instant::now(),
            local_camera: None,
            cameras: HashMap::new(),
            drawing_path: None,
            straight_sent: None,
            straight_held: None,
            ink_changed: Instant::now(),
            // Ink stays until you clear it, by default here and on the Mac.
            keep_ink: true,
            incoming_paths: HashMap::new(),
            marks_announced: false,
            next_path: 0,
            forwarded_keys: HashMap::new(),
            forwarded_buttons: HashMap::new(),
            last_move: Instant::now(),
            pens: HashSet::new(),
            stamp_count: 0,
            tones: Tones::default(),
            cards: Vec::new(),
        }
    }

    /// Ring while a call is coming in or going out, and stop as soon as neither is.
    fn sync_ring(&mut self) {
        let tone = if self.ringing.is_some() {
            Some(Tone::Incoming)
        } else if self.dialing.is_some() && self.room.is_none() {
            Some(Tone::Outgoing)
        } else {
            None
        };
        self.tones.ring(tone);
    }

    pub fn in_call(&self) -> bool {
        self.room.is_some()
    }

    pub fn display_name(&self) -> String {
        self.user
            .as_ref()
            .map(|user| full_name(&user.first_name, &user.last_name))
            .unwrap_or_else(|| "You".into())
    }

    /// Display name for a LiveKit identity (any `:audio`/`:camera` suffix).
    pub fn name_for(&self, identity: &str) -> String {
        let wanted = canonical_identity(identity);
        self.participants
            .iter()
            .find(|participant| canonical_identity(&participant.identity) == wanted)
            .map(|participant| sanitize(&participant.name))
            .unwrap_or_else(|| "Teammate".into())
    }

    /// Everything people have put on a shared screen, to paint. Over your own screen
    /// (`over_own`), whoever controls it gets only their name beside your cursor, which
    /// they move; over someone else's, theirs is left out, being in the video already.
    pub fn scene(&self, over_own: bool) -> super::marks::Scene {
        let skip = if over_own {
            None
        } else {
            self.remote_identity.as_deref().map(canonical_identity)
        };
        let controller = if over_own {
            self.grant.as_deref().map(canonical_identity)
        } else {
            None
        };
        let pointers: Vec<super::marks::ScenePointer> = self
            .pointers
            .iter()
            .filter(|(sender, _)| skip.is_none_or(|skip| canonical_identity(sender) != skip))
            .map(|(sender, pointer)| {
                let who = canonical_identity(sender);
                let drawing = self.pens.contains(who);
                // While they draw, the cursor sits on the tip of their ink.
                let pen_tip = self
                    .incoming_paths
                    .get(sender)
                    .and_then(|index| self.paths.get(*index))
                    .filter(|path| drawing && path.ended.is_none() && path.fading.is_none())
                    .and_then(|path| path.points.last().copied());
                let tool = if controller == Some(who) {
                    super::marks::Tool::Name
                } else if drawing {
                    super::marks::Tool::Pencil
                } else {
                    super::marks::Tool::Arrow
                };
                super::marks::ScenePointer {
                    sender: sender.clone(),
                    pointer: pointer.clone(),
                    name: self.name_for(sender),
                    tool,
                    pen_tip,
                }
            })
            .collect();
        let colours = super::marks::colours_for(
            self.user.as_ref().map(|user| user.id.clone()),
            self.paths
                .iter()
                .map(|path| path.sender.clone())
                .chain(pointers.iter().map(|shown| shown.sender.clone()))
                .chain(self.ripples.iter().map(|ripple| ripple.sender.clone()))
                .chain(self.stamps.iter().map(|stamp| stamp.owner.clone())),
        );
        super::marks::Scene {
            paths: self.paths.clone(),
            ripples: self.ripples.clone(),
            stamps: self.stamps.clone(),
            pointers,
            colours: std::rc::Rc::new(colours),
        }
    }

    /// The remote participant's camera, keyed the way the media worker stores it.
    pub fn camera_for(&self, identity: &str) -> Option<&Picture> {
        let wanted = canonical_identity(identity);
        self.cameras
            .iter()
            .find(|(key, _)| canonical_identity(key) == wanted)
            .map(|(_, picture)| picture)
    }

    pub fn is_granted(&self, identity: &str) -> bool {
        self.grant
            .as_deref()
            .is_some_and(|grant| canonical_identity(grant) == canonical_identity(identity))
    }

    pub fn send(&mut self, command: Command) {
        if let Err(error) = self.engine.commands.try_send(command) {
            self.push_error(
                if matches!(error, tokio::sync::mpsc::error::TrySendError::Closed(_)) {
                    "The connection worker stopped. Restart Hopp to reconnect."
                } else {
                    "The connection is busy. Try that action again."
                },
            );
        }
    }

    pub fn push_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        // A repeating failure shows (and logs) once rather than filling the list.
        if self.errors.last() == Some(&error) {
            return;
        }
        log(&format!("error: {error}"));
        self.errors.push(error);
        if self.errors.len() > 5 {
            self.errors.remove(0);
        }
    }

    // ---- actions shared by every window -------------------------------------------------

    pub fn toggle_microphone(&mut self, cx: &mut Context<Self>) {
        if self.connected {
            self.send(Command::Microphone(!self.microphone));
            cx.notify();
        }
    }

    pub fn toggle_camera(&mut self, cx: &mut Context<Self>) {
        if self.connected {
            self.send(Command::Camera(!self.camera));
            cx.notify();
        }
    }

    /// Start capturing the call, or stop.
    pub fn toggle_capture(&mut self, cx: &mut Context<Self>) {
        if self.capture.is_some() {
            self.send(Command::StopCapture);
        } else {
            self.start_capture(cx);
        }
        cx.notify();
    }

    pub fn start_capture(&mut self, cx: &mut Context<Self>) {
        if self.connected && self.capture.is_none() {
            self.send(Command::StartCapture);
            cx.notify();
        }
    }

    /// Others capturing this call.
    pub fn capturing_others(&self) -> Vec<String> {
        self.participants
            .iter()
            .filter(|participant| participant.capturing)
            .map(|participant| sanitize(&participant.name))
            .collect()
    }

    /// Start sharing, stop sharing, or cancel a pending chooser.
    pub fn toggle_share(&mut self, window_only: bool, cx: &mut Context<Self>) {
        if self.selecting_source {
            self.cancel_selection();
        } else if self.sharing {
            self.send(Command::StopShare);
        } else if self.connected {
            self.send(Command::Share { window_only });
        }
        cx.notify();
    }

    pub fn cancel_selection(&mut self) {
        if !self.selecting_source || self.selection_canceled {
            return;
        }
        match self.engine.commands.try_send(Command::StopShare) {
            Ok(()) => self.selection_canceled = true,
            Err(_) => self.push_error("The connection is busy. Try canceling selection again."),
        }
    }

    pub fn leave(&mut self, cx: &mut Context<Self>) {
        if self.room.is_some() || self.dialing.is_some() {
            self.release_input();
            if let Some((_, _, Some(_))) = self.dialing {
                self.send(Command::CancelInvite);
            } else {
                self.send(Command::Leave);
            }
            self.dialing = None;
            self.sync_ring();
            cx.notify();
        }
    }

    pub fn revoke(&mut self, cx: &mut Context<Self>) {
        self.release_input();
        self.peer_control = false;
        self.grant = None;
        self.drawing = false;
        self.send(Command::RevokeControl);
        cx.notify();
    }

    pub fn grant_control(&mut self, identity: String, cx: &mut Context<Self>) {
        if self.connected && self.sharing {
            self.send(Command::GrantControl(identity));
            cx.notify();
        }
    }

    pub fn call(&mut self, teammate: &Teammate, cx: &mut Context<Self>) {
        let name = full_name(&teammate.first_name, &teammate.last_name);
        let user_id = teammate.id.clone();
        self.send(if self.room.is_some() {
            Command::InvitePeer { user_id, name }
        } else {
            Command::CallPeer { user_id, name }
        });
        cx.notify();
    }

    pub fn join_room(&mut self, room: RoomInfo, cx: &mut Context<Self>) {
        if !self.connected
            || self
                .room
                .as_ref()
                .is_none_or(|current| current.id != room.id)
        {
            self.release_input();
            self.send(Command::JoinRoom(room));
            self.status = "Joining room…".into();
            cx.notify();
        }
    }

    pub fn accept_ringing(&mut self, cx: &mut Context<Self>) {
        match self.ringing.take() {
            Some(Ringing::Call { caller_id, .. }) => self.send(Command::AcceptCall(caller_id)),
            Some(Ringing::Invite {
                inviter_id,
                invite_id,
                ..
            }) => self.send(Command::AcceptInvite {
                inviter_id,
                invite_id,
            }),
            None => return,
        }
        self.sync_ring();
        cx.emit(SessionEvent::RingingChanged);
        cx.notify();
    }

    pub fn decline_ringing(&mut self, cx: &mut Context<Self>) {
        match self.ringing.take() {
            Some(Ringing::Call { caller_id, .. }) => self.send(Command::RejectCall(caller_id)),
            Some(Ringing::Invite {
                inviter_id,
                invite_id,
                ..
            }) => self.send(Command::RejectInvite {
                inviter_id,
                invite_id,
            }),
            None => return,
        }
        self.sync_ring();
        cx.emit(SessionEvent::RingingChanged);
        cx.notify();
    }

    pub fn sign_in(&mut self, server: String, token: String, cx: &mut Context<Self>) {
        self.errors.clear();
        self.status = "Signing in…".into();
        self.send(Command::SignIn { server, token });
        cx.notify();
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.send(Command::SignOut);
        cx.notify();
    }

    pub fn refresh(&mut self) {
        self.send(Command::Refresh);
    }

    pub fn create_room(&mut self, name: String, cx: &mut Context<Self>) {
        if !name.is_empty() && self.user.is_some() {
            self.send(Command::CreateRoom(name));
            cx.notify();
        }
    }

    pub fn select_audio(&mut self, input: Option<String>, output: Option<String>) {
        if input != self.settings.input_device || output != self.settings.output_device {
            self.settings.input_device = input.clone();
            self.settings.output_device = output.clone();
            self.send(Command::SelectAudio { input, output });
        }
    }

    pub fn select_camera(&mut self, camera: Option<String>) {
        if camera != self.settings.camera_device {
            self.settings.camera_device = camera.clone();
            self.send(Command::SelectCamera(camera));
        }
    }

    pub fn paste_clipboard(&mut self) {
        if self.connected && self.peer_control && self.remote_identity.is_some() {
            self.send(Command::ShareClipboard);
        }
    }

    pub fn copy_clipboard(&mut self, keep: bool) {
        if self.connected && self.peer_control && self.remote_identity.is_some() {
            self.send(Command::CopyClipboard(keep));
        }
    }

    /// The engine says "Sharing your screen" when a share starts but nothing when it
    /// stops; don't leave that standing (it's what `hopp status` prints).
    fn stopped_sharing_status(&mut self) {
        if self.status == "Sharing your screen" {
            self.status = "Stopped sharing".into();
        }
    }

    /// Whether anything would show over your shared screen: a cursor (they fade out a few
    /// seconds after their owner stops moving), ink, a click ring or a stamp, or you're
    /// drawing on it.
    pub fn has_marks(&self) -> bool {
        self.drawing
            || !self.pointers.is_empty()
            || !self.paths.is_empty()
            || !self.ripples.is_empty()
            || !self.stamps.is_empty()
    }

    pub fn toggle_drawing(&mut self, cx: &mut Context<Self>) {
        if !self.connected || (self.remote_identity.is_none() && !self.sharing) {
            return;
        }
        self.release_input();
        self.drawing = !self.drawing;
        if self.drawing {
            self.finish_stamp(cx);
            self.stamping = false;
        }
        self.drawing_path = None;
        self.straight_sent = None;
        self.straight_held = None;
        self.marking_changed(cx);
        cx.emit(SessionEvent::DrawingChanged);
        cx.notify();
    }

    /// You picked up or put down the pen or the stamp tool: say so to everyone. With both
    /// down your ink goes, as it does on the Mac when someone stops drawing.
    fn marking_changed(&mut self, cx: &mut Context<Self>) {
        if !self.drawing && !self.stamping {
            self.clear_own_drawings(cx);
        }
        self.send_drawing_mode();
    }

    /// Switch between ink that fades and ink kept until you right-click or clear it.
    pub fn toggle_keep_ink(&mut self, cx: &mut Context<Self>) {
        self.keep_ink = !self.keep_ink;
        // Ink you stop keeping gets the usual time before it fades.
        self.ink_changed = Instant::now();
        if self.drawing || self.stamping {
            self.send_drawing_mode();
        }
        cx.notify();
    }

    fn send_drawing_mode(&mut self) {
        self.send(Command::SendInput(InputEvent::DrawingMode(
            // Stamping is drawing to everyone else, so your ink stays up.
            if self.drawing || self.stamping {
                DrawingMode::Draw(DrawSettings {
                    permanent: self.keep_ink,
                })
            } else {
                DrawingMode::Disabled
            },
        )));
    }

    /// Take back your last finished stroke, here and for everyone. Not while drawing one.
    pub fn undo_stroke(&mut self, cx: &mut Context<Self>) {
        if self.drawing_path.is_some() {
            return;
        }
        let now = Instant::now();
        let Some(path) =
            self.paths.iter_mut().rev().find(|path| {
                path.sender.is_empty() && path.ended.is_some() && path.fading.is_none()
            })
        else {
            return;
        };
        path.fading = Some(now);
        let path_id = path.id;
        self.ink_changed = now;
        if self.connected {
            self.send(Command::SendInput(InputEvent::DrawClearPath { path_id }));
        }
        cx.emit(SessionEvent::Screen);
        cx.notify();
    }

    /// Drawing keys, from the viewer or the draw pad: Ctrl+Z (or Super+Z) undoes your
    /// last stroke. Returns whether the key was used.
    pub fn drawing_key(&mut self, key: &gpui_kit::Keystroke, cx: &mut Context<Self>) -> bool {
        let modifiers = key.modifiers;
        if self.drawing
            && key.key == "z"
            && (modifiers.control || modifiers.platform)
            && !modifiers.shift
            && !modifiers.alt
        {
            self.undo_stroke(cx);
            return true;
        }
        false
    }

    /// Send the straight line from `start` to `end` as stroke `path_id`: cleared, then
    /// started again, since there's no message for changing a stroke.
    fn send_line(&mut self, path_id: u64, start: Point, end: Option<Point>) {
        self.send(Command::SendInput(InputEvent::DrawClearPath { path_id }));
        self.send(Command::SendInput(InputEvent::DrawStart(PathPoint {
            point: start,
            path_id,
        })));
        if let Some(end) = end {
            self.send(Command::SendInput(InputEvent::DrawAddPoint(end)));
        }
    }

    pub fn toggle_stamping(&mut self, cx: &mut Context<Self>) {
        if !self.connected || (self.remote_identity.is_none() && !self.sharing) {
            return;
        }
        self.finish_stamp(cx);
        self.release_input();
        self.stamping = !self.stamping;
        if self.stamping && self.drawing {
            self.toggle_drawing(cx);
        } else {
            self.marking_changed(cx);
        }
        // The draw pad opens for stamps on your own screen too.
        cx.emit(SessionEvent::DrawingChanged);
        cx.notify();
    }

    /// Drop a stamp of yours at `point`; it takes words until you finish it.
    fn place_stamp(&mut self, point: Point, cx: &mut Context<Self>) {
        self.finish_stamp(cx);
        self.next_stamp = self.next_stamp.wrapping_add(1);
        let id = self.next_stamp;
        let name = self
            .user
            .as_ref()
            .map(|user| user.first_name.clone())
            .unwrap_or_else(|| "You".into());
        self.stamp_count += 1;
        self.stamps.push(super::marks::Stamp {
            id,
            number: self.stamp_count,
            owner: String::new(),
            name,
            point,
            text: String::new(),
            draft: String::new(),
            listening: true,
            voice: false,
            placed: Instant::now(),
            removed: None,
        });
        self.active_stamp = Some(id);
        self.stamp_words = StampWords {
            typed: String::new(),
            heard: self.listen_for_stamp(),
        };
        let voice = self.stamp_words.heard.is_some();
        if let Some(stamp) = self.own_stamp(id) {
            stamp.voice = voice;
        }
        if voice {
            // Your words reach the transcript a moment after you say them; look often.
            cx.spawn(async move |session, cx| {
                loop {
                    cx.background_executor().timer(STAMP_LISTEN_EVERY).await;
                    let listening = session
                        .update(cx, |session, cx| session.hear_stamp(id, cx))
                        .unwrap_or(false);
                    if !listening {
                        break;
                    }
                }
            })
            .detach();
        }
        self.send(Command::SendStamp(StampMessage::StampPlace {
            id,
            x: point.x,
            y: point.y,
        }));
        cx.notify();
    }

    fn own_stamp(&mut self, id: u64) -> Option<&mut super::marks::Stamp> {
        self.stamps
            .iter_mut()
            .find(|stamp| stamp.owner.is_empty() && stamp.id == id)
    }

    fn send_stamp_text(&mut self, done: bool) {
        let Some(id) = self.active_stamp else {
            return;
        };
        let Some(stamp) = self.own_stamp(id) else {
            return;
        };
        let (text, draft) = (stamp.text.clone(), stamp.draft.clone());
        self.send(Command::SendStamp(StampMessage::StampText {
            id,
            text,
            draft,
            done,
        }));
    }

    /// Words typed into your stamp. Backspace is `None`.
    pub fn stamp_typed(&mut self, typed: Option<&str>, cx: &mut Context<Self>) {
        if self.active_stamp.is_none() {
            return;
        }
        let words = &mut self.stamp_words.typed;
        match typed {
            Some(text) if words.len() + text.len() <= crate::protocol::STAMP_TEXT_MAX => {
                words.push_str(text)
            }
            Some(_) => return,
            None => {
                words.pop();
            }
        }
        self.show_stamp_words();
        self.send_stamp_text(false);
        cx.notify();
    }

    /// Keys while your stamp takes words: Enter finishes it, Escape takes it away,
    /// anything else is typed into it. Returns whether the key was used.
    pub fn stamp_key(&mut self, key: &gpui_kit::Keystroke, cx: &mut Context<Self>) -> bool {
        if self.active_stamp.is_none() || key.modifiers.control || key.modifiers.alt {
            return false;
        }
        match key.key.as_str() {
            "enter" => self.finish_stamp(cx),
            "escape" => self.cancel_stamp(cx),
            "backspace" => self.stamp_typed(None, cx),
            _ => {
                if let Some(typed) = key.key_char.as_deref() {
                    self.stamp_typed(Some(typed), cx);
                }
            }
        }
        true
    }

    /// Start hearing your words for a new stamp: what you say from now, as the capture
    /// transcribes it. `None` when the call isn't being captured.
    fn listen_for_stamp(&self) -> Option<Heard> {
        let call = self.capture.as_ref()?.id.clone();
        let me = format!(":{}", self.user.as_ref()?.id);
        let store = crate::capture::store::Store::open_default().ok()?;
        let after_id = store.last_line_id(&call).ok()?;
        let rev = store.latest_rev(&call).ok()?;
        Some(Heard {
            store,
            call,
            me,
            after_id,
            rev,
            lines: BTreeMap::new(),
            changed: Instant::now(),
        })
    }

    /// Bring in what you've said into stamp `id` since the last look. A stamp you've
    /// spoken into finishes itself once you've stopped talking for [`STAMP_SAID_DONE`].
    /// Returns whether it is still listening.
    fn hear_stamp(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        use crate::capture::store::Kind;
        if self.active_stamp != Some(id) {
            return false;
        }
        let Some(heard) = self.stamp_words.heard.as_mut() else {
            return false;
        };
        let Ok(lines) = heard.store.changes(&heard.call, heard.rev) else {
            return true;
        };
        let mut changed = false;
        for line in lines {
            heard.rev = heard.rev.max(line.rev);
            let mine = line
                .speaker_id
                .as_deref()
                .is_some_and(|speaker| speaker.ends_with(&heard.me));
            if line.id <= heard.after_id || !mine {
                continue;
            }
            match line.kind {
                Kind::Speech => {
                    heard.lines.insert(line.id, (true, line.text));
                }
                Kind::Draft => {
                    heard.lines.insert(line.id, (false, line.text));
                }
                Kind::Retracted => {
                    heard.lines.remove(&line.id);
                }
                _ => continue,
            }
            changed = true;
        }
        if changed {
            heard.changed = Instant::now();
            self.show_stamp_words();
            self.send_stamp_text(false);
            cx.notify();
            return true;
        }
        let said = heard.lines.values().any(|(done, _)| *done);
        let talking = heard.lines.values().any(|(done, _)| !done);
        if said
            && !talking
            && self.stamp_words.typed.is_empty()
            && heard.changed.elapsed() >= STAMP_SAID_DONE
        {
            self.finish_stamp(cx);
            return false;
        }
        true
    }

    /// Put the words heard and typed so far on your stamp: what's settled as its text,
    /// what's still being recognised as its draft.
    fn show_stamp_words(&mut self) {
        let Some(id) = self.active_stamp else {
            return;
        };
        let (mut said, mut draft) = (Vec::new(), Vec::new());
        if let Some(heard) = &self.stamp_words.heard {
            for (done, text) in heard.lines.values() {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                if *done {
                    said.push(text);
                } else {
                    draft.push(text);
                }
            }
        }
        let mut text = said.join(" ");
        let typed = &self.stamp_words.typed;
        if !typed.is_empty() {
            if !text.is_empty() && !typed.starts_with(' ') {
                text.push(' ');
            }
            text.push_str(typed);
        }
        let mut draft = draft.join(" ");
        let max = crate::protocol::STAMP_TEXT_MAX;
        truncate(&mut text, max);
        truncate(&mut draft, max - text.len());
        if let Some(stamp) = self.own_stamp(id) {
            stamp.text = text;
            stamp.draft = draft;
        }
    }

    /// Put the stamp tool and the pen down, finishing a stamp you were on.
    pub fn stop_marking(&mut self, cx: &mut Context<Self>) {
        if self.stamping {
            self.finish_stamp(cx);
            self.stamping = false;
            if !self.drawing {
                self.marking_changed(cx);
            }
            cx.emit(SessionEvent::DrawingChanged);
            cx.notify();
        }
        if self.drawing {
            self.toggle_drawing(cx);
        }
    }

    /// How to use the stamp tool right now, for whatever shows it.
    pub fn stamp_hint(&self) -> &'static str {
        match (
            self.active_stamp.is_some(),
            self.stamp_words.heard.is_some(),
        ) {
            (true, true) => "Say it or type it · Enter to finish · Esc removes it",
            (true, false) => "Type your note · Enter to finish · Esc removes it",
            (false, _) if self.capture.is_some() => "Click a spot, then say or type your note",
            (false, _) => "Click a spot, then type your note",
        }
    }

    /// The number on your stamp that's still taking words.
    pub fn active_stamp_number(&self) -> Option<u32> {
        let id = self.active_stamp?;
        self.stamps
            .iter()
            .find(|stamp| stamp.owner.is_empty() && stamp.id == id)
            .map(|stamp| stamp.number)
    }

    /// Your stamp is finished: its words are final. An empty one is removed.
    pub fn finish_stamp(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active_stamp else {
            return;
        };
        let empty = self.own_stamp(id).is_none_or(|stamp| {
            let draft = std::mem::take(&mut stamp.draft);
            if !stamp.text.is_empty() && !draft.is_empty() {
                stamp.text.push(' ');
            }
            stamp.text.push_str(&draft);
            stamp.listening = false;
            stamp.text.trim().is_empty()
        });
        if empty {
            self.cancel_stamp(cx);
            return;
        }
        self.send_stamp_text(true);
        self.active_stamp = None;
        self.stamp_words = StampWords::default();
        cx.notify();
    }

    /// Take your unfinished stamp away.
    pub fn cancel_stamp(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active_stamp.take() else {
            return;
        };
        self.stamp_words = StampWords::default();
        if let Some(stamp) = self.own_stamp(id) {
            stamp.listening = false;
            stamp.removed = Some(Instant::now());
        }
        self.send(Command::SendStamp(StampMessage::StampRemove { id }));
        cx.notify();
    }

    /// Someone else's stamp.
    fn receive_stamp(&mut self, sender: String, message: StampMessage) {
        let found = self
            .stamps
            .iter()
            .position(|stamp| stamp.owner == sender && stamp.id == message.id());
        match (message, found) {
            (StampMessage::StampPlace { id, x, y }, None) => {
                if self.stamps.len() >= 64 {
                    self.stamps.remove(0);
                }
                let name = self.name_for(&sender);
                self.stamp_count += 1;
                self.stamps.push(super::marks::Stamp {
                    id,
                    number: self.stamp_count,
                    owner: sender,
                    name,
                    point: Point { x, y },
                    text: String::new(),
                    draft: String::new(),
                    listening: true,
                    voice: false,
                    placed: Instant::now(),
                    removed: None,
                });
            }
            (
                StampMessage::StampText {
                    text, draft, done, ..
                },
                Some(index),
            ) => {
                let stamp = &mut self.stamps[index];
                stamp.text = text;
                stamp.draft = draft;
                stamp.listening = !done;
            }
            (StampMessage::StampRemove { .. }, Some(index)) => {
                let stamp = &mut self.stamps[index];
                stamp.listening = false;
                stamp.removed.get_or_insert_with(Instant::now);
            }
            _ => {}
        }
    }

    /// Clear all your ink, here and for everyone: it fades out. Right-click while drawing
    /// does this, as in the Mac app. A stroke being drawn goes too.
    pub fn clear_own_drawings(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let mut own = Vec::new();
        for path in &mut self.paths {
            if path.sender.is_empty() && path.fading.is_none() {
                path.fading = Some(now);
                own.push(path.id);
            }
        }
        self.drawing_path = None;
        self.straight_held = None;
        self.ink_changed = now;
        if self.connected {
            for path_id in own {
                self.send(Command::SendInput(InputEvent::DrawClearPath { path_id }));
            }
        }
        cx.emit(SessionEvent::Screen);
        cx.notify();
    }

    pub fn dismiss_error(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.errors.len() {
            self.errors.remove(index);
            cx.notify();
        }
    }

    // ---- input forwarded from the viewer ------------------------------------------------

    /// Mouse movement over the remote screen. Coordinates are normalized to the frame.
    /// `straight`: Shift is held, so the whole stroke is a straight line from where it
    /// began to the pointer, as in the Mac app. Others see the line move, at most every
    /// [`STRAIGHT_EVERY`]; let go of Shift and freehand ink carries on from its end.
    pub fn pointer_moved(
        &mut self,
        point: Point,
        dragging: bool,
        straight: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.connected {
            return;
        }
        if self.drawing {
            if let Some(index) = self.drawing_path
                && dragging
                && let Some(path) = self.paths.get_mut(index)
                && path.points.len() < 20_000
            {
                let (path_id, start) = (path.id, path.points[0]);
                let now = Instant::now();
                self.ink_changed = now;
                if straight {
                    path.points.truncate(1);
                    path.points.push(point);
                    if self
                        .straight_sent
                        .is_some_and(|sent| now.duration_since(sent) < STRAIGHT_EVERY)
                    {
                        self.straight_held = Some(point);
                    } else {
                        self.straight_sent = Some(now);
                        self.straight_held = None;
                        self.send_line(path_id, start, Some(point));
                    }
                } else {
                    path.points.push(point);
                    // Others need the line's latest end before the ink carries on from it.
                    if let Some(end) = self.straight_held.take() {
                        self.send_line(path_id, start, Some(end));
                    }
                    self.send(Command::SendInput(InputEvent::DrawAddPoint(point)));
                }
                cx.notify();
            }
        } else if self.remote_identity.is_some()
            && self.last_move.elapsed() >= Duration::from_millis(12)
        {
            self.last_move = Instant::now();
            self.send(Command::SendInput(InputEvent::MouseMove(point)));
        }
    }

    pub fn pointer_down(
        &mut self,
        point: Point,
        button: MouseButton,
        modifiers: Modifiers,
        clicks: u32,
        cx: &mut Context<Self>,
    ) {
        if !self.connected {
            return;
        }
        if self.stamping && button == MouseButton::Left {
            self.place_stamp(point, cx);
        } else if self.drawing && button == MouseButton::Right {
            // Right-click clears your ink, as in the Mac app.
            self.clear_own_drawings(cx);
        } else if self.drawing && button == MouseButton::Left {
            self.next_path = self.next_path.wrapping_add(1);
            self.paths.push(Stroke {
                sender: String::new(),
                id: self.next_path,
                points: vec![point],
                ended: None,
                fading: None,
            });
            self.drawing_path = Some(self.paths.len() - 1);
            self.straight_sent = None;
            self.straight_held = None;
            self.ink_changed = Instant::now();
            self.send(Command::SendInput(InputEvent::DrawStart(PathPoint {
                point,
                path_id: self.next_path,
            })));
            cx.notify();
        } else if self.peer_control && self.remote_identity.is_some() {
            self.forwarded_buttons.insert(button, point);
            self.send(Command::SendInput(InputEvent::MouseClick(mouse_click(
                point, button, modifiers, clicks, true,
            ))));
        }
    }

    pub fn pointer_up(
        &mut self,
        point: Option<Point>,
        button: MouseButton,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.drawing_path.take() {
            let held = self.straight_held.take();
            self.straight_sent = None;
            if let Some(path) = self.paths.get_mut(index) {
                let (path_id, start) = (path.id, path.points[0]);
                if modifiers.shift
                    && let Some(end) = point
                {
                    // A straight line ends exactly where the pointer is, for everyone.
                    path.points.truncate(1);
                    path.points.push(end);
                    self.send_line(path_id, start, None);
                } else if let Some(end) = held {
                    self.send_line(path_id, start, Some(end));
                }
            }
            let last = self
                .paths
                .get(index)
                .and_then(|path| path.points.last())
                .copied();
            if let Some(point) = point.or(last) {
                self.send(Command::SendInput(InputEvent::DrawEnd(point)));
            }
            let now = Instant::now();
            if let Some(path) = self.paths.get_mut(index) {
                path.ended = Some(now);
            }
            self.ink_changed = now;
            cx.notify();
        } else if let Some(last) = self.forwarded_buttons.remove(&button)
            && self.connected
        {
            self.send(Command::SendInput(InputEvent::MouseClick(mouse_click(
                point.unwrap_or(last),
                button,
                modifiers,
                1,
                false,
            ))));
        }
    }

    pub fn wheel(&mut self, x: f32, y: f32) {
        if self.connected && self.peer_control && self.remote_identity.is_some() && !self.drawing {
            self.send(Command::SendInput(InputEvent::WheelEvent(Wheel {
                x: f64::from(-x),
                y: f64::from(-y),
            })));
        }
    }

    /// Forward a key to the controlled screen. Returns whether it was consumed.
    pub fn remote_key(&mut self, key: &gpui_kit::Keystroke, down: bool) -> bool {
        if !self.connected
            || !self.peer_control
            || self.remote_identity.is_none()
            || self.screen.is_none()
            || self.drawing
        {
            return false;
        }
        let logical = super::keys::dom_key(key);
        if logical == "Escape" {
            return false;
        }
        if !down && !self.forwarded_keys.contains_key(&key.key) {
            return false;
        }
        let modifiers = key.modifiers;
        let event = RemoteKey {
            key: vec![logical],
            meta: modifiers.platform,
            ctrl: modifiers.control,
            shift: modifiers.shift,
            alt: modifiers.alt,
            down,
        };
        if down {
            self.forwarded_keys.insert(key.key.clone(), event.clone());
        } else {
            self.forwarded_keys.remove(&key.key);
        }
        self.send(Command::SendInput(InputEvent::Keystroke(event)));
        true
    }

    /// Release every key and button held on the remote side.
    pub fn release_input(&mut self) {
        let buttons: Vec<_> = self.forwarded_buttons.drain().collect();
        for (button, point) in buttons {
            self.send(Command::SendInput(InputEvent::MouseClick(mouse_click(
                point,
                button,
                Default::default(),
                1,
                false,
            ))));
        }
        let keys: Vec<_> = self
            .forwarded_keys
            .drain()
            .map(|(_, mut key)| {
                key.down = false;
                key.shift = false;
                key.ctrl = false;
                key.alt = false;
                key.meta = false;
                key
            })
            .collect();
        for key in keys {
            self.send(Command::SendInput(InputEvent::Keystroke(key)));
        }
    }

    // ---- engine pump ----------------------------------------------------------------------

    /// Drain engine events and new frames. Runs on the app timer, independent of windows.
    /// Take in whatever the engine has sent since it last woke us. State changes redraw
    /// every window; frames and pointer moves only redraw the windows that show them, so
    /// 30 fps of video doesn't re-render the panel, the call window and the pills.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        self.expire_cards(cx);
        let (mut dirty, mut screen, mut cameras) = (false, false, false);
        let mut handled = 0;
        while let Ok(event) = self.engine.events.try_recv() {
            if matches!(event, Event::Pointer { .. }) {
                screen = true;
            } else {
                dirty = true;
            }
            self.apply(event, cx);
            handled += 1;
            if handled == 256 {
                // Leave the rest of a burst for the next tick so the windows keep painting.
                self.engine.wake.notify_one();
                break;
            }
        }
        if dirty {
            self.sync_ring();
        }
        // Your ink, left alone for INK_HOLD and not kept, fades all together, here and (by
        // clearing it, which fades it there) on everyone's screen: a drawing shouldn't
        // vanish one stroke at a time. Faded strokes, ripples and stamps are let go.
        let now = Instant::now();
        let mut cleared = Vec::new();
        if !self.keep_ink
            && self.drawing_path.is_none()
            && now.duration_since(self.ink_changed) >= super::marks::INK_HOLD
        {
            for path in &mut self.paths {
                if path.sender.is_empty() && path.ended.is_some() && path.fading.is_none() {
                    path.fading = Some(now);
                    cleared.push(path.id);
                }
            }
        }
        if !cleared.is_empty() {
            screen = true;
        }
        if self.connected {
            for path_id in cleared {
                self.send(Command::SendInput(InputEvent::DrawClearPath { path_id }));
            }
        }
        if self.paths.iter().any(|path| path.faded(now)) {
            self.keep_paths(|path| !path.faded(now));
            screen = true;
        }
        self.ripples
            .retain(|ripple| now.duration_since(ripple.at) < Duration::from_secs(2));
        self.stamps.retain(|stamp| !stamp.gone(now));
        if let Some(frame) = take(&self.engine.shared.remote) {
            // The blurred band copy changes slowly; a few updates a second is plenty.
            if self.ambient.is_none() || self.ambient_at.elapsed() >= Duration::from_millis(250) {
                self.ambient_at = Instant::now();
                let first = self.ambient.is_none();
                if let Some((width, height, pixels)) =
                    super::ambient::ambient(frame.width, frame.height, &frame.bgra)
                {
                    self.ambient = to_picture(width, height, pixels);
                }
                if first {
                    log(&format!(
                        "screen {}×{}, blurred background {}",
                        frame.width,
                        frame.height,
                        if self.ambient.is_some() {
                            "ready"
                        } else {
                            "unavailable"
                        }
                    ));
                }
            }
            if let Some(picture) = to_picture(frame.width, frame.height, frame.bgra) {
                self.screen = Some(picture);
                screen = true;
            }
        }
        cameras |= pull(&self.engine.shared.local_camera, &mut self.local_camera);
        let slots = self
            .engine
            .shared
            .cameras
            .lock()
            .map(|slots| slots.clone())
            .unwrap_or_default();
        let before = self.cameras.len();
        self.cameras.retain(|key, _| slots.contains_key(key));
        cameras |= before != self.cameras.len();
        for (identity, slot) in slots {
            let mut picture = self.cameras.remove(&identity);
            cameras |= pull(&slot, &mut picture);
            if let Some(picture) = picture {
                self.cameras.insert(identity, picture);
            }
        }
        let before = self.pointers.len();
        self.pointers.retain(|_, pointer| pointer.visible(now));
        screen |= before != self.pointers.len();
        // Compared with what was last announced, so a change made outside a tick (drawing
        // switched on) is never missed.
        if self.marks_announced != self.has_marks() {
            self.marks_announced = !self.marks_announced;
            cx.emit(SessionEvent::MarksChanged);
        }
        if dirty {
            cx.notify();
        } else {
            if screen {
                cx.emit(SessionEvent::Screen);
            }
            if cameras {
                cx.emit(SessionEvent::Cameras);
            }
        }
    }

    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Some(line) = describe(&event) {
            log(&line);
        }
        let dialing = self.dialing.is_some();
        let outcome = match &event {
            Event::Joined(_) if dialing || self.ringing.is_some() => Some(Tone::Accepted),
            Event::Disconnected(_) if dialing => Some(Tone::Unavailable),
            Event::InviteResolved { message } if dialing => {
                Some(if message == "Teammate accepted" {
                    Tone::Accepted
                } else {
                    Tone::Unavailable
                })
            }
            _ => None,
        };
        if let Some(tone) = outcome {
            self.tones.once(tone);
        }
        match event {
            Event::LoggedIn(user) => {
                self.user = Some(user);
                self.status = "Choose a teammate or room to start pairing".into();
                self.errors.clear();
                cx.emit(SessionEvent::SignedIn);
            }
            Event::Rooms(rooms) => self.rooms = rooms,
            Event::RoomPresence(presence) => self.room_presence = presence,
            Event::SignedOut => {
                self.release_input();
                self.end_call();
                self.settings.token = None;
                self.user = None;
                self.rooms.clear();
                self.room_presence.clear();
                self.teammates.clear();
                self.ringing = None;
                self.errors.clear();
                self.status = "Signed out".into();
                cx.emit(SessionEvent::CallEnded);
                cx.emit(SessionEvent::SignedOut);
            }
            Event::Teammates(teammates) => self.teammates = teammates,
            Event::IncomingCall { caller_id, name } => {
                self.ringing = Some(Ringing::Call {
                    caller_id,
                    name: sanitize(&name),
                });
                cx.emit(SessionEvent::RingingChanged);
            }
            Event::Calling { peer_id, name } => {
                self.dialing = Some((peer_id, sanitize(&name), None));
            }
            Event::IncomingInvite {
                inviter_id,
                invite_id,
                name,
            } => {
                self.ringing = Some(Ringing::Invite {
                    inviter_id,
                    invite_id,
                    name: sanitize(&name),
                });
                cx.emit(SessionEvent::RingingChanged);
            }
            Event::Inviting {
                peer_id,
                invite_id,
                name,
            } => self.dialing = Some((peer_id, sanitize(&name), Some(invite_id))),
            Event::InviteResolved { message } => {
                self.dialing = None;
                if self.ringing.take().is_some() {
                    cx.emit(SessionEvent::RingingChanged);
                }
                self.status = message;
            }
            Event::Activate => cx.emit(SessionEvent::Activate),
            Event::Ui(request) => {
                if !self.handle_request(&request, cx) {
                    cx.emit(SessionEvent::Request(request));
                }
            }
            Event::DesktopLocked => {
                self.release_input();
                self.desktop_locked = true;
                self.grant = None;
                self.peer_control = false;
                self.status = "Desktop locked · screen control paused".into();
            }
            Event::DesktopUnlocked => {
                self.desktop_locked = false;
                self.status = if self.connected {
                    "Connected · grant control to resume"
                } else {
                    "Ready when you are"
                }
                .into();
            }
            Event::Status(status) => self.status = status,
            Event::Joined(room) => {
                self.dialing = None;
                if self.ringing.take().is_some() {
                    cx.emit(SessionEvent::RingingChanged);
                }
                self.room = Some(room);
                self.joined_at = Some(Instant::now());
                self.status = "Connecting audio and video…".into();
                cx.emit(SessionEvent::CallStarted);
            }
            Event::Connected => {
                self.connected = true;
                self.status = "Connected".into();
            }
            Event::TransportReconnecting => {
                self.release_input();
                self.connected = false;
                self.peer_control = false;
                self.grant = None;
                self.status = "Reconnecting…".into();
            }
            Event::TransportReconnected => {
                self.connected = self.room.is_some();
                self.status = "Connected".into();
            }
            Event::Disconnected(reason) => {
                let had_share = self.remote_identity.is_some();
                self.release_input();
                self.end_call();
                self.status = reason;
                if had_share {
                    cx.emit(SessionEvent::RemoteShareEnded);
                }
                cx.emit(SessionEvent::CallEnded);
            }
            Event::Participants(participants) => {
                // Say when someone else starts or stops capturing; badges show it too.
                let before = self.capturing_others();
                self.participants = participants;
                let after = self.capturing_others();
                for name in after.iter().filter(|name| !before.contains(name)) {
                    self.status = format!("{name} is capturing this call");
                }
                for name in before.iter().filter(|name| !after.contains(name)) {
                    self.status = format!("{name} stopped capturing");
                }
            }
            Event::RemoteShare { identity, active } => {
                if active {
                    self.remote_identity = Some(identity);
                    cx.emit(SessionEvent::RemoteShareStarted);
                } else if self.remote_identity.as_ref() == Some(&identity) {
                    self.release_input();
                    self.remote_identity = None;
                    self.peer_control = false;
                    self.screen = None;
                    self.ambient = None;
                    cx.emit(SessionEvent::RemoteShareEnded);
                }
            }
            Event::Sharing { active, target } => {
                if self.sharing != active {
                    cx.emit(SessionEvent::SharingChanged);
                }
                self.sharing = active;
                self.share_target = target.clone().filter(|_| active);
                self.capture_name = target.and_then(|target| target.output_name);
                if !active {
                    if self.selecting_source {
                        self.selection_canceled = true;
                    }
                    self.grant = None;
                    self.stopped_sharing_status();
                }
            }
            Event::SelectingSource(active) => {
                self.selecting_source = active;
                self.selection_canceled = false;
            }
            Event::Microphone(active) => self.microphone = active,
            Event::Camera(active) => {
                self.camera = active;
                if !active {
                    self.local_camera = None;
                }
            }
            Event::ControlGranted(grant) => self.grant = grant,
            Event::Capture(status) => {
                if let Some(device) = status.as_ref().and_then(|status| status.device.clone()) {
                    self.capture_device = Some(device);
                }
                self.capture = status;
            }
            Event::CaptureChanged => {}
            Event::PeerControl { enabled } => {
                if !enabled {
                    self.release_input();
                }
                self.peer_control = enabled;
            }
            Event::Error(error) => self.push_error(error),
            // Handled inside the engine, which reopens the device.
            Event::AudioDeviceFailed { .. } => {}
            Event::CaptureClosed => {
                if self.sharing {
                    cx.emit(SessionEvent::SharingChanged);
                }
                self.sharing = false;
                self.stopped_sharing_status();
                self.grant = None;
                self.capture_name = None;
            }
            Event::Drawing { sender, event } => self.receive_drawing(sender, event),
            Event::Stamp { sender, message } => self.receive_stamp(sender, message),
            Event::Pointer { sender, point } => {
                self.pointers
                    .entry(sender)
                    .and_modify(|pointer| pointer.moved_to(point))
                    .or_insert_with(|| super::marks::Pointer::new(point));
            }
            Event::Devices {
                inputs,
                outputs,
                cameras,
            } => {
                self.devices = Some(Devices {
                    inputs,
                    outputs,
                    cameras,
                })
            }
            Event::Data { .. } => {}
        }
    }

    fn end_call(&mut self) {
        self.dialing = None;
        self.connected = false;
        self.room = None;
        self.joined_at = None;
        self.participants.clear();
        self.sharing = false;
        self.selecting_source = false;
        self.microphone = false;
        self.camera = false;
        self.peer_control = false;
        self.grant = None;
        self.remote_identity = None;
        self.drawing = false;
        self.drawing_path = None;
        self.straight_sent = None;
        self.straight_held = None;
        self.paths.clear();
        self.incoming_paths.clear();
        self.pointers.clear();
        self.pens.clear();
        self.stamp_count = 0;
        self.screen = None;
        self.ambient = None;
        self.local_camera = None;
        self.cameras.clear();
    }

    /// Their cursor follows their pen: drawing sends points, not cursor moves.
    fn pen_moved(&mut self, sender: &str, point: Point) {
        self.pointers
            .entry(sender.to_owned())
            .and_modify(|pointer| pointer.moved_to(point))
            .or_insert_with(|| super::marks::Pointer::new(point));
    }

    fn receive_drawing(&mut self, sender: String, event: InputEvent) {
        match &event {
            InputEvent::DrawingMode(mode) => {
                let who = canonical_identity(&sender).to_owned();
                if matches!(mode, DrawingMode::Draw(_)) {
                    self.pens.insert(who);
                } else {
                    self.pens.remove(&who);
                    // They stopped drawing: their ink goes, as the Mac app does.
                    let now = Instant::now();
                    self.keep_paths(|path| path.sender != sender || path.ended.is_some());
                    for path in &mut self.paths {
                        if path.sender == sender && path.fading.is_none() {
                            path.fading = Some(now);
                        }
                    }
                }
            }
            InputEvent::DrawStart(path) => {
                // Clients that don't announce drawing mode still draw with a pencil.
                self.pens.insert(canonical_identity(&sender).to_owned());
                self.pen_moved(&sender, path.point);
            }
            InputEvent::DrawAddPoint(point) | InputEvent::DrawEnd(point) => {
                self.pen_moved(&sender, *point);
            }
            _ => {}
        }
        match event {
            InputEvent::DrawStart(path) => {
                if self.paths.len() >= 500 {
                    self.paths.remove(0);
                    self.incoming_paths.clear();
                }
                self.paths.push(Stroke {
                    sender: sender.clone(),
                    id: path.path_id,
                    points: vec![path.point],
                    ended: None,
                    fading: None,
                });
                self.incoming_paths.insert(sender, self.paths.len() - 1);
            }
            InputEvent::DrawAddPoint(point) => {
                if let Some(index) = self.incoming_paths.get(&sender).copied()
                    && let Some(path) = self.paths.get_mut(index)
                    && path.ended.is_none()
                    && path.points.len() < 20_000
                {
                    path.points.push(point);
                }
            }
            InputEvent::DrawEnd(point) => {
                if let Some(index) = self.incoming_paths.get(&sender).copied()
                    && let Some(path) = self.paths.get_mut(index)
                    && path.ended.is_none()
                {
                    if path.points.len() < 20_000
                        && path
                            .points
                            .last()
                            .is_none_or(|last| last.x != point.x || last.y != point.y)
                    {
                        path.points.push(point);
                    }
                    // Theirs fade when they clear them; this only marks it finished.
                    path.ended = Some(Instant::now());
                }
            }
            // Cleared strokes fade out rather than vanish. A stroke cleared while still
            // being drawn goes at once: that's a straight line being redrawn (cleared,
            // then started again with the same id), and fading each copy would leave a
            // trail of ghost lines.
            InputEvent::DrawClearAllPaths => {
                let now = Instant::now();
                self.keep_paths(|path| path.sender != sender || path.ended.is_some());
                for path in &mut self.paths {
                    if path.sender == sender && path.fading.is_none() {
                        path.fading = Some(now);
                    }
                }
            }
            InputEvent::DrawClearPath { path_id } => {
                let now = Instant::now();
                self.keep_paths(|path| {
                    path.sender != sender || path.id != path_id || path.ended.is_some()
                });
                for path in &mut self.paths {
                    if path.sender == sender && path.id == path_id && path.fading.is_none() {
                        path.fading = Some(now);
                    }
                }
            }
            InputEvent::ClickAnimation(point) => self.ripples.push(Ripple {
                sender,
                point,
                at: Instant::now(),
            }),
            _ => {}
        }
    }

    fn keep_paths(&mut self, keep: impl Fn(&Stroke) -> bool) {
        let active = self
            .drawing_path
            .and_then(|index| self.paths.get(index))
            .map(|path| path.id);
        self.paths.retain(keep);
        self.incoming_paths.clear();
        for (index, path) in self.paths.iter().enumerate() {
            if !path.sender.is_empty() {
                self.incoming_paths.insert(path.sender.clone(), index);
            }
        }
        self.drawing_path = active.and_then(|id| {
            self.paths
                .iter()
                .position(|path| path.sender.is_empty() && path.id == id)
        });
    }

    /// Leave any call and exit Hopp; the shell does the shutdown.
    pub fn quit(&mut self, cx: &mut Context<Self>) {
        cx.emit(SessionEvent::Request(crate::protocol::UiRequest::Quit));
    }

    /// Show a card. Posting an id again replaces that card.
    pub fn post_card(&mut self, request: crate::cards::CardRequest, cx: &mut Context<Self>) {
        let request = request.tidy();
        if request.title.is_empty() {
            return;
        }
        self.cards.retain(|card| card.request.id != request.id);
        self.cards.push(Card {
            request,
            state: crate::cards::CardState::Showing,
            posted: Instant::now(),
        });
        cx.emit(SessionEvent::CardsChanged);
        // Forget the oldest answered cards; showing ones stay until answered.
        while self.cards.len() > CARDS_KEPT {
            let Some(index) = self
                .cards
                .iter()
                .position(|card| card.state != crate::cards::CardState::Showing)
            else {
                break;
            };
            self.cards.remove(index);
        }
        cx.notify();
    }

    /// Answer, dismiss or withdraw a card that is still showing.
    pub fn settle_card(
        &mut self,
        id: &str,
        state: crate::cards::CardState,
        cx: &mut Context<Self>,
    ) {
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.request.id == id && card.state == crate::cards::CardState::Showing)
        {
            card.state = state;
            cx.emit(SessionEvent::CardsChanged);
            cx.notify();
        }
    }

    /// Cards still waiting, newest first.
    pub fn showing_cards(&self) -> Vec<Card> {
        self.cards
            .iter()
            .rev()
            .filter(|card| card.state == crate::cards::CardState::Showing)
            .cloned()
            .collect()
    }

    /// Ephemeral notices go by themselves.
    fn expire_cards(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for card in &mut self.cards {
            if card.state == crate::cards::CardState::Showing
                && card.request.ephemeral
                && card.request.question.is_none()
                && card.posted.elapsed() >= crate::cards::EPHEMERAL
            {
                card.state = crate::cards::CardState::Expired;
                changed = true;
            }
        }
        if changed {
            cx.emit(SessionEvent::CardsChanged);
            cx.notify();
        }
    }

    /// Call controls requested from the desktop. Returns false for window requests,
    /// which the shell handles.
    fn handle_request(
        &mut self,
        request: &crate::protocol::UiRequest,
        cx: &mut Context<Self>,
    ) -> bool {
        use crate::protocol::UiRequest;
        match request {
            UiRequest::Call(id) => {
                if let Some(teammate) = self
                    .teammates
                    .iter()
                    .find(|teammate| &teammate.id == id)
                    .cloned()
                {
                    self.call(&teammate, cx);
                }
            }
            UiRequest::JoinRoom(id) => {
                if let Some(room) = self.rooms.iter().find(|room| &room.id == id).cloned() {
                    self.join_room(room, cx);
                }
            }
            UiRequest::Accept => self.accept_ringing(cx),
            UiRequest::Decline => {
                if self.ringing.is_some() {
                    self.decline_ringing(cx);
                } else if self.dialing.is_some() {
                    self.leave(cx);
                }
            }
            UiRequest::Leave => self.leave(cx),
            UiRequest::Microphone => self.toggle_microphone(cx),
            UiRequest::Camera => self.toggle_camera(cx),
            UiRequest::Share => self.toggle_share(false, cx),
            UiRequest::Capture => self.toggle_capture(cx),
            UiRequest::StartCapture => self.start_capture(cx),
            UiRequest::StopCapture => self.send(Command::StopCapture),
            UiRequest::Bookmark(label) => self.send(Command::Bookmark(label.clone())),
            UiRequest::Card(request) => self.post_card(request.clone(), cx),
            UiRequest::Draw => self.toggle_drawing(cx),
            UiRequest::Stamp => self.toggle_stamping(cx),
            UiRequest::CancelCard(id) => {
                self.settle_card(id, crate::cards::CardState::Cancelled, cx)
            }
            _ => return false,
        }
        true
    }

    /// Everything the bar plugin shows, as JSON. Ids are Hopp ids; nothing secret.
    pub fn snapshot(&self) -> serde_json::Value {
        use serde_json::json;
        let own = self.user.as_ref().map(|user| user.id.as_str());
        let room_of = |id: &str| {
            self.room_presence
                .iter()
                .find(|(_, members)| members.iter().any(|member| member == id))
                .and_then(|(room, _)| self.rooms.iter().find(|candidate| &candidate.id == room))
                .map(|room| room.name.clone())
        };
        let started = self.joined_at.map(|joined| {
            std::time::SystemTime::now()
                .checked_sub(joined.elapsed())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs())
                .unwrap_or_default()
        });
        let name_of = |id: &str| {
            self.teammates
                .iter()
                .find(|teammate| teammate.id == id)
                .map(|teammate| full_name(&teammate.first_name, &teammate.last_name))
        };
        json!({
            "version": 1,
            "signed_in": self.user.is_some(),
            "user": self.user.as_ref().map(|user| json!({
                "id": user.id,
                "name": full_name(&user.first_name, &user.last_name),
                "email": user.email,
            })),
            "status": self.status,
            "teammates": self.teammates.iter()
                .filter(|teammate| Some(teammate.id.as_str()) != own)
                .map(|teammate| json!({
                    "id": teammate.id,
                    "name": full_name(&teammate.first_name, &teammate.last_name),
                    "online": teammate.is_active,
                    "room": room_of(&teammate.id),
                }))
                .collect::<Vec<_>>(),
            "rooms": self.rooms.iter().map(|room| json!({
                "id": room.id,
                "name": room.name,
                "here": self.room.as_ref().is_some_and(|current| current.id == room.id),
                "members": self.room_presence.get(&room.id)
                    .map(|members| members.iter().filter_map(|id| name_of(id)).collect::<Vec<_>>())
                    .unwrap_or_default(),
            })).collect::<Vec<_>>(),
            "call": self.room.as_ref().map(|room| json!({
                "room": room.name,
                "connected": self.connected,
                "started": started,
                "participants": self.participants.iter().map(|participant| json!({
                    "id": canonical_identity(&participant.identity),
                    "name": sanitize(&participant.name),
                    "camera": self.camera_for(&participant.identity).is_some(),
                    "granted": self.is_granted(&participant.identity),
                    "capturing": participant.capturing,
                })).collect::<Vec<_>>(),
                "capture": self.capture,
                "microphone": self.microphone,
                "camera": self.camera,
                "sharing": self.sharing,
                "selecting": self.selecting_source,
                "viewing": self.remote_identity.as_deref().map(|identity| self.name_for(identity)),
                "controlling": self.peer_control,
            })),
            "ringing": self.ringing.as_ref().map(|ringing| json!({
                "name": ringing.name(),
                "kind": match ringing { Ringing::Call { .. } => "call", Ringing::Invite { .. } => "invite" },
            })),
            "dialing": self.dialing.as_ref().map(|(_, name, _)| json!({ "name": name })),
            "errors": self.errors,
            // Where each card has got to: how `hopp ask` hears the answer.
            "cards": self.cards.iter().map(|card| json!({
                "id": card.request.id,
                "state": card.state,
                "title": card.request.title,
                "question": card.request.question.is_some(),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn shutdown(&mut self) {
        self.tones.stop();
        self.release_input();
        let _ = self.engine.commands.try_send(Command::Shutdown);
    }
}

/// Call lifecycle on stderr, for diagnosing live sessions. Never includes tokens.
fn log(line: &str) {
    crate::logger::line(line);
}

fn describe(event: &Event) -> Option<String> {
    Some(match event {
        Event::LoggedIn(user) => format!("signed in as {}", user.id),
        Event::SignedOut => "signed out".into(),
        Event::Status(status) => format!("status: {status}"),
        Event::Joined(room) => format!("joined room {}", room.id),
        Event::Connected => "media connected".into(),
        Event::Disconnected(reason) => format!("disconnected: {reason}"),
        Event::Participants(list) => format!("{} other participant(s)", list.len()),
        Event::RemoteShare { identity, active } => {
            format!("remote share from {identity}: {active}")
        }
        Event::Sharing { active, target } => format!(
            "sharing: {active}{}",
            target
                .as_ref()
                .and_then(|target| target.output_name.as_deref())
                .map(|name| format!(" ({name})"))
                .unwrap_or_default()
        ),
        Event::SelectingSource(active) => format!("choosing a source: {active}"),
        Event::CaptureClosed => "capture closed".into(),
        Event::IncomingCall { caller_id, .. } => format!("incoming call from {caller_id}"),
        Event::Calling { peer_id, .. } => format!("calling {peer_id}"),
        Event::IncomingInvite { inviter_id, .. } => format!("invited by {inviter_id}"),
        Event::InviteResolved { message } => format!("invite resolved: {message}"),
        Event::ControlGranted(grant) => format!("control granted to {grant:?}"),
        Event::PeerControl { enabled } => format!("controlling peer: {enabled}"),
        Event::TransportReconnecting => "reconnecting".into(),
        Event::TransportReconnected => "reconnected".into(),
        Event::DesktopLocked => "desktop locked".into(),
        Event::DesktopUnlocked => "desktop unlocked".into(),
        _ => return None,
    })
}

/// Move a freshly captured frame out of its slot and into a paintable picture.
fn pull(slot: &FrameSlot, picture: &mut Option<Picture>) -> bool {
    let Some(frame) = take(slot) else {
        return false;
    };
    match to_picture(frame.width, frame.height, frame.bgra) {
        Some(new) => {
            *picture = Some(new);
            true
        }
        None => false,
    }
}

fn take(slot: &FrameSlot) -> Option<crate::frames::VideoFrame> {
    slot.lock().ok().and_then(|mut slot| slot.take())
}

fn to_picture(width: u32, height: u32, bgra: Vec<u8>) -> Option<Picture> {
    // The bytes stay BGRA, which is the order GPUI's renderer expects.
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Picture {
        image: Arc::new(RenderImage::new([image::Frame::new(buffer)])),
        source: [width, height],
    })
}

pub fn mouse_click(
    position: Point,
    button: MouseButton,
    modifiers: Modifiers,
    clicks: u32,
    down: bool,
) -> MouseClick {
    MouseClick {
        x: position.x,
        y: position.y,
        button: match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => 0,
        },
        clicks,
        down,
        shift: modifiers.shift,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.platform,
    }
}

pub fn full_name(first: &str, last: &str) -> String {
    sanitize(format!("{first} {last}").trim())
}

/// Names arrive from the network; keep them printable and short.
pub fn sanitize(name: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(48)
        .collect();
    if clean.trim().is_empty() {
        "Teammate".into()
    } else {
        clean
    }
}

pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .collect::<String>()
        .to_uppercase()
}
