//! Call sounds, as upstream Hopp plays them: a ring while a call comes in or goes out, a
//! pop when it connects and a short tone when it's declined. Played with PipeWire's
//! `pw-play` on the default output, so they cost nothing while silent and need no decoder
//! in Hopp itself.
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Someone is calling us (looped).
    Incoming,
    /// We're calling someone (looped).
    Outgoing,
    Accepted,
    Unavailable,
    /// One of the last seconds off the record.
    Beep,
    /// Back on the record.
    Resumed,
}

impl Tone {
    /// File and volume, matching upstream Hopp's `get_all_sounds`. The beeps have no
    /// file: see [`Tone::beep`].
    fn sound(self) -> (&'static str, f32) {
        match self {
            Self::Incoming => ("incoming-call.mp3", 0.3),
            Self::Outgoing => ("ring1.mp3", 0.4),
            Self::Accepted => ("bubble-pop.mp3", 0.4),
            Self::Unavailable => ("rejected.mp3", 0.1),
            Self::Beep => ("hopp-beep.wav", 0.25),
            Self::Resumed => ("hopp-resumed.wav", 0.25),
        }
    }

    /// Pitch in Hz and length in milliseconds, for the tones Hopp makes itself.
    fn beep(self) -> Option<(f32, u32)> {
        match self {
            Self::Beep => Some((880., 90)),
            Self::Resumed => Some((1320., 380)),
            _ => None,
        }
    }

    fn looped(self) -> bool {
        matches!(self, Self::Incoming | Self::Outgoing)
    }
}

#[derive(Default)]
pub struct Tones {
    looping: Option<(Tone, Arc<AtomicBool>)>,
}

impl Tones {
    /// Ring with `tone`, or stop ringing with `None`. Repeating the current tone is free.
    pub fn ring(&mut self, tone: Option<Tone>) {
        if self.looping.as_ref().map(|(current, _)| *current) == tone {
            return;
        }
        self.stop();
        let Some(tone) = tone.filter(|tone| tone.looped()) else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("hopp-ring".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    let Some(mut child) = play(tone) else { return };
                    // Poll rather than block so a stop silences the ring at once.
                    loop {
                        if stopped.load(Ordering::Acquire) {
                            let _ = child.kill();
                            let _ = child.wait();
                            return;
                        }
                        match child.try_wait() {
                            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                            Ok(Some(status)) if status.success() => break,
                            _ => return,
                        }
                    }
                }
            });
        if spawned.is_ok() {
            self.looping = Some((tone, stop));
        }
    }

    /// Play a one-off tone, stopping any ring.
    pub fn once(&mut self, tone: Tone) {
        self.stop();
        if let Some(mut child) = play(tone) {
            // Reap it off the UI thread once it finishes.
            let _ = std::thread::Builder::new()
                .name("hopp-tone".into())
                .spawn(move || child.wait());
        }
    }

    /// Play a one-off tone over whatever else is playing.
    pub fn blip(&self, tone: Tone) {
        if let Some(mut child) = play(tone) {
            let _ = std::thread::Builder::new()
                .name("hopp-tone".into())
                .spawn(move || child.wait());
        }
    }

    pub fn stop(&mut self) {
        if let Some((_, stop)) = self.looping.take() {
            stop.store(true, Ordering::Release);
        }
    }
}

impl Drop for Tones {
    fn drop(&mut self) {
        self.stop();
    }
}

fn play(tone: Tone) -> Option<Child> {
    let (file, volume) = tone.sound();
    let path = match tone.beep() {
        Some((pitch, length)) => made(file, pitch, length)?,
        None => sounds()?.join(file),
    };
    match Command::new("pw-play")
        .arg(format!("--volume={volume}"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            crate::logger::line(format!("call sound unavailable: pw-play: {error}"));
            None
        }
    }
}

const RATE: u32 = 44_100;

/// A sine tone as a WAV file: 16-bit mono, with the ends faded so it doesn't click.
fn wav(pitch: f32, length_ms: u32) -> Vec<u8> {
    let count = RATE * length_ms / 1000;
    let fade = (RATE / 100).min(count / 2).max(1);
    let mut bytes = Vec::with_capacity(44 + count as usize * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + count * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    // PCM, one channel.
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(count * 2).to_le_bytes());
    for index in 0..count {
        let edge = index.min(count - 1 - index).min(fade) as f32 / fade as f32;
        let wave = (index as f32 / RATE as f32 * pitch * std::f32::consts::TAU).sin();
        bytes.extend_from_slice(&((wave * edge * 0.8 * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    bytes
}

/// The file for a tone Hopp makes itself, written once per session.
fn made(file: &str, pitch: f32, length_ms: u32) -> Option<PathBuf> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = directory.join(file);
    if !path.is_file()
        && let Err(error) = std::fs::write(&path, wav(pitch, length_ms))
    {
        crate::logger::line(format!(
            "call sound unavailable: {}: {error}",
            path.display()
        ));
        return None;
    }
    Some(path)
}

/// Installed next to the binary (`<prefix>/share/hopp/sounds`), or upstream Hopp's
/// resources when running from the source tree.
fn sounds() -> Option<PathBuf> {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/hopp/sounds")));
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tauri/src-tauri/resources/sounds");
    let found = installed
        .into_iter()
        .chain([source])
        .find(|dir| dir.is_dir());
    if found.is_none() {
        crate::logger::line("call sounds not installed");
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tone_has_a_sound_the_installer_ships() {
        let installer = include_str!("../../packaging/install.py");
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tauri/src-tauri/resources/sounds");
        for tone in [
            Tone::Incoming,
            Tone::Outgoing,
            Tone::Accepted,
            Tone::Unavailable,
        ] {
            let (file, volume) = tone.sound();
            assert!(source.join(file).is_file(), "{file} missing");
            assert!(
                installer.contains(&format!("\"{file}\"")),
                "{file} not installed"
            );
            assert!((0.0..=1.0).contains(&volume));
        }
    }

    #[test]
    fn the_beeps_are_whole_wav_files() {
        for tone in [Tone::Beep, Tone::Resumed] {
            let (pitch, length) = tone.beep().unwrap();
            let bytes = wav(pitch, length);
            let samples = (RATE * length / 1000) as usize;
            assert_eq!(bytes.len(), 44 + samples * 2);
            assert_eq!(&bytes[..4], b"RIFF");
            let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            assert_eq!(riff as usize, bytes.len() - 8);
            // Starts and ends at silence, and is loud in between.
            let sample = |index: usize| {
                i16::from_le_bytes(bytes[44 + index * 2..46 + index * 2].try_into().unwrap())
            };
            assert_eq!(sample(0), 0);
            assert_eq!(sample(samples - 1), 0);
            assert!((0..samples).any(|index| sample(index) > 20_000));
            assert!(!tone.looped());
        }
        assert!(Tone::Accepted.beep().is_none());
    }

    #[test]
    fn one_off_tones_never_loop() {
        let mut tones = Tones::default();
        tones.ring(Some(Tone::Accepted));
        assert!(tones.looping.is_none());
        tones.ring(None);
        assert!(tones.looping.is_none());
    }
}
