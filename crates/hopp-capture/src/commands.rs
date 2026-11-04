//! `hopp capture …`: read, search and annotate captured calls. These work with Hopp
//! closed; everything they need is in the store.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde_json::json;
use std::{io::Read, path::PathBuf, time::Duration};

use super::{
    config, models,
    recorder::{Input, Options, Recorder, Speaker},
    resample,
    store::{self, Call, Store},
    vad, wav,
};

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum CaptureCommand {
    /// List captured calls, newest first.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Print a call's transcript as Markdown.
    Show {
        /// Capture id, `latest` or `current`.
        #[arg(default_value = "latest")]
        id: String,
        /// Print the call and its lines as JSON instead.
        #[arg(long)]
        json: bool,
    },
    /// Search every transcript.
    Search {
        #[arg(required = true, num_args = 1..)]
        query: Vec<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Set a call's title or summary.
    Annotate {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        summary: Option<String>,
        /// Read {"title": …, "summary": …} from stdin.
        #[arg(long)]
        json: bool,
    },
    /// Delete a captured call and its transcript.
    Delete { id: String },
    /// Print where captures are stored.
    Path,
    /// Choose and download transcription models.
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    /// Transcribe 16-bit WAV files as if they were a call (for testing): one file, or
    /// one per person as NAME=FILE, each that person's own track from the call's start.
    #[command(hide = true)]
    Transcribe {
        #[arg(required = true)]
        files: Vec<String>,
        /// Who is speaking in a file given without a name.
        #[arg(long, default_value = "You")]
        speaker: String,
        /// Play the files at their own pace, as a call would, with drafts while people
        /// speak: to watch or measure the live transcript.
        #[arg(long)]
        realtime: bool,
    },
}

#[derive(Subcommand, Clone, Debug, PartialEq, Eq)]
pub enum ModelCommand {
    /// List the models and which are downloaded.
    List,
    /// Download a model.
    Download { name: String },
    /// Transcribe future captures with this model, downloading it if needed.
    Use { name: String },
    /// Delete a downloaded model (not the one in use).
    Delete { name: String },
}

pub fn run(command: CaptureCommand) -> Result<()> {
    match command {
        CaptureCommand::Path => {
            println!("{}", store::default_path()?.display());
            return Ok(());
        }
        CaptureCommand::Model { command } => return model(command),
        CaptureCommand::Transcribe {
            files,
            speaker,
            realtime,
        } => return transcribe(&files, &speaker, realtime),
        _ => {}
    }
    let store = Store::open_default()?;
    // A capture whose recorder died would otherwise show as capturing for good.
    if let Err(error) = super::liveness::close_orphans(&store) {
        eprintln!("Could not close an abandoned capture: {error:#}");
    }
    match command {
        CaptureCommand::Path | CaptureCommand::Model { .. } | CaptureCommand::Transcribe { .. } => {
            unreachable!()
        }
        CaptureCommand::List { json, limit } => {
            let calls = store.calls(limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&calls)?);
            } else if calls.is_empty() {
                println!("No captured calls yet");
            } else {
                for call in calls {
                    println!("{}", list_row(&call));
                }
            }
        }
        CaptureCommand::Show { id, json } => {
            let call = store.require(&id)?;
            if json {
                let lines = store.transcript(&call.id)?;
                let messages = store::messages(&lines);
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({ "call": call, "messages": messages, "lines": lines })
                    )?
                );
            } else {
                print!("{}", store.markdown(&call)?);
            }
        }
        CaptureCommand::Search { query, json, limit } => {
            let hits = store.search(&query.join(" "), limit)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&hits)?);
            } else if hits.is_empty() {
                println!("Nothing found");
            } else {
                for hit in hits {
                    let who = hit.line.speaker.as_deref().unwrap_or("—");
                    println!(
                        "{}  {}  {who}: {}",
                        hit.call_id,
                        store::clock(hit.line.t_ms),
                        hit.snippet
                    );
                }
            }
        }
        CaptureCommand::Annotate {
            id,
            mut title,
            mut summary,
            json,
        } => {
            if json {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input)?;
                let value = loose_json(&input)?;
                title = title.or_else(|| value["title"].as_str().map(str::to_owned));
                summary = summary.or_else(|| value["summary"].as_str().map(str::to_owned));
            }
            if title.is_none() && summary.is_none() {
                bail!("Give --title, --summary or --json");
            }
            let call = store.require(&id)?;
            store.annotate(&call.id, title.as_deref(), summary.as_deref())?;
        }
        CaptureCommand::Delete { id } => {
            let call = store.require(&id)?;
            if call.live() {
                bail!("{} is still being captured; stop it first", call.id);
            }
            store.delete(&call.id)?;
            let markdown = super::markdown_path(&call.id)?;
            let _ = std::fs::remove_file(markdown);
            println!("Deleted {}", call.id);
        }
    }
    Ok(())
}

fn download_with_progress(model: &'static models::Model) -> Result<PathBuf> {
    if models::installed(model) {
        return models::path(model);
    }
    let mut shown = u64::MAX;
    let path = models::download(model, |done, total| {
        let percent = done * 100 / total.max(1);
        if percent != shown {
            shown = percent;
            eprint!("\rDownloading {} … {percent}%", model.name);
        }
    })?;
    eprintln!();
    Ok(path)
}

