use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub type FrameSlot = Arc<Mutex<Option<VideoFrame>>>;
pub type CameraSlots = Arc<Mutex<std::collections::HashMap<String, FrameSlot>>>;

pub fn slot() -> FrameSlot {
    Arc::new(Mutex::new(None))
}
