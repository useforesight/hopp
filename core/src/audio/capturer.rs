use livekit::webrtc::native::audio_resampler::AudioResampler;
use rodio::microphone::{self, Input, MicrophoneBuilder};
use rodio::Source;
use std::collections::{HashMap, HashSet};
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thread_priority::{set_current_thread_priority, ThreadPriority};
use tokio::sync::mpsc;

use super::capture_health::{silence_timeout_before_signal, CaptureHealth, CaptureStallReason};

const TARGET_SAMPLE_RATE: u32 = 16000;
const TARGET_CHANNELS: u16 = 1;
pub const SAMPLES_DIVIDER: u32 = 100;
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const CAPTURE_MONITOR_INTERVAL: Duration = Duration::from_millis(500);
const MAX_STALL_RECOVERY_ATTEMPTS: u32 = 3;
const FAILED_RESTART_RETRY_DELAY: Duration = Duration::from_secs(3);

/// Suffix duplicate names with `(2)`, `(3)`, ..., skipping suffixes that collide
/// with existing raw names.
fn assign_ui_names(raw: &[String]) -> Vec<String> {
    let mut taken: HashSet<String> = raw.iter().cloned().collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut emitted_count: HashMap<&str, u32> = HashMap::new();
    for name in raw {
        let count = emitted_count.entry(name.as_str()).or_insert(0);
        if *count == 0 {
            out.push(name.clone());
        } else {
            let mut n = *count + 1;
            loop {
                let candidate = format!("{name} ({n})");
                if !taken.contains(&candidate) {
                    taken.insert(candidate.clone());
                    out.push(candidate);
                    break;
                }
                n += 1;
            }
        }
        *count += 1;
    }
    out
}

pub fn list_audio_inputs() -> Vec<socket_lib::AudioDevice> {
    match microphone::available_inputs() {
        Ok(mics) => {
            let raw: Vec<String> = mics.iter().map(|m| m.to_string()).collect();
            let ui = assign_ui_names(&raw);
            ui.into_iter()
                .enumerate()
                .map(|(i, name)| socket_lib::AudioDevice {
                    name,
                    default: i == 0,
                })
                .collect()
        }
        Err(e) => {
            log::error!("Failed to list audio inputs: {e}");
            vec![]
        }
    }
}

#[derive(Debug)]
struct AudioDevice {
    name: String,
    ui_name: String,
    input: Input,
}

/// Accumulates RMS over captured samples and emits mic level at a fixed cadence.
struct LevelEmitter {
    socket: socket_lib::SocketSender,
    sum_sq: f64,
    sample_count: usize,
    last_emit: std::time::Instant,
}

impl LevelEmitter {
    const EMIT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

    fn new(socket: socket_lib::SocketSender) -> Self {
        Self {
            socket,
            sum_sq: 0.0,
            sample_count: 0,
            last_emit: std::time::Instant::now(),
        }
    }

    fn observe(&mut self, sample: f32) {
        self.sum_sq += (sample as f64) * (sample as f64);
        self.sample_count += 1;
    }

    fn emit_if_due(&mut self) {
        if self.last_emit.elapsed() < Self::EMIT_INTERVAL || self.sample_count == 0 {
            return;
        }
        let rms = (self.sum_sq / self.sample_count as f64).sqrt() as f32;
        let level = rms.clamp(0.0, 1.0);
        if let Err(e) = self
            .socket
            .send(socket_lib::Message::MicrophoneAudioLevel(level))
        {
            log::warn!("capturer: failed to send mic level: {e:?}");
        }
        self.sum_sq = 0.0;
        self.sample_count = 0;
        self.last_emit = std::time::Instant::now();
    }
}

pub enum CaptureStallOutcome {
    /// Reported by a stream that has since been replaced or stopped, or nothing the
    /// user needs to hear about (yet: a failed restart gets retried).
    Ignored,
    Restarted,
    /// Reopening the stream did not bring sound back.
    GaveUp {
        device_name: String,
    },
}

