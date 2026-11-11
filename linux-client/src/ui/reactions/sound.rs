//! Playing the reactions' sounds: each is rendered off the UI thread, written as a WAV
//! file and played with PipeWire's `pw-play`, as the call sounds are (see
//! `ui::tones`). A tagged sound can be stopped before it ends.

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{stage::Sound, synth};

/// How loud the reactions play, under the call's own audio.
const VOLUME: f32 = 0.8;

/// A sound being made or played: its player once it has one, and whether it's been
/// stopped (so one stopped while still being made never starts).
#[derive(Default)]
struct Playing {
    child: Option<Child>,
    stopped: bool,
}

#[derive(Default)]
pub struct Speaker {
    playing: Vec<(Option<String>, Arc<Mutex<Playing>>)>,
}

impl Speaker {
    pub fn play(&mut self, sound: Sound) {
        self.playing
            .retain(|(_, playing)| Arc::strong_count(playing) > 1);
        let playing = Arc::new(Mutex::new(Playing::default()));
        self.playing.push((sound.tag, playing.clone()));
        let graph = sound.graph;
        let _ = std::thread::Builder::new()
            .name("hopp-reaction-sound".into())
            .spawn(move || {
                let samples = graph.render();
                let Some(path) = write(&synth::wav(&samples)) else {
                    return;
                };
                let started = {
                    let mut playing = playing.lock().unwrap_or_else(|e| e.into_inner());
                    if !playing.stopped {
                        playing.child = spawn(&path);
                    }
                    playing.child.is_some()
                };
                if started {
                    // Wait without holding the lock, so it can be stopped meanwhile.
                    loop {
                        let mut playing = playing.lock().unwrap_or_else(|e| e.into_inner());
                        let Some(child) = playing.child.as_mut() else {
                            break;
                        };
                        match child.try_wait() {
                            Ok(None) => {}
                            _ => break,
                        }
                        drop(playing);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                let _ = std::fs::remove_file(&path);
            });
    }

    /// Stop the sounds tagged `tag`.
    pub fn hush(&mut self, tag: &str) {
        for (_, playing) in self
            .playing
            .iter()
            .filter(|(sound, _)| sound.as_deref() == Some(tag))
        {
            stop(playing);
        }
    }

    pub fn hush_all(&mut self) {
        for (_, playing) in self.playing.drain(..) {
            stop(&playing);
        }
    }
}

fn stop(playing: &Mutex<Playing>) {
    let mut playing = playing.lock().unwrap_or_else(|e| e.into_inner());
    playing.stopped = true;
    if let Some(child) = playing.child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Write a sound to a file of its own in the runtime directory.
fn write(bytes: &[u8]) -> Option<PathBuf> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = directory.join(format!(
        "hopp-reaction-{}-{}.wav",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    match std::fs::write(&path, bytes) {
        Ok(()) => Some(path),
        Err(error) => {
            crate::logger::line(format!(
                "reaction sound unavailable: {}: {error}",
                path.display()
            ));
            None
        }
    }
}

fn spawn(path: &PathBuf) -> Option<Child> {
    match Command::new("pw-play")
        .arg(format!("--volume={VOLUME}"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(error) => {
            crate::logger::line(format!("reaction sound unavailable: pw-play: {error}"));
            None
        }
    }
}
