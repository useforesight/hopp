//! Whether a call's devices are still working, and what to do when one stops.
//!
//! A device can stall without an error: a Bluetooth headset that switches profile, a USB
//! microphone that is unplugged and replugged, a PipeWire node that is reconfigured, or a
//! camera whose driver stops queueing frames. The stream stays open and simply delivers
//! nothing, or only zeros. These are the pure decisions; `media.rs` and `camera.rs` record
//! what they see and act on them. The thresholds match the Mac app
//! (`core/src/audio/capture_health.rs`, `core/src/camera/capturer.rs`) so both clients
//! notice the same failures.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A freshly opened stream gets longer to deliver its first samples: Bluetooth headsets
/// can take a few seconds to switch profile.
const NO_SAMPLES_TIMEOUT_NEW_STREAM: Duration = Duration::from_secs(5);
/// Once a stream has been delivering, a gap this long means it stopped.
const NO_SAMPLES_TIMEOUT_ACTIVE_STREAM: Duration = Duration::from_secs(2);
/// A real microphone always has a noise floor, so nothing but exact zeros from a stream
/// that never produced sound most likely means the stream is dead.
const SILENCE_TIMEOUT_NEW_STREAM: Duration = Duration::from_secs(6);
/// A reopened stream that is still silent may just be a noise-gated microphone whose
/// owner hasn't spoken yet, so it gets long enough for them to have done so.
const SILENCE_TIMEOUT_REOPENED_STREAM: Duration = Duration::from_secs(12);
/// Noise-gated and virtual microphones output exact zeros between words, so a stream
/// that has produced sound gets far more leeway.
const SILENCE_TIMEOUT_ACTIVE_STREAM: Duration = Duration::from_secs(60);

/// Reopens per kind of stall before Hopp tells you and stops trying.
pub const MAX_STALL_RESTARTS: u32 = 3;

/// How long a stream that hasn't produced sound yet may stay silent.
pub fn silence_timeout_before_signal(had_signal: bool, silence_restarts: u32) -> Duration {
    if had_signal {
        SILENCE_TIMEOUT_ACTIVE_STREAM
    } else if silence_restarts > 0 {
        SILENCE_TIMEOUT_REOPENED_STREAM
    } else {
        SILENCE_TIMEOUT_NEW_STREAM
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallReason {
    /// The stream is open but stopped delivering samples.
    NoSamples,
    /// The stream delivers samples, but they are all exactly zero.
    DigitalSilence,
}

impl StallReason {
    pub fn describe(self) -> &'static str {
        match self {
            Self::NoSamples => "stopped sending audio",
            Self::DigitalSilence => "sent only silence",
        }
    }
}

/// One open stream's liveness. Atomics, so a device callback can record into it while
/// another thread checks it, without a lock in the audio path.
pub struct CaptureHealth {
    started_at: Instant,
    silence_timeout_before_signal: Duration,
    last_frame_ms: AtomicU64,
    last_signal_ms: AtomicU64,
    has_delivered_frame: AtomicBool,
    has_had_signal: AtomicBool,
}

impl CaptureHealth {
    pub fn new(started_at: Instant, silence_timeout_before_signal: Duration) -> Self {
        Self {
            started_at,
            silence_timeout_before_signal,
            last_frame_ms: AtomicU64::new(0),
            last_signal_ms: AtomicU64::new(0),
            has_delivered_frame: AtomicBool::new(false),
            has_had_signal: AtomicBool::new(false),
        }
    }

    fn millis_since_start(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.started_at).as_millis() as u64
    }

    /// A block arrived; `has_signal` is whether any sample in it was not exactly zero.
    pub fn record_frame(&self, now: Instant, has_signal: bool) {
        let elapsed_ms = self.millis_since_start(now);
        self.last_frame_ms.store(elapsed_ms, Ordering::Relaxed);
        self.has_delivered_frame.store(true, Ordering::Relaxed);
        if has_signal {
            self.last_signal_ms.store(elapsed_ms, Ordering::Relaxed);
            self.has_had_signal.store(true, Ordering::Relaxed);
        }
    }

    pub fn has_had_signal(&self) -> bool {
        self.has_had_signal.load(Ordering::Relaxed)
    }

    pub fn check(&self, now: Instant) -> Option<StallReason> {
        let elapsed_ms = self.millis_since_start(now);
        let no_samples_timeout = if self.has_delivered_frame.load(Ordering::Relaxed) {
            NO_SAMPLES_TIMEOUT_ACTIVE_STREAM
        } else {
            NO_SAMPLES_TIMEOUT_NEW_STREAM
        };
        let frame_age_ms = elapsed_ms.saturating_sub(self.last_frame_ms.load(Ordering::Relaxed));
        if frame_age_ms >= millis(no_samples_timeout) {
            return Some(StallReason::NoSamples);
        }
        let silence_timeout = if self.has_had_signal() {
            SILENCE_TIMEOUT_ACTIVE_STREAM
        } else {
            self.silence_timeout_before_signal
        };
        let signal_age_ms = elapsed_ms.saturating_sub(self.last_signal_ms.load(Ordering::Relaxed));
        if signal_age_ms >= millis(silence_timeout) {
            return Some(StallReason::DigitalSilence);
        }
        None
    }
}

