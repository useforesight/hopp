//! Existing Hopp data-channel JSON. Input is authenticated by participant
//! identity in the room, never by an identity supplied inside the JSON payload.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MouseClick {
    pub x: f64,
    pub y: f64,
    pub button: u32,
    pub clicks: u32,
    pub down: bool,
    pub shift: bool,
    pub meta: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Keystroke {
    pub key: Vec<String>,
    pub meta: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub down: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Wheel {
    #[serde(rename = "deltaX")]
    pub x: f64,
    #[serde(rename = "deltaY")]
    pub y: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Enabled {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PathPoint {
    pub point: Point,
    pub path_id: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipboardPacket {
    pub packet_id: u64,
    pub total_packets: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipboardPaste {
    pub data: Option<ClipboardPacket>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipboardCopy {
    pub is_copy: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClipboardData {
    pub requester_sid: String,
    pub data: Option<ClipboardPacket>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct DrawSettings {
    pub permanent: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", content = "settings")]
pub enum DrawingMode {
    Disabled,
    Draw(DrawSettings),
    ClickAnimation,
    Any,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum InputEvent {
    MouseMove(Point),
    MouseClick(MouseClick),
    Keystroke(Keystroke),
    WheelEvent(Wheel),
    RemoteControlEnabled(Enabled),
    DrawStart(PathPoint),
    DrawAddPoint(Point),
    DrawEnd(Point),
    DrawClearAllPaths,
    DrawClearPath { path_id: u64 },
    DrawingMode(DrawingMode),
    ClickAnimation(Point),
    PasteFromClipboard(ClipboardPaste),
    AddToClipboard(ClipboardCopy),
    ClipboardData(ClipboardData),
}

impl InputEvent {
    pub fn valid(&self) -> bool {
        match self {
            Self::MouseMove(p)
            | Self::DrawAddPoint(p)
            | Self::DrawEnd(p)
            | Self::ClickAnimation(p) => p.valid(),
            Self::DrawStart(p) => p.point.valid(),
            Self::MouseClick(click) => {
                Point {
                    x: click.x,
                    y: click.y,
                }
                .valid()
                    && click.button <= 4
            }
            Self::Keystroke(key) => {
                key.key.len() == 1 && !key.key[0].is_empty() && key.key[0].len() <= 64
            }
            Self::WheelEvent(wheel) => {
                wheel.x.is_finite()
                    && wheel.y.is_finite()
                    && wheel.x.abs() <= 4096.0
                    && wheel.y.abs() <= 4096.0
            }
            Self::PasteFromClipboard(paste) => {
                paste.data.as_ref().is_none_or(valid_clipboard_packet)
            }
            Self::ClipboardData(data) => {
                !data.requester_sid.is_empty()
                    && data.requester_sid.len() <= 256
                    && data.data.as_ref().is_none_or(valid_clipboard_packet)
            }
            _ => true,
        }
    }

    pub fn topic(&self) -> Option<&'static str> {
        match self {
            Self::MouseMove(_) => Some("participant_location"),
            Self::RemoteControlEnabled(_) => Some("remote_control_enabled"),
            Self::DrawStart(_)
            | Self::DrawAddPoint(_)
            | Self::DrawEnd(_)
            | Self::DrawClearAllPaths
            | Self::DrawClearPath { .. }
            | Self::DrawingMode(_)
            | Self::ClickAnimation(_) => Some("draw"),
            _ => None,
        }
    }
}

/// Stamps: a pin someone drops on the shared screen and speaks (or types) over. Sent on
/// their own topic, [`STAMP_TOPIC`], so clients that don't know them ignore them; the
/// placer also sends a `ClickAnimation` there, which every Hopp client shows.
pub const STAMP_TOPIC: &str = "hopp_stamp";
/// The longest stamp text, in bytes.
pub const STAMP_TEXT_MAX: usize = 2_000;

// The variant names are the wire format's message types, so they keep their prefix.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum StampMessage {
    /// A new stamp at a point on the shared screen. `id` is the sender's own.
    StampPlace {
        id: u64,
        x: f64,
        y: f64,
    },
    /// What it says so far: final `text`, then `draft` words still being recognised.
    /// `done` once the placer has finished.
    StampText {
        id: u64,
        text: String,
        draft: String,
        done: bool,
    },
    StampRemove {
        id: u64,
    },
}

impl StampMessage {
    pub fn valid(&self) -> bool {
        match self {
            Self::StampPlace { x, y, .. } => Point { x: *x, y: *y }.valid(),
            Self::StampText { text, draft, .. } => text.len() + draft.len() <= STAMP_TEXT_MAX,
            Self::StampRemove { .. } => true,
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            Self::StampPlace { id, .. } | Self::StampText { id, .. } | Self::StampRemove { id } => {
                *id
            }
        }
    }
}

/// Reactions: confetti, a gong, fireworks… that play on everyone's screen in the call. They
/// go out on their own topic, [`REACTION_TOPIC`], so clients that don't know them ignore
/// them, and clients that know them ignore kinds they don't.
pub const REACTION_TOPIC: &str = "hopp_reaction";
/// Packets bigger than this are dropped unread.
pub const REACTION_PACKET_MAX: usize = 2 * 1024;
/// The most characters of a reaction's words.
pub const REACTION_TEXT_MAX: usize = 60;
pub const REACTION_KINDS: [&str; 11] = [
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
pub const REACTION_EMOJI: [&str; 8] = [
    "party", "fire", "applause", "lol", "rocket", "hundred", "love", "shock",
];
pub const REACTION_SOUNDS: [&str; 8] = [
    "airhorn",
    "badumtss",
    "sad_trombone",
    "crickets",
    "drumroll",
    "chaching",
    "wooow",
    "applause",
];

/// One reaction, as it's sent to the call.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Reaction {
    /// One of [`REACTION_KINDS`].
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
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            choice: None,
            level: 0,
            text: None,
            to: None,
        }
    }

    /// The reaction in a packet from the call, if it's one this client knows.
    pub fn from_packet(payload: &[u8]) -> Option<Self> {
        if payload.len() > REACTION_PACKET_MAX {
            return None;
        }
        serde_json::from_slice::<Self>(payload).ok()?.tidy()
    }

    /// Within bounds, or None for a kind or choice this client doesn't know. The Mac
    /// client's `socket_lib::Reaction::tidy` keeps the same rules.
    pub fn tidy(mut self) -> Option<Self> {
        if !REACTION_KINDS.contains(&self.kind.as_str()) {
            return None;
        }
        let choices: &[&str] = match self.kind.as_str() {
            "emoji" => &REACTION_EMOJI,
            "sound" => &REACTION_SOUNDS,
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

fn valid_clipboard_packet(p: &ClipboardPacket) -> bool {
    p.total_packets > 0
        && p.total_packets <= 256
        && p.packet_id < p.total_packets
        && p.data.len() <= 64 * 1024
}

pub fn canonical_identity(identity: &str) -> &str {
    identity
        .strip_suffix(":audio")
        .or_else(|| identity.strip_suffix(":video"))
        .or_else(|| identity.strip_suffix(":camera"))
        .unwrap_or(identity)
}

#[derive(Default)]
pub struct ControlGrant {
    owner: Option<String>,
}

impl ControlGrant {
    pub fn grant(&mut self, identity: String) {
        self.owner = Some(identity);
    }
    pub fn active(&self) -> bool {
        self.owner.is_some()
    }

    pub fn revoke(&mut self) {
        self.owner = None;
    }
    pub fn allows(&self, sender: &str, sharing: bool) -> bool {
        sharing
            && self
                .owner
                .as_ref()
                .is_some_and(|owner| canonical_identity(owner) == canonical_identity(sender))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request the bar plugin sends must still parse: changing a request's shape
    /// here (as `captures` gaining a call id did) silently broke its button.
    #[test]
    fn every_request_the_bar_sends_parses() {
        let backend = include_str!("../omarchy-plugin/local.hopp/backend.py");
        let table = backend
            .split("SIMPLE = {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("the bar's SIMPLE table");
        let mut checked = 0;
        for line in table.lines() {
            let Some((_, value)) = line.split_once(':') else {
                continue;
            };
            let request = value.trim().trim_end_matches(',').trim_matches('"');
            if request.is_empty() {
                continue;
            }
            // `captures` is sent with an empty call id instead (see `command`).
            let payload = if request == "captures" {
                serde_json::json!({ "captures": null })
            } else {
                serde_json::json!(request)
            };
            assert!(
                serde_json::from_value::<UiRequest>(payload.clone()).is_ok(),
                "the bar sends {payload}, which Hopp doesn't accept"
            );
            checked += 1;
        }
        assert!(checked >= 10, "read {checked} requests from the bar");
        // Requests that carry an id: `call`, `join` and `transcript`.
        let with_id = backend
            .split("WITH_ID = {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("the bar's WITH_ID table");
        for pair in with_id.split(',') {
            let Some((_, request)) = pair.split_once(':') else {
                continue;
            };
            let request = request.trim().trim_matches('"');
            if request.is_empty() {
                continue;
            }
            // The panel sends a status where Hopp expects one, and an id everywhere else.
            let value = if request == "set_status" {
                "dnd"
            } else {
                "c_20261103-230152-9e5e"
            };
            let payload = serde_json::json!({ request: value });
            assert!(
                serde_json::from_value::<UiRequest>(payload.clone()).is_ok(),
                "the bar sends {payload}, which Hopp doesn't accept"
            );
        }
        assert!(backend.contains(r#"payload = {"ui": {"captures": None}}"#));
    }

    #[test]
    fn invalid_packets_and_ungranted_peers_cannot_inject_input() {
        let mut grant = ControlGrant::default();
        assert!(!grant.allows("peer:audio", true));
        grant.grant("peer:audio".into());
        assert!(grant.allows("peer:audio", true));
        assert!(!grant.allows("attacker:audio", true));
        assert!(!grant.allows("peer:audio", false));
        grant.revoke();
        assert!(!grant.allows("peer:audio", true));
        assert!(
            !InputEvent::MouseMove(Point {
                x: f64::NAN,
                y: 0.5
            })
            .valid()
        );
        assert!(
            !InputEvent::Keystroke(Keystroke {
                key: vec![],
                meta: false,
                ctrl: false,
                shift: false,
                alt: false,
                down: true
            })
            .valid()
        );
        let fixture = r#"{"type":"WheelEvent","payload":{"deltaX":1.0,"deltaY":40.0}}"#;
        let event: InputEvent = serde_json::from_str(fixture).unwrap();
        assert!(event.valid());
        assert_eq!(serde_json::to_string(&event).unwrap(), fixture);
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
            r#"{"kind":"sound","choice":"sad_trombone"}"#,
        ];
        for example in examples {
            let reaction = Reaction::from_packet(example.as_bytes()).expect(example);
            assert_eq!(serde_json::to_string(&reaction).unwrap(), example);
        }
        // The spec's examples are spaced out.
        let spaced = r#"{"kind": "emoji", "choice": "fire"}"#;
        assert_eq!(
            Reaction::from_packet(spaced.as_bytes()).unwrap().choice,
            Some("fire".into())
        );
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
            r#"{"kind":"gong","level":-1}"#,
            r#"{"kind":7}"#,
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
        assert_eq!(read(r#"{"kind":"ship_it","to":"x"}"#).to, None);
    }
}

/// Requests from the desktop (keybindings, the Omarchy bar plugin) for the client UI.
/// Sent over the private instance socket; ids are opaque Hopp ids.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRequest {
    /// Open or focus the Hopp window.
    Open,
    /// Open the Hopp window on its settings page.
    Settings,
    /// Toggle the whole shared screen in `special:hopp`.
    Peek,
    ShowFaces,
    ShowViewer,
    /// Call a teammate, or invite them when already in a call.
    Call(String),
    JoinRoom(String),
    Accept,
    Decline,
    /// Forget one person's missed calls (`Call` with their id calls them back).
    DismissMissed(String),
    /// Forget every missed call.
    ClearMissed,
    /// Set what teammates see before ringing you.
    SetStatus(crate::api::UserStatus),
    Leave,
    Microphone,
    Camera,
    Share,
    Quit,
    /// Start or stop capturing the call.
    Capture,
    StartCapture,
    StopCapture,
    /// Mark this moment in the capture, with an optional label.
    Bookmark(Option<String>),
    /// Open the captures window: the live transcript and past calls, or one call by id,
    /// `last` or `live`.
    Captures(Option<String>),
    /// Show a notice or question from an agent or script (see `cards`).
    Card(crate::cards::CardRequest),
    /// Withdraw a card that is still showing.
    CancelCard(String),
    /// Start or stop drawing: on the screen you're viewing, or on your own while you
    /// share it.
    Draw,
    /// Pick up or put down the stamp tool, on the same screens as `Draw`.
    Stamp,
}
