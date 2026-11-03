//! The Scrawl-in-Hopp end-to-end check: a scripted pairing call on a local Hopp server.
//!
//! - **Alex** shares a screen, talks and draws through a bare call session, as a Mac
//!   client would: slides from `scenario.json`, a WAV for a voice, upstream draw events.
//! - **Sam** is the real engine, capturing: their voice is a WAV too, they draw, pin and
//!   bookmark with the same commands the UI sends, and their speaker is silent.
//!
//! Afterwards the capture must hold both people's speech and a frame for each mark,
//! with the ink where it was drawn. Run with `dev/scrawl-e2e/run.sh`, which sets:
//! HOPP_API_TEST_FIXTURE, HOPP_SCRAWL_SCENARIO, HOPP_SCRAWL_OUT, and temporary
//! XDG_CONFIG_HOME and XDG_DATA_HOME (checked: this never touches your own captures).

use crate::{
    api::ApiClient,
    capture::{moments, store::Store},
    engine::{Command, Engine, Event},
    media::{SILENT_SPEAKER, Session, WAV_MICROPHONE},
    protocol::{InputEvent, PathPoint, Point},
    settings::Settings,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct User {
    token: String,
}
#[derive(Deserialize)]
struct Fixture {
    api_base: String,
    livekit_url: String,
    room_id: String,
    publisher: User,
    viewer: User,
}

#[derive(Deserialize, Clone)]
struct Slide {
    at: f64,
    png: PathBuf,
}
#[derive(Deserialize, Clone)]
struct Mark {
    at: f64,
    by: String,
    kind: String,
    about: String,
    #[serde(default)]
    points: Vec<[f64; 2]>,
    point: Option<[f64; 2]>,
}
#[derive(Deserialize, Clone)]
struct Bookmark {
    at: f64,
    label: String,
}
#[derive(Deserialize, Clone)]
struct StampStep {
    at: f64,
    by: String,
    point: [f64; 2],
    text: String,
}
#[derive(Deserialize)]
struct Voices {
    alex: PathBuf,
    sam: PathBuf,
}
#[derive(Deserialize)]
struct Scenario {
    duration: f64,
    slides: Vec<Slide>,
    voices: Voices,
    marks: Vec<Mark>,
    bookmarks: Vec<Bookmark>,
    #[serde(default)]
    stamps: Vec<StampStep>,
}

/// A slide: when it's shown, and its pixels (width, height, BGRA).
type TimedSlide = (f64, (u32, u32, Vec<u8>));
/// Something the script does at a time in seconds.
type Step = (f64, Box<dyn FnOnce() -> Result<()>>);

fn temporary(variable: &str) -> Result<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os(variable).with_context(|| format!("Set {variable}"))?);
    ensure!(
        path.starts_with("/tmp") && path != std::path::Path::new("/tmp"),
        "{variable} must be a dedicated directory under /tmp"
    );
    Ok(path)
}

fn wait(
    engine: &Engine,
    what: &str,
    timeout: Duration,
    mut done: impl FnMut(&Event) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(event) = engine.events.recv_timeout(Duration::from_millis(50)) {
            if let Event::Error(error) = &event {
                eprintln!("[sam] error: {error}");
            }
            if done(&event) {
                return Ok(());
            }
        }
    }
    bail!("Timed out waiting for {what}")
}

fn bgra(path: &PathBuf) -> Result<(u32, u32, Vec<u8>)> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok((width, height, pixels))
}

/// A stroke's events at about 60 per second, ending with DrawEnd.
fn stroke_events(points: &[[f64; 2]], path_id: u64) -> Vec<InputEvent> {
    let point = |p: &[f64; 2]| Point {
        x: p[0].clamp(0., 1.),
        y: p[1].clamp(0., 1.),
    };
    let mut events = vec![InputEvent::DrawStart(PathPoint {
        point: point(&points[0]),
        path_id,
    })];
    for (index, p) in points.iter().enumerate().skip(1) {
        events.push(if index + 1 == points.len() {
            InputEvent::DrawEnd(point(p))
        } else {
            InputEvent::DrawAddPoint(point(p))
        });
    }
    events
}