/// Saturating, so `Duration::MAX` means never.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// What to do about a stalled microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallAction {
    /// Close and reopen it, as choosing it again by hand would.
    Reopen,
    /// Leave it: most likely a noise-gated microphone while you only listen.
    Ignore,
    /// Reopening hasn't helped: tell the user, who can choose another microphone.
    GiveUp,
}

/// A microphone's stall history across reopens, for one device in one call. Each kind of
/// stall is counted apart, so a noise-gated microphone that is silent can't use up the
/// reopens for a stream that really has stopped.
#[derive(Debug, Default)]
pub struct MicrophoneStalls {
    no_samples_restarts: u32,
    silence_restarts: u32,
    /// Whether the user has been told, so they hear when it's back.
    reported: bool,
    /// This device produced sound at some point in this call.
    had_signal: bool,
}

impl MicrophoneStalls {
    /// How long the next stream may stay silent before it has produced sound.
    pub fn silence_timeout(&self) -> Duration {
        silence_timeout_before_signal(self.had_signal, self.silence_restarts)
    }

    pub fn stalled(&mut self, reason: StallReason) -> StallAction {
        let restarts = match reason {
            StallReason::NoSamples => &mut self.no_samples_restarts,
            StallReason::DigitalSilence => &mut self.silence_restarts,
        };
        if *restarts < MAX_STALL_RESTARTS {
            *restarts += 1;
            return StallAction::Reopen;
        }
        // Having worked earlier in the call, a silent microphone is most likely gated.
        if reason == StallReason::DigitalSilence && self.had_signal {
            return StallAction::Ignore;
        }
        self.reported = true;
        StallAction::GiveUp
    }

    /// The stream produced sound. Returns whether the user was told it had stopped, so
    /// they can be told it's back.
    pub fn heard(&mut self) -> bool {
        self.no_samples_restarts = 0;
        self.silence_restarts = 0;
        self.had_signal = true;
        std::mem::take(&mut self.reported)
    }
}

/// The camera may take this long to deliver its first frame after opening.
const CAMERA_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
/// A camera that has been delivering and stops for this long is stuck.
const CAMERA_STALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Reopens, for a stall or a read error, before giving up. A frame resets the count.
pub const MAX_CAMERA_RESTARTS: u32 = 3;
/// The wait before the first reopen, doubled for each one after it.
const CAMERA_RETRY_FIRST: Duration = Duration::from_millis(500);

/// One camera's liveness and retry budget.
#[derive(Debug)]
pub struct CameraWatch {
    opened_at: Instant,
    last_frame: Option<Instant>,
    restarts: u32,
}

impl CameraWatch {
    pub fn new(now: Instant) -> Self {
        Self {
            opened_at: now,
            last_frame: None,
            restarts: 0,
        }
    }

    /// A frame arrived. Only a real frame counts as recovery: a reopened stream looks
    /// healthy until its timeout passes.
    pub fn frame(&mut self, now: Instant) {
        self.last_frame = Some(now);
        self.restarts = 0;
    }

    /// Whether the camera has stopped delivering frames.
    pub fn stalled(&self, now: Instant) -> bool {
        match self.last_frame {
            Some(at) => now.saturating_duration_since(at) >= CAMERA_STALL_TIMEOUT,
            None => now.saturating_duration_since(self.opened_at) >= CAMERA_FIRST_FRAME_TIMEOUT,
        }
    }

