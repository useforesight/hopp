//! Talking to `hopp-transcribe`, the transcription helper process.

use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

pub enum Reply {
    /// The model is loaded, on this device ("CPU" or the GPU's name). `words`: the
    /// helper times each word, so one request can hold several people's speech.
    Ready { device: String, words: bool },
    Transcript {
        id: u64,
        /// Text pieces with the time they were spoken, in request milliseconds.
        pieces: Vec<(i64, String)>,
        /// How long the helper took.
        ms: u64,
    },
    /// It couldn't load the model.
    Failed(String),
    /// The process ended.
    Exited,
}

pub struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
    next: u64,
}

/// `HOPP_TRANSCRIBE_HELPER` when set (tests, debugging); else next to the `hopp`
/// binary when installed or built; otherwise on PATH.
fn executable() -> PathBuf {
    if let Some(path) = std::env::var_os("HOPP_TRANSCRIBE_HELPER") {
        return PathBuf::from(path);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("hopp-transcribe")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("hopp-transcribe"))
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Threads: all but four cores, at least half of them, at most eight. They only ever
/// get idle time (the helper's scope has `CPUWeight=idle`), so more threads just use
/// more of what nothing else wants: on a 12-core M2 Max, 8 threads transcribe about
/// three times faster than speech with no effect on the desktop.
fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|cores| {
            let cores = cores.get();
            cores.saturating_sub(4).max(cores / 2).clamp(1, 8)
        })
        .unwrap_or(2)
}

impl Helper {
    /// Start the helper with `model`. Replies arrive through `reply` from a reader thread.
    pub fn spawn(
        model: &Path,
        english_only: bool,
        cpu: bool,
        memory_limit: u64,
        reply: impl Fn(Reply) + Send + 'static,
    ) -> Result<Self> {
        let helper = executable();
        // Its own scope (idle CPU only, its own memory limit), or at least the lowest
        // priority, so transcription never takes anything from the call or the desktop.
        let mut command = match crate::scope::isolated("transcribe", memory_limit) {
            Some(command) => command,
            None if on_path("nice") => {
                let mut command = Command::new("nice");
                command.args(["-n", "19"]);
                command
            }
            None => Command::new(&helper),
        };
        if command.get_program() != helper.as_os_str() {
            command.arg(&helper);
        }
        command
            .arg("--model")
            .arg(model)
            .args(["--threads", &threads().to_string()])
            .args(["--language", if english_only { "en" } else { "auto" }]);
        if cpu {
            command.arg("--cpu");
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("Starting {}", helper.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().context("helper stdout")?;
        let stderr = child.stderr.take().context("helper stderr")?;
        std::thread::Builder::new()
            .name("hopp-transcribe-out".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if message["ready"] == true {
                        let device = message["device"].as_str().unwrap_or("CPU").to_owned();
                        let words = message["words"] == true;
                        reply(Reply::Ready { device, words });
                    } else if let Some(error) = message["error"].as_str() {
                        reply(Reply::Failed(error.to_owned()));
                    } else if let Some(id) = message["id"].as_u64() {
                        let pieces = message["segments"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|piece| {
                                // A word is placed by where it starts: Parakeet stretches
                                // the last word before a pause across the silence after
                                // it, so its middle can land in the next utterance.
                                (
                                    piece["t0_ms"].as_i64().unwrap_or(0),
                                    piece["text"].as_str().unwrap_or_default().to_owned(),
                                )
                            })
                            .collect();
                        let ms = message["ms"].as_u64().unwrap_or(0);
                        reply(Reply::Transcript { id, pieces, ms });
                    }
                }
                reply(Reply::Exited);
            })?;
        std::thread::Builder::new()
            .name("hopp-transcribe-err".into())
            .spawn(move || {
                for line in BufReader::new(stderr)
                    .lines()
                    .map_while(Result::ok)
                    .take(20)
                {
                    crate::logger::line(format!("hopp-transcribe: {line}"));
                }
            })?;
        Ok(Self {
            child,
            stdin,
            next: 0,
        })
    }

    /// Send one request of 16 kHz audio; the transcript comes back with the returned id.
    pub fn send(&mut self, samples: &[f32]) -> Result<u64> {
        self.next += 1;
        let header = serde_json::to_vec(&serde_json::json!({
            "id": self.next,
            "samples": samples.len(),
        }))?;
        let mut frame = Vec::with_capacity(4 + header.len() + samples.len() * 4);
        frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
        frame.extend_from_slice(&header);
        for sample in samples {
            frame.extend_from_slice(&sample.to_le_bytes());
        }
        let stdin = self.stdin.as_mut().context("Helper is closed")?;
        stdin.write_all(&frame)?;
        stdin.flush()?;
        Ok(self.next)
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // Closing stdin tells the helper to exit; give it a moment, then make sure.
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
