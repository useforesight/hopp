//! Native LiveKit media. No browser or WebKit is involved in this module.
use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use cpal::{
    BufferSize, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig, StreamError,
    SupportedBufferSize,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use futures_util::StreamExt;
use livekit::{
    options::{TrackPublishOptions, VideoCodec, VideoEncoding},
    prelude::{
        DataPacket, LocalAudioTrack, LocalTrack, LocalVideoTrack, RemoteTrack, Room, RoomEvent,
        RoomOptions, TrackSource,
    },
    webrtc::{
        audio_frame::AudioFrame,
        audio_source::{AudioSourceOptions, RtcAudioSource, native::NativeAudioSource},
        audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions},
        native::{apm::AudioProcessingModule, audio_resampler::AudioResampler},
        video_frame::{VideoFrame as RtcVideoFrame, VideoRotation},
        video_source::{RtcVideoSource, VideoResolution, native::NativeVideoSource},
        video_stream::native::NativeVideoStream,
    },
};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    convert::{CAMERA_PREVIEW_EDGE, ScreenFrame, display_bgra},
    engine::{Event, Participant},
    frames::{CameraSlots, FrameSlot, VideoFrame},
    wake::Sender,
};

const MAX_FRAME_PIXELS: usize = 7680 * 4320;
/// Upstream Hopp's H.264 screen bitrate. Lower caps queue frames behind the encoder's
/// rate control and add latency on busy screens.
const SCREEN_MAX_BITRATE: u64 = 12_000_000;
/// Long-edge caps for an outgoing share and the estimated bandwidth each needs. A share
/// starts at the first so its first frames arrive quickly, then follows WebRTC's
/// bandwidth estimate up (and back down if the link degrades).
const SCREEN_TIERS: [(u32, f64); 4] = [
    (1920, 0.),
    (2560, 2_500_000.),
    (3840, 4_000_000.),
    (u32::MAX, 5_000_000.),
];
/// How far below a tier's need the estimate must fall before stepping down, so the
/// resolution doesn't flap (each change costs a keyframe).
const SCREEN_TIER_HYSTERESIS: f64 = 0.7;
/// Longest average encode that still sustains 30 fps with headroom. The H.264 encoder is
/// single-threaded; past this, frames queue behind it and latency climbs, so the share
/// settles on the largest size this machine encodes in time.
const SCREEN_ENCODE_BUDGET: Duration = Duration::from_millis(28);
const AUDIO_LATENCY_MS: usize = 60;
const AUDIO_BLOCK_SAMPLES: usize = 480;
const MAX_CAPTURE_RATE: u32 = 192000;
const MAX_AUDIO_BLOCK_SAMPLES: usize = MAX_CAPTURE_RATE as usize / 100 * 2;
const AUDIO_PROCESSING_QUEUE: usize = 4;
const AUDIO_MAX_BLOCK_AGE: Duration = Duration::from_millis(60);
type AudioQueues = Arc<Mutex<HashMap<String, VecDeque<f32>>>>;

struct AudioBlock {
    samples: [i16; MAX_AUDIO_BLOCK_SAMPLES],
    len: usize,
    queued: Instant,
    device_delay_ms: u32,
}
impl AudioBlock {
    fn new(samples: &[i16], device_delay_ms: u32) -> Self {
        let mut block = Self {
            samples: [0; MAX_AUDIO_BLOCK_SAMPLES],
            len: samples.len(),
            queued: Instant::now(),
            device_delay_ms,
        };
        block.samples[..samples.len()].copy_from_slice(samples);
        block
    }
}

/// All storage is reserved before starting the device. A callback can drop an
/// old block or skip a contended queue, but it never allocates or waits here.
struct AudioBlockQueue {
    blocks: Mutex<VecDeque<AudioBlock>>,
    ready: tokio::sync::Notify,
}
impl AudioBlockQueue {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            blocks: Mutex::new(VecDeque::with_capacity(AUDIO_PROCESSING_QUEUE)),
            ready: tokio::sync::Notify::new(),
        })
    }

    fn push(&self, block: AudioBlock) {
        if let Ok(mut blocks) = self.blocks.try_lock() {
            if blocks.len() == AUDIO_PROCESSING_QUEUE {
                blocks.pop_front();
            }
            blocks.push_back(block);
            drop(blocks);
            self.ready.notify_one();
        }
    }

    fn try_pop(&self) -> Option<AudioBlock> {
        self.blocks.try_lock().ok()?.pop_front()
    }

    async fn next(&self) -> AudioBlock {
        loop {
            let ready = self.ready.notified();
            if let Some(block) = self.try_pop() {
                return block;
            }
            ready.await;
        }
    }
}

#[derive(Default)]
struct RenderReference {
    sender: Mutex<Option<Arc<AudioBlockQueue>>>,
}

/// The callback never waits for the audio processor. Muting drops the receiver
/// and removes its sender, so no microphone samples or reference backlog persist.
struct RenderSubscription(Arc<RenderReference>);
impl RenderSubscription {
    fn new(reference: Arc<RenderReference>) -> (Self, Arc<AudioBlockQueue>) {
        let queue = AudioBlockQueue::new();
        *reference.sender.lock().unwrap() = Some(queue.clone());
        (Self(reference), queue)
    }
}
impl Drop for RenderSubscription {
    fn drop(&mut self) {
        self.0.sender.lock().unwrap().take();
    }
}

struct AudioProcessor {
    apm: AudioProcessingModule,
    resampler: AudioResampler,
    render_delay_ms: u32,
}
impl AudioProcessor {
    fn new() -> Self {
        Self {
            apm: AudioProcessingModule::new(true, false, true, true),
            resampler: AudioResampler::default(),
            render_delay_ms: 0,
        }
    }

    fn render(&mut self, mut block: AudioBlock) -> Result<()> {
        if block.queued.elapsed() > AUDIO_MAX_BLOCK_AGE {
            return Ok(());
        }
        self.render_delay_ms = block.device_delay_ms;
        self.apm
            .process_reverse_stream(&mut block.samples[..block.len], 48000, 1)?;
        Ok(())
    }

    fn capture(&mut self, block: AudioBlock, rate: u32, channels: u32) -> Result<Vec<i16>> {
        // WebRTC's APM accepts native rates (8/16/32/48 kHz). Preserve capture
        // device compatibility, including 44.1 kHz, by resampling each 10ms block.
        let mut samples = self
            .resampler
            .remix_and_resample(
                &block.samples[..block.len],
                rate / 100,
                channels,
                rate,
                1,
                48000,
            )
            .to_vec();
        if samples.len() != AUDIO_BLOCK_SAMPLES {
            bail!("Microphone resampling did not produce a 10ms audio block");
        }
        let delay = self
            .render_delay_ms
            .saturating_add(block.device_delay_ms)
            .saturating_add(block.queued.elapsed().as_millis().min(500) as u32);
        self.apm.set_stream_delay_ms(delay.min(500) as i32)?;
        self.apm.process_stream(&mut samples, 48000, 1)?;
        Ok(samples)
    }
}

#[derive(Default)]
struct ConnectingRooms(Vec<Arc<Room>>);
impl Drop for ConnectingRooms {
    fn drop(&mut self) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            for room in self.0.drain(..) {
                runtime.spawn(async move {
                    let _ = room.close().await;
                });
            }
        }
    }
}

pub struct Session {
    audio_room: Arc<Room>,
    video_room: Arc<Room>,
    video_source: NativeVideoSource,
    screen_track: LocalVideoTrack,
    screen_edge: Arc<AtomicU32>,
    events: Sender<Event>,
    event_tasks: Vec<JoinHandle<()>>,
    playback: Option<Playback>,
    audio_queues: AudioQueues,
    render_reference: Arc<RenderReference>,
    /// The open microphone device and the work feeding it into `voice`.
    microphone: Option<Microphone>,
    /// Your microphone track, published muted when the call connects and kept for the
    /// whole call: muting mutes it rather than unpublishing it. Other Hopp apps show
    /// you as muted from this track's mute state, so with no track (or a new one after
    /// each unmute) they showed the wrong state.
    voice: Option<(LocalAudioTrack, NativeAudioSource)>,
    camera: Option<Camera>,
    camera_slots: CameraSlots,
    input_device: Option<String>,
    output_device: Option<String>,
    camera_device: Option<String>,
    remote_frame: Arc<Mutex<Option<VideoFrame>>>,
    /// Where call audio goes while it is being captured.
    tap: crate::capture::Tap,
}

/// Participant attribute Hopp for Linux sets on its audio connection while capturing.
pub const CAPTURE_ATTRIBUTE: &str = "hopp.capture";