    /// Asks for another reopen: how long to wait first, or `None` to give up.
    pub fn retry(&mut self) -> Option<Duration> {
        if self.restarts >= MAX_CAMERA_RESTARTS {
            return None;
        }
        let delay = CAMERA_RETRY_FIRST * 2_u32.pow(self.restarts);
        self.restarts += 1;
        Some(delay)
    }

    /// The camera reopened at `now`; it gets a new stream's time to deliver.
    pub fn reopened(&mut self, now: Instant) {
        self.opened_at = now;
        self.last_frame = None;
    }
}

/// How often your microphone level reaches the UI: enough for a smooth ring, few enough
/// that it doesn't redraw the call window on every 10 ms block.
const LEVEL_EVERY: Duration = Duration::from_millis(40);
/// Raw RMS for speech is about 0.001–0.03, so the Mac app scales by 40 until peak speech
/// fills its ring. The same here, so the rings match.
const LEVEL_SCALE: f32 = 40.;
/// Levels are sent in steps of this, so a steady level (or silence) sends nothing.
const LEVEL_STEPS: f32 = 20.;
/// How much of the previous level each 10 ms block keeps while the level falls: a quick
/// rise and a gentler fall, so the ring follows speech without flickering.
const LEVEL_RELEASE: f32 = 0.85;

/// Turns microphone blocks into a 0–1 level for your ring, smoothed and throttled.
#[derive(Debug, Default)]
pub struct LevelMeter {
    smoothed: f32,
    sent: Option<(Instant, f32)>,
}

impl LevelMeter {
    /// Takes one block; returns a level when one is due and it has changed.
    pub fn block(&mut self, samples: &[i16], now: Instant) -> Option<f32> {
        let level = (rms(samples) * LEVEL_SCALE).min(1.);
        self.smoothed = if level >= self.smoothed {
            level
        } else {
            self.smoothed * LEVEL_RELEASE + level * (1. - LEVEL_RELEASE)
        };
        let stepped = (self.smoothed * LEVEL_STEPS).round() / LEVEL_STEPS;
        match self.sent {
            Some((at, _)) if now.saturating_duration_since(at) < LEVEL_EVERY => None,
            Some((_, last)) if last == stepped => None,
            _ => {
                self.sent = Some((now, stepped));
                Some(stepped)
            }
        }
    }
}

