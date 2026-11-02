use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A freshly opened stream gets longer to deliver its first frame: Bluetooth
/// headsets can take a few seconds to switch profile.
const NO_SAMPLES_TIMEOUT_NEW_STREAM: Duration = Duration::from_secs(5);
/// Once a stream has been delivering, a gap this long means it stopped.
const NO_SAMPLES_TIMEOUT_ACTIVE_STREAM: Duration = Duration::from_secs(2);
/// A real microphone always has a noise floor, so nothing but exact zeros from a
/// stream that never produced sound most likely means the stream is dead.
const SILENCE_TIMEOUT_NEW_STREAM: Duration = Duration::from_secs(6);
/// A reopened stream that is still silent may just be a noise-gated microphone whose
/// owner hasn't spoken yet, so the warning comes late enough for them to have done so.
const SILENCE_TIMEOUT_REOPENED_STREAM: Duration = Duration::from_secs(12);
/// Noise-gated and virtual microphones output exact zeros between words, so a
/// stream that has produced sound gets far more leeway.
const SILENCE_TIMEOUT_ACTIVE_STREAM: Duration = Duration::from_secs(60);

/// How long a stream that hasn't produced sound yet may stay silent.
pub fn silence_timeout_before_signal(
    has_had_signal_this_session: bool,
    silence_restarts: u32,
) -> Duration {
    if has_had_signal_this_session {
        SILENCE_TIMEOUT_ACTIVE_STREAM
    } else if silence_restarts > 0 {
        SILENCE_TIMEOUT_REOPENED_STREAM
    } else {
        SILENCE_TIMEOUT_NEW_STREAM
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStallReason {
    /// The stream is open but stopped delivering samples.
    NoSamples,
    /// The stream delivers samples, but they are all exactly zero.
    DigitalSilence,
}

/// Shared between the capture thread, which records what it reads, and the
/// monitor thread, which decides whether the stream is still alive.
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

    pub fn check(&self, now: Instant) -> Option<CaptureStallReason> {
        let elapsed_ms = self.millis_since_start(now);

        let has_delivered_frame = self.has_delivered_frame.load(Ordering::Relaxed);
        let no_samples_timeout = if has_delivered_frame {
            NO_SAMPLES_TIMEOUT_ACTIVE_STREAM
        } else {
            NO_SAMPLES_TIMEOUT_NEW_STREAM
        };
        let frame_age_ms = elapsed_ms.saturating_sub(self.last_frame_ms.load(Ordering::Relaxed));
        if frame_age_ms >= no_samples_timeout.as_millis() as u64 {
            return Some(CaptureStallReason::NoSamples);
        }

        let silence_timeout = if self.has_had_signal.load(Ordering::Relaxed) {
            SILENCE_TIMEOUT_ACTIVE_STREAM
        } else {
            self.silence_timeout_before_signal
        };
        let signal_age_ms = elapsed_ms.saturating_sub(self.last_signal_ms.load(Ordering::Relaxed));
        if signal_age_ms >= silence_timeout.as_millis() as u64 {
            return Some(CaptureStallReason::DigitalSilence);
        }

        None
    }
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
    fn new_stream_gets_time_to_deliver_its_first_frame() {
        let start = Instant::now();
        let health = new_stream(start);
        assert_eq!(health.check(start + secs(4)), None);
    }

    #[test]
    fn stream_that_never_delivers_is_stalled() {
        let start = Instant::now();
        let health = new_stream(start);
        assert_eq!(
            health.check(start + secs(5)),
            Some(CaptureStallReason::NoSamples)
        );
    }

    #[test]
    fn delivering_stream_that_stops_is_stalled_quickly() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(1), true);
        assert_eq!(health.check(start + secs(2)), None);
        assert_eq!(
            health.check(start + secs(3)),
            Some(CaptureStallReason::NoSamples)
        );
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
            Some(CaptureStallReason::DigitalSilence)
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
    }

    #[test]
    fn mic_that_goes_silent_for_a_long_time_is_stalled() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(1), true);
        health.record_frame(start + secs(61), false);
        assert_eq!(
            health.check(start + secs(61)),
            Some(CaptureStallReason::DigitalSilence)
        );
    }

    #[test]
    fn sound_clears_a_silence_stall() {
        let start = Instant::now();
        let health = new_stream(start);
        health.record_frame(start + secs(6), false);
        assert_eq!(
            health.check(start + secs(6)),
            Some(CaptureStallReason::DigitalSilence)
        );
        health.record_frame(start + secs(7), true);
        assert_eq!(health.check(start + secs(7)), None);
    }

    #[test]
    fn a_silent_mic_gets_longer_each_reopen_and_longest_if_it_has_worked() {
        assert_eq!(silence_timeout_before_signal(false, 0), secs(6));
        assert_eq!(silence_timeout_before_signal(false, 1), secs(12));
        assert_eq!(silence_timeout_before_signal(false, 3), secs(12));
        assert_eq!(silence_timeout_before_signal(true, 0), secs(60));
        assert_eq!(silence_timeout_before_signal(true, 2), secs(60));
    }

    #[test]
    fn a_reopened_stream_on_a_mic_that_has_worked_may_stay_silent_a_long_time() {
        let start = Instant::now();
        let health = CaptureHealth::new(start, silence_timeout_before_signal(true, 1));
        health.record_frame(start + secs(59), false);
        assert_eq!(health.check(start + secs(59)), None);
        health.record_frame(start + secs(60), false);
        assert_eq!(
            health.check(start + secs(60)),
            Some(CaptureStallReason::DigitalSilence)
        );
    }

    #[test]
    fn a_reopened_stream_still_stalls_fast_when_samples_stop() {
        let start = Instant::now();
        let health = CaptureHealth::new(start, silence_timeout_before_signal(true, 1));
        assert_eq!(
            health.check(start + secs(5)),
            Some(CaptureStallReason::NoSamples)
        );
    }
}