impl Session {
    pub async fn connect(
        url: &str,
        audio_token: &str,
        video_token: &str,
        events: Sender<Event>,
        remote_frame: Arc<Mutex<Option<VideoFrame>>>,
        camera_slots: CameraSlots,
        tap: crate::capture::Tap,
    ) -> Result<Self> {
        // Show each screen and camera frame as soon as it decodes, as upstream Hopp does:
        // the jitter buffer otherwise holds frames back by 100 ms or more.
        if let Err(error) = livekit::webrtc::enable_zero_playout_delay() {
            let _ = events.send(Event::Error(format!("Low-latency video: {error}")));
        }
        let mut connecting_rooms = ConnectingRooms::default();
        let (audio_room, audio_events) = tokio::time::timeout(
            Duration::from_secs(30),
            Room::connect(url, audio_token, RoomOptions::default()),
        )
        .await
        .context("Audio room connection timed out")??;
        let audio_room = Arc::new(audio_room);
        connecting_rooms.0.push(audio_room.clone());
        let mut video_options = RoomOptions::default();
        video_options.auto_subscribe = false;
        let video_connection = tokio::time::timeout(
            Duration::from_secs(30),
            Room::connect(url, video_token, video_options),
        )
        .await;
        let (video_room, video_events) = match video_connection {
            Ok(Ok(connection)) => connection,
            other => {
                let _ = audio_room.close().await;
                bail!("Video room connection failed: {other:?}");
            }
        };
        let video_room = Arc::new(video_room);
        connecting_rooms.0.push(video_room.clone());
        let video_source = NativeVideoSource::new(
            VideoResolution {
                width: 1920,
                height: 1080,
            },
            true,
        );
        let screen_track = LocalVideoTrack::create_video_track(
            "screen_share",
            RtcVideoSource::Native(video_source.clone()),
        );
        screen_track.mute();
        let publish = video_room
            .local_participant()
            .publish_track(
                LocalTrack::Video(screen_track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Screenshare,
                    video_codec: VideoCodec::H264,
                    video_encoding: Some(VideoEncoding {
                        max_bitrate: SCREEN_MAX_BITRATE,
                        max_framerate: 30.0,
                    }),
                    simulcast: false,
                    ..Default::default()
                },
            )
            .await;
        if let Err(error) = publish {
            let _ = audio_room.close().await;
            let _ = video_room.close().await;
            return Err(error).context("Publishing native screen track");
        }
        let audio_queues = Arc::new(Mutex::new(HashMap::new()));
        let render_reference = Arc::new(RenderReference::default());
        let playback = match Playback::new_selected(
            None,
            events.clone(),
            audio_queues.clone(),
            render_reference.clone(),
        )
        .and_then(Playback::start)
        {
            Ok(playback) => Some(playback),
            Err(error) => {
                let _ = events.send(Event::Error(format!(
                    "Speaker playback unavailable: {error:#}"
                )));
                None
            }
        };
        let audio_output = Some((audio_queues.clone(), 48000, 1, tap.clone()));
        subscribe_screens(&audio_room);
        let audio_task = tokio::spawn(room_events(
            audio_room.clone(),
            audio_events,
            false,
            events.clone(),
            remote_frame.clone(),
            audio_output,
            camera_slots.clone(),
        ));
        let video_task = tokio::spawn(room_events(
            video_room.clone(),
            video_events,
            true,
            events.clone(),
            remote_frame.clone(),
            None,
            camera_slots.clone(),
        ));
        let session = Self {
            audio_room,
            video_room,
            video_source,
            screen_track,
            screen_edge: Arc::new(AtomicU32::new(SCREEN_TIERS[0].0)),
            events,
            event_tasks: vec![audio_task, video_task],
            playback,
            audio_queues,
            render_reference,
            microphone: None,
            voice: None,
            camera: None,
            camera_slots,
            input_device: None,
            output_device: None,
            camera_device: None,
            remote_frame,
            tap,
        };
        connecting_rooms.0.clear();
        let _ = session
            .events
            .send(Event::Participants(session.participants()));
        let _ = session.events.send(Event::Connected);
        Ok(session)
    }

    /// The outgoing screen track, to encode captured frames away from the session loop.
    pub fn screen_publisher(&self) -> ScreenPublisher {
        ScreenPublisher {
            source: self.video_source.clone(),
            track: self.screen_track.clone(),
            max_edge: self.screen_edge.clone(),
        }
    }