/// Root mean square of 16-bit samples, 0–1.
pub fn rms(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.;
    }
    let sum: f64 = samples
        .iter()
        .map(|&sample| {
            let value = sample as f64 / 32768.;
            value * value
        })
        .sum();
    (sum / samples.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn new_stream(start: Instant) -> CaptureHealth {
        CaptureHealth::new(start, SILENCE_TIMEOUT_NEW_STREAM)
    }

    #[test]
    fn new_stream_gets_time_to_deliver_its_first_samples() {
        let start = Instant::now();
        let health = new_stream(start);
        assert_eq!(health.check(start + secs(4)), None);
        assert_eq!(health.check(start + secs(5)), Some(StallReason::NoSamples));
    }

    #[test]
    fn delivering_stream_that_stops_is_stalled_quickly() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(1), true);
        assert_eq!(health.check(start + secs(2)), None);
        assert_eq!(health.check(start + secs(3)), Some(StallReason::NoSamples));
    }

    #[test]
    fn stream_of_only_zeros_from_the_start_is_stalled() {
        let start = Instant::now();
        let health = new_stream(start);
        for second in 1..=5 {
            health.record_frame(start + secs(second), false);
            assert_eq!(health.check(start + secs(second)), None);
        }
        health.record_frame(start + secs(6), false);
        assert_eq!(
            health.check(start + secs(6)),
            Some(StallReason::DigitalSilence)
        );
    }

    #[test]
    fn noise_gated_mic_is_not_flagged_for_a_normal_pause() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(1), true);
        for second in 2..=60 {
            health.record_frame(start + secs(second), false);
            assert_eq!(health.check(start + secs(second)), None);
        }
        health.record_frame(start + secs(61), false);
        assert_eq!(
            health.check(start + secs(61)),
            Some(StallReason::DigitalSilence)
        );
    }

    #[test]
    fn sound_clears_a_silence_stall() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(6), false);
        assert!(health.check(start + secs(6)).is_some());
        health.record_frame(start + secs(7), true);
        assert_eq!(health.check(start + secs(7)), None);
    }

    #[test]
    fn a_silent_mic_gets_longer_each_reopen_and_longest_if_it_has_worked() {
        assert_eq!(silence_timeout_before_signal(false, 0), secs(6));
        assert_eq!(silence_timeout_before_signal(false, 1), secs(12));
        assert_eq!(silence_timeout_before_signal(false, 3), secs(12));
        assert_eq!(silence_timeout_before_signal(true, 0), secs(60));
    }

    #[test]
    fn a_stopped_mic_is_reopened_three_times_then_reported() {
        let mut stalls = MicrophoneStalls::default();
        for _ in 0..MAX_STALL_RESTARTS {
            assert_eq!(stalls.stalled(StallReason::NoSamples), StallAction::Reopen);
        }
        assert_eq!(stalls.stalled(StallReason::NoSamples), StallAction::GiveUp);
        assert!(stalls.heard(), "the user hears it's back");
        assert!(!stalls.heard(), "and only once");
        assert_eq!(stalls.stalled(StallReason::NoSamples), StallAction::Reopen);
    }

    #[test]
    fn silence_and_stopping_are_counted_apart() {
        let mut stalls = MicrophoneStalls::default();
        for _ in 0..MAX_STALL_RESTARTS {
            stalls.stalled(StallReason::DigitalSilence);
        }
        assert_eq!(stalls.stalled(StallReason::NoSamples), StallAction::Reopen);
        assert_eq!(stalls.silence_timeout(), secs(12));
    }

    #[test]
    fn a_mic_that_has_worked_is_left_alone_when_it_stays_silent() {
        let mut stalls = MicrophoneStalls::default();
        stalls.heard();
        assert_eq!(stalls.silence_timeout(), secs(60));
        for _ in 0..MAX_STALL_RESTARTS {
            assert_eq!(
                stalls.stalled(StallReason::DigitalSilence),
                StallAction::Reopen
            );
        }
        assert_eq!(
            stalls.stalled(StallReason::DigitalSilence),
            StallAction::Ignore
        );
        assert!(!stalls.heard(), "nothing was reported");
    }

    #[test]
    fn a_camera_gets_longer_for_its_first_frame_than_between_frames() {
        let start = Instant::now();
        let mut watch = CameraWatch::new(start);
        assert!(!watch.stalled(start + secs(14)));
        assert!(watch.stalled(start + secs(15)));
        watch.frame(start + secs(1));
        assert!(!watch.stalled(start + secs(5)));
        assert!(watch.stalled(start + secs(6)));
    }

    #[test]
    fn a_camera_is_retried_with_backoff_then_given_up_until_a_frame_arrives() {
        let start = Instant::now();
        let mut watch = CameraWatch::new(start);
        let delays: Vec<_> = std::iter::from_fn(|| watch.retry()).collect();
        assert_eq!(
            delays,
            [500, 1000, 2000].map(Duration::from_millis),
            "three tries, doubling"
        );
        watch.reopened(start + secs(10));
        assert!(!watch.stalled(start + secs(20)), "a new stream gets 15 s");
        watch.frame(start + secs(11));
        assert_eq!(watch.retry(), Some(Duration::from_millis(500)));
    }

    #[test]
    fn the_level_rises_fast_falls_gently_and_is_throttled() {
        let start = Instant::now();
        let mut meter = LevelMeter::default();
        let speech = [800_i16; 480];
        let silence = [0_i16; 480];
        assert_eq!(meter.block(&speech, start), Some(1.));
        // Within 40 ms: nothing, however loud.
        assert_eq!(
            meter.block(&silence, start + Duration::from_millis(10)),
            None
        );
        let fallen = meter
            .block(&silence, start + Duration::from_millis(40))
            .unwrap();
        assert!(fallen > 0.4 && fallen < 1., "falls gently: {fallen}");
        // Silence long enough reaches zero, then sends nothing more.
        let mut last = None;
        for block in 5..200 {
            if let Some(level) = meter.block(&silence, start + Duration::from_millis(10 * block)) {
                last = Some(level);
            }
        }
        assert_eq!(last, Some(0.));
        assert_eq!(meter.block(&silence, start + secs(10)), None);
    }

    #[test]
    fn rms_of_a_full_scale_square_wave_is_one() {
        assert_eq!(rms(&[]), 0.);
        let square: Vec<i16> = (0..480)
            .map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN + 1 })
            .collect();
        assert!((rms(&square) - 1.).abs() < 0.001);
    }
}