fn model(command: ModelCommand) -> Result<()> {
    match command {
        ModelCommand::List => {
            let chosen = config::load().model;
            for model in &models::MODELS {
                let mark = if model.name == chosen { "*" } else { " " };
                let state = if models::shared(model) {
                    "Hex's copy"
                } else if models::installed(model) {
                    "downloaded"
                } else {
                    ""
                };
                println!(
                    "{mark} {:<15} {:>5} MB  {:<10}  {}",
                    model.name,
                    model.bytes / 1_000_000,
                    state,
                    model.note
                );
            }
        }
        ModelCommand::Download { name } => {
            let path = download_with_progress(models::find(&name)?)?;
            println!("{}", path.display());
        }
        ModelCommand::Delete { name } => {
            let model = models::find(&name)?;
            if config::load().model == model.name {
                bail!("{} is in use; choose another first", model.name);
            }
            if !models::installed(model) {
                bail!("{} isn't downloaded", model.name);
            }
            models::delete(model)?;
            println!("Deleted {} ({} MB)", model.name, model.bytes / 1_000_000);
        }
        ModelCommand::Use { name } => {
            let model = models::find(&name)?;
            download_with_progress(model)?;
            let mut chosen = config::load();
            chosen.model = model.name.into();
            config::save(&chosen)?;
            println!("Captures will use {}", model.name);
        }
    }
    Ok(())
}

fn transcribe(files: &[String], speaker: &str, realtime: bool) -> Result<()> {
    let mut tracks = Vec::new();
    for file in files {
        let (name, path) = match file.split_once('=') {
            Some((name, path)) if !name.is_empty() => (name, path),
            _ => (speaker, file.as_str()),
        };
        let (rate, samples) =
            wav::read(&std::fs::read(path).with_context(|| format!("Reading {path}"))?)?;
        let who = std::sync::Arc::new(Speaker {
            id: name.to_lowercase(),
            name: name.into(),
        });
        tracks.push((who, resample::to_call_rate(rate, &samples)));
    }
    let model = models::find(&config::load().model)?;
    download_with_progress(model)?;
    let recorder = Recorder::start(Options {
        room: files
            .iter()
            .map(|file| file.rsplit('/').next().unwrap_or(file))
            .collect::<Vec<_>>()
            .join(" + "),
        model,
        backlog_ms: i64::MAX,
        drain_all: true,
        drafts: realtime,
        gpu: config::load().gpu,
        store: store::default_path()?,
        changed: std::sync::Arc::new(|| {}),
    })?;
    let id = recorder.id();
    // 100 ms of every track at a time, timed as if it were live.
    const CHUNK: usize = vad::FRAME * 10;
    let longest = tracks
        .iter()
        .map(|(_, samples)| samples.len())
        .max()
        .unwrap_or(0);
    let playing = std::time::Instant::now();
    for start in (0..longest).step_by(CHUNK) {
        if realtime {
            let due = Duration::from_millis((start * 1000 / vad::RATE) as u64);
            if let Some(wait) = due.checked_sub(playing.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        for (who, samples) in &tracks {
            if let Some(chunk) = samples.get(start..(start + CHUNK).min(samples.len())) {
                recorder.send(Input::Audio {
                    speaker: who.clone(),
                    t_ms: (start * 1000 / vad::RATE) as i64,
                    samples: chunk.to_vec(),
                });
            }
        }
    }
    // Trailing silence ends the last utterances as it would in a call.
    for (who, samples) in tracks {
        recorder.send(Input::Audio {
            speaker: who,
            t_ms: (samples.len() * 1000 / vad::RATE) as i64,
            samples: vec![0; vad::RATE],
        });
    }
    let started = std::time::Instant::now();
    let status = recorder.finish()?;
    let store = Store::open_default()?;
    let call = store.require(&id)?;
    print!("{}", store.markdown(&call)?);
    eprintln!("Transcribed in {:.1} s", started.elapsed().as_secs_f64());
    let audio_s = longest as f64 / vad::RATE as f64;
    eprintln!(
        "Transcriber busy: {:.1} s on drafts, {:.1} s on final text, for {audio_s:.0} s of audio ({:.0}% of it), on {}",
        status.draft_ms as f64 / 1000.0,
        status.final_ms as f64 / 1000.0,
        (status.draft_ms + status.final_ms) as f64 / 10.0 / audio_s.max(1.0),
        status.device.as_deref().unwrap_or("nothing"),
    );
    Ok(())
}

fn list_row(call: &Call) -> String {
    let length = match call.ended_ms {
        Some(ended) => store::duration(ended - call.started_ms),
        None => "capturing".into(),
    };
    let with = if call.participants.is_empty() {
        "—".into()
    } else {
        call.participants.join(", ")
    };
    format!(
        "{}  {}  {:>10}  {:<20}  {}",
        call.id,
        store::local_time(call.started_ms, "%a %-d %b %H:%M"),
        length,
        with,
        call.heading()
    )
}

/// Agents often wrap JSON in prose or code fences; take the outermost object.
fn loose_json(input: &str) -> Result<serde_json::Value> {
    let start = input.find('{').context("No JSON object on stdin")?;
    let end = input.rfind('}').context("No JSON object on stdin")?;
    serde_json::from_str(&input[start..=end]).context("Reading the JSON on stdin")
}