pub struct Capturer {
    available_devices: Vec<AudioDevice>,
    stop_flag: Arc<AtomicBool>,
    sample_tx: Option<mpsc::UnboundedSender<Vec<i16>>>,
    active_device_name: Option<String>,
    _device_monitor: super::device_monitor::DeviceMonitor,
    socket: socket_lib::SocketSender,
    proxy: winit::event_loop::EventLoopProxy<crate::UserEvent>,
    capture_id: u64,
    // Counted apart, so a mic that's silent because it's noise gated can't use up the
    // restarts for a stream that really has stopped.
    no_samples_restarts: u32,
    silence_restarts: u32,
    stall_reported_to_user: bool,
    has_had_signal_this_session: bool,
}

impl Capturer {
    pub fn new(
        proxy: winit::event_loop::EventLoopProxy<crate::UserEvent>,
        socket: socket_lib::SocketSender,
    ) -> Self {
        let proxy_for_monitor = proxy.clone();
        Self {
            available_devices: vec![],
            stop_flag: Arc::new(AtomicBool::new(false)),
            sample_tx: None,
            active_device_name: None,
            _device_monitor: super::device_monitor::DeviceMonitor::new(
                super::device_monitor::DeviceKind::Input,
                proxy_for_monitor,
            )
            .expect("Failed to start input device monitor"),
            socket,
            proxy,
            capture_id: 0,
            no_samples_restarts: 0,
            silence_restarts: 0,
            stall_reported_to_user: false,
            has_had_signal_this_session: false,
        }
    }

    pub fn list_sources(&mut self) -> Vec<socket_lib::AudioDevice> {
        let mics = microphone::available_inputs().unwrap_or_default();
        let raw_names: Vec<String> = mics.iter().map(|m| m.to_string()).collect();
        let ui_names = assign_ui_names(&raw_names);
        self.available_devices = mics
            .into_iter()
            .zip(raw_names)
            .zip(ui_names)
            .map(|((input, name), ui_name)| AudioDevice {
                name,
                ui_name,
                input,
            })
            .collect();
        self.available_devices
            .iter()
            .enumerate()
            .map(|(i, dev)| socket_lib::AudioDevice {
                name: dev.ui_name.clone(),
                default: i == 0,
            })
            .collect()
    }

    // Instead of reporting the device name to the frontend we are
    // sending a custom name we create which handles conflicts. For example
    // if we have two mics with name Mic, the first will be Mic and the second will
    // be Mic 2. We are making the assumption that during a session if the mics are not
    // plugged/unplugged will keep their relative order. Then we are using this name to match
    // with our internal vector which has stored the actual mic devices.
    pub fn start_capture(
        &mut self,
        device_name: Option<&str>,
        sample_tx: mpsc::UnboundedSender<Vec<i16>>,
    ) -> Result<u32, String> {
        self.reset_stall_history();
        self.open_capture(device_name, sample_tx)
    }

    /// Starts capture, falling back to the default device if the requested one
    /// can't be opened, so a broken device doesn't stop a call from starting.
    pub fn start_capture_with_fallback(
        &mut self,
        device_name: Option<&str>,
        sample_tx: mpsc::UnboundedSender<Vec<i16>>,
    ) -> Result<u32, String> {
        self.reset_stall_history();
        self.open_capture_with_fallback(device_name, sample_tx)
    }

    /// Each device has its own history, so this runs whenever one gets picked.
    fn reset_stall_history(&mut self) {
        self.no_samples_restarts = 0;
        self.silence_restarts = 0;
        self.has_had_signal_this_session = false;
    }

    fn open_capture_with_fallback(
        &mut self,
        device_name: Option<&str>,
        sample_tx: mpsc::UnboundedSender<Vec<i16>>,
    ) -> Result<u32, String> {
        match self.open_capture(device_name, sample_tx.clone()) {
            Err(error) if device_name.is_some() => {
                log::warn!("Failed to open mic {device_name:?}: {error}, trying the default");
                self.list_sources();
                self.open_capture(None, sample_tx)
            }
            result => result,
        }
    }

