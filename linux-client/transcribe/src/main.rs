//! `hopp-transcribe`: transcribe.cpp in a process of its own, so a model's memory and
//! any crash stay out of the Hopp client. Hopp starts it when a capture starts and
//! stops it when the capture ends.
//!
//! It runs GGUF models of any family transcribe.cpp knows: Parakeet (the default) and
//! Whisper are the ones Hopp offers. It runs on the GPU through Vulkan, which is faster
//! and uses less energy, or on the CPU (`--cpu`), in a scope that only gets idle time so
//! the desktop never notices it. On Apple silicon the GPU is only smooth with a kernel
//! whose driver lets the firmware preempt compute work; without it the compositor
//! waits behind each pass and the cursor lags (tasks/capture-bench). Hopp sends several
//! utterances at once, a second apart, and splits the reply back up by word timings.
//!
//! Protocol, one request at a time:
//! - stdin: a big-endian u32 length, a JSON header `{"id": n, "samples": count}`, then
//!   `count` little-endian f32 samples of 16 kHz mono audio.
//! - stdout: JSON lines. First `{"ready": true, "device": …}` or `{"error": …}`, then
//!   one `{"id", "text", "segments": [{t0_ms, t1_ms, text}], "lang", "ms"}` per
//!   request. With word timings each segment is one word, and its text starts with
//!   the space before it, as Whisper's tokens do.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufWriter, Read, Write};
use transcribe_cpp::{
    Backend, DeviceType, Model, ModelOptions, RunExtension, RunOptions, Session, SessionOptions,
    TimestampKind, WhisperRunOptions,
};

const SAMPLE_RATE: usize = 16_000;
/// Hopp never sends more than 30 s (Whisper's window).
const MAX_SAMPLES: usize = SAMPLE_RATE * 30;
/// Shorter requests are padded with silence: Whisper needs a second, and Parakeet
/// mishears audio shorter than that.
const MIN_SAMPLES: usize = SAMPLE_RATE + SAMPLE_RATE / 10;
/// Whisper segments it rates as probably not speech.
const NO_SPEECH_LIMIT: f32 = 0.6;

#[derive(Deserialize)]
struct Header {
    id: u64,
    samples: usize,
}

#[derive(Serialize)]
struct Segment {
    t0_ms: i64,
    t1_ms: i64,
    text: String,
}

#[derive(Serialize)]
struct Transcript {
    id: u64,
    text: String,
    segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lang: Option<String>,
    /// How long transcribing took.
    ms: u64,
}

struct Options {
    model: String,
    threads: i32,
    language: String,
    /// Stay off the GPU even when there is one.
    cpu: bool,
    /// One segment per word, so batched utterances can be split back apart.
    words: bool,
}

fn switch(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("Expected on or off, not {value}"),
    }
}

fn options() -> Result<Options> {
    let mut options = Options {
        model: String::new(),
        threads: 2,
        language: "auto".into(),
        cpu: false,
        words: true,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || {
            arguments
                .next()
                .with_context(|| format!("{argument} needs a value"))
        };
        match argument.as_str() {
            "--model" => options.model = value()?,
            "--threads" => options.threads = value()?.parse().context("--threads")?,
            "--language" => options.language = value()?,
            "--words" => options.words = switch(&value()?)?,
            "--cpu" => options.cpu = true,
            "--help" | "-h" => {
                println!(
                    "hopp-transcribe --model PATH.gguf [--threads N] [--language auto|en|…]\n\
                     \x20               [--words on|off] [--cpu]"
                );
                std::process::exit(0);
            }
            _ => bail!("Unknown argument {argument}"),
        }
    }
    if options.model.is_empty() {
        bail!("--model is required");
    }
    options.threads = options.threads.clamp(1, 8);
    Ok(options)
}

/// The GPU to run on: the first real one Vulkan offers. Software renderers such as
/// llvmpipe run on the CPU anyway, only slower than ggml's own CPU code.
fn gpu() -> Option<transcribe_cpp::Device> {
    transcribe_cpp::devices().into_iter().find(|device| {
        let name = format!("{} {}", device.name, device.description).to_lowercase();
        device.kind == "vulkan"
            && matches!(device.device_type, DeviceType::Gpu | DeviceType::Igpu)
            && !["llvmpipe", "lavapipe", "swiftshader", "softpipe"]
                .iter()
                .any(|software| name.contains(software))
    })
}

/// A word as Hopp's splitter expects it: led by the space before it.
fn spaced(text: &str) -> String {
    if text.starts_with(char::is_whitespace) {
        text.to_owned()
    } else {
        format!(" {text}")
    }
}

