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
}

impl Tone {
    /// File and volume, matching upstream Hopp's `get_all_sounds`.
    fn sound(self) -> (&'static str, f32) {
        match self {
            Self::Incoming => ("incoming-call.mp3", 0.3),
            Self::Outgoing => ("ring1.mp3", 0.4),
            Self::Accepted => ("bubble-pop.mp3", 0.4),
            Self::Unavailable => ("rejected.mp3", 0.1),
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
    let path = sounds()?.join(file);
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
    fn one_off_tones_never_loop() {
        let mut tones = Tones::default();
        tones.ring(Some(Tone::Accepted));
        assert!(tones.looping.is_none());
        tones.ring(None);
        assert!(tones.looping.is_none());
    }
}