    /// Start a share small, then follow the bandwidth estimate up as far as the encoder
    /// keeps pace. Runs until aborted.
    pub fn follow_screen_bandwidth(&self) -> tokio::task::JoinHandle<()> {
        self.screen_edge.store(SCREEN_TIERS[0].0, Ordering::Relaxed);
        let (track, edge) = (self.screen_track.clone(), self.screen_edge.clone());
        tokio::spawn(async move {
            let mut ramp = ScreenRamp::default();
            let mut encoded = (0, 0.);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let (mut available, mut frames, mut seconds) = (0., encoded.0, encoded.1);
                for stats in track.get_stats().await.unwrap_or_default() {
                    match stats {
                        livekit::webrtc::stats::RtcStats::CandidatePair(pair) => {
                            available =
                                f64::max(available, pair.candidate_pair.available_outgoing_bitrate);
                        }
                        livekit::webrtc::stats::RtcStats::OutboundRtp(rtp) => {
                            frames = rtp.outbound.frames_encoded;
                            seconds = rtp.outbound.total_encode_time;
                        }
                        _ => {}
                    }
                }
                // Too few frames (a still screen) says nothing about the encoder's speed.
                let encode = (frames >= encoded.0 + 5).then(|| {
                    Duration::from_secs_f64((seconds - encoded.1) / f64::from(frames - encoded.0))
                });
                encoded = (frames, seconds);
                edge.store(
                    SCREEN_TIERS[ramp.next(available, encode)].0,
                    Ordering::Relaxed,
                );
            }
        })
    }

    pub fn set_sharing(&self, enabled: bool) {
        if enabled {
            self.screen_track.unmute();
        } else {
            self.screen_track.mute();
        }
    }

    pub async fn send_data(
        &self,
        payload: Vec<u8>,
        topic: Option<String>,
        destinations: Vec<String>,
    ) -> Result<()> {
        if payload.len() > 15 * 1024 {
            bail!("Room data exceeds the 15 KiB packet limit; send smaller chunks");
        }
        let reliable = topic.as_deref() != Some("participant_location");
        self.audio_room
            .local_participant()
            .publish_data(DataPacket {
                payload,
                topic,
                reliable,
                destination_identities: destinations.into_iter().map(Into::into).collect(),
            })
            .await
            .context("Sending Hopp room data")
    }

    /// Your microphone track, published muted the first time it's needed.
    async fn voice(&mut self) -> Result<(LocalAudioTrack, NativeAudioSource)> {
        if let Some(voice) = &self.voice {
            return Ok(voice.clone());
        }
        let source = microphone_source();
        let track = LocalAudioTrack::create_audio_track(
            "microphone",
            RtcAudioSource::Native(source.clone()),
        );
        self.audio_room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                },
            )
            .await
            .context("Publishing microphone")?;
        // Muted after publishing, not before: the Mac app updates someone's mute state
        // from mute events, and a track published already muted sends none. Nothing is
        // fed to it in between, so nobody hears anything.
        track.mute();
        self.voice = Some((track.clone(), source.clone()));
        Ok((track, source))
    }

    /// Publish your microphone track, muted, so others see you as muted from the start.
    pub async fn publish_muted_microphone(&mut self) -> Result<()> {
        self.voice().await.map(|_| ())
    }

    /// The device is opened only after an explicit unmute. Muting closes it and mutes
    /// your track, which stays published.
    pub async fn set_microphone(&mut self, enabled: bool) -> Result<()> {
        if !enabled {
            // Dropping it mutes the track and stops the work; the device closes.
            self.microphone.take();
            return Ok(());
        }
        if self.microphone.is_some() {
            return Ok(());
        }
        let (track, source) = self.voice().await?;
        let (mut stream, samples, sample_rate, channels, failed) =
            microphone_device(self.input_device.as_deref(), self.events.clone())?;
        stream
            .play(&samples, &self.events, &failed)
            .context("Starting microphone device")?;
        let events = self.events.clone();
        let worker_track = track.clone();
        let worker_failed = failed.clone();
        let tap = self.tap.clone();
        let me = Arc::new(crate::capture::recorder::Speaker {
            id: canonical_identity(self.audio_room.local_participant().identity().as_str())
                .to_owned(),
            name: self.own_name(),
        });
        let (render_subscription, render_samples) =
            RenderSubscription::new(self.render_reference.clone());
        let processing = tokio::spawn(async move {
            let mut processor = AudioProcessor::new();
            loop {
                let block = samples.next().await;
                // Process exactly the mixed, clipped samples written to CPAL,
                // including silence on underflow, before the near-end signal.
                let result = (|| {
                    while let Some(render) = render_samples.try_pop() {
                        processor.render(render)?;
                    }
                    if block.queued.elapsed() > AUDIO_MAX_BLOCK_AGE {
                        return Ok(None);
                    }
                    processor.capture(block, sample_rate, channels).map(Some)
                })();
                let data = match result {
                    Ok(Some(data)) => data,
                    Ok(None) => continue,
                    Err(error) => {
                        let _ = events.send(Event::Error(format!(
                            "Microphone audio processing failed: {error:#}"
                        )));
                        break;
                    }
                };
                // Your own voice, after echo cancellation: what everyone else hears.
                tap.audio(&me, &data);
                let frame = AudioFrame {
                    samples_per_channel: AUDIO_BLOCK_SAMPLES as u32,
                    data: Cow::Owned(data),
                    sample_rate: 48000,
                    num_channels: 1,
                };
                if let Err(error) = source.capture_frame(&frame).await {
                    let _ = events.send(Event::Error(format!("Microphone stream failed: {error}")));
                    break;
                }
            }
            worker_track.mute();
            worker_failed.store(true, Ordering::Release);
            let _ = events.send(Event::Microphone(false));
        });
        track.unmute();
        self.microphone = Some(Microphone {
            _stream: stream,
            track,
            processing,
            _render_subscription: render_subscription,
            failed,
        });
        Ok(())
    }

    /// Failure events can remain queued after device replacement. The engine
    /// must consult the current microphone before reacting to an old event.
    pub fn microphone_failed(&self) -> bool {
        self.microphone.as_ref().is_some_and(|microphone| {
            microphone.failed.load(Ordering::Acquire) || microphone.processing.is_finished()
        })
    }

    /// Like `microphone_failed`, for the speaker stream.
    pub fn speaker_failed(&self) -> bool {
        self.playback
            .as_ref()
            .is_some_and(|playback| playback.failed.load(Ordering::Acquire))
    }

    /// Close a failed speaker stream straight away: CPAL keeps polling a broken device in a
    /// tight loop until the stream is dropped.
    pub fn close_speaker(&mut self) {
        self.playback.take();
    }

    /// Open the speaker again after a failure, on the chosen device or else the default.
    pub fn reopen_speaker(&mut self) -> Result<()> {
        self.playback.take();
        let open = |device: Option<&str>| {
            Playback::new_selected(
                device,
                self.events.clone(),
                self.audio_queues.clone(),
                self.render_reference.clone(),
            )
            .and_then(Playback::start)
        };
        let playback = match open(self.output_device.as_deref()) {
            Ok(playback) => playback,
            Err(error) if self.output_device.is_some() => open(None).map_err(|_| error)?,
            Err(error) => return Err(error),
        };
        self.playback = Some(playback);
        Ok(())
    }

    pub fn camera_failed(&self) -> bool {
        self.camera
            .as_ref()
            .is_some_and(|camera| camera._capture.failed())
    }

    pub async fn set_camera(&mut self, enabled: bool, local_preview: FrameSlot) -> Result<()> {
        if !enabled {
            if let Some(camera) = self.camera.take() {
                camera.track.mute();
                let sid = camera.track.sid();
                drop(camera);
                self.audio_room
                    .local_participant()
                    .unpublish_track(&sid)
                    .await?;
            }
            return Ok(());
        }
        if self.camera.is_some() {
            return Ok(());
        }
        let source = NativeVideoSource::new(
            VideoResolution {
                width: 1280,
                height: 720,
            },
            false,
        );
        let track =
            LocalVideoTrack::create_video_track("camera", RtcVideoSource::Native(source.clone()));
        track.mute();
        self.audio_room
            .local_participant()
            .publish_track(
                LocalTrack::Video(track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    video_codec: VideoCodec::H264,
                    video_encoding: Some(VideoEncoding {
                        max_bitrate: 1_500_000,
                        max_framerate: 30.0,
                    }),
                    simulcast: false,
                    ..Default::default()
                },
            )
            .await
            .context("Publishing camera")?;
        let capture = match crate::camera::CameraCapture::start(
            self.camera_device.clone(),
            local_preview,
            source,
            self.events.clone(),
        ) {
            Ok(capture) => capture,
            Err(error) => {
                let _ = self
                    .audio_room
                    .local_participant()
                    .unpublish_track(&track.sid())
                    .await;
                return Err(error);
            }
        };
        track.unmute();
        self.camera = Some(Camera {
            track,
            _capture: capture,
        });
        Ok(())
    }

    pub fn select_camera_device(&mut self, device: Option<String>) -> Result<()> {
        if self.camera.is_some() {
            bail!("Turn the camera off before choosing another device");
        }
        if let Some(id) = &device
            && !crate::camera::devices().iter().any(|d| &d.id == id)
        {
            bail!("Selected camera is unavailable");
        }
        self.camera_device = device;
        Ok(())
    }

    pub async fn select_audio_devices(
        &mut self,
        input: Option<String>,
        output: Option<String>,
    ) -> Result<()> {
        let queues = self.audio_queues.clone();
        let next_playback = Playback::new_selected(
            output.as_deref(),
            self.events.clone(),
            queues,
            self.render_reference.clone(),
        )?;
        if let Some(name) = &input
            && !name.starts_with(WAV_MICROPHONE)
            && crate::pipewire_audio::find(name).is_none()
        {
            let _ = selected_audio_device(true, Some(name))?;
        }
        let was_enabled = self.microphone.is_some();
        if was_enabled {
            self.set_microphone(false).await?;
        }
        let old_input = std::mem::replace(&mut self.input_device, input);
        if was_enabled && let Err(error) = self.set_microphone(true).await {
            self.input_device = old_input;
            let _ = self.set_microphone(true).await;
            return Err(error);
        }
        // Build the replacement quietly; two active callbacks must never drain
        // the same playback queues or supply competing acoustic references.
        let previous = self.playback.take();
        let previous_device = previous.as_ref().map(|playback| playback.selected.clone());
        drop(previous);
        self.playback = match next_playback.start() {
            Ok(playback) => {
                self.output_device = output;
                Some(playback)
            }
            Err(error) => {
                if let Some(selected) = previous_device {
                    self.playback = Playback::new_selected(
                        selected.as_deref(),
                        self.events.clone(),
                        self.audio_queues.clone(),
                        self.render_reference.clone(),
                    )
                    .and_then(Playback::start)
                    .ok();
                }
                return Err(error).context("Starting selected speaker");
            }
        };
        Ok(())
    }

    pub fn participants(&self) -> Vec<Participant> {
        participants(&self.audio_room)
    }

    pub fn room_name(&self) -> String {
        self.audio_room.name()
    }

    pub fn own_identity(&self) -> String {
        self.audio_room.local_participant().identity().to_string()
    }

    /// Your display name as the room knows it.
    pub fn own_name(&self) -> String {
        let name = self.audio_room.local_participant().name();
        if name.is_empty() { "You".into() } else { name }
    }

    /// Tell everyone in the call whether you are capturing it. An empty value removes
    /// the attribute.
    pub async fn set_capturing(&self, capturing: bool) -> Result<()> {
        let value = if capturing { "1" } else { "" };
        self.audio_room
            .local_participant()
            .set_attributes(HashMap::from([(CAPTURE_ATTRIBUTE.into(), value.into())]))
            .await
            .context("Telling the call about the capture")
    }

    pub async fn disconnect(&mut self) {
        for task in self.event_tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        self.microphone.take();
        self.voice.take();
        self.camera.take();
        self.playback.take();
        if let Ok(mut slots) = self.camera_slots.lock() {
            slots.clear();
        }
        self.screen_track.mute();
        let _ = self.audio_room.close().await;
        let _ = self.video_room.close().await;
        if let Ok(mut frame) = self.remote_frame.lock() {
            *frame = None;
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in self.event_tasks.drain(..) {
            task.abort();
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let audio = self.audio_room.clone();
            let video = self.video_room.clone();
            runtime.spawn(async move {
                let _ = audio.close().await;
                let _ = video.close().await;
            });
        }
    }
}

fn participants(room: &Room) -> Vec<Participant> {
    let mut result: Vec<_> = room
        .remote_participants()
        .values()
        .filter(|p| {
            !p.identity().as_str().ends_with(":video")
                && !p.identity().as_str().ends_with(":camera")
        })
        .map(|p| {
            let attributes = p.attributes();
            Participant {
                identity: p.identity().to_string(),
                name: p.name(),
                capturing: attributes
                    .get(CAPTURE_ATTRIBUTE)
                    .is_some_and(|value| value == "1"),
            }
        })
        .collect();
    result.sort_by(|a, b| a.name.cmp(&b.name).then(a.identity.cmp(&b.identity)));
    result
}

fn canonical_identity(identity: &str) -> &str {
    identity
        .strip_suffix(":video")
        .or_else(|| identity.strip_suffix(":audio"))
        .or_else(|| identity.strip_suffix(":camera"))
        .unwrap_or(identity)
}

fn audio_identity(identity: &str) -> String {
    if identity.ends_with(":camera") || identity.ends_with(":video") {
        format!("{}:audio", canonical_identity(identity))
    } else {
        identity.to_string()
    }
}

fn subscribe_screens(room: &Room) {
    let own = room.local_participant().identity().to_string();
    for participant in room.remote_participants().values() {
        if canonical_identity(participant.identity().as_str()) == canonical_identity(&own) {
            for publication in participant.track_publications().values() {
                publication.set_subscribed(false);
            }
            continue;
        }
        for publication in participant.track_publications().values() {
            if publication.source() == TrackSource::Screenshare
                || publication.name() == "screen_share"
            {
                publication.set_subscribed(true);
            }
        }
    }
}

#[derive(Default)]
struct TrackTasks(HashMap<String, TrackTask>);
struct TrackTask {
    task: JoinHandle<()>,
    live: Arc<AtomicBool>,
}
impl TrackTask {
    fn new(task: JoinHandle<()>) -> Self {
        Self {
            task,
            live: Arc::new(AtomicBool::new(true)),
        }
    }
    fn abort(&self) {
        self.live.store(false, Ordering::Release);
        self.task.abort();
    }
}
impl Drop for TrackTask {
    fn drop(&mut self) {
        self.abort();
    }
}
impl Drop for TrackTasks {
    fn drop(&mut self) {
        for task in self.0.values() {
            task.abort();
        }
    }
}

/// Decode a remote track into `output`, waking the session for each frame. Cameras pass
/// `short_edge` to shrink frames to tile size before they reach the UI.
fn video_task(
    video: livekit::prelude::RemoteVideoTrack,
    output: FrameSlot,
    events: Sender<Event>,
    short_edge: Option<u32>,
) -> TrackTask {
    let live = Arc::new(AtomicBool::new(true));
    let worker_live = live.clone();
    let task = tokio::spawn(async move {
        let mut stream = NativeVideoStream::new(video.rtc_track());
        while let Some(frame) = stream.next().await {
            let Some(picture) = display_bgra(&mut frame.buffer.to_i420(), short_edge) else {
                continue;
            };
            if let Ok(mut latest) = output.lock() {
                if !worker_live.load(Ordering::Acquire) {
                    break;
                }
                *latest = Some(picture);
            }
            events.wake();
        }
    });
    TrackTask { task, live }
}

