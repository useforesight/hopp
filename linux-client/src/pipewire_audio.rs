//! PipeWire microphones by name. cpal only lists ALSA devices, which on a PipeWire
//! desktop are the raw cards plus "default"; virtual sources such as Omalive's
//! processed mic or a filter chain never appear. This lists PipeWire's sources and
//! opens one through the ALSA PipeWire plugin (`pipewire:NODE=<name>`), which links
//! the stream straight to that node.
use alsa::{
    Direction, ValueOr,
    pcm::{Access, Format, HwParams, PCM},
};
use anyhow::{Context, Result};
use std::{
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

/// Capture runs at the rate and layout Hopp processes; PipeWire converts.
pub const RATE: u32 = 48000;
const PERIOD: usize = RATE as usize / 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// `node.name`, what the plugin targets.
    pub node: String,
    /// `node.description`, what people see.
    pub description: String,
}

/// PipeWire's audio sources, by description; empty without PipeWire.
pub fn sources() -> Vec<Source> {
    let Ok(output) = Command::new("pw-dump").output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse(&output.stdout)
}

fn parse(dump: &[u8]) -> Vec<Source> {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_slice(dump) else {
        return Vec::new();
    };
    let mut sources: Vec<_> = objects
        .iter()
        .filter(|object| object["type"] == "PipeWire:Interface:Node")
        .filter_map(|object| {
            let props = &object["info"]["props"];
            // Not "Audio/Source/Internal": a card's raw node behind its split inputs.
            if !matches!(
                props["media.class"].as_str()?,
                "Audio/Source" | "Audio/Source/Virtual"
            ) {
                return None;
            }
            let node = props["node.name"].as_str()?.to_owned();
            let description = props["node.description"]
                .as_str()
                .or_else(|| props["node.nick"].as_str())
                .unwrap_or(&node)
                .to_owned();
            Some(Source { node, description })
        })
        .collect();
    sources.sort_by(|a, b| a.description.cmp(&b.description));
    sources.dedup_by(|a, b| a.description == b.description);
    sources
}

/// The source with this description, if PipeWire has it.
pub fn find(description: &str) -> Option<Source> {
    sources()
        .into_iter()
        .find(|source| source.description == description)
}

/// An open, not yet started, mono capture from one PipeWire source.
pub struct Capture {
    pcm: Option<PCM>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn open(node: &str) -> Result<Self> {
        let pcm = PCM::new(&format!("pipewire:NODE={node}"), Direction::Capture, true)
            .with_context(|| format!("Opening PipeWire source {node}"))?;
        {
            let params = HwParams::any(&pcm)?;
            params.set_access(Access::RWInterleaved)?;
            params.set_format(Format::float())?;
            params.set_channels(1)?;
            params.set_rate(RATE, ValueOr::Nearest)?;
            params.set_period_size_near(PERIOD as _, ValueOr::Nearest)?;
            params.set_buffer_size_near(PERIOD as alsa::pcm::Frames * 8)?;
            pcm.hw_params(&params)
                .context("PipeWire source rejected 48 kHz mono")?;
        }
        Ok(Self {
            pcm: Some(pcm),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        })
    }

    /// Delivers 10 ms of samples at a time with the device delay in milliseconds, on a
    /// thread of its own; `failed` gets the error that ends it.
    pub fn start(
        &mut self,
        mut deliver: impl FnMut(&[f32], u32) + Send + 'static,
        failed: impl FnOnce(String) + Send + 'static,
    ) -> Result<()> {
        let pcm = self.pcm.take().context("Capture already started")?;
        let stop = self.stop.clone();
        let thread = thread::Builder::new()
            .name("hopp-microphone".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    pcm.start()?;
                    let io = pcm.io_f32()?;
                    let mut pending = [0_f32; PERIOD];
                    let mut filled = 0;
                    while !stop.load(Ordering::Relaxed) {
                        // A bounded wait, so stopping never waits on a silent source.
                        if !pcm
                            .wait(Some(200))
                            .or_else(|error| pcm.try_recover(error, true).map(|()| false))?
                        {
                            continue;
                        }
                        match io.readi(&mut pending[filled..]) {
                            Ok(read) => filled += read,
                            Err(error)
                                if std::io::Error::from_raw_os_error(error.errno()).kind()
                                    == std::io::ErrorKind::WouldBlock =>
                            {
                                continue;
                            }
                            Err(error) => {
                                pcm.try_recover(error, true)?;
                                continue;
                            }
                        }
                        if filled == PERIOD {
                            let delay = pcm.delay().unwrap_or(0).max(0) as u64;
                            deliver(&pending, (delay * 1000 / RATE as u64).min(500) as u32);
                            filled = 0;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    failed(format!("{error:#}"));
                }
            })?;
        self.thread = Some(thread);
        Ok(())
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_sources_but_not_raw_cards_or_sinks() {
        let dump = br#"[
            {"id": 1, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Source", "node.name": "omalive_voice",
                "node.description": "Omalive Mic"}}},
            {"id": 2, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Source/Internal", "node.name": "alsa_input.hw_USB_0",
                "node.description": "Scarlett Solo 0"}}},
            {"id": 3, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Sink", "node.name": "speakers",
                "node.description": "Speakers"}}},
            {"id": 4, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Source/Virtual", "node.name": "null_source"}}},
            {"id": 5, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Source", "node.name": "built_in",
                "node.description": "Built-in Microphone"}}},
            {"id": 6, "type": "PipeWire:Interface:Client", "info": {"props": {}}}
        ]"#;
        let names: Vec<_> = parse(dump)
            .into_iter()
            .map(|source| (source.description, source.node))
            .collect();
        assert_eq!(
            names,
            [
                ("Built-in Microphone".into(), "built_in".into()),
                ("Omalive Mic".into(), "omalive_voice".into()),
                ("null_source".into(), "null_source".into()),
            ]
        );
    }

    /// Reads a few blocks from a real PipeWire source (HOPP_TEST_SOURCE, a node name,
    /// or the first listed) and discards them.
    #[test]
    #[ignore = "needs a running PipeWire"]
    fn captures_ten_millisecond_blocks_and_stops_promptly() {
        let node = std::env::var("HOPP_TEST_SOURCE")
            .unwrap_or_else(|_| sources().first().expect("a PipeWire source").node.clone());
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut capture = Capture::open(&node).unwrap();
        capture
            .start(
                move |samples, delay_ms| {
                    let _ = sender.send((samples.len(), delay_ms));
                },
                |error| panic!("capture failed: {error}"),
            )
            .unwrap();
        for _ in 0..20 {
            let (len, delay_ms) = receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            assert_eq!(len, PERIOD);
            assert!(delay_ms <= 500);
        }
        let stopping = std::time::Instant::now();
        drop(capture);
        assert!(stopping.elapsed() < std::time::Duration::from_millis(500));
    }

    #[test]
    fn a_broken_dump_lists_nothing() {
        assert!(parse(b"not json").is_empty());
        assert!(parse(b"{}").is_empty());
    }
}