    fn open_capture(
        &mut self,
        device_name: Option<&str>,
        sample_tx: mpsc::UnboundedSender<Vec<i16>>,
    ) -> Result<u32, String> {
        // Stop any existing capture before spawning a new one
        if self.sample_tx.is_some() {
            self.stop_thread();
        }

        let builder = MicrophoneBuilder::new();

        let mut fell_back_to_default = false;
        let chosen: Option<&AudioDevice> = device_name.and_then(|name| {
            self.available_devices
                .iter()
                .find(|d| d.ui_name == name)
                .or_else(|| self.available_devices.iter().find(|d| d.name == name))
        });
        let builder = match chosen {
            Some(device) => match builder.device(device.input.clone()) {
                Ok(b) => b,
                Err(e) => {
                    log::warn!("Failed to set device: {}, falling back to default", e);
                    fell_back_to_default = true;
                    builder
                        .default_device()
                        .map_err(|e| format!("Failed to get default device: {}", e))?
                }
            },
            None => {
                if let Some(name) = device_name {
                    log::warn!("Device not found: {}, falling back to default", name);
                    fell_back_to_default = true;
                }
                builder
                    .default_device()
                    .map_err(|e| format!("Failed to get default device: {}", e))?
            }
        };

        // Try to configure for 16kHz mono, fall back to default if not supported
        let target_sample_rate = NonZero::new(TARGET_SAMPLE_RATE).unwrap();
        let target_channels = NonZero::new(TARGET_CHANNELS).unwrap();

        let mic = match builder
            .default_config()
            .and_then(|b| b.try_sample_rate(target_sample_rate))
            .and_then(|b| b.try_channels(target_channels))
        {
            Ok(configured) => {
                log::info!("Hardware supports 16kHz mono, using native config");
                configured
                    .open_stream()
                    .map_err(|e| format!("Failed to open stream: {}", e))?
            }
            Err(_) => {
                log::info!("Hardware doesn't support 16kHz mono, will resample");
                builder
                    .default_config()
                    .map_err(|e| format!("Failed to get default config: {}", e))?
                    .open_stream()
                    .map_err(|e| format!("Failed to open stream: {}", e))?
            }
        };

        let actual_sample_rate = mic.sample_rate().get();
        let actual_channels = mic.channels();

        let num_channels = actual_channels.get() as usize;

        if actual_sample_rate != TARGET_SAMPLE_RATE || actual_channels.get() != TARGET_CHANNELS {
            log::info!(
                "Audio conversion: {}Hz {}ch → {}Hz {}ch",
                actual_sample_rate,
                actual_channels.get(),
                TARGET_SAMPLE_RATE,
                TARGET_CHANNELS,
            );
        }

        // Reset and clone the stop flag
        self.stop_flag.store(false, Ordering::Relaxed);
        let stop_flag = Arc::clone(&self.stop_flag);

        self.active_device_name = if device_name.is_none() || fell_back_to_default {
            use cpal::traits::{DeviceTrait, HostTrait};
            #[allow(deprecated)]
            let cpal_raw = cpal::default_host()
                .default_input_device()
                .and_then(|d| d.name().ok());
            cpal_raw.map(|raw| {
                self.available_devices
                    .iter()
                    .find(|d| d.name == raw)
                    .map(|d| d.ui_name.clone())
                    .unwrap_or(raw)
            })
        } else {
            chosen.map(|d| d.ui_name.clone())
        };
        self.sample_tx = Some(sample_tx.clone());

        self.capture_id += 1;
        let capture_id = self.capture_id;
        let health = Arc::new(CaptureHealth::new(
            Instant::now(),
            silence_timeout_before_signal(self.has_had_signal_this_session, self.silence_restarts),
        ));
        spawn_capture_monitor(
            capture_id,
            Arc::clone(&health),
            Arc::clone(&stop_flag),
            self.proxy.clone(),
        );

        let socket_for_level = self.socket.clone();

        std::thread::spawn(move || {
            match set_current_thread_priority(ThreadPriority::Max) {
                Ok(()) => log::info!("Microphone capture thread priority set to maximum"),
                Err(error) => {
                    log::warn!("Failed to raise microphone capture thread priority: {error}")
                }
            }
            let mut mic = mic;
            let mut resampler = AudioResampler::default();
            let mut level = LevelEmitter::new(socket_for_level.clone());

            let in_samples_per_frame = actual_sample_rate / SAMPLES_DIVIDER;
            let samples_needed = in_samples_per_frame as usize * num_channels;
            let mut scratch_i16: Vec<i16> = Vec::with_capacity(samples_needed);
            let mut frame_has_signal = false;

            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }

                // Microphone::next() sleeps forever once a stream stops delivering,
                // so only read what the ring buffer says is there. Otherwise this
                // thread could never see stop_flag and the device would stay open.
                // CaptureHealth is what notices that the samples stopped coming.
                let available_samples = mic.size_hint().0;
                if available_samples == 0 {
                    std::thread::sleep(CAPTURE_POLL_INTERVAL);
                    continue;
                }

                let samples_to_read = available_samples.min(samples_needed - scratch_i16.len());
                for sample in mic.by_ref().take(samples_to_read) {
                    frame_has_signal |= sample != 0.0;
                    let clamped = sample.clamp(-1.0, 1.0);
                    level.observe(clamped);
                    scratch_i16.push((clamped * i16::MAX as f32) as i16);
                }

                if scratch_i16.len() < samples_needed {
                    continue;
                }

                health.record_frame(Instant::now(), frame_has_signal);
                frame_has_signal = false;

                let output = resampler.remix_and_resample(
                    &scratch_i16,
                    in_samples_per_frame,
                    actual_channels.get() as u32,
                    actual_sample_rate,
                    TARGET_CHANNELS as u32,
                    TARGET_SAMPLE_RATE,
                );

                if sample_tx.send(output.to_vec()).is_err() {
                    break;
                }

                scratch_i16.clear();
                level.emit_if_due();
            }

            let _ = socket_for_level.send(socket_lib::Message::MicrophoneAudioLevel(0.0));
        });

        Ok(TARGET_SAMPLE_RATE)
    }

    pub fn is_capturing(&self) -> bool {
        self.sample_tx.is_some()
    }

    pub fn switch_device(&mut self, device_name: Option<&str>) -> Result<u32, String> {
        let sample_tx = self
            .sample_tx
            .clone()
            .ok_or_else(|| "No active capture to switch".to_string())?;
        self.list_sources();
        self.stop_thread();
        let result = self.start_capture(device_name, sample_tx.clone());
        if result.is_err() {
            // Still in the call, so picking another device has to keep working.
            self.sample_tx = Some(sample_tx);
        }
        result
    }

    pub fn active_device_name(&self) -> Option<&str> {
        self.active_device_name.as_deref()
    }

    /// Does what reselecting the device by hand does: re-enumerates and reopens it.
    pub fn handle_capture_stalled(
        &mut self,
        capture_id: u64,
        reason: CaptureStallReason,
    ) -> CaptureStallOutcome {
        if capture_id != self.capture_id {
            return CaptureStallOutcome::Ignored;
        }
        let Some(sample_tx) = self.sample_tx.clone() else {
            return CaptureStallOutcome::Ignored;
        };
        let device_name = self.active_device_name.clone();
        let device_label = device_name.clone().unwrap_or_default();

        let restarts_so_far = match reason {
            CaptureStallReason::NoSamples => self.no_samples_restarts,
            CaptureStallReason::DigitalSilence => self.silence_restarts,
        };
        if restarts_so_far >= MAX_STALL_RECOVERY_ATTEMPTS {
            log::error!(
                "Mic '{device_label}' still stalled ({reason:?}) after {restarts_so_far} restarts, giving up"
            );
            // Noise-gated and virtual mics output exact zeros while the user just listens.
            // Having worked earlier in the session, this one is most likely doing that.
            if reason == CaptureStallReason::DigitalSilence && self.has_had_signal_this_session {
                return CaptureStallOutcome::Ignored;
            }
            self.stall_reported_to_user = true;
            return CaptureStallOutcome::GaveUp {
                device_name: device_label,
            };
        }

        let attempt = restarts_so_far + 1;
        match reason {
            CaptureStallReason::NoSamples => self.no_samples_restarts = attempt,
            CaptureStallReason::DigitalSilence => self.silence_restarts = attempt,
        }
        log::warn!(
            "Mic '{device_label}' stalled ({reason:?}), restarting capture (attempt {attempt}/{MAX_STALL_RECOVERY_ATTEMPTS})"
        );
        self.list_sources();
        self.stop_thread();
        match self.open_capture_with_fallback(device_name.as_deref(), sample_tx.clone()) {
            Ok(_) => CaptureStallOutcome::Restarted,
            Err(error) => {
                log::error!("Failed to restart mic '{device_label}': {error}");
                // Still in the call, so a later attempt or picking another device has
                // to keep working. A device that just changed often opens a moment later.
                self.sample_tx = Some(sample_tx);
                if attempt < MAX_STALL_RECOVERY_ATTEMPTS {
                    self.report_stall_after_delay(reason);
                    return CaptureStallOutcome::Ignored;
                }
                self.stall_reported_to_user = true;
                CaptureStallOutcome::GaveUp {
                    device_name: device_label,
                }
            }
        }
    }

    /// capture_id only changes when a stream opens, so a stop or a new capture in the
    /// meantime makes handle_capture_stalled ignore this.
    fn report_stall_after_delay(&self, reason: CaptureStallReason) {
        let capture_id = self.capture_id;
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(FAILED_RESTART_RETRY_DELAY);
            let _ = proxy.send_event(crate::UserEvent::AudioCaptureStalled { capture_id, reason });
        });
    }

    /// Called once a stream has produced sound. Returns true if the user was told
    /// the mic is silent and should now be told it is back.
    pub fn handle_capture_healthy(&mut self, capture_id: u64) -> bool {
        if capture_id != self.capture_id {
            return false;
        }
        let restarts = self.no_samples_restarts + self.silence_restarts;
        if restarts > 0 {
            log::info!("Mic recovered after {restarts} capture restart(s)");
        }
        self.no_samples_restarts = 0;
        self.silence_restarts = 0;
        self.has_had_signal_this_session = true;
        std::mem::take(&mut self.stall_reported_to_user)
    }

    /// Called when the OS default input device changes.
    /// If using default: reconnect (the default changed).
    /// If using a named device: only switch if that device disappeared.
    pub fn handle_default_device_changed(&mut self, force: bool) {
        if !self.is_capturing() {
            return;
        }
        match &self.active_device_name {
            None => {
                log::info!("Was using default mic, reconnecting to new default...");
                if let Err(e) = self.switch_device(None) {
                    log::error!("Failed to reconnect mic to new default: {e}");
                }
            }
            Some(name) => {
                let name = name.clone();
                if force {
                    log::info!("Force-switching mic '{name}' to default");
                    if let Err(e) = self.switch_device(None) {
                        log::error!("Failed to force-switch mic to default: {e}");
                    }
                } else {
                    let available = self.list_sources();
                    if !available.iter().any(|d| d.name == name) {
                        log::info!("Active mic '{name}' removed, switching to default");
                        if let Err(e) = self.switch_device(None) {
                            log::error!("Failed to switch mic to default: {e}");
                        }
                    }
                }
            }
        }
    }

    fn stop_thread(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.sample_tx = None;
        self.stop_flag = Arc::new(AtomicBool::new(false));
    }

    pub fn stop_capture(&mut self) {
        self.stop_thread();
        self.active_device_name = None;
        self.reset_stall_history();
        self.stall_reported_to_user = false;
    }
}

fn spawn_capture_monitor(
    capture_id: u64,
    health: Arc<CaptureHealth>,
    stop_flag: Arc<AtomicBool>,
    proxy: winit::event_loop::EventLoopProxy<crate::UserEvent>,
) {
    std::thread::spawn(move || {
        let mut reported_reason = None;
        let mut healthy_reported = false;
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(CAPTURE_MONITOR_INTERVAL);
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            match health.check(Instant::now()) {
                Some(reason) => {
                    healthy_reported = false;
                    // A silent stream can go on to stop altogether, which is worth reporting.
                    if reported_reason != Some(reason) {
                        reported_reason = Some(reason);
                        let _ = proxy.send_event(crate::UserEvent::AudioCaptureStalled {
                            capture_id,
                            reason,
                        });
                    }
                }
                None if health.has_had_signal() && !healthy_reported => {
                    reported_reason = None;
                    healthy_reported = true;
                    let _ = proxy.send_event(crate::UserEvent::AudioCaptureHealthy { capture_id });
                }
                None => {}
            }
        }
    });
}