fn refresh_screen(
    room: &Room,
    tasks: &mut TrackTasks,
    active: &mut Option<(String, String)>,
    output: &FrameSlot,
    events: &Sender<Event>,
) {
    let own = room.local_participant().identity().to_string();
    let mut available = Vec::new();
    for participant in room.remote_participants().values() {
        let identity = participant.identity().to_string();
        if canonical_identity(&identity) == canonical_identity(&own) {
            continue;
        }
        for publication in participant.track_publications().values() {
            if !publication.is_muted()
                && (publication.source() == TrackSource::Screenshare
                    || publication.name() == "screen_share")
                && let Some(RemoteTrack::Video(track)) = publication.track()
            {
                available.push((publication.sid().to_string(), identity.clone(), track));
            }
        }
    }
    available.sort_by(|a, b| a.0.cmp(&b.0));
    if active
        .as_ref()
        .is_some_and(|(sid, _)| available.iter().any(|(candidate, _, _)| candidate == sid))
    {
        return;
    }
    if let Some((sid, identity)) = active.take() {
        if let Some(task) = tasks.0.remove(&sid) {
            task.abort();
        }
        if let Ok(mut latest) = output.lock() {
            *latest = None;
        }
        let _ = events.send(Event::RemoteShare {
            identity,
            active: false,
        });
    }
    if let Some((sid, identity, track)) = available.into_iter().next() {
        tasks.0.insert(
            sid.clone(),
            video_task(track, output.clone(), events.clone(), None),
        );
        *active = Some((sid, identity.clone()));
        let _ = events.send(Event::RemoteShare {
            identity,
            active: true,
        });
    }
}

async fn room_events(
    room: Arc<Room>,
    mut incoming: mpsc::UnboundedReceiver<RoomEvent>,
    is_video: bool,
    events: Sender<Event>,
    remote_frame: FrameSlot,
    audio_output: Option<(AudioQueues, u32, u32, crate::capture::Tap)>,
    camera_slots: CameraSlots,
) {
    let mut tasks = TrackTasks::default();
    let mut active_screen = None;
    while let Some(event) = incoming.recv().await {
        match event {
            RoomEvent::ParticipantAttributesChanged { .. } if !is_video => {
                let _ = events.send(Event::Participants(participants(&room)));
            }
            RoomEvent::ParticipantConnected(_) | RoomEvent::ParticipantDisconnected(_) => {
                if !is_video {
                    subscribe_screens(&room);
                    let _ = events.send(Event::Participants(participants(&room)));
                    if let Ok(mut slots) = camera_slots.lock() {
                        slots.retain(|identity, _| {
                            room.remote_participants().keys().any(|id| {
                                canonical_identity(id.as_str()) == canonical_identity(identity)
                            })
                        });
                    }
                }
            }
            RoomEvent::TrackPublished {
                publication,
                participant,
            } if !is_video => {
                if canonical_identity(participant.identity().as_str())
                    == canonical_identity(room.local_participant().identity().as_str())
                {
                    publication.set_subscribed(false);
                } else {
                    publication.set_subscribed(true);
                }
            }
            RoomEvent::TrackSubscribed {
                track,
                publication,
                participant,
            } if !is_video => {
                if canonical_identity(participant.identity().as_str())
                    == canonical_identity(room.local_participant().identity().as_str())
                {
                    publication.set_subscribed(false);
                    continue;
                }
                let sid = publication.sid().to_string();
                match track {
                    RemoteTrack::Video(video)
                        if publication.source() == TrackSource::Camera
                            && !publication.is_muted() =>
                    {
                        if let Some(task) = tasks.0.remove(&sid) {
                            task.abort();
                        }
                        let output = {
                            let mut slots = camera_slots.lock().unwrap();
                            slots
                                .entry(audio_identity(participant.identity().as_str()))
                                .or_insert_with(crate::frames::slot)
                                .clone()
                        };
                        tasks.0.insert(
                            sid,
                            video_task(video, output, events.clone(), Some(CAMERA_PREVIEW_EDGE)),
                        );
                    }
                    RemoteTrack::Audio(audio) => {
                        if let Some(old) = tasks.0.remove(&sid) {
                            old.abort();
                        }
                        if let Some((queues, rate, channels, tap)) = audio_output.clone() {
                            let stream_sid = sid.clone();
                            let speaker = Arc::new(crate::capture::recorder::Speaker {
                                id: canonical_identity(participant.identity().as_str()).to_owned(),
                                name: participant.name(),
                            });
                            let task = tokio::spawn(async move {
                                let mut stream = NativeAudioStream::with_options(
                                    audio.rtc_track(),
                                    rate as i32,
                                    channels as i32,
                                    NativeAudioStreamOptions {
                                        queue_size_frames: Some(AUDIO_PROCESSING_QUEUE),
                                    },
                                );
                                let max_samples =
                                    rate as usize * channels as usize * AUDIO_LATENCY_MS / 1000;
                                while let Some(frame) = stream.next().await {
                                    tap.audio(&speaker, &frame.data);
                                    if let Ok(mut all) = queues.lock() {
                                        let queue = all.entry(stream_sid.clone()).or_default();
                                        for sample in frame.data.iter() {
                                            queue.push_back(*sample as f32 / 32768.0);
                                        }
                                        if queue.len() > max_samples {
                                            queue.drain(..queue.len() - max_samples);
                                        }
                                    }
                                }
                                if let Ok(mut all) = queues.lock() {
                                    all.remove(&stream_sid);
                                }
                            });
                            tasks.0.insert(sid, TrackTask::new(task));
                        }
                    }
                    _ => {}
                }
            }
            RoomEvent::TrackMuted {
                publication,
                participant,
            } if !is_video => {
                if publication.source() == TrackSource::Camera {
                    if let Some(task) = tasks.0.remove(&publication.sid().to_string()) {
                        task.abort();
                    }
                    if let Ok(mut slots) = camera_slots.lock() {
                        slots.remove(&audio_identity(participant.identity().as_str()));
                    }
                }
            }
            RoomEvent::TrackUnmuted {
                publication,
                participant,
            } if !is_video => {
                if canonical_identity(participant.identity().as_str())
                    == canonical_identity(room.local_participant().identity().as_str())
                {
                    continue;
                }
                if publication.source() == TrackSource::Camera
                    && let livekit::prelude::TrackPublication::Remote(publication) = publication
                    && let Some(RemoteTrack::Video(video)) = publication.track()
                {
                    let sid = publication.sid().to_string();
                    if let Some(task) = tasks.0.remove(&sid) {
                        task.abort();
                    }
                    let output = {
                        let mut slots = camera_slots.lock().unwrap();
                        slots
                            .entry(audio_identity(participant.identity().as_str()))
                            .or_insert_with(crate::frames::slot)
                            .clone()
                    };
                    tasks.0.insert(
                        sid,
                        video_task(video, output, events.clone(), Some(CAMERA_PREVIEW_EDGE)),
                    );
                }
            }
            RoomEvent::TrackUnsubscribed {
                publication,
                participant,
                ..
            }
            | RoomEvent::TrackUnpublished {
                publication,
                participant,
            } => {
                let sid = publication.sid().to_string();
                if let Some(task) = tasks.0.remove(&sid) {
                    task.abort();
                }
                if let Some((queues, _, _, _)) = &audio_output
                    && let Ok(mut all) = queues.lock()
                {
                    all.remove(&sid);
                }
                if publication.source() == TrackSource::Camera
                    && let Ok(mut slots) = camera_slots.lock()
                {
                    slots.remove(&audio_identity(participant.identity().as_str()));
                }
                if active_screen
                    .as_ref()
                    .is_some_and(|(current, _)| current == &sid)
                {
                    if let Some((_, identity)) = active_screen.take() {
                        let _ = events.send(Event::RemoteShare {
                            identity,
                            active: false,
                        });
                    }
                    if let Ok(mut latest) = remote_frame.lock() {
                        *latest = None;
                    }
                }
            }
            RoomEvent::DataReceived {
                payload,
                topic,
                participant,
                ..
            } if !is_video => {
                let _ = events.send(Event::Data {
                    sender: participant
                        .map(|p| p.identity().to_string())
                        .unwrap_or_default(),
                    payload: (*payload).clone(),
                    topic,
                });
            }
            RoomEvent::Reconnecting if !is_video => {
                let _ = events.send(Event::TransportReconnecting);
            }
            RoomEvent::Reconnected if !is_video => {
                let _ = events.send(Event::TransportReconnected);
            }
            RoomEvent::Disconnected { reason } => {
                let _ = events.send(Event::Disconnected(format!("{reason:?}")));
                break;
            }
            RoomEvent::TrackSubscriptionFailed { error, .. } => {
                let _ = events.send(Event::Error(format!(
                    "Media subscription failed: {error:?}"
                )));
            }
            _ => {}
        }
        if !is_video {
            refresh_screen(
                &room,
                &mut tasks,
                &mut active_screen,
                &remote_frame,
                &events,
            );
        }
    }
    if let Some((_, identity)) = active_screen {
        let _ = events.send(Event::RemoteShare {
            identity,
            active: false,
        });
    }
    if !is_video {
        if let Ok(mut frame) = remote_frame.lock() {
            *frame = None;
        }
        if let Ok(mut slots) = camera_slots.lock() {
            slots.clear();
        }
    }
    if let Some((queues, _, _, _)) = audio_output
        && let Ok(mut all) = queues.lock()
    {
        all.clear();
    }
}

