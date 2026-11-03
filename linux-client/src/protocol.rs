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
}
