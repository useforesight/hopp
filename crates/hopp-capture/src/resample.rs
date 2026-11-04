//! Sample-rate conversion for the transcription models, which want 16 kHz.

use crate::vad::RATE;

#[cfg(feature = "rate-48k")]
mod from_48k {
    use std::sync::OnceLock;

    /// Taps of the anti-aliasing filter for 48 → 16 kHz.
    const TAPS: usize = 63;
    /// Pass speech up to here; 16 kHz output can only hold up to 8 kHz.
    const CUTOFF_HZ: f64 = 7_000.0;

    fn filter() -> &'static [f32; TAPS] {
        static FILTER: OnceLock<[f32; TAPS]> = OnceLock::new();
        FILTER.get_or_init(|| {
            let middle = (TAPS / 2) as f64;
            let cutoff = CUTOFF_HZ / 48_000.0;
            let mut taps = [0f64; TAPS];
            for (index, tap) in taps.iter_mut().enumerate() {
                let x = index as f64 - middle;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                // Blackman window.
                let phase = 2.0 * std::f64::consts::PI * index as f64 / (TAPS - 1) as f64;
                *tap = sinc * (0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos());
            }
            let sum: f64 = taps.iter().sum();
            taps.map(|tap| (tap / sum) as f32)
        })
    }

    /// 48 kHz 16-bit to 16 kHz float, filtered so nothing above 8 kHz folds back in.
    pub fn to_16k(input: &[i16]) -> Vec<f32> {
        let taps = filter();
        let middle = TAPS / 2;
        (0..input.len() / 3)
            .map(|output| {
                let centre = output * 3;
                let mut sum = 0f32;
                for (index, tap) in taps.iter().enumerate() {
                    if let Some(&sample) = (centre + index)
                        .checked_sub(middle)
                        .and_then(|position| input.get(position))
                    {
                        sum += tap * f32::from(sample);
                    }
                }
                sum / 32_768.0
            })
            .collect()
    }
}

#[cfg(feature = "rate-48k")]
pub use from_48k::to_16k;

/// The call's audio is already 16 kHz: only the format changes.
#[cfg(not(feature = "rate-48k"))]
pub fn to_16k(input: &[i16]) -> Vec<f32> {
    input
        .iter()
        .map(|sample| f32::from(*sample) / 32_768.0)
        .collect()
}

/// Any rate to the call's rate by linear interpolation; for files fed to `hopp capture
/// transcribe`, where speed matters more than the last bit of quality.
pub fn to_call_rate(rate: u32, input: &[i16]) -> Vec<i16> {
    if rate as usize == RATE || input.is_empty() {
        return input.to_vec();
    }
    let step = f64::from(rate) / RATE as f64;
    let length = (input.len() as f64 / step) as usize;
    (0..length)
        .map(|index| {
            let position = index as f64 * step;
            let before = position.floor() as usize;
            let after = (before + 1).min(input.len() - 1);
            let weight = position - before as f64;
            let value =
                f64::from(input[before]) * (1.0 - weight) + f64::from(input[after]) * weight;
            value.round() as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, seconds: f32, rate: f32) -> Vec<i16> {
        (0..(seconds * rate) as usize)
            .map(|index| {
                (16_000.0 * (2.0 * std::f32::consts::PI * hz * index as f32 / rate).sin()) as i16
            })
            .collect()
    }

    fn peak(samples: &[f32]) -> f32 {
        // Skip the filter's start-up.
        samples[100..samples.len() - 100]
            .iter()
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    #[cfg(feature = "rate-48k")]
    #[test]
    fn keeps_speech_and_removes_what_would_alias() {
        let speech = to_16k(&sine(1_000.0, 0.5, 48_000.0));
        assert_eq!(speech.len(), 8_000);
        assert!((peak(&speech) - 16_000.0 / 32_768.0).abs() < 0.02);
        let high = to_16k(&sine(10_000.0, 0.5, 48_000.0));
        assert!(peak(&high) < 0.005, "10 kHz must not fold back to 6 kHz");
    }

    #[cfg(not(feature = "rate-48k"))]
    #[test]
    fn audio_already_at_16k_only_changes_format() {
        let speech = to_16k(&sine(1_000.0, 0.5, 16_000.0));
        assert_eq!(speech.len(), 8_000);
        assert!((peak(&speech) - 16_000.0 / 32_768.0).abs() < 0.02);
    }

    #[test]
    fn files_are_converted_to_the_calls_rate() {
        let other = if RATE == 16_000 { 48_000 } else { 16_000 };
        let input = sine(500.0, 0.25, other as f32);
        let output = to_call_rate(other, &input);
        assert_eq!(output.len(), input.len() * RATE / other as usize);
        assert_eq!(to_call_rate(RATE as u32, &input), input);
    }
}