struct Microphone {
    _stream: MicrophoneStream,
    track: LocalAudioTrack,
    processing: JoinHandle<()>,
    _render_subscription: RenderSubscription,
    failed: Arc<AtomicBool>,
}
impl Drop for Microphone {
    fn drop(&mut self) {
        self.track.mute();
        self.processing.abort();
    }
}

/// Speaker name that plays nothing, for scripted test calls: remote audio still reaches
/// the capture, just not the speakers.
pub const SILENT_SPEAKER: &str = "hopp-test:silent";
/// Microphone names starting with this play a WAV file as your voice, in real time,
/// then silence: `hopp-test:wav:/path/to/voice.wav`. For scripted test calls.
pub const WAV_MICROPHONE: &str = "hopp-test:wav:";

struct Playback {
    _stream: Option<Stream>,
    selected: Option<String>,
    failed: Arc<AtomicBool>,
}
impl Playback {
    fn new_selected(
        selected: Option<&str>,
        events: Sender<Event>,
        queues: AudioQueues,
        render_reference: Arc<RenderReference>,
    ) -> Result<Self> {
        if selected == Some(SILENT_SPEAKER) {
            return Ok(Self {
                _stream: None,
                selected: selected.map(str::to_owned),
                failed: Arc::new(AtomicBool::new(false)),
            });
        }
        let device = selected_audio_device(false, selected)?;
        let supported = device
            .supported_output_configs()?
            .filter(|c| {
                c.channels() <= 2
                    && c.channels() > 0
                    && c.min_sample_rate() <= 48000
                    && c.max_sample_rate() >= 48000
            })
            .find(|c| {
                matches!(
                    c.sample_format(),
                    SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
                )
            })
            .context("Selected speaker must support 48 kHz mono/stereo audio")?
            .with_sample_rate(48000);
        let format = supported.sample_format();
        let mut config = low_latency_audio_config(supported.config(), supported.buffer_size());
        let failed = Arc::new(AtomicBool::new(false));
        let build = |config: &StreamConfig| match format {
            SampleFormat::F32 => output_stream::<f32>(
                &device,
                config,
                queues.clone(),
                events.clone(),
                render_reference.clone(),
                failed.clone(),
            ),
            SampleFormat::I16 => output_stream::<i16>(
                &device,
                config,
                queues.clone(),
                events.clone(),
                render_reference.clone(),
                failed.clone(),
            ),
            SampleFormat::U16 => output_stream::<u16>(
                &device,
                config,
                queues.clone(),
                events.clone(),
                render_reference.clone(),
                failed.clone(),
            ),
            other => Err(anyhow::anyhow!(
                "Unsupported speaker sample format: {other:?}"
            )),
        };
        let stream = match build(&config) {
            Ok(stream) => stream,
            Err(error) if matches!(config.buffer_size, BufferSize::Fixed(_)) => {
                config.buffer_size = BufferSize::Default;
                build(&config).with_context(|| {
                    format!("Speaker rejected low-latency and default buffers: {error:#}")
                })?
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            _stream: Some(stream),
            selected: selected.map(str::to_owned),
            failed,
        })
    }

    fn start(self) -> Result<Self> {
        if let Some(stream) = &self._stream {
            stream.play()?;
        }
        Ok(self)
    }
}

/// 20 ms periods: short enough for conversation, but half the wakeups of 10 ms and enough
/// slack that a busy moment (encoding a large shared screen) doesn't starve the device.
fn low_latency_audio_config(
    mut config: StreamConfig,
    supported: &SupportedBufferSize,
) -> StreamConfig {
    if let SupportedBufferSize::Range { min, max } = supported {
        config.buffer_size = BufferSize::Fixed((config.sample_rate / 50).clamp(*min, *max));
    }
    config
}

#[derive(Debug, Clone)]
pub struct AudioDevices {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}
pub fn audio_devices() -> Result<AudioDevices> {
    let host = cpal::default_host();
    // With PipeWire, its sources by name: these include virtual ones (Omalive's mic,
    // filter chains) that the ALSA list lacks, and leave out raw cards PipeWire holds.
    let sources = crate::pipewire_audio::sources();
    let mut inputs: Vec<_> = if sources.is_empty() {
        host.input_devices()?
            .filter_map(|d| {
                d.description()
                    .ok()
                    .map(|description| description.name().to_string())
            })
            .collect()
    } else {
        sources
            .into_iter()
            .map(|source| source.description)
            .collect()
    };
    let mut outputs: Vec<_> = host
        .output_devices()?
        .filter_map(|d| {
            d.description()
                .ok()
                .map(|description| description.name().to_string())
        })
        .collect();
    inputs.sort();
    inputs.dedup();
    outputs.sort();
    outputs.dedup();
    Ok(AudioDevices { inputs, outputs })
}
pub fn camera_devices() -> Vec<crate::camera::CameraDevice> {
    crate::camera::devices()
}
fn selected_audio_device(input: bool, selected: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(name) = selected {
        let mut devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        };
        devices
            .find(|d| {
                d.description()
                    .is_ok_and(|description| description.name() == name)
            })
            .context("Selected audio device is unavailable")
    } else if input {
        host.default_input_device().context("No microphone device")
    } else {
        host.default_output_device().context("No speaker device")
    }
}

struct Camera {
    track: LocalVideoTrack,
    _capture: crate::camera::CameraCapture,
}
impl Drop for Camera {
    fn drop(&mut self) {
        self.track.mute();
    }
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    queues: AudioQueues,
    events: Sender<Event>,
    render_reference: Arc<RenderReference>,
    failed: Arc<AtomicBool>,
) -> Result<Stream> {
    let channels = config.channels as usize;
    let mut reference = [0_i16; AUDIO_BLOCK_SAMPLES];
    let mut reference_len = 0;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], info| {
            let sender = render_reference
                .sender
                .try_lock()
                .ok()
                .and_then(|sender| sender.clone());
            let timestamp = info.timestamp();
            let device_delay_ms = timestamp
                .playback
                .duration_since(&timestamp.callback)
                .unwrap_or_default()
                .as_millis()
                .min(500) as u32;
            let mut all = queues.try_lock().ok();
            if sender.is_none() {
                reference_len = 0;
            }
            for (frame_index, frame) in output.chunks_mut(channels).enumerate() {
                let mixed = all
                    .as_mut()
                    .map(|all| {
                        all.values_mut()
                            .map(|q| q.pop_front().unwrap_or(0.0))
                            .sum::<f32>()
                    })
                    .unwrap_or(0.0)
                    .clamp(-1.0, 1.0);
                for sample in frame {
                    *sample = T::from_sample(mixed);
                }
                if let Some(sender) = &sender {
                    reference[reference_len] = (mixed * 32767.0) as i16;
                    reference_len += 1;
                    if reference_len == AUDIO_BLOCK_SAMPLES {
                        let block_delay_ms = device_delay_ms
                            .saturating_add(((frame_index + 1) * 1000 / 48000) as u32)
                            .saturating_sub(10);
                        sender.push(AudioBlock::new(&reference, block_delay_ms));
                        reference_len = 0;
                    }
                }
            }
        },
        move |error| audio_failed(&failed, &events, false, error),
        None,
    )?)
}

/// A cpal device stream, or a PipeWire source cpal can't open.
enum MicrophoneStream {
    Device(Stream),
    PipeWire(crate::pipewire_audio::Capture),
    /// A WAV file played as the microphone ([`WAV_MICROPHONE`]); stops when dropped.
    File(Vec<i16>, Arc<AtomicBool>),
}

impl Drop for MicrophoneStream {
    fn drop(&mut self) {
        if let Self::File(_, stop) = self {
            stop.store(true, Ordering::Release);
        }
    }
}

