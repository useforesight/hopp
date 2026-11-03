use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::livekit::audio::AudioTrackHandle;
use crate::livekit::video::VideoBufferManager;

/// Participant attribute Hopp for Linux sets on its audio connection while it captures
/// (transcribes) the call: `"1"` while capturing, empty otherwise.
pub const CAPTURE_ATTRIBUTE: &str = "hopp.capture";

/// Whether these participant attributes say the participant is capturing the call.
pub fn capturing_from(attributes: &HashMap<String, String>) -> bool {
    attributes
        .get(CAPTURE_ATTRIBUTE)
        .is_some_and(|value| value == "1")
}

pub struct ParticipantInfo {
    name: String,
    muted: bool,
    is_speaking: bool,
    is_screensharing: bool,
    /// Capturing (transcribing) the call, from [`CAPTURE_ATTRIBUTE`].
    capturing: bool,
    camera_buffers: Arc<VideoBufferManager>,
    audio_handle: Option<AudioTrackHandle>,
    camera_stop_tx: Option<mpsc::UnboundedSender<()>>,
}

impl std::fmt::Debug for ParticipantInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticipantInfo")
            .field("name", &self.name)
            .field("muted", &self.muted)
            .field("is_speaking", &self.is_speaking)
            .field("capturing", &self.capturing)
            .finish()
    }
}

impl ParticipantInfo {
    pub fn new(name: String, muted: bool, is_speaking: bool) -> Self {
        Self {
            name,
            muted,
            is_speaking,
            is_screensharing: false,
            capturing: false,
            camera_buffers: Arc::new(VideoBufferManager::default()),
            audio_handle: None,
            camera_stop_tx: None,
        }
    }

    /// Creates a ParticipantInfo from a LiveKit remote participant.
    /// Extracts name, muted state (from audio tracks), and speaking state.
    pub fn from_remote_participant(participant: &livekit::participant::RemoteParticipant) -> Self {
        let name = participant.name();
        let is_speaking = participant.is_speaking();

        let mut muted = false;
        for (_, publication) in participant.track_publications() {
            if publication.kind() == livekit::track::TrackKind::Audio {
                muted = publication.is_muted();
                break;
            }
        }

        let mut info = Self::new(name.clone(), muted, is_speaking);
        info.capturing = capturing_from(&participant.attributes());
        info
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn is_speaking(&self) -> bool {
        self.is_speaking
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_is_speaking(&mut self, is_speaking: bool) {
        self.is_speaking = is_speaking;
    }

    pub fn capturing(&self) -> bool {
        self.capturing
    }

    pub fn set_capturing(&mut self, capturing: bool) {
        self.capturing = capturing;
    }

    pub fn is_screensharing(&self) -> bool {
        self.is_screensharing
    }

    pub fn set_is_screensharing(&mut self, is_screensharing: bool) {
        self.is_screensharing = is_screensharing;
    }

    pub fn camera_buffers(&self) -> Arc<VideoBufferManager> {
        self.camera_buffers.clone()
    }

    pub fn camera_active(&self) -> bool {
        !self.camera_buffers.is_inactive()
    }

    pub fn set_audio_handle(&mut self, handle: AudioTrackHandle) {
        self.audio_handle = Some(handle);
    }

    pub fn set_camera_stop_tx(&mut self, tx: mpsc::UnboundedSender<()>) {
        self.camera_stop_tx = Some(tx);
    }

    pub fn stop_audio_stream(&mut self) {
        // Dropping the handle removes source from mixer and aborts task
        self.audio_handle.take();
    }

    pub fn stop_camera_stream(&mut self) {
        self.camera_buffers.set_inactive(true);
        if let Some(tx) = self.camera_stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capturing_follows_the_capture_attribute() {
        let mut attributes = HashMap::new();
        assert!(!capturing_from(&attributes));
        attributes.insert(CAPTURE_ATTRIBUTE.to_string(), "1".to_string());
        assert!(capturing_from(&attributes));
        attributes.insert(CAPTURE_ATTRIBUTE.to_string(), String::new());
        assert!(!capturing_from(&attributes));
    }
}
