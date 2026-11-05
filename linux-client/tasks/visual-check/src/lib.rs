#[path = "../../../src/badge.rs"]
pub mod badge;
#[path = "../../../src/cards.rs"]
pub mod cards;
#[path = "../../../src/frames.rs"]
pub mod frames;
#[path = "../../../src/ink.rs"]
pub mod ink;
#[path = "../../../src/ink_raster.rs"]
pub mod ink_raster;
#[path = "../../../src/logger.rs"]
pub mod logger;
#[path = "../../../src/missed.rs"]
pub mod missed;
#[path = "../../../src/paths.rs"]
pub mod paths;
#[path = "../../../src/protocol.rs"]
pub mod protocol;
#[path = "../../../src/settings.rs"]
pub mod settings;
#[path = "../../../src/ui/mod.rs"]
pub mod ui;
#[path = "../../../src/wake.rs"]
pub mod wake;
pub mod api {
    pub type RoomsPresence = std::collections::HashMap<String, Vec<String>>;
    pub const DEFAULT_API_BASE: &str = "https://pair.gethopp.app";
    #[derive(Clone, Debug)]
    pub struct RoomInfo {
        pub id: String,
        pub name: String,
    }
    #[derive(Clone, Debug)]
    pub struct UserInfo {
        pub id: String,
        pub first_name: String,
        pub last_name: String,
        pub email: String,
    }
    #[derive(Clone, Debug)]
    pub struct Teammate {
        pub id: String,
        pub first_name: String,
        pub last_name: String,
        pub email: String,
        pub is_active: bool,
    }
}
pub mod screencast {
    #[derive(Clone, Debug)]
    pub struct CaptureTarget {
        pub output_name: Option<String>,
        pub logical_width: u32,
        pub logical_height: u32,
        pub logical_origin: Option<(f64, f64)>,
    }
}
/// The capture pieces the UI reads, from the client itself; the recorder isn't needed.
pub mod capture;
/// The instance socket, which `cards` uses to reach a running Hopp. The fixture is that
/// Hopp, so nothing calls these; they only satisfy the types.
pub mod instance {
    use anyhow::Result;
    pub fn request_existing(_: &str, _: crate::protocol::UiRequest) -> Result<bool> {
        Ok(false)
    }
    pub fn watch<T>(
        _: &str,
        _: Option<std::time::Instant>,
        _: impl FnMut(&str) -> Option<T>,
    ) -> Result<Option<T>> {
        Ok(None)
    }
}
pub mod engine {
    use crate::{api::*, frames::*, protocol::InputEvent, screencast::CaptureTarget};
    #[derive(Clone, Debug, Default)]
    pub struct Participant {
        pub identity: String,
        pub name: String,
        pub capturing: bool,
    }
    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
    pub struct CaptureStatus {
        pub id: String,
        pub model: String,
        pub state: String,
        pub percent: Option<u8>,
        pub message: Option<String>,
        pub behind_ms: i64,
        pub lines: u64,
        pub device: Option<String>,
    }
    pub use crate::protocol::UiRequest;
    pub enum Command {
        SignIn {
            server: String,
            token: String,
        },
        Refresh,
        CreateRoom(String),
        JoinRoom(RoomInfo),
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
        /// The UI paints others' marks over your shared monitor; the engine's overlays stand down.
        UiMarks(bool),
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
        CopyClipboard(bool),
        SignOut,
        OverlayColor([u8; 3]),
        StartCapture,
        StopCapture,
        Bookmark(Option<String>),
    }
    pub enum Event {
        LoggedIn(UserInfo),
        Rooms(Vec<RoomInfo>),
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
        ControlGranted(Option<String>),
        PeerControl {
            enabled: bool,
        },
        Error(String),
        AudioDeviceFailed {
            input: bool,
            error: String,
        },
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
        Stamp {
            sender: String,
            message: crate::protocol::StampMessage,
        },
        Devices {
            inputs: Vec<String>,
            outputs: Vec<String>,
            cameras: Vec<(String, String)>,
        },
        Teammates(Vec<Teammate>),
        IncomingCall {
            caller_id: String,
            name: String,
        },
        Missed {
            caller_id: String,
            name: String,
            invite: bool,
            at_ms: i64,
        },
        Calling {
            peer_id: String,
            name: String,
        },
        Activate,
        Ui(UiRequest),
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
        Pointer {
            sender: String,
            point: crate::protocol::Point,
        },
        TransportReconnecting,
        TransportReconnected,
        SignedOut,
        RoomPresence(crate::api::RoomsPresence),
        Capture(Option<CaptureStatus>),
        CaptureChanged,
    }
    pub struct SharedFrames {
        pub remote: FrameSlot,
        pub local_camera: FrameSlot,
        pub cameras: CameraSlots,
    }
    pub struct Engine {
        pub commands: tokio::sync::mpsc::Sender<Command>,
        pub events: std::sync::mpsc::Receiver<Event>,
        pub shared: SharedFrames,
        pub wake: std::sync::Arc<tokio::sync::Notify>,
    }
}