type MicrophoneDevice = (
    MicrophoneStream,
    Arc<AudioBlockQueue>,
    u32,
    u32,
    Arc<AtomicBool>,
);
fn microphone_device(selected: Option<&str>, events: Sender<Event>) -> Result<MicrophoneDevice> {
    if let Some(path) = selected.and_then(|name| name.strip_prefix(WAV_MICROPHONE)) {
        let bytes = std::fs::read(path).with_context(|| format!("Reading {path}"))?;
        let (rate, samples) = crate::capture::wav::read(&bytes)?;
        let samples = if rate == 48000 {
            samples
        } else {
            crate::capture::resample::to_48k(rate, &samples)
        };
        return Ok((
            MicrophoneStream::File(samples, Arc::new(AtomicBool::new(false))),
            AudioBlockQueue::new(),
            48000,
            1,
            Arc::new(AtomicBool::new(false)),
        ));
    }
    if let Some(source) = selected.and_then(crate::pipewire_audio::find) {
        return pipewire_microphone(&source.node);
    }
    let device = selected_audio_device(true, selected)?;
    let usable = |config: &cpal::SupportedStreamConfig| {
        config.channels() > 0
            && config.channels() <= 2
            && config.sample_rate() >= 8000
            && config.sample_rate().is_multiple_of(100)
            && config.sample_rate() <= MAX_CAPTURE_RATE
            && matches!(
                config.sample_format(),
                SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16
            )
    };
    let supported = if let Some(default) = device.default_input_config().ok().filter(usable) {
        default
    } else {
        // A multichannel interface can advertise an unusable default while
        // still offering a perfectly usable mono/stereo capture configuration.
        device
            .supported_input_configs()?
            .find_map(|range| {
                [48000, 44100, 32000, 16000, 8000, 96000, 192000]
                    .into_iter()
                    .find(|rate| {
                        range.min_sample_rate() <= *rate && range.max_sample_rate() >= *rate
                    })
                    .map(|rate| range.with_sample_rate(rate))
                    .filter(usable)
            })
            .context("Microphone requires supported mono/stereo PCM capture")?
    };
    let format = supported.sample_format();
    let mut config = low_latency_audio_config(supported.config(), supported.buffer_size());
    let sample_rate = config.sample_rate;
    let channels = config.channels as u32;
    if channels == 0
        || channels > 2
        || !(8000..=MAX_CAPTURE_RATE).contains(&sample_rate)
        || !sample_rate.is_multiple_of(100)
    {
        bail!("Microphone requires mono/stereo PCM and a sample rate divisible by 100");
    }
    let samples = AudioBlockQueue::new();
    let failed = Arc::new(AtomicBool::new(false));
    let build = |config: &StreamConfig| match format {
        SampleFormat::F32 => input_stream::<f32>(
            &device,
            config,
            samples.clone(),
            events.clone(),
            failed.clone(),
        ),
        SampleFormat::I16 => input_stream::<i16>(
            &device,
            config,
            samples.clone(),
            events.clone(),
            failed.clone(),
        ),
        SampleFormat::U16 => input_stream::<u16>(
            &device,
            config,
            samples.clone(),
            events.clone(),
            failed.clone(),
        ),
        other => Err(anyhow::anyhow!(
            "Unsupported microphone sample format: {other:?}"
        )),
    };
    let stream = match build(&config) {
        Ok(stream) => stream,
        Err(error) if matches!(config.buffer_size, BufferSize::Fixed(_)) => {
            config.buffer_size = BufferSize::Default;
            build(&config).with_context(|| {
                format!("Microphone rejected low-latency and default buffers: {error:#}")
            })?
        }
        Err(error) => return Err(error),
    };
    Ok((
        MicrophoneStream::Device(stream),
        samples,
        sample_rate,
        channels,
        failed,
    ))
}

fn microphone_source() -> NativeAudioSource {
    // NativeAudioSource bypasses ADM capture processing. The standalone
    // AudioProcessor supplies real AEC/NS before frames reach this source.
    NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 0)
}

/// A PipeWire source by node name: 48 kHz mono, 10 ms blocks, like the cpal path.
fn pipewire_microphone(node: &str) -> Result<MicrophoneDevice> {
    use crate::pipewire_audio::{Capture, RATE};
    let capture = Capture::open(node)?;
    Ok((
        MicrophoneStream::PipeWire(capture),
        AudioBlockQueue::new(),
        RATE,
        1,
        Arc::new(AtomicBool::new(false)),
    ))
}

impl MicrophoneStream {
    fn play(
        &mut self,
        samples: &Arc<AudioBlockQueue>,
        events: &Sender<Event>,
        failed: &Arc<AtomicBool>,
    ) -> Result<()> {
        match self {
            Self::Device(stream) => Ok(stream.play()?),
            Self::File(voice, stop) => {
                let (voice, stop, samples) = (std::mem::take(voice), stop.clone(), samples.clone());
                std::thread::Builder::new()
                    .name("hopp-test-voice".into())
                    .spawn(move || {
                        // 10 ms blocks on a fixed schedule, so the voice keeps real time.
                        let started = Instant::now();
                        let silence = [0_i16; AUDIO_BLOCK_SAMPLES];
                        for index in 0.. {
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            let from = index * AUDIO_BLOCK_SAMPLES;
                            let block = voice
                                .get(from..(from + AUDIO_BLOCK_SAMPLES).min(voice.len()))
                                .filter(|block| block.len() == AUDIO_BLOCK_SAMPLES)
                                .unwrap_or(&silence);
                            samples.push(AudioBlock::new(block, 10));
                            let next = started + Duration::from_millis(10 * (index as u64 + 1));
                            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                                std::thread::sleep(wait);
                            }
                        }
                    })?;
                Ok(())
            }
            Self::PipeWire(capture) => {
                let samples = samples.clone();
                let mut block = [0_i16; AUDIO_BLOCK_SAMPLES];
                let (events, failed) = (events.clone(), failed.clone());
                capture.start(
                    move |input, delay_ms| {
                        for (sample, value) in block.iter_mut().zip(input) {
                            *sample = (value.clamp(-1.0, 1.0) * 32767.0) as i16;
                        }
                        samples.push(AudioBlock::new(&block[..input.len()], delay_ms));
                    },
                    move |error| {
                        if !failed.swap(true, Ordering::AcqRel) {
                            let _ = events.send(Event::AudioDeviceFailed { input: true, error });
                        }
                    },
                )
            }
        }
    }
}

fn input_stream<T: SizedSample>(
    device: &cpal::Device,
    config: &StreamConfig,
    samples: Arc<AudioBlockQueue>,
    events: Sender<Event>,
    failed: Arc<AtomicBool>,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let sample_rate = config.sample_rate as usize;
    let channels = config.channels as usize;
    let block_size = (sample_rate / 100) * channels;
    let mut pending = [0_i16; MAX_AUDIO_BLOCK_SAMPLES];
    let mut pending_len = 0;
    Ok(device.build_input_stream(
        config,
        move |input: &[T], info| {
            let timestamp = info.timestamp();
            let device_delay_ms = timestamp
                .callback
                .duration_since(&timestamp.capture)
                .unwrap_or_default()
                .as_millis()
                .min(500) as u32;
            for (sample_index, value) in input.iter().enumerate() {
                let value = f32::from_sample(*value).clamp(-1.0, 1.0);
                pending[pending_len] = (value * 32767.0) as i16;
                pending_len += 1;
                if pending_len == block_size {
                    let block_end = (sample_index + 1) / channels;
                    let block_start = block_end.saturating_sub(sample_rate / 100);
                    let block_delay_ms =
                        device_delay_ms.saturating_sub((block_start * 1000 / sample_rate) as u32);
                    samples.push(AudioBlock::new(&pending[..block_size], block_delay_ms));
                    pending_len = 0;
                }
            }
        },
        move |error| audio_failed(&failed, &events, true, error),
        None,
    )?)
}

/// A device stream reported an error. CPAL recovers underruns itself; anything else leaves
/// the stream broken (and, on ALSA, calling this in a tight loop), so report it once and
/// let the engine replace the stream.
fn audio_failed(failed: &AtomicBool, events: &Sender<Event>, input: bool, error: StreamError) {
    if matches!(error, StreamError::BufferUnderrun) || failed.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = events.send(Event::AudioDeviceFailed {
        input,
        error: error.to_string(),
    });
}

/// Encodes captured screen frames; cloneable so it can run on a blocking thread.
#[derive(Clone)]
pub struct ScreenPublisher {
    source: NativeVideoSource,
    track: LocalVideoTrack,
    max_edge: Arc<AtomicU32>,
}

impl ScreenPublisher {
    /// The source adapts to each captured frame's size.
    pub fn publish(&self, frame: ScreenFrame) -> Result<()> {
        let pixels = (frame.width as usize)
            .checked_mul(frame.height as usize)
            .context("Screen dimensions overflow")?;
        if pixels == 0 || pixels > MAX_FRAME_PIXELS {
            bail!("Invalid screen frame dimensions");
        }
        let (width, height) = screen_size(
            frame.width,
            frame.height,
            self.max_edge.load(Ordering::Relaxed),
        );
        let mut buffer = frame.buffer;
        let buffer = if (width, height) == (frame.width, frame.height) {
            buffer
        } else {
            buffer.scale(width as i32, height as i32)
        };
        let mut video_frame = RtcVideoFrame::new(VideoRotation::VideoRotation0, buffer);
        video_frame.timestamp_us = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros() as i64;
        self.track.unmute();
        self.source.capture_frame(&video_frame);
        Ok(())
    }
}

/// Which size tier an outgoing share uses, from second to second.
struct ScreenRamp {
    tier: usize,
    /// Highest tier this machine has encoded in time during this share.
    ceiling: usize,
    /// The tier just changed: the next encode timing includes the new size's keyframe.
    settling: bool,
}

impl Default for ScreenRamp {
    fn default() -> Self {
        Self {
            tier: 0,
            ceiling: SCREEN_TIERS.len() - 1,
            settling: true,
        }
    }
}

impl ScreenRamp {
    /// Follow the bandwidth estimate, but step down (for good) from a size the encoder
    /// can't keep up with.
    fn next(&mut self, available: f64, encode: Option<Duration>) -> usize {
        if !std::mem::take(&mut self.settling)
            && self.tier > 0
            && encode.is_some_and(|encode| encode > SCREEN_ENCODE_BUDGET)
        {
            self.ceiling = self.tier - 1;
        }
        let tier = screen_tier(self.tier, available).min(self.ceiling);
        self.settling = tier != self.tier;
        self.tier = tier;
        self.tier
    }
}

/// The highest tier the estimate supports, stepping down only once it is clearly short.
/// No estimate yet (0) leaves the tier alone.
fn screen_tier(current: usize, available: f64) -> usize {
    if available <= 0. {
        return current;
    }
    let supported = SCREEN_TIERS
        .iter()
        .rposition(|(_, needs)| available >= *needs)
        .unwrap_or(0);
    if supported >= current {
        supported
    } else if available < SCREEN_TIERS[current].1 * SCREEN_TIER_HYSTERESIS {
        current - 1
    } else {
        current
    }
}