#[test]
#[ignore = "run via dev/scrawl-e2e/run.sh: needs the local Hopp server and a scenario"]
fn scripted_call_becomes_frames_and_transcript() -> Result<()> {
    let _config = temporary("XDG_CONFIG_HOME")?;
    let _data = temporary("XDG_DATA_HOME")?;
    let out = PathBuf::from(std::env::var_os("HOPP_SCRAWL_OUT").context("Set HOPP_SCRAWL_OUT")?);
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(
        std::env::var("HOPP_API_TEST_FIXTURE").context("Set HOPP_API_TEST_FIXTURE")?,
    )?)?;
    let scenario: Scenario = serde_json::from_slice(&std::fs::read(
        std::env::var("HOPP_SCRAWL_SCENARIO").context("Set HOPP_SCRAWL_SCENARIO")?,
    )?)?;
    let slides: Vec<TimedSlide> = scenario
        .slides
        .iter()
        .map(|slide| Ok((slide.at, bgra(&slide.png)?)))
        .collect::<Result<_>>()?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    // LiveKit mutes and unmutes tracks on the runtime, including when Alex's session drops.
    let _runtime = runtime.enter();
    let api = ApiClient::new(&fixture.api_base, Some(fixture.viewer.token.clone()))?;
    let room = runtime
        .block_on(api.rooms())?
        .into_iter()
        .find(|room| room.id == fixture.room_id)
        .context("Fixture room no longer exists")?;

    // Sam: the engine under test.
    let sam = Engine::start(Settings {
        server: fixture.api_base.clone(),
        token: Some(fixture.viewer.token.clone()),
        input_device: Some(format!("{WAV_MICROPHONE}{}", scenario.voices.sam.display())),
        output_device: Some(SILENT_SPEAKER.into()),
        ..Default::default()
    });
    wait(&sam, "Sam signing in", Duration::from_secs(20), |e| {
        matches!(e, Event::LoggedIn(_))
    })?;
    sam.commands
        .blocking_send(Command::JoinRoom(room.clone()))?;
    wait(&sam, "Sam joining", Duration::from_secs(30), |e| {
        matches!(e, Event::Joined(_))
    })?;

    // Alex: a bare session, like any other Hopp client in the room.
    let alex_api = ApiClient::new(&fixture.api_base, Some(fixture.publisher.token.clone()))?;
    let tokens = runtime.block_on(alex_api.join_room(&fixture.room_id))?;
    let (alex_events, _alex_rx) = crate::wake::detached();
    let mut alex = runtime.block_on(Session::connect(
        &fixture.livekit_url,
        &tokens.audio_token,
        &tokens.video_token,
        alex_events,
        crate::frames::slot(),
        Arc::default(),
        crate::capture::Tap::default(),
    ))?;
    runtime.block_on(alex.select_audio_devices(
        Some(format!(
            "{WAV_MICROPHONE}{}",
            scenario.voices.alex.display()
        )),
        Some(SILENT_SPEAKER.into()),
    ))?;

    // Alex shares the first slide; Sam starts capturing and waits for the model.
    let current = Arc::new(Mutex::new(0usize));
    let screen = alex.screen_publisher();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let publisher = {
        let (current, stop) = (current.clone(), stop.clone());
        let slides: Vec<(u32, u32, Vec<u8>)> = slides.iter().map(|(_, s)| s.clone()).collect();
        let handle = runtime.handle().clone();
        std::thread::spawn(move || -> Result<()> {
            // Publishing unmutes the track, which LiveKit does on the runtime.
            let _runtime = handle.enter();
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                let index = *current.lock().unwrap();
                let (width, height, pixels) = &slides[index];
                screen.publish(crate::convert::ScreenFrame::from_bgra(
                    *width, *height, pixels,
                )?)?;
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(())
        })
    };
    wait(&sam, "Alex's screen", Duration::from_secs(30), |e| {
        matches!(e, Event::RemoteShare { active: true, .. })
    })?;
    sam.commands.blocking_send(Command::StartCapture)?;
    let mut capture_id = String::new();
    wait(
        &sam,
        "the capture to be ready",
        Duration::from_secs(120),
        |e| match e {
            Event::Capture(Some(status)) => {
                capture_id = status.id.clone();
                eprintln!("[sam] capture {} {}", status.id, status.state);
                status.state == "ready"
            }
            _ => false,
        },
    )?;

    // The call: both voices start together, then the script runs on that clock.
    let started = Instant::now();
    runtime.block_on(alex.set_microphone(true))?;
    sam.commands.blocking_send(Command::Microphone(true))?;
    eprintln!("[call] started; capture {capture_id}");
    let mut timeline: Vec<Step> = Vec::new();
    for (index, (at, _)) in slides.iter().enumerate().skip(1) {
        let current = current.clone();
        timeline.push((
            *at,
            Box::new(move || {
                *current.lock().unwrap() = index;
                eprintln!("[alex] slide {}", index + 1);
                Ok(())
            }),
        ));
    }
    // Shared with the script's steps, all on this thread.
    #[allow(clippy::arc_with_non_send_sync)]
    let alex = Arc::new(alex);
    let handle = runtime.handle().clone();
    for (number, mark) in scenario.marks.iter().cloned().enumerate() {
        let path_id = 1000 + number as u64;
        let events: Vec<InputEvent> = match mark.kind.as_str() {
            "pin" => {
                let [x, y] = mark.point.context("A pin needs a point")?;
                vec![InputEvent::ClickAnimation(Point { x, y })]
            }
            _ => stroke_events(&mark.points, path_id),
        };
        let (commands, alex, handle) = (sam.commands.clone(), alex.clone(), handle.clone());
        timeline.push((
            mark.at,
            Box::new(move || {
                eprintln!("[{}] marks {}", mark.by, mark.about);
                for event in events {
                    if mark.by == "sam" {
                        commands.blocking_send(Command::SendInput(event))?;
                    } else {
                        let payload = serde_json::to_vec(&event)?;
                        handle.block_on(alex.send_data(payload, Some("draw".into()), vec![]))?;
                    }
                    std::thread::sleep(Duration::from_millis(16));
                }
                Ok(())
            }),
        ));
    }
    for bookmark in scenario.bookmarks.iter().cloned() {
        let commands = sam.commands.clone();
        timeline.push((
            bookmark.at,
            Box::new(move || {
                eprintln!("[sam] bookmarks \"{}\"", bookmark.label);
                commands.blocking_send(Command::Bookmark(Some(bookmark.label)))?;
                Ok(())
            }),
        ));
    }
    for (number, step) in scenario.stamps.iter().cloned().enumerate() {
        let (commands, alex, handle) = (sam.commands.clone(), alex.clone(), handle.clone());
        timeline.push((
            step.at,
            Box::new(move || {
                use crate::protocol::{STAMP_TOPIC, StampMessage};
                eprintln!("[{}] stamps \"{}\"", step.by, step.text);
                let id = 500 + number as u64;
                let [x, y] = step.point;
                // The pin, then the words arriving as they're said, then done.
                let words: Vec<&str> = step.text.split_whitespace().collect();
                let mut messages = vec![StampMessage::StampPlace { id, x, y }];
                for count in 1..=words.len() {
                    messages.push(StampMessage::StampText {
                        id,
                        text: words[..count].join(" "),
                        draft: String::new(),
                        done: count == words.len(),
                    });
                }
                for message in messages {
                    if step.by == "sam" {
                        commands.blocking_send(Command::SendStamp(message))?;
                    } else {
                        let place = matches!(message, StampMessage::StampPlace { .. });
                        handle.block_on(alex.send_data(
                            serde_json::to_vec(&message)?,
                            Some(STAMP_TOPIC.into()),
                            vec![],
                        ))?;
                        if place {
                            let ripple = InputEvent::ClickAnimation(Point { x, y });
                            handle.block_on(alex.send_data(
                                serde_json::to_vec(&ripple)?,
                                Some("draw".into()),
                                vec![],
                            ))?;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(180));
                }
                Ok(())
            }),
        ));
    }
    timeline.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (at, action) in timeline {
        let due = started + Duration::from_secs_f64(at);
        while Instant::now() < due {
            let _ = sam.events.recv_timeout(Duration::from_millis(20));
        }
        action()?;
    }
    while started.elapsed() < Duration::from_secs_f64(scenario.duration) {
        let _ = sam.events.recv_timeout(Duration::from_millis(50));
    }

    // Stop, and wait for the last transcription and frames to be saved.
    sam.commands.blocking_send(Command::StopCapture)?;
    let store = Store::open_default()?;
    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        if store.call(&capture_id)?.is_some_and(|call| !call.live()) {
            break;
        }
        ensure!(Instant::now() < deadline, "The capture never finished");
        let _ = sam.events.recv_timeout(Duration::from_millis(200));
    }
    stop.store(true, std::sync::atomic::Ordering::Release);
    let _ = publisher.join();
    let _ = sam.commands.blocking_send(Command::Leave);
    let _ = sam.commands.blocking_send(Command::Shutdown);

    // What the capture holds.
    let call = store.require(&capture_id)?;
    let transcript = store.markdown(&call)?;
    let frames = moments::list(&capture_id)?;
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("transcript.md"), &transcript)?;
    std::fs::write(
        out.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "capture": capture_id,
            "frames_dir": moments::dir(&capture_id)?,
            "frames": frames,
        }))?,
    )?;
    eprintln!("{transcript}");

    let lines = store.transcript(&capture_id)?;
    let speakers: std::collections::BTreeSet<_> = lines
        .iter()
        .filter(|l| l.kind == crate::capture::store::Kind::Speech)
        .filter_map(|l| l.speaker.clone())
        .collect();
    ensure!(
        speakers.len() == 2,
        "Expected both people's speech, got {speakers:?}"
    );
    // Every mark is on a frame, inside the bounds it was drawn in.
    let mut problems = Vec::new();
    for mark in &scenario.marks {
        let (left, top, right, bottom) = match mark.point {
            Some([x, y]) => (x, y, x, y),
            None => mark.points.iter().fold((1f64, 1f64, 0f64, 0f64), |b, p| {
                (b.0.min(p[0]), b.1.min(p[1]), b.2.max(p[0]), b.3.max(p[1]))
            }),
        };
        let found = frames.iter().any(|frame| {
            frame.marks.iter().any(|m| {
                (m.bounds[0] - left).abs() < 0.01
                    && (m.bounds[1] - top).abs() < 0.01
                    && (m.bounds[2] - right).abs() < 0.01
                    && (m.bounds[3] - bottom).abs() < 0.01
            })
        });
        if !found {
            problems.push(format!(
                "no frame shows {} ({} at {})",
                mark.about, mark.by, mark.at
            ));
        }
    }
    ensure!(
        frames.iter().any(|f| f.trigger == "bookmark"),
        "No frame for the bookmark"
    );
    for step in &scenario.stamps {
        let found = frames.iter().any(|frame| {
            frame.marks.iter().any(|m| {
                m.shape == "stamp"
                    && m.text.as_deref() == Some(step.text.as_str())
                    && (m.bounds[0] - step.point[0]).abs() < 0.01
                    && (m.bounds[1] - step.point[1]).abs() < 0.01
            })
        });
        if !found {
            problems.push(format!(
                "no frame shows {}'s stamp \"{}\"",
                step.by, step.text
            ));
        }
    }
    ensure!(problems.is_empty(), "{}", problems.join("; "));
    Ok(())
}