/// Transcribe one request (at least a second long).
///
/// One session serves every request. Creating one costs about 220 ms on the M2 Max's
/// GPU, nearly all of a short request, and drafts send several a second. A session
/// keeps scheduler buffers sized for the longest audio it has run, which Hopp caps at
/// 30 s (`MAX_SAMPLES`), so its memory stays bounded. After a failed run it is
/// replaced, in case the failure left it in a bad state.
fn transcribe(
    model: &Model,
    session: &mut Option<Session>,
    session_options: &SessionOptions,
    run: &RunOptions,
    words: bool,
    samples: &[f32],
) -> Result<(Vec<Segment>, Option<String>)> {
    let current = match session {
        Some(current) => current,
        None => session.insert(model.session_with(session_options)?),
    };
    let transcript = match current.run(samples, run) {
        Ok(transcript) => transcript,
        Err(error) => {
            *session = None;
            return Err(error.into());
        }
    };
    let segments = if words && !transcript.words.is_empty() {
        transcript
            .words
            .iter()
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| Segment {
                t0_ms: word.t0_ms,
                t1_ms: word.t1_ms,
                text: spaced(&word.text),
            })
            .collect()
    } else {
        transcript
            .segments
            .iter()
            .filter(|segment| !segment.text.trim().is_empty())
            .map(|segment| Segment {
                t0_ms: segment.t0_ms,
                t1_ms: segment.t1_ms,
                text: spaced(&segment.text),
            })
            .collect()
    };
    Ok((segments, transcript.language))
}

fn emit(out: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

fn fail(out: &mut impl Write, message: String) -> Result<()> {
    emit(out, &serde_json::json!({ "error": message }))?;
    std::process::exit(1);
}

fn main() -> Result<()> {
    let options = options()?;
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    // Without a `log` logger installed, transcribe.cpp's and ggml's messages are
    // dropped; Hopp only wants our JSON.
    transcribe_cpp::init_logging();
    if let Err(error) = transcribe_cpp::init_backends_default() {
        fail(&mut out, format!("Starting transcribe.cpp: {error}"))?;
    }
    let device = if options.cpu { None } else { gpu() };
    let model_options = match &device {
        Some(device) => ModelOptions {
            backend: Backend::Vulkan,
            device: Some(device.clone()),
        },
        None => ModelOptions {
            backend: Backend::Cpu,
            device: None,
        },
    };
    let model = match Model::load_with(&options.model, &model_options) {
        Ok(model) => model,
        Err(error) => return fail(&mut out, format!("Loading {}: {error}", options.model)),
    };
    let capabilities = model.capabilities();
    let architecture = model.arch();
    // On the GPU only the per-token decoder runs on the CPU; extra spinning ggml
    // workers just stall it (Hex measured this), so it keeps to one thread.
    let session_options = SessionOptions {
        n_threads: if device.is_some() { 1 } else { options.threads },
        ..Default::default()
    };
    let words = options.words
        && matches!(
            capabilities.max_timestamp_kind,
            TimestampKind::Word | TimestampKind::Token
        );
    // A language hint only where the model knows that language; otherwise it detects
    // the language itself, or has only one.
    let language = Some(options.language.as_str())
        .filter(|language| *language != "auto")
        .filter(|language| {
            capabilities.languages.is_empty()
                || capabilities.languages.iter().any(|known| known == language)
        })
        .map(str::to_owned);
    let run = RunOptions {
        timestamps: if words {
            TimestampKind::Word
        } else {
            TimestampKind::Segment
        },
        language,
        // Each request stands alone: carrying text over is how Whisper starts
        // repeating itself.
        family: (architecture == "whisper").then(|| {
            RunExtension::Whisper(WhisperRunOptions {
                condition_on_prev_tokens: Some(false),
                temperature: Some(0.0),
                no_speech_thold: Some(NO_SPEECH_LIMIT),
                ..Default::default()
            })
        }),
        ..Default::default()
    };
    emit(
        &mut out,
        &serde_json::json!({
            "ready": true,
            "model": options.model,
            "architecture": architecture,
            "variant": model.variant(),
            "threads": session_options.n_threads,
            "languages": capabilities.languages,
            "device": device.map_or_else(|| "CPU".to_owned(), |device| device.description),
            "words": words,
        }),
    )?;

    let mut session = None;
    let mut stdin = std::io::stdin().lock();
    let mut samples = Vec::new();
    let mut bytes = Vec::new();
    loop {
        let mut length = [0u8; 4];
        match stdin.read_exact(&mut length) {
            Ok(()) => {}
            // Hopp closed the pipe: the capture is over.
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > 4096 {
            bail!("Bad header length {length}");
        }
        let mut header = vec![0; length];
        stdin.read_exact(&mut header)?;
        let header: Header = serde_json::from_slice(&header)?;
        if header.samples > MAX_SAMPLES {
            bail!("Request too long: {} samples", header.samples);
        }
        bytes.resize(header.samples * 4, 0);
        stdin.read_exact(&mut bytes)?;
        samples.clear();
        samples.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes(*chunk)),
        );
        if samples.len() < MIN_SAMPLES {
            samples.resize(MIN_SAMPLES, 0.0);
        }
        let started = std::time::Instant::now();
        // A failed request comes back empty rather than ending the helper.
        let (segments, lang) = transcribe(
            &model,
            &mut session,
            &session_options,
            &run,
            words,
            &samples,
        )
        .unwrap_or_default();
        let text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<String>()
            .trim()
            .to_owned();
        emit(
            &mut out,
            &Transcript {
                id: header.id,
                text,
                segments,
                lang,
                ms: started.elapsed().as_millis() as u64,
            },
        )?;
    }
}