/// Encoded size: within the H.264 limits and the current bandwidth tier's long edge.
fn screen_size(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let (width, height) = h264_screen_size(width, height);
    let long = width.max(height);
    if long <= max_edge {
        return (width, height);
    }
    let scale = max_edge as f64 / long as f64;
    (
        ((width as f64 * scale) as u32 / 2 * 2).max(2),
        ((height as f64 * scale) as u32 / 2 * 2).max(2),
    )
}

pub(crate) fn h264_screen_size(width: u32, height: u32) -> (u32, u32) {
    const MAX_PIXELS: f64 = 9_437_184.0;
    const MAX_EDGE: f64 = 4096.0;
    let scale = 1.0_f64
        .min(MAX_EDGE / width as f64)
        .min(MAX_EDGE / height as f64)
        .min((MAX_PIXELS / (width as f64 * height as f64)).sqrt());
    if scale >= 1.0 {
        return (width, height);
    }
    // Even dimensions avoid chroma edge padding after scaling.
    (
        ((width as f64 * scale) as u32 / 2 * 2).max(2),
        ((height as f64 * scale) as u32 / 2 * 2).max(2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn ultrawide_screen_fits_native_h264_encoder() {
        assert_eq!(h264_screen_size(5120, 2160), (4096, 1728));
        assert_eq!(h264_screen_size(3840, 2160), (3840, 2160));
        assert_eq!(h264_screen_size(2160, 5120), (1728, 4096));
        assert_eq!(h264_screen_size(4096, 4096), (3072, 3072));
    }

    #[test]
    fn shares_start_small_and_follow_the_bandwidth_estimate() {
        assert_eq!(screen_size(5120, 2160, SCREEN_TIERS[0].0), (1920, 810));
        assert_eq!(screen_size(5120, 2160, u32::MAX), (4096, 1728));
        assert_eq!(screen_size(1920, 1080, 2560), (1920, 1080));
        // No estimate yet: stay put.
        assert_eq!(screen_tier(0, 0.), 0);
        // A healthy link goes straight to full size; a middling one stops part way.
        assert_eq!(screen_tier(0, 6e6), 3);
        assert_eq!(screen_tier(0, 3e6), 1);
        // Small dips don't flap; a clear drop steps down one tier at a time.
        assert_eq!(screen_tier(3, 4.5e6), 3);
        assert_eq!(screen_tier(3, 2e6), 2);
    }

    #[test]
    fn shares_settle_on_the_largest_size_the_encoder_keeps_up_with() {
        let fast = Some(Duration::from_millis(12));
        let slow = Some(Duration::from_millis(45));
        let mut ramp = ScreenRamp::default();
        assert_eq!(ramp.next(6e6, fast), 3);
        // The second after a change includes its keyframe, so it isn't judged.
        assert_eq!(ramp.next(6e6, slow), 3);
        // Full size is too slow to encode: step down, and stay down despite bandwidth.
        assert_eq!(ramp.next(6e6, slow), 2);
        assert_eq!(ramp.next(6e6, fast), 2);
        assert_eq!(ramp.next(9e6, None), 2);
        // Bandwidth can still push it lower, and a slow encode lower again.
        assert_eq!(ramp.next(2e6, fast), 1);
        assert_eq!(ramp.next(2e6, slow), 1);
        assert_eq!(ramp.next(2e6, slow), 0);
        // The smallest size is never given up.
        let mut ramp = ScreenRamp::default();
        assert_eq!(ramp.next(0., slow), 0);
    }

    fn audio_block(samples: Vec<i16>, device_delay_ms: u32) -> AudioBlock {
        AudioBlock::new(&samples, device_delay_ms)
    }

    #[test]
    fn native_apm_cancels_a_delayed_acoustic_echo() -> Result<()> {
        // Reproducible colored far-end signal and a 50ms two-tap acoustic path.
        // This invokes the real native AEC, with no audio devices or network.
        let blocks = 600;
        let mut state = 0x31415926_u32;
        let mut previous = 0.0_f32;
        let mut far_end = Vec::with_capacity(blocks * AUDIO_BLOCK_SAMPLES);
        for _ in 0..blocks * AUDIO_BLOCK_SAMPLES {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = state as i32 as f32 / i32::MAX as f32;
            previous = previous * 0.75 + noise * 0.25;
            far_end.push((previous * 16000.0) as i16);
        }
        let mut processor = AudioProcessor::new();
        let mut without_reference = AudioProcessor::new();
        let mut before = 0.0;
        let mut after = 0.0;
        let mut no_reference_energy = 0.0;
        let started = Instant::now();
        for block in 0..blocks {
            let offset = block * AUDIO_BLOCK_SAMPLES;
            processor.render(audio_block(
                far_end[offset..offset + AUDIO_BLOCK_SAMPLES].to_vec(),
                50,
            ))?;
            let echo: Vec<i16> = (offset..offset + AUDIO_BLOCK_SAMPLES)
                .map(|sample| {
                    let direct = sample
                        .checked_sub(2400)
                        .map(|at| far_end[at] as f32)
                        .unwrap_or(0.0);
                    let reflection = sample
                        .checked_sub(2557)
                        .map(|at| far_end[at] as f32)
                        .unwrap_or(0.0);
                    (direct * 0.55 + reflection * 0.15) as i16
                })
                .collect();
            if block >= 400 {
                before += echo
                    .iter()
                    .map(|sample| (*sample as f64).powi(2))
                    .sum::<f64>();
            }
            without_reference.render(audio_block(vec![0; AUDIO_BLOCK_SAMPLES], 50))?;
            let baseline = without_reference.capture(audio_block(echo.clone(), 0), 48000, 1)?;
            let result = processor.capture(audio_block(echo, 0), 48000, 1)?;
            if block >= 400 {
                after += result
                    .iter()
                    .map(|sample| (*sample as f64).powi(2))
                    .sum::<f64>();
                no_reference_energy += baseline
                    .iter()
                    .map(|sample| (*sample as f64).powi(2))
                    .sum::<f64>();
            }
        }
        let reduction_db = 10.0 * (before / after.max(1.0)).log10();
        let reference_improvement_db = 10.0 * (no_reference_energy / after.max(1.0)).log10();
        println!(
            "APM: delayed echo reduction {reduction_db:.1} dB, actual reverse reference improved suppression by {reference_improvement_db:.1} dB; mean per processor/10ms block {:.3}ms (includes synthetic test overhead)",
            started.elapsed().as_secs_f64() * 1000.0 / blocks as f64 / 2.0
        );
        assert!(
            reference_improvement_db > 15.0,
            "Suppression did not depend on the actual speaker reference: {reference_improvement_db:.1} dB"
        );
        assert!(
            reduction_db > 15.0,
            "Native AEC did not suppress the actual reference echo: {reduction_db:.1} dB"
        );

        // A near-end signal must remain audible when the speaker is silent.
        let mut near_energy = 0.0;
        for block in 0..100 {
            processor.render(audio_block(vec![0; AUDIO_BLOCK_SAMPLES], 0))?;
            let near: Vec<_> = (0..AUDIO_BLOCK_SAMPLES)
                .map(|sample| {
                    let time = (block * AUDIO_BLOCK_SAMPLES + sample) as f64 / 48000.0;
                    ((time * 2.0 * std::f64::consts::PI * 310.0).sin() * 6000.0
                        + (time * 2.0 * std::f64::consts::PI * 730.0).sin() * 3000.0)
                        as i16
                })
                .collect();
            let result = processor.capture(audio_block(near, 0), 48000, 1)?;
            if block >= 50 {
                near_energy += result
                    .iter()
                    .map(|sample| (*sample as f64).powi(2))
                    .sum::<f64>();
            }
        }
        assert!(
            near_energy > 1e10,
            "AEC suppressed the near-end signal with no far-end audio"
        );
        Ok(())
    }

    #[test]
    fn processing_resamples_devices_and_releases_bounded_reference() -> Result<()> {
        for _ in 0..24 {
            let reference = Arc::new(RenderReference::default());
            let weak = Arc::downgrade(&reference);
            let (subscription, receiver) = RenderSubscription::new(reference.clone());
            let queue_weak = Arc::downgrade(&receiver);
            let sender = reference.sender.lock().unwrap().as_ref().unwrap().clone();
            for sample in 0..=AUDIO_PROCESSING_QUEUE {
                sender.push(audio_block(vec![sample as i16; 480], 0));
            }
            assert_eq!(
                receiver.blocks.lock().unwrap().len(),
                AUDIO_PROCESSING_QUEUE
            );
            assert_eq!(
                receiver.try_pop().unwrap().samples[0],
                1,
                "Overflow retained the old audio instead of the latest blocks"
            );
            let mut processor = AudioProcessor::new();
            while let Some(block) = receiver.try_pop() {
                processor.render(block)?;
            }
            let output = processor.capture(audio_block(vec![0; 441 * 2], 0), 44100, 2)?;
            assert_eq!(output.len(), 480);
            assert!(output.iter().all(|sample| *sample == 0));
            drop(subscription);
            assert!(reference.sender.lock().unwrap().is_none());
            drop(sender);
            drop(receiver);
            assert!(
                queue_weak.upgrade().is_none(),
                "Reference retained PCM after teardown"
            );
            drop(reference);
            assert!(
                weak.upgrade().is_none(),
                "Audio callback reference retained after teardown"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn processing_queue_wakes_without_losing_a_block_and_cancels() -> Result<()> {
        let queue = AudioBlockQueue::new();
        let worker_queue = queue.clone();
        let worker = tokio::spawn(async move { worker_queue.next().await.samples[0] });
        tokio::task::yield_now().await;
        queue.push(audio_block(vec![42; 480], 0));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), worker).await??,
            42
        );
        let weak = Arc::downgrade(&queue);
        let worker_queue = queue.clone();
        let worker = tokio::spawn(async move {
            let _processor = AudioProcessor::new();
            worker_queue.next().await;
        });
        tokio::task::yield_now().await;
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
        drop(queue);
        assert!(
            weak.upgrade().is_none(),
            "Cancelled processor retained its capture queue"
        );
        Ok(())
    }

    #[derive(Deserialize)]
    struct FixtureUser {
        token: String,
    }
    #[derive(Deserialize)]
    struct Fixture {
        api_base: String,
        livekit_url: String,
        room_id: String,
        publisher: FixtureUser,
        viewer: FixtureUser,
    }

    /// Uses dedicated accounts on a running local Hopp backend. Never records
    /// the microphone or opens a camera: media inputs are synthetic patterns.
    #[tokio::test]
    #[ignore = "requires a local Hopp/LiveKit server and HOPP_API_TEST_FIXTURE"]
    async fn native_hopp_round_trip() -> Result<()> {
        let fixture: Fixture = serde_json::from_slice(&std::fs::read(
            std::env::var("HOPP_API_TEST_FIXTURE")
                .context("Set HOPP_API_TEST_FIXTURE to a private local two-user API fixture")?,
        )?)?;
        let publisher_api =
            crate::api::ApiClient::new(&fixture.api_base, Some(fixture.publisher.token))?;
        let viewer_api = crate::api::ApiClient::new(&fixture.api_base, Some(fixture.viewer.token))?;
        let publisher_tokens = publisher_api.join_room(&fixture.room_id).await?;
        let viewer_tokens = viewer_api.join_room(&fixture.room_id).await?;
        let (publisher_events, _) = crate::wake::detached();
        let (viewer_events, viewer_rx) = crate::wake::detached();
        let remote_screen = crate::frames::slot();
        let remote_cameras: CameraSlots = Arc::default();
        let mut publisher = Session::connect(
            &fixture.livekit_url,
            &publisher_tokens.audio_token,
            &publisher_tokens.video_token,
            publisher_events,
            crate::frames::slot(),
            Arc::default(),
            crate::capture::Tap::default(),
        )
        .await?;
        let mut viewer = Session::connect(
            &fixture.livekit_url,
            &viewer_tokens.audio_token,
            &viewer_tokens.video_token,
            viewer_events,
            remote_screen.clone(),
            remote_cameras.clone(),
            crate::capture::Tap::default(),
        )
        .await?;
        // Decode verification is silent; hardware capture remains disabled.
        publisher.playback.take();
        viewer.playback.take();
        assert!(publisher.microphone.is_none());
        assert!(viewer.microphone.is_none());
        assert_eq!(publisher.room_name(), fixture.room_id);
        for (width, height) in [(320, 180), (5120, 2160)] {
            let mut bgra = Vec::with_capacity(width * height * 4);
            for _ in 0..height {
                for x in 0..width {
                    bgra.extend_from_slice(if x < width / 2 {
                        &[10, 20, 220, 255]
                    } else {
                        &[220, 20, 10, 255]
                    });
                }
            }
            // Full size from the start; the bandwidth tiers are tested separately.
            publisher.screen_edge.store(u32::MAX, Ordering::Relaxed);
            let screen = publisher.screen_publisher();
            let expected = h264_screen_size(width as u32, height as u32);
            let mut video_ok = false;
            for _ in 0..150 {
                screen.publish(ScreenFrame::from_bgra(width as u32, height as u32, &bgra)?)?;
                tokio::time::sleep(Duration::from_millis(33)).await;
                let frame = remote_screen.lock().unwrap();
                if let Some(frame) = frame.as_ref() {
                    let left = (frame.height as usize / 2 * frame.width as usize
                        + frame.width as usize / 4)
                        * 4;
                    let right = (frame.height as usize / 2 * frame.width as usize
                        + frame.width as usize * 3 / 4)
                        * 4;
                    if (frame.width, frame.height) == expected
                        && frame.bgra[left + 2] > 150
                        && frame.bgra[left] < 70
                        && frame.bgra[right] > 150
                        && frame.bgra[right + 2] < 70
                    {
                        video_ok = true;
                        break;
                    }
                }
            }
            assert!(video_ok, "Native H264 screen did not preserve size/color");
        }
        assert!(
            viewer_rx
                .try_iter()
                .any(|event| matches!(event, Event::RemoteShare { active: true, .. })),
            "No selected screen source was announced"
        );
        publisher
            .send_data(
                b"hopp-native-probe".to_vec(),
                Some("native-probe".into()),
                vec![viewer.own_identity()],
            )
            .await?;
        let mut data_ok = false;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if viewer_rx.try_iter().any(|event|matches!(event,Event::Data{payload,topic,..} if payload==b"hopp-native-probe" && topic.as_deref()==Some("native-probe"))) {data_ok=true;break;}
        }
        assert!(data_ok, "Reliable room data did not arrive");
        publisher.set_sharing(false);
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if remote_screen.lock().unwrap().is_none() {
                break;
            }
        }
        assert!(
            remote_screen.lock().unwrap().is_none(),
            "Muted screen retained a frozen frame"
        );
        assert!(
            viewer_rx
                .try_iter()
                .any(|event| matches!(event, Event::RemoteShare { active: false, .. })),
            "Muted screen did not release its selected source"
        );

        let camera_source = NativeVideoSource::new(
            VideoResolution {
                width: 320,
                height: 180,
            },
            false,
        );
        let camera_track = LocalVideoTrack::create_video_track(
            "camera",
            RtcVideoSource::Native(camera_source.clone()),
        );
        let camera_publication = publisher
            .audio_room
            .local_participant()
            .publish_track(
                LocalTrack::Video(camera_track.clone()),
                TrackPublishOptions {
                    source: TrackSource::Camera,
                    video_codec: VideoCodec::H264,
                    simulcast: false,
                    ..Default::default()
                },
            )
            .await?;
        let mut camera_ok = false;
        for _ in 0..100 {
            let mut buffer = livekit::webrtc::prelude::I420Buffer::new(320, 180);
            let (y, u, v) = buffer.data_mut();
            y.fill(100);
            u.fill(80);
            v.fill(180);
            camera_source.capture_frame(&RtcVideoFrame::new(VideoRotation::VideoRotation0, buffer));
            tokio::time::sleep(Duration::from_millis(33)).await;
            if let Some(slot) = remote_cameras
                .lock()
                .unwrap()
                .get(&publisher.own_identity())
                && let Some(frame) = slot.lock().unwrap().as_ref()
                && frame.width == 320
                && frame.height == 180
            {
                camera_ok = true;
                break;
            }
        }
        assert!(
            camera_ok,
            "Native camera track did not produce a separate participant preview"
        );
        camera_track.mute();
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if remote_cameras.lock().unwrap().is_empty() {
                break;
            }
        }
        assert!(
            remote_cameras.lock().unwrap().is_empty(),
            "Muted camera retained a frozen participant preview"
        );
        publisher
            .audio_room
            .local_participant()
            .unpublish_track(&camera_publication.sid())
            .await?;

        let audio_source = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 0);
        let audio_track = LocalAudioTrack::create_audio_track(
            "microphone",
            RtcAudioSource::Native(audio_source.clone()),
        );
        publisher
            .audio_room
            .local_participant()
            .publish_track(
                LocalTrack::Audio(audio_track),
                TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                },
            )
            .await?;
        let received = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                for participant in viewer.audio_room.remote_participants().values() {
                    for publication in participant.track_publications().values() {
                        if let Some(RemoteTrack::Audio(track)) = publication.track() {
                            return track;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("Audio subscription timed out")?;
        let mut audio_stream = NativeAudioStream::new(received.rtc_track(), 48000, 1);
        let audio_task = tokio::spawn(async move {
            for block in 0..200 {
                let data: Vec<i16> = (0..480)
                    .map(|n| {
                        let phase =
                            (block * 480 + n) as f32 * 440.0 * std::f32::consts::TAU / 48000.0;
                        (phase.sin() * 8000.0) as i16
                    })
                    .collect();
                audio_source
                    .capture_frame(&AudioFrame {
                        data: Cow::Owned(data),
                        sample_rate: 48000,
                        num_channels: 1,
                        samples_per_channel: 480,
                    })
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let audio_result = tokio::time::timeout(Duration::from_secs(8), async {
            let mut samples = 0usize;
            let mut energy = 0u64;
            while let Some(frame) = audio_stream.next().await {
                samples += frame.data.len();
                energy += frame
                    .data
                    .iter()
                    .map(|s| s.unsigned_abs() as u64)
                    .sum::<u64>();
                if samples >= 9600 && energy > 1000000 {
                    return true;
                }
            }
            false
        })
        .await;
        audio_task.abort();
        let _ = audio_task.await;
        assert!(
            audio_result.unwrap_or(false),
            "Native Opus audio did not preserve a synthetic signal"
        );
        publisher.disconnect().await;
        viewer.disconnect().await;
        assert!(remote_screen.lock().unwrap().is_none());
        assert!(remote_cameras.lock().unwrap().is_empty());
        println!(
            "Native Hopp round trip passed: H264 screen/camera, muted preview cleanup, reliable data, Opus PCM; hardware capture remained off."
        );
        Ok(())
    }
}
