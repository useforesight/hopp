//! Moments: Scrawl-style keyframes for a captured call.
//!
//! While a call is captured and someone shares a screen, the moments that matter get a
//! still frame of the shared screen with everyone's ink drawn on it, saved beside the
//! transcript and listed in it as a numbered frame line:
//!
//! - **A stroke ends** (anyone's, yours included): the frame is taken 250 ms later, so
//!   the screen has settled and the whole stroke is on it.
//! - **A pin**, Hopp's click animation, the same.
//! - **A bookmark**, at once.
//! - **The screen changes** and then holds still for [`QUIET`]: a new scene the viewer
//!   would want to see. A new scene (most of the screen different: another app, window
//!   or file) may follow the last frame after [`SCENE_GAP`]; anything smaller (a scroll,
//!   some typing) waits [`SCREEN_GAP`], and then the frame shows where it ended up.
//!
//! Strokes or pins close together in time share one frame. The ideas, and the scene
//! detector's numbers, come from Kit Langton's Scrawl (MIT): a 64×36 luma thumbnail,
//! compared by mean difference and by the fourth most changed block so a moving cursor
//! doesn't count.
//!
//! Each frame is saved as three files:
//!
//! - `frame-NNN.full.webp`: the size it was shared, WebP quality [`QUALITY`], for
//!   zooming into marks and the captures window's full-size view.
//! - `frame-NNN.webp`: at most [`AGENT_EDGE`] on its long side, for agents and cards.
//! - `frame-NNN.json`: what was drawn and where ([`FrameInfo`]).
//!
//! Older captures have `.full.jpg` and `.jpg`, or a full-size `.png`; [`full`] and
//! [`crate::frame_files::small`] find whichever there is.
//!
//! Everything happens on this module's own thread: the engine only hands it events
//! and, twice a second, a copy of the shared screen.

use anyhow::{Context, Result};
use image::RgbaImage;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use crate::recorder::Feed;
use hopp_marks::{colours::canonical_identity, stamps::StampMessage};

/// How often the engine hands over a copy of the shared screen.
pub const SAMPLE_EVERY: Duration = Duration::from_millis(500);
/// After a stroke or pin ends, wait this long for a frame showing it.
const SETTLE: Duration = Duration::from_millis(250);
/// With no newer frame by then (a still screen sends none), use the last one.
const STALE: Duration = Duration::from_millis(1_500);
/// A burst of marks shares one frame, taken once they pause, but no later than this
/// after the first.
const BURST: Duration = Duration::from_secs(3);
/// Strokes stay on frames this long after they end; Hopp's clients fade them at 3 s.
const VISIBLE: Duration = Duration::from_secs(4);
/// The screen has settled once no sample has changed more than a cursor for this long.
/// One still sample isn't enough: scrolling or typing pauses that long all the time.
const QUIET: Duration = Duration::from_secs(2);
/// A new scene, settled, is a frame once this long has passed since the last frame.
const SCENE_GAP: Duration = Duration::from_secs(3);
/// Any smaller settled change waits this long since the last frame. A scroll-heavy
/// call otherwise takes a frame every few seconds, each much like the one before.
/// 10 s rather than 20 since frames became WebP: about the same storage an hour, and a
/// frame is never far behind what people are talking about.
const SCREEN_GAP: Duration = Duration::from_secs(10);
/// A screen that never settles (tests streaming output, a log, a video, a long scroll)
/// still gets a frame this often, rather than none for minutes.
const BUSY: Duration = Duration::from_secs(30);
/// A new scene: at least this many of the thumbnail's 2,304 cells (30%) differ from the
/// last frame. Dark windows share dark backgrounds, so a window opening over half a dark
/// screen changes only about 35%; scrolling a chat or transcript beside other windows
/// stayed under 21% over 25 s on a real call.
const SCENE_CELLS: usize = 2_304 * 3 / 10;
/// Scrawl's thresholds: a frame whose difference passes this is a change.
const SCENE_DELTA: f32 = 0.08;
/// Scrawl's measure ignores anything as small as a cursor, and so also a one-line code
/// edit, which in a pairing call is often the change that matters. A change also counts
/// when at least this many of the thumbnail's 2,304 cells change (a word or two of
/// text; a cursor or blinking caret touches two to four).
const EDIT_CELLS: usize = 6;
/// How much a cell's mean brightness must change to count.
const CELL_DELTA: f32 = 0.025;
/// Long side of the copy agents read.
pub const AGENT_EDGE: u32 = 1440;
/// WebP quality of both copies. On real call frames, quality 85 WebP was 42% smaller
/// than quality 85 JPEG and kept text cleaner (PSNR 41.9 against 39.5 dB); a PNG was
/// eight times the size.
pub const QUALITY: f32 = 85.;
/// Bounds, so a long call or a misbehaving peer can't grow memory.
const MAX_STROKES: usize = 256;
const MAX_POINTS: usize = 4_096;
const MAX_FRAMES: u32 = 2_000;
/// Copies of the screen waiting for the thread. Each can be 15 MB, and it can stall on a
/// slow disk, so the rest are skipped.
const MAX_QUEUED_SAMPLES: usize = 4;

pub use hopp_marks::colours::colour;
use hopp_marks::colours::{Colours, hex};

#[cfg(test)]
use crate::frame_files::full_in;
use crate::frame_files::name;
pub use crate::frame_files::{FrameInfo, MarkInfo, dir, full, info, list, number, remove};

/// A point on the shared screen, each 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub point: Point,
    pub path_id: u64,
}

/// What someone did to the shared screen, from whichever client's wire format.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawEvent {
    DrawStart(PathPoint),
    DrawAddPoint(Point),
    DrawEnd(Point),
    DrawClearAllPaths,
    DrawClearPath { path_id: u64 },
    ClickAnimation(Point),
}

impl DrawEvent {
    pub fn valid(&self) -> bool {
        match self {
            Self::DrawAddPoint(point) | Self::DrawEnd(point) | Self::ClickAnimation(point) => {
                point.valid()
            }
            Self::DrawStart(start) => start.point.valid(),
            Self::DrawClearAllPaths | Self::DrawClearPath { .. } => true,
        }
    }
}

/// A copy of the shared screen, in BGRA.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

enum Message {
    Draw {
        owner: String,
        name: String,
        event: DrawEvent,
        t_ms: i64,
    },
    Bookmark {
        who: Option<String>,
        label: String,
        t_ms: i64,
    },
    Stamp {
        owner: String,
        name: String,
        message: StampMessage,
        t_ms: i64,
    },
    Sample(VideoFrame),
    /// The shared screen changed hands or stopped: older frames and ink no longer apply.
    Screen,
    /// Who's in the call, for everyone's colours.
    People(Vec<String>),
    Stop,
}

/// The engine's side: cheap calls that hand work to the moments thread.
pub struct Moments {
    sender: SyncSender<Message>,
    queued_samples: Arc<AtomicUsize>,
    feed: Feed,
    last_sample: Option<Instant>,
    /// A mark is waiting for a frame: sample the next one whatever the interval.
    urgent: bool,
    thread: Option<JoinHandle<()>>,
}

impl Moments {
    pub fn start(call_id: &str, feed: Feed) -> Result<Self> {
        Self::start_in(dir(call_id)?, feed)
    }

    /// Frames go into `directory`.
    pub fn start_in(directory: PathBuf, feed: Feed) -> Result<Self> {
        let (sender, receiver) = sync_channel(64);
        let queued_samples = Arc::new(AtomicUsize::new(0));
        let worker = Worker::new(directory, feed.clone(), queued_samples.clone());
        let thread = std::thread::Builder::new()
            .name("hopp-moments".into())
            .spawn(move || worker.run(receiver))
            .context("Starting the moments thread")?;
        Ok(Self {
            sender,
            queued_samples,
            feed,
            last_sample: None,
            urgent: false,
            thread: Some(thread),
        })
    }

    fn send(&self, message: Message) {
        // A point in a stroke may be dropped when the thread is busy, because those come
        // fastest and the stroke keeps its shape. Other marks wait their turn.
        match message {
            Message::Draw {
                event: DrawEvent::DrawAddPoint(_),
                ..
            } => {
                let _ = self.sender.try_send(message);
            }
            other => {
                if let Err(TrySendError::Full(other)) = self.sender.try_send(other) {
                    let _ = self.sender.send(other);
                }
            }
        }
    }

    /// A draw event from `owner` (a participant identity), `name` their display name.
    pub fn draw(&mut self, owner: &str, name: &str, event: &DrawEvent) {
        if !event.valid() {
            return;
        }
        if matches!(event, DrawEvent::DrawEnd(_) | DrawEvent::ClickAnimation(_)) {
            self.urgent = true;
        }
        self.send(Message::Draw {
            owner: canonical_identity(owner).into(),
            name: name.into(),
            event: event.clone(),
            t_ms: self.feed.now_ms(),
        });
    }

    /// A stamp: a frame is taken once its placer has finished it, with its words.
    pub fn stamp(&mut self, owner: &str, name: &str, message: &StampMessage) {
        if !message.valid() {
            return;
        }
        if matches!(message, StampMessage::StampText { done: true, .. }) {
            self.urgent = true;
        }
        self.send(Message::Stamp {
            owner: canonical_identity(owner).into(),
            name: name.into(),
            message: message.clone(),
            t_ms: self.feed.now_ms(),
        });
    }

    pub fn bookmark(&mut self, who: Option<String>, label: &str) {
        self.urgent = true;
        self.send(Message::Bookmark {
            who,
            label: label.into(),
            t_ms: self.feed.now_ms(),
        });
    }

    /// The shared screen started, stopped or changed hands.
    pub fn screen_changed(&mut self) {
        self.last_sample = None;
        self.send(Message::Screen);
    }

    /// Whether to copy the next frame of the shared screen for [`Moments::sample`].
    pub fn wants_sample(&self) -> bool {
        self.urgent
            || self
                .last_sample
                .is_none_or(|last| last.elapsed() >= SAMPLE_EVERY)
    }

    /// Note that a sample is on its way (taken off the engine thread).
    pub fn sampling(&mut self) {
        self.last_sample = Some(Instant::now());
        self.urgent = false;
    }

    /// A copy of the shared screen, in BGRA.
    pub fn sample(&mut self, frame: VideoFrame) {
        self.sampling();
        self.sampler().sample(frame);
    }

    /// For other threads (the share encoder) to hand over samples.
    pub fn sampler(&self) -> Sampler {
        Sampler {
            sender: self.sender.clone(),
            queued: self.queued_samples.clone(),
        }
    }

    /// Who's in the call, as identities, so ink has the colours it has on screen.
    pub fn people(&mut self, people: Vec<String>) {
        self.send(Message::People(people));
    }

    /// Write the frames still waiting, then stop. Waits at most a few seconds.
    pub fn finish(mut self) {
        let _ = self.sender.send(Message::Stop);
        if let Some(thread) = self.thread.take() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

#[derive(Clone)]
pub struct Sampler {
    sender: SyncSender<Message>,
    queued: Arc<AtomicUsize>,
}

impl Sampler {
    pub fn sample(&self, frame: VideoFrame) {
        if self.queued.fetch_add(1, Ordering::AcqRel) >= MAX_QUEUED_SAMPLES
            || self.sender.try_send(Message::Sample(frame)).is_err()
        {
            self.queued.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

struct Stroke {
    owner: String,
    name: String,
    id: u64,
    points: Vec<(f64, f64)>,
    ended: Option<Instant>,
    /// Already in a frame's caption.
    reported: bool,
}

struct Pin {
    owner: String,
    name: String,
    /// What was said over it, for a stamp.
    text: Option<String>,
    number: u32,
    point: (f64, f64),
    at: Instant,
    reported: bool,
}

struct Sample {
    frame: VideoFrame,
    at: Instant,
    thumb: Vec<f32>,
}

enum Pending {
    Marks {
        due: Instant,
        since: Instant,
        t_ms: i64,
    },
    Bookmark {
        due: Instant,
        t_ms: i64,
        who: Option<String>,
        label: String,
    },
    Screen {
        due: Instant,
        t_ms: i64,
        first: bool,
    },
}

impl Pending {
    fn due(&self) -> Instant {
        match self {
            Pending::Marks { due, .. }
            | Pending::Bookmark { due, .. }
            | Pending::Screen { due, .. } => *due,
        }
    }
}

struct Worker {
    directory: PathBuf,
    feed: Feed,
    strokes: Vec<Stroke>,
    active: HashMap<String, u64>,
    pins: Vec<Pin>,
    pin_count: u32,
    latest: Option<Sample>,
    /// When the screen alone is worth a frame.
    scene: Scene,
    pending: Vec<Pending>,
    count: u32,
    last_frame: Option<Instant>,
    /// Stamps placed but not finished: (owner, id) to where.
    stamps: HashMap<(String, u64), (f64, f64)>,
    /// The thumbnail of the last frame saved.
    saved_thumb: Option<Vec<f32>>,
    /// Colours given out in this call: one each, different where the palette allows.
    colours: Colours,
    queued_samples: Arc<AtomicUsize>,
}

impl Worker {
    fn new(directory: PathBuf, feed: Feed, queued_samples: Arc<AtomicUsize>) -> Self {
        Self {
            directory,
            feed,
            queued_samples,
            strokes: Vec::new(),
            active: HashMap::new(),
            pins: Vec::new(),
            pin_count: 0,
            latest: None,
            scene: Scene::default(),
            pending: Vec::new(),
            count: 0,
            last_frame: None,
            stamps: HashMap::new(),
            saved_thumb: None,
            colours: Colours::default(),
        }
    }

    fn run(mut self, receiver: Receiver<Message>) {
        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Message::Draw {
                    owner,
                    name,
                    event,
                    t_ms,
                }) => self.draw(owner, name, event, t_ms),
                Ok(Message::Bookmark { who, label, t_ms }) => {
                    self.pending.push(Pending::Bookmark {
                        due: Instant::now(),
                        t_ms,
                        who,
                        label,
                    })
                }
                Ok(Message::Stamp {
                    owner,
                    name,
                    message,
                    t_ms,
                }) => self.stamp(owner, name, message, t_ms),
                Ok(Message::Sample(frame)) => {
                    self.queued_samples.fetch_sub(1, Ordering::AcqRel);
                    self.sample(frame);
                }
                Ok(Message::People(people)) => {
                    self.colours = Colours::for_call(people.iter().map(String::as_str));
                }
                Ok(Message::Screen) => {
                    self.latest = None;
                    self.scene = Scene::default();
                    self.strokes.clear();
                    self.active.clear();
                    self.pins.clear();
                    self.pending
                        .retain(|p| !matches!(p, Pending::Screen { .. }));
                }
                Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    self.flush(true);
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.watch_screen(Instant::now());
            self.flush(false);
        }
    }

    fn draw(&mut self, owner: String, name: String, event: DrawEvent, t_ms: i64) {
        let now = Instant::now();
        match event {
            DrawEvent::DrawStart(start) => {
                self.strokes
                    .retain(|s| s.owner != owner || s.id != start.path_id);
                self.forget_old(now);
                if self.strokes.len() >= MAX_STROKES {
                    self.strokes.remove(0);
                }
                self.colours.of(&owner);
                self.strokes.push(Stroke {
                    owner: owner.clone(),
                    name,
                    id: start.path_id,
                    points: vec![(start.point.x, start.point.y)],
                    ended: None,
                    reported: false,
                });
                self.active.insert(owner, start.path_id);
            }
            DrawEvent::DrawAddPoint(point) | DrawEvent::DrawEnd(point) => {
                let ending = matches!(event, DrawEvent::DrawEnd(_));
                if let Some(id) = self.active.get(&owner).copied()
                    && let Some(stroke) = self
                        .strokes
                        .iter_mut()
                        .find(|s| s.owner == owner && s.id == id)
                {
                    if stroke.points.len() < MAX_POINTS {
                        stroke.points.push((point.x, point.y));
                    }
                    if ending {
                        stroke.ended = Some(now);
                    }
                }
                if ending {
                    self.active.remove(&owner);
                    self.marked(now, t_ms);
                }
            }
            DrawEvent::DrawClearPath { path_id } => {
                // Hopp's clients clear a stroke as it fades. A stroke not yet in a frame
                // stays until it is; only its owner can clear it.
                for stroke in &mut self.strokes {
                    if stroke.owner == owner && stroke.id == path_id && stroke.ended.is_none() {
                        stroke.ended = Some(now);
                    }
                }
                self.strokes
                    .retain(|s| s.owner != owner || s.id != path_id || !s.reported);
            }
            DrawEvent::DrawClearAllPaths => {
                self.strokes.retain(|s| s.owner != owner || !s.reported);
                self.active.remove(&owner);
            }
            DrawEvent::ClickAnimation(point) => {
                // A stamp ripples where it lands. It's numbered when it's finished.
                if self.stamps.iter().any(|((by, _), spot)| {
                    *by == owner
                        && (spot.0 - point.x).abs() < 1e-6
                        && (spot.1 - point.y).abs() < 1e-6
                }) {
                    return;
                }
                self.pin_count += 1;
                self.colours.of(&owner);
                self.pins.push(Pin {
                    owner,
                    name,
                    text: None,
                    number: self.pin_count,
                    point: (point.x, point.y),
                    at: now,
                    reported: false,
                });
                if self.pins.len() > 64 {
                    self.pins.remove(0);
                }
                self.marked(now, t_ms);
            }
        }
    }

    /// A mark ended: take a frame once marks pause for [`SETTLE`].
    fn marked(&mut self, now: Instant, t_ms: i64) {
        for pending in &mut self.pending {
            if let Pending::Marks { due, since, .. } = pending
                && now.duration_since(*since) < BURST
            {
                *due = (now + SETTLE).min(*since + BURST);
                return;
            }
        }
        self.pending.push(Pending::Marks {
            due: now + SETTLE,
            since: now,
            t_ms,
        });
    }

    fn stamp(&mut self, owner: String, name: String, message: StampMessage, t_ms: i64) {
        use StampMessage;
        match message {
            StampMessage::StampPlace { id, x, y } => {
                if self.stamps.len() < 64 {
                    self.stamps.insert((owner, id), (x, y));
                }
            }
            StampMessage::StampText {
                id,
                text,
                done: true,
                ..
            } => {
                let Some(point) = self.stamps.remove(&(owner.clone(), id)) else {
                    return;
                };
                let text = text.trim().to_owned();
                self.pin_count += 1;
                self.colours.of(&owner);
                let now = Instant::now();
                self.pins.push(Pin {
                    owner,
                    name,
                    text: (!text.is_empty()).then_some(text),
                    number: self.pin_count,
                    point,
                    at: now,
                    reported: false,
                });
                if self.pins.len() > 64 {
                    self.pins.remove(0);
                }
                self.marked(now, t_ms);
            }
            StampMessage::StampRemove { id } => {
                self.stamps.remove(&(owner, id));
            }
            StampMessage::StampText { .. } => {}
        }
    }

    fn forget_old(&mut self, now: Instant) {
        self.strokes.retain(|s| {
            s.ended
                .is_none_or(|ended| !s.reported || now.duration_since(ended) < VISIBLE)
        });
        self.pins
            .retain(|p| !p.reported || now.duration_since(p.at) < VISIBLE);
    }

    fn sample(&mut self, frame: VideoFrame) {
        if frame.width < 16
            || frame.height < 16
            || frame.bgra.len() < (frame.width * frame.height * 4) as usize
        {
            return;
        }
        let thumb = thumbnail(&frame);
        let at = Instant::now();
        if let Some(previous) = &self.latest {
            self.scene.sampled(&previous.thumb, &thumb, at);
        } else {
            // The first view of a newly shared screen.
            self.pending.push(Pending::Screen {
                due: at,
                t_ms: self.feed.now_ms(),
                first: true,
            });
        }
        self.latest = Some(Sample { frame, at, thumb });
    }

    /// Queue a frame of the screen alone once [`Scene::due`] says it's worth one.
    fn watch_screen(&mut self, now: Instant) {
        let (Some(latest), Some(saved)) = (&self.latest, &self.saved_thumb) else {
            return;
        };
        if self
            .pending
            .iter()
            .any(|p| matches!(p, Pending::Screen { .. }))
        {
            return;
        }
        if self.scene.due(saved, &latest.thumb, now, self.last_frame) {
            self.pending.push(Pending::Screen {
                due: now,
                t_ms: self.feed.now_ms(),
                first: false,
            });
        }
    }

    /// Take the frames that are due. With `all`, take every waiting frame now, from the
    /// last sample.
    fn flush(&mut self, all: bool) {
        loop {
            let Some(first) = self.pending.iter().map(Pending::due).min() else {
                return;
            };
            let now = Instant::now();
            if !all && first > now {
                return;
            }
            let Some(latest) = &self.latest else {
                if all || now.duration_since(first) > STALE * 4 {
                    // Nothing shared to show: the marks go into the transcript as text.
                    self.pending.clear();
                }
                return;
            };
            // A mark waits for a frame from after it settled, unless the screen is
            // still and sends none. Bookmarks and screen changes use what's there.
            let marks_waiting = self.pending.iter().any(
                |p| matches!(p, Pending::Marks { due, .. } if *due <= now && latest.at < *due),
            );
            if !all && marks_waiting && now.duration_since(first) < STALE {
                return;
            }
            // Everything due by now shares this frame.
            let cutoff = now;
            let (taken, rest): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|p| all || p.due() <= cutoff);
            self.pending = rest;
            if taken.is_empty() {
                return;
            }
            if let Err(error) = self.frame(taken) {
                crate::logger::line(format!("Capture: could not save a frame: {error:#}"));
            }
        }
    }

    fn frame(&mut self, taken: Vec<Pending>) -> Result<()> {
        if self.count >= MAX_FRAMES {
            return Ok(());
        }
        let Some(latest) = &self.latest else {
            return Ok(());
        };
        let now = Instant::now();
        let aspect = f64::from(latest.frame.width) / f64::from(latest.frame.height);
        let t_ms = taken
            .iter()
            .map(|p| match p {
                Pending::Marks { t_ms, .. }
                | Pending::Bookmark { t_ms, .. }
                | Pending::Screen { t_ms, .. } => *t_ms,
            })
            .min()
            .unwrap_or_else(|| self.feed.now_ms());
        let has_marks = taken.iter().any(|p| matches!(p, Pending::Marks { .. }));
        let first_view = taken
            .iter()
            .any(|p| matches!(p, Pending::Screen { first: true, .. }));
        let bookmark = taken.iter().find_map(|p| match p {
            Pending::Bookmark { who, label, .. } => Some((who.clone(), label.clone())),
            _ => None,
        });

        // What's showing, and what's new since the last frame.
        let visible: Vec<&Stroke> = self
            .strokes
            .iter()
            .filter(|s| {
                s.points.len() > 1
                    && s.ended
                        .is_none_or(|ended| now.duration_since(ended) < VISIBLE)
            })
            .collect();
        let visible_pins: Vec<&Pin> = self
            .pins
            .iter()
            .filter(|p| now.duration_since(p.at) < VISIBLE)
            .collect();
        let mut marks = Vec::new();
        let mut ink = Vec::new();
        for stroke in &visible {
            let info = MarkInfo {
                by: stroke.name.clone(),
                shape: shape(&stroke.points, aspect).into(),
                pin: None,
                text: None,
                bounds: bounds(&stroke.points),
                colour: hex(self
                    .colours
                    .get(&stroke.owner)
                    .unwrap_or_else(|| colour(&stroke.owner))),
            };
            if !stroke.reported && stroke.ended.is_some() {
                marks.push(info.clone());
            }
            ink.push(info);
        }
        for pin in &visible_pins {
            let (x, y) = pin.point;
            let info = MarkInfo {
                by: pin.name.clone(),
                shape: if pin.text.is_some() { "stamp" } else { "pin" }.into(),
                pin: Some(pin.number),
                text: pin.text.clone(),
                bounds: [x, y, x, y],
                colour: hex(self
                    .colours
                    .get(&pin.owner)
                    .unwrap_or_else(|| colour(&pin.owner))),
            };
            if !pin.reported {
                marks.push(info.clone());
            }
            ink.push(info);
        }
        if has_marks && marks.is_empty() && bookmark.is_none() && !first_view {
            // The marks were cleared before a frame could show them.
            return Ok(());
        }
        let unchanged = self
            .saved_thumb
            .as_ref()
            .is_some_and(|saved| !moved(saved, &latest.thumb));
        if marks.is_empty() && bookmark.is_none() && !first_view && unchanged {
            // A change that came back, or one an earlier frame already shows.
            return Ok(());
        }
        let trigger = if bookmark.is_some() {
            "bookmark"
        } else if !marks.is_empty() {
            "marks"
        } else {
            "screen"
        };
        self.count += 1;
        let number = self.count;
        let caption = caption(number, &marks, bookmark.as_ref(), trigger, first_view);
        let who = bookmark
            .as_ref()
            .and_then(|(who, _)| who.clone())
            .or_else(|| marks.first().map(|m| m.by.clone()));

        let mut image = rgba(&latest.frame);
        let width = image.width();
        let strokes: Vec<InkStroke> = visible
            .iter()
            .map(|s| {
                (
                    s.points.clone(),
                    self.colours
                        .get(&s.owner)
                        .unwrap_or_else(|| colour(&s.owner)),
                )
            })
            .collect();
        let pins: Vec<InkPin> = visible_pins
            .iter()
            .map(|p| {
                (
                    p.point,
                    p.number,
                    self.colours
                        .get(&p.owner)
                        .unwrap_or_else(|| colour(&p.owner)),
                )
            })
            .collect();
        draw_ink(&mut image, &strokes, &pins);
        chip(&mut image, number, (width as f32 / 560.).clamp(2., 6.));

        let info = FrameInfo {
            number,
            t_ms,
            width: image.width(),
            height: image.height(),
            trigger: trigger.into(),
            caption: caption.clone(),
            marks,
            ink,
        };
        self.write(&image, &info)?;
        for stroke in &mut self.strokes {
            if stroke.ended.is_some() {
                stroke.reported = true;
            }
        }
        for pin in &mut self.pins {
            pin.reported = true;
        }
        self.last_frame = Some(now);
        self.saved_thumb = Some(latest.thumb.clone());
        self.feed.frame(t_ms, who, caption);
        Ok(())
    }

    fn write(&self, image: &RgbaImage, info: &FrameInfo) -> Result<()> {
        crate::paths::private_dir(&self.directory)?;
        let name = |kind: &str| name(&self.directory, info.number, kind);
        crate::write_private(&name("full.webp"), &webp(image)?)?;
        crate::write_private(&name("webp"), &agent_image(image, AGENT_EDGE)?)?;
        crate::write_private(&name("json"), &serde_json::to_vec_pretty(info)?)?;
        Ok(())
    }
}

/// The whole image at its own size, as WebP.
fn webp(image: &RgbaImage) -> Result<Vec<u8>> {
    use image::buffer::ConvertBuffer;
    let rgb: image::RgbImage = image.convert();
    encode_webp(&rgb)
}

fn encode_webp(rgb: &image::RgbImage) -> Result<Vec<u8>> {
    let encoded = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height())
        .encode_simple(false, QUALITY)
        .map_err(|error| anyhow::anyhow!("WebP encoding failed: {error:?}"))?;
    Ok(encoded.to_vec())
}

/// A WebP at most `edge` pixels on its long side.
pub fn agent_image(image: &RgbaImage, edge: u32) -> Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    let scale = (f64::from(edge) / f64::from(width.max(height))).min(1.);
    let rgb = image::DynamicImage::ImageRgba8(image.clone());
    let rgb = if scale < 1. {
        rgb.resize(
            (f64::from(width) * scale).round() as u32,
            (f64::from(height) * scale).round() as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        rgb
    }
    .to_rgb8();
    encode_webp(&rgb)
}

fn rgba(frame: &VideoFrame) -> RgbaImage {
    let mut pixels = frame.bgra[..(frame.width * frame.height * 4) as usize].to_vec();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    RgbaImage::from_raw(frame.width, frame.height, pixels).expect("sized above")
}

/// 64×36 Rec. 709 luma, averaged over each cell.
fn thumbnail(frame: &VideoFrame) -> Vec<f32> {
    const W: usize = 64;
    const H: usize = 36;
    let (fw, fh) = (frame.width as usize, frame.height as usize);
    let mut out = vec![0f32; W * H];
    for (cell, value) in out.iter_mut().enumerate() {
        let (cx, cy) = (cell % W, cell / W);
        let (x0, x1) = (cx * fw / W, ((cx + 1) * fw / W).max(cx * fw / W + 1));
        let (y0, y1) = (cy * fh / H, ((cy + 1) * fh / H).max(cy * fh / H + 1));
        // Every other pixel in each direction is plenty for a thumbnail.
        let (mut sum, mut n) = (0f32, 0f32);
        for y in (y0..y1.min(fh)).step_by(2) {
            for x in (x0..x1.min(fw)).step_by(2) {
                let i = (y * fw + x) * 4;
                let (b, g, r) = (
                    f32::from(frame.bgra[i]),
                    f32::from(frame.bgra[i + 1]),
                    f32::from(frame.bgra[i + 2]),
                );
                sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
                n += 1.;
            }
        }
        *value = if n > 0. { sum / n / 255. } else { 0. };
    }
    out
}

/// Scrawl's frame difference: the larger of the mean absolute difference and 1.4× the
/// fourth most changed block of an 8×6 grid.
fn difference(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.;
    }
    let mean = a.iter().zip(b).map(|(a, b)| (a - b).abs()).sum::<f32>() / a.len() as f32;
    let mut blocks = [0f32; 48];
    let mut counts = [0f32; 48];
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        let (x, y) = (i % 64, i / 64);
        let block = (y * 6 / 36) * 8 + x * 8 / 64;
        blocks[block] += (a - b).abs();
        counts[block] += 1.;
    }
    let mut means: Vec<f32> = blocks
        .iter()
        .zip(counts)
        .map(|(sum, n)| if n > 0. { sum / n } else { 0. })
        .collect();
    means.sort_by(|a, b| b.total_cmp(a));
    mean.max(1.4 * means[3])
}

/// When the shared screen, with nothing new drawn on it, is worth a frame.
#[derive(Default)]
struct Scene {
    /// When a sample last differed from the one before by more than a cursor.
    changed: Option<Instant>,
    /// When the screen started changing after last being quiet.
    busy_since: Option<Instant>,
}

impl Scene {
    /// A sample, `thumb`, arrived at `at` after `previous`.
    fn sampled(&mut self, previous: &[f32], thumb: &[f32], at: Instant) {
        if moved(previous, thumb) {
            if self
                .changed
                .is_none_or(|changed| at.saturating_duration_since(changed) >= QUIET)
            {
                self.busy_since = Some(at);
            }
            self.changed = Some(at);
        }
    }

    /// Whether the screen, `latest`, is worth a frame at `now`: it differs from the last
    /// frame's (`saved`, taken at `last_frame`), has been still for [`QUIET`], and the
    /// last frame is [`SCENE_GAP`] ago for a new scene or [`SCREEN_GAP`] for less. A
    /// screen that keeps changing gets one every [`BUSY`] instead.
    fn due(
        &self,
        saved: &[f32],
        latest: &[f32],
        now: Instant,
        last_frame: Option<Instant>,
    ) -> bool {
        let settled = self
            .changed
            .is_none_or(|changed| now.saturating_duration_since(changed) >= QUIET);
        if !moved(saved, latest) {
            return false;
        }
        if !settled {
            // Counted from the later of the last frame and the start of the changes.
            let since = last_frame.max(self.busy_since);
            return since.is_some_and(|since| now.saturating_duration_since(since) >= BUSY);
        }
        let gap = if new_scene(saved, latest) {
            SCENE_GAP
        } else {
            SCREEN_GAP
        };
        last_frame.is_none_or(|last| now.saturating_duration_since(last) >= gap)
    }
}

/// More than a cursor differs between two thumbnails.
fn moved(a: &[f32], b: &[f32]) -> bool {
    difference(a, b) > SCENE_DELTA || changed_cells(a, b) >= EDIT_CELLS
}

/// Most of the screen differs: another app, window or file, not a scroll.
fn new_scene(a: &[f32], b: &[f32]) -> bool {
    changed_cells(a, b) >= SCENE_CELLS
}

/// Thumbnail cells whose brightness changed by more than [`CELL_DELTA`].
fn changed_cells(a: &[f32], b: &[f32]) -> usize {
    if a.len() != b.len() {
        return usize::MAX;
    }
    a.iter()
        .zip(b)
        .filter(|(a, b)| (*a - *b).abs() > CELL_DELTA)
        .count()
}

/// `[left, top, right, bottom]` of a stroke, in screen fractions.
fn bounds(points: &[(f64, f64)]) -> [f64; 4] {
    let mut out = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for &(x, y) in points {
        out[0] = out[0].min(x);
        out[1] = out[1].min(y);
        out[2] = out[2].max(x);
        out[3] = out[3].max(y);
    }
    out
}

/// What a stroke looks like, roughly: a ring around something, a line under it, a
/// straight line (pointing), or anything else.
fn shape(points: &[(f64, f64)], aspect: f64) -> &'static str {
    let scaled: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (x * aspect, y)).collect();
    let [left, top, right, bottom] = bounds(&scaled);
    let (w, h) = (right - left, bottom - top);
    let size = w.max(h);
    if size < 0.005 {
        return "stroke";
    }
    let length: f64 = scaled
        .windows(2)
        .map(|pair| ((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt())
        .sum();
    let (first, last) = (scaled[0], scaled[scaled.len() - 1]);
    let gap = ((last.0 - first.0).powi(2) + (last.1 - first.1).powi(2)).sqrt();
    if gap < 0.3 * size && length > 2.2 * size && w > 0.2 * size && h > 0.2 * size {
        "circle"
    } else if gap > 0.85 * length && h < 0.25 * w {
        "underline"
    } else if gap > 0.85 * length {
        "line"
    } else {
        "stroke"
    }
}

/// "top left", "middle", "bottom right" …
pub fn place(x: f64, y: f64) -> String {
    let row = if y < 1. / 3. {
        "top"
    } else if y < 2. / 3. {
        "middle"
    } else {
        "bottom"
    };
    let column = if x < 1. / 3. {
        "left"
    } else if x < 2. / 3. {
        "centre"
    } else {
        "right"
    };
    match (row, column) {
        ("middle", "centre") => "middle".into(),
        (row, column) => format!("{row} {column}"),
    }
}

fn caption(
    number: u32,
    marks: &[MarkInfo],
    bookmark: Option<&(Option<String>, String)>,
    trigger: &str,
    first_view: bool,
) -> String {
    let mut parts = Vec::new();
    if let Some((who, label)) = bookmark {
        parts.push(match who {
            Some(who) => format!("{who} bookmarked \"{label}\""),
            None => format!("bookmarked \"{label}\""),
        });
    }
    // A scribble can be several strokes of one shape in one place: say it once.
    let mut said = std::collections::HashSet::new();
    for mark in marks {
        let (x, y) = mark.centre();
        let place = place(x, y);
        if !said.insert((
            mark.by.as_str(),
            mark.shape.as_str(),
            place.clone(),
            mark.pin,
        )) {
            continue;
        }
        let at = format!("{place} ({:.0}%, {:.0}%)", x * 100., y * 100.);
        parts.push(match (mark.shape.as_str(), mark.pin) {
            ("pin", Some(pin)) => format!("{} dropped pin {pin} at {at}", mark.by),
            ("stamp", Some(pin)) => format!(
                "{} stamped pin {pin} at {at}: \"{}\"",
                mark.by,
                mark.text.as_deref().unwrap_or_default()
            ),
            ("circle", _) => format!("{} circled something at {at}", mark.by),
            ("underline", _) => format!("{} underlined something at {at}", mark.by),
            ("line", _) => format!("{} drew a line at {at}", mark.by),
            _ => format!("{} drew at {at}", mark.by),
        });
    }
    if parts.is_empty() && trigger == "screen" {
        parts.push(if first_view {
            "the shared screen".into()
        } else {
            "the shared screen changed".into()
        });
    }
    format!("#{number} {}", parts.join("; "))
}

/// A stroke to draw: its points in screen fractions, and its colour.
type InkStroke = (Vec<(f64, f64)>, [u8; 3]);
/// A pin to draw: where, its number, and its colour.
type InkPin = ((f64, f64), u32, [u8; 3]);

/// Anti-aliased round strokes on a soft halo, then numbered pins. Every halo goes down
/// before any ink, as on screen, so one stroke's halo never dims another's ink.
fn draw_ink(image: &mut RgbaImage, strokes: &[InkStroke], pins: &[InkPin]) {
    use crate::ink_raster;
    let (width, height) = (image.width() as f32, image.height() as f32);
    // Drawn as the viewer draws them, at about the size they appear on a 1440p screen.
    let scale = (width.max(height) / 1440.).clamp(1., 3.);
    let ink_width = (width / 420.).clamp(3.4, 8.);
    // The halo is as much wider than the ink as on screen (5 against 3.5).
    let halo_width = ink_width * 5. / 3.5;
    let device: Vec<(Vec<ink_raster::Xy>, [u8; 3])> = strokes
        .iter()
        .map(|(points, colour)| {
            let pixels = points
                .iter()
                .map(|&(x, y)| (x as f32 * width, y as f32 * height))
                .collect();
            (pixels, *colour)
        })
        .collect();
    for (pixels, colour) in &device {
        let halo = ink_raster::halo_rgb(*colour);
        if let Some(sprite) =
            ink_raster::stroke(pixels, halo_width, halo, ink_raster::HALO_ALPHA, scale)
        {
            ink_raster::composite(image, &sprite, (0., 0.), 1.);
        }
    }
    for (pixels, colour) in &device {
        if let Some(sprite) = ink_raster::stroke(pixels, ink_width, *colour, 1., scale) {
            ink_raster::composite(image, &sprite, (0., 0.), 1.);
        }
    }
    for &((x, y), number, colour) in pins {
        let tip = (x as f32 * width, y as f32 * height);
        if let Some(sprite) = ink_raster::pin(colour, scale) {
            ink_raster::composite(image, &sprite, tip, 1.);
        }
        // Its number in a small dark badge at the head's upper right.
        let head = (
            tip.0 + ink_raster::PIN_RADIUS * scale * 0.85,
            tip.1 - (ink_raster::PIN_RISE + ink_raster::PIN_RADIUS * 0.85) * scale,
        );
        badge(image, &number.to_string(), head, 1.6 * scale);
    }
}

/// Digits in a rounded dark badge centred on `centre`.
fn badge(image: &mut RgbaImage, digits: &str, centre: (f32, f32), scale: f32) {
    let (w, h) = ((digits.len() as f32 * 6. + 5.) * scale, 11. * scale);
    let (x0, y0) = (centre.0 - w / 2., centre.1 - h / 2.);
    let radius = h / 2.;
    for y in y0.floor() as i64..(y0 + h).ceil() as i64 {
        for x in x0.floor() as i64..(x0 + w).ceil() as i64 {
            if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
                continue;
            }
            // Distance outside a rounded rectangle, for an anti-aliased edge.
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let qx = (px - (x0 + w / 2.)).abs() - (w / 2. - radius);
            let qy = (py - (y0 + h / 2.)).abs() - (h / 2. - radius);
            let outside = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius;
            let cover = (0.5 - outside).clamp(0., 1.);
            if cover > 0. {
                blend(image, x as u32, y as u32, [22, 23, 28], 0.9 * cover);
            }
        }
    }
    text(image, digits, centre, scale, [255, 255, 255]);
}

/// The frame number in a dark chip at the top left, as Scrawl labels its frames.
fn chip(image: &mut RgbaImage, number: u32, scale: f32) {
    let label = number.to_string();
    let (w, h) = ((label.len() as f32 * 6. + 8.) * scale, 13. * scale);
    let (x0, y0) = (10. * scale, 10. * scale);
    for y in y0 as u32..(y0 + h) as u32 {
        for x in x0 as u32..(x0 + w) as u32 {
            if x < image.width() && y < image.height() {
                blend(image, x, y, [20, 20, 24], 0.82);
            }
        }
    }
    text(
        image,
        &label,
        (x0 + w / 2., y0 + h / 2.),
        scale,
        [255, 255, 255],
    );
}

fn blend(image: &mut RgbaImage, x: u32, y: u32, colour: [u8; 3], alpha: f32) {
    let pixel = image.get_pixel_mut(x, y);
    for channel in 0..3 {
        let old = f32::from(pixel[channel]);
        pixel[channel] = (old + (f32::from(colour[channel]) - old) * alpha).round() as u8;
    }
}

/// A 5×7 pixel font for digits, enough for frame and pin numbers.
const DIGITS: [[u8; 7]; 10] = [
    [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
    [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
    [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
    [0x1f, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0e],
    [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
    [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
    [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
    [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
    [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
];

/// Digits centred on `centre`, each font pixel `scale` image pixels.
fn text(image: &mut RgbaImage, digits: &str, centre: (f32, f32), scale: f32, colour: [u8; 3]) {
    let glyphs: Vec<usize> = digits
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| d as usize))
        .collect();
    if glyphs.is_empty() {
        return;
    }
    let advance = 6. * scale;
    let total = glyphs.len() as f32 * advance - scale;
    let (left, top) = (centre.0 - total / 2., centre.1 - 3.5 * scale);
    for (index, glyph) in glyphs.iter().enumerate() {
        for (row, bits) in DIGITS[*glyph].iter().enumerate() {
            for column in 0..5 {
                if bits & (0x10 >> column) == 0 {
                    continue;
                }
                let px = left + index as f32 * advance + column as f32 * scale;
                let py = top + row as f32 * scale;
                for y in py.round() as i64..(py + scale).round() as i64 {
                    for x in px.round() as i64..(px + scale).round() as i64 {
                        if x >= 0
                            && y >= 0
                            && (x as u32) < image.width()
                            && (y as u32) < image.height()
                        {
                            blend(image, x as u32, y as u32, colour, 1.);
                        }
                    }
                }
            }
        }
    }
}

/// Crop `image` around `bounds` (screen fractions), padded so there's context: at
/// least a third of the screen each way, or 15% beyond the marks.
pub fn crop_around(image: &RgbaImage, bounds: [f64; 4]) -> RgbaImage {
    let (width, height) = (f64::from(image.width()), f64::from(image.height()));
    let (cx, cy) = ((bounds[0] + bounds[2]) / 2., (bounds[1] + bounds[3]) / 2.);
    let half_w = ((bounds[2] - bounds[0]) / 2. + 0.15).max(1. / 6.);
    let half_h = ((bounds[3] - bounds[1]) / 2. + 0.15).max(1. / 6.);
    let left = (cx - half_w).clamp(0., 1.);
    let right = (cx + half_w).clamp(0., 1.);
    let top = (cy - half_h).clamp(0., 1.);
    let bottom = (cy + half_h).clamp(0., 1.);
    let (x, y) = ((left * width) as u32, (top * height) as u32);
    let (w, h) = (
        (((right - left) * width).round() as u32).max(1),
        (((bottom - top) * height).round() as u32).max(1),
    );
    image::imageops::crop_imm(
        image,
        x,
        y,
        w.min(image.width() - x),
        h.min(image.height() - y),
    )
    .to_image()
}

/// Read a frame image (WebP, JPEG or PNG).
pub fn load_image(path: &Path) -> Result<RgbaImage> {
    Ok(image::open(path)
        .with_context(|| format!("Reading {}", path.display()))?
        .to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, value: u8) -> VideoFrame {
        VideoFrame {
            width,
            height,
            bgra: vec![value; (width * height * 4) as usize],
        }
    }

    #[test]
    fn frame_numbers_come_from_the_caption() {
        assert_eq!(number("#12 Lee circled something"), Some(12));
        assert_eq!(number("Lee"), None);
    }

    #[test]
    fn shapes_are_told_apart() {
        let ring: Vec<(f64, f64)> = (0..=40)
            .map(|i| {
                let a = f64::from(i) / 40. * std::f64::consts::TAU;
                (0.5 + 0.1 * a.cos(), 0.5 + 0.1 * a.sin())
            })
            .collect();
        assert_eq!(shape(&ring, 1.), "circle");
        let under: Vec<(f64, f64)> = (0..=20).map(|i| (0.2 + f64::from(i) * 0.01, 0.6)).collect();
        assert_eq!(shape(&under, 1.6), "underline");
        let diagonal: Vec<(f64, f64)> = (0..=20)
            .map(|i| (0.2 + f64::from(i) * 0.01, 0.2 + f64::from(i) * 0.01))
            .collect();
        assert_eq!(shape(&diagonal, 1.), "line");
    }

    #[test]
    fn a_cursor_sized_change_is_not_a_scene_change() {
        let a = flat(640, 360, 30);
        let mut b = a.clone();
        // A 20×20 cursor moves.
        for y in 100..120 {
            for x in 100..120 {
                let i = ((y * 640 + x) * 4) as usize;
                b.bgra[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        assert!(difference(&thumbnail(&a), &thumbnail(&b)) < SCENE_DELTA);
        let c = flat(640, 360, 220);
        assert!(difference(&thumbnail(&a), &thumbnail(&c)) > SCENE_DELTA);
        assert!(changed_cells(&thumbnail(&a), &thumbnail(&b)) < EDIT_CELLS);
    }

    #[test]
    fn a_one_line_edit_is_a_change_a_cursor_is_not() {
        // A dark editor; one line of light text changes across a third of the width.
        let a = flat(1920, 1080, 30);
        let mut edited = a.clone();
        for y in 540..562 {
            for x in 150..800 {
                if (x / 7) % 2 == 0 {
                    let i = ((y * 1920 + x) * 4) as usize;
                    edited.bgra[i..i + 3].copy_from_slice(&[220, 220, 220]);
                }
            }
        }
        let (ta, te) = (thumbnail(&a), thumbnail(&edited));
        assert!(
            difference(&ta, &te) < SCENE_DELTA,
            "Scrawl's measure misses it"
        );
        assert!(
            changed_cells(&ta, &te) >= EDIT_CELLS,
            "{}",
            changed_cells(&ta, &te)
        );
    }

    #[test]
    fn ink_and_pins_are_drawn_in_their_colours() {
        let mut image = RgbaImage::from_pixel(400, 200, image::Rgba([255, 255, 255, 255]));
        let line: Vec<(f64, f64)> = vec![(0.1, 0.5), (0.9, 0.5)];
        draw_ink(
            &mut image,
            &[(line, [255, 0, 0])],
            &[((0.5, 0.2), 7, [0, 0, 255])],
        );
        let on_line = image.get_pixel(200, 100);
        assert!(on_line[0] > 200 && on_line[1] < 60, "{on_line:?}");
        // The pin's head, above its point, is blue beside its white centre.
        let head = image.get_pixel(200 - 8, 40 - 27);
        assert!(head[2] > 180 && head[0] < 90, "{head:?}");
    }

    use crate::recorder::Input;

    fn frames(
        receiver: &std::sync::mpsc::Receiver<Input>,
        wait: Duration,
    ) -> Vec<(i64, Option<String>, String)> {
        let mut out = Vec::new();
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Ok(Input::Frame { t_ms, who, text }) = receiver.recv_timeout(left) {
                out.push((t_ms, who, text));
            }
        }
        out
    }

    /// As soon as there's a frame, for what has to happen but has no deadline: a busy machine
    /// can take its time making a still.
    fn first_frame(
        receiver: &std::sync::mpsc::Receiver<Input>,
    ) -> Vec<(i64, Option<String>, String)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if let Ok(Input::Frame { t_ms, who, text }) = receiver.recv_timeout(left) {
                return vec![(t_ms, who, text)];
            }
        }
        Vec::new()
    }

    /// What a frame costs the moments thread. Run with `--release`: debug builds are many
    /// times slower.
    #[test]
    #[ignore = "a measurement, it asserts nothing"]
    fn what_a_frame_costs() -> Result<()> {
        // Busy like a code editor: columns of short lines, no two rows alike.
        let (width, height) = (2560_u32, 1440_u32);
        let mut bgra = vec![30; (width * height * 4) as usize];
        for y in 0..height {
            let indent = (y / 24 * 37 % 300) * 2;
            let length = 200 + (y / 24 * 131 % 1200);
            for x in 0..width {
                let inside = x >= indent && x < indent + length && y % 24 < 14;
                if inside && (x / 7 + y / 24).wrapping_mul(2_654_435_761) % 5 != 0 {
                    let pixel = ((y * width + x) * 4) as usize;
                    bgra[pixel] = 210;
                    bgra[pixel + 1] = (x / 11 % 200 + 40) as u8;
                    bgra[pixel + 2] = (y / 13 % 200 + 40) as u8;
                }
            }
        }
        let frame = VideoFrame {
            width,
            height,
            bgra,
        };
        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        let started = Instant::now();
        moments.sample(frame);
        while !matches!(receiver.recv()?, Input::Frame { .. }) {}
        let took = started.elapsed();
        let mut bytes = 0;
        for file in std::fs::read_dir(directory.path())? {
            let file = file?;
            let size = file.metadata()?.len();
            println!("{:?}: {size} bytes", file.file_name());
            bytes += size;
        }
        println!("{width}x{height}: {took:?} on the moments thread, {bytes} bytes");
        moments.finish();
        Ok(())
    }

    #[test]
    fn a_stalled_thread_holds_only_a_few_copies_of_the_screen() {
        let (sender, receiver) = sync_channel(64);
        let queued = Arc::new(AtomicUsize::new(0));
        let sampler = Sampler {
            sender,
            queued: queued.clone(),
        };
        for _ in 0..10 {
            sampler.sample(flat(16, 16, 0));
        }
        assert_eq!(receiver.try_iter().count(), MAX_QUEUED_SAMPLES);
        assert_eq!(queued.load(Ordering::Acquire), MAX_QUEUED_SAMPLES);
    }

    #[test]
    fn copies_of_the_screen_keep_coming_once_the_thread_has_them() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (feed, _receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        for _ in 0..12 {
            moments.sample(flat(16, 16, 0));
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(moments.queued_samples.load(Ordering::Acquire), 0);
        moments.finish();
        Ok(())
    }

    #[test]
    fn ink_has_the_colours_it_has_on_screen() -> Result<()> {
        // Two people who want the same colour: the one sorting first keeps it.
        let id = |n: u32| format!("room:r:user-{n:03}:audio");
        let mut ids = (0..100).map(id);
        let first = ids.next().unwrap();
        let second = ids.find(|other| colour(other) == colour(&first)).unwrap();

        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        moments.people(vec![first.clone(), second.clone()]);
        moments.sample(flat(800, 500, 90));
        let _ = first_frame(&receiver);

        // The later one marks first, and still doesn't get the colour they both want.
        let point = Point { x: 0.5, y: 0.5 };
        moments.draw(&second, "Second", &DrawEvent::ClickAnimation(point));
        std::thread::sleep(Duration::from_millis(300));
        moments.sample(flat(800, 500, 90));
        assert_eq!(first_frame(&receiver).len(), 1);
        let info: FrameInfo =
            serde_json::from_slice(&std::fs::read(directory.path().join("frame-002.json"))?)?;
        let expected = Colours::for_call([first.as_str(), second.as_str()]);
        assert_eq!(info.marks[0].colour, hex(expected.get(&second).unwrap()));
        assert_ne!(info.marks[0].colour, hex(colour(&second)));
        moments.finish();
        Ok(())
    }

    #[test]
    fn a_stamps_ripple_is_not_a_pin_of_its_own() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        moments.sample(flat(800, 500, 90));
        let _ = first_frame(&receiver);
        let lee = "room:r:lee:audio";
        let (x, y) = (0.7, 0.2);
        moments.stamp(lee, "Lee", &StampMessage::StampPlace { id: 5, x, y });
        moments.draw(lee, "Lee", &DrawEvent::ClickAnimation(Point { x, y }));
        moments.stamp(
            lee,
            "Lee",
            &StampMessage::StampText {
                id: 5,
                text: "this one".into(),
                draft: String::new(),
                done: true,
            },
        );
        // Someone else's pin in the same place, and the same person's elsewhere, do count.
        moments.draw(
            "room:r:alex:audio",
            "Alex",
            &DrawEvent::ClickAnimation(Point { x, y }),
        );
        moments.draw(
            lee,
            "Lee",
            &DrawEvent::ClickAnimation(Point { x: 0.1, y: 0.1 }),
        );
        std::thread::sleep(Duration::from_millis(300));
        moments.sample(flat(800, 500, 90));
        let taken = frames(&receiver, Duration::from_millis(2_500));
        assert_eq!(taken.len(), 1, "{taken:?}");
        let info: FrameInfo =
            serde_json::from_slice(&std::fs::read(directory.path().join("frame-002.json"))?)?;
        let pins: Vec<_> = info
            .marks
            .iter()
            .map(|mark| (mark.by.as_str(), mark.shape.as_str(), mark.pin))
            .collect();
        assert_eq!(
            pins,
            [
                ("Lee", "stamp", Some(1)),
                ("Alex", "pin", Some(2)),
                ("Lee", "pin", Some(3))
            ]
        );
        moments.finish();
        Ok(())
    }

    #[test]
    fn strokes_and_pins_become_numbered_frames_with_their_ink() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        // A grey screen is shared: its first view is frame 1.
        moments.sample(flat(800, 500, 90));
        let first = frames(&receiver, Duration::from_millis(1_500));
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].2, "#1 the shared screen");

        // Lee circles something near the top right, then Alex drops a pin: one frame.
        let lee = "room:r:lee:audio";
        moments.draw(
            lee,
            "Lee",
            &DrawEvent::DrawStart(PathPoint {
                point: Point { x: 0.8, y: 0.2 },
                path_id: 7,
            }),
        );
        for i in 1..=40 {
            let a = f64::from(i) / 40. * std::f64::consts::TAU;
            let point = Point {
                x: 0.7 + 0.1 * a.cos(),
                y: 0.2 + 0.1 * a.sin(),
            };
            let event = if i == 40 {
                DrawEvent::DrawEnd(point)
            } else {
                DrawEvent::DrawAddPoint(point)
            };
            moments.draw(lee, "Lee", &event);
        }
        moments.draw(
            "room:r:alex:audio",
            "Alex",
            &DrawEvent::ClickAnimation(Point { x: 0.25, y: 0.75 }),
        );
        // The screen keeps sending frames; one after the marks settle is used.
        std::thread::sleep(Duration::from_millis(300));
        moments.sample(flat(800, 500, 90));
        let marked = frames(&receiver, Duration::from_millis(2_500));
        assert_eq!(marked.len(), 1, "{marked:?}");
        let (_, who, caption) = &marked[0];
        assert_eq!(who.as_deref(), Some("Lee"));
        assert!(
            caption.starts_with("#2 Lee circled something at top right"),
            "{caption}"
        );
        assert!(
            caption.contains("Alex dropped pin 1 at bottom left"),
            "{caption}"
        );

        // Its files: the ink is on the full-size frame in Lee's colour.
        let info: FrameInfo =
            serde_json::from_slice(&std::fs::read(directory.path().join("frame-002.json"))?)?;
        assert_eq!(info.marks.len(), 2);
        assert!(!directory.path().join("frame-002.png").exists());
        let image = load_image(&directory.path().join("frame-002.full.webp"))?;
        assert_eq!(image.dimensions(), (800, 500));
        let [r, g, b] = colour(lee);
        // The rightmost point of the ring: x = 0.8, y = 0.2.
        let pixel = image.get_pixel(640 - 1, 100);
        let close = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs() < 40;
        assert!(
            close(pixel[0], r) && close(pixel[1], g) && close(pixel[2], b),
            "{pixel:?}"
        );
        assert!(directory.path().join("frame-002.webp").exists());
        assert!(!directory.path().join("frame-002.jpg").exists());

        // A bookmark is a frame of its own, at once; the screen hasn't changed.
        moments.bookmark(Some("Alex".into()), "the bug");
        let booked = frames(&receiver, Duration::from_millis(1_500));
        assert_eq!(booked.len(), 1, "{booked:?}");
        assert!(
            booked[0].2.starts_with("#3 Alex bookmarked \"the bug\""),
            "{booked:?}"
        );
        moments.finish();
        Ok(())
    }

    #[test]
    fn a_settled_screen_change_is_a_frame_but_not_too_often() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        moments.sample(flat(320, 180, 20));
        assert_eq!(first_frame(&receiver).len(), 1);
        // A new page: changes, then holds still. Within SCENE_GAP of frame 1: no frame.
        moments.sample(flat(320, 180, 200));
        moments.sample(flat(320, 180, 200));
        assert!(frames(&receiver, Duration::from_millis(400)).is_empty());
        moments.finish();
        Ok(())
    }

    #[test]
    fn a_new_scene_is_a_frame_once_the_screen_is_quiet() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (feed, receiver) = Feed::for_tests();
        let mut moments = Moments::start_in(directory.path().into(), feed)?;
        moments.sample(flat(320, 180, 20));
        assert_eq!(first_frame(&receiver).len(), 1);
        // Another app: the whole screen changes, then nothing more is sent.
        moments.sample(flat(320, 180, 200));
        assert!(frames(&receiver, Duration::from_millis(1_500)).is_empty());
        let later = first_frame(&receiver);
        assert_eq!(later.len(), 1, "{later:?}");
        assert_eq!(later[0].2, "#2 the shared screen changed");
        moments.finish();
        Ok(())
    }

    /// A 640×360 desktop: a dark editor, a chat in the right quarter scrolled
    /// `scroll` lines, or with `other`, a different, light app over all of it.
    fn desktop(scroll: u32, other: bool) -> VideoFrame {
        let (width, height) = (640u32, 360u32);
        let mut frame = flat(width, height, 30);
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                let value = if other {
                    // Light, with text in different places.
                    if y % 16 < 6 && (x / 9 + y / 16) % 3 != 0 {
                        60
                    } else {
                        235
                    }
                } else if x < 480 {
                    // Code: short lines of text, still.
                    if y % 14 < 5 && x % 8 < 5 && x < 40 + (y / 14 * 37) % 300 {
                        200
                    } else {
                        30
                    }
                } else {
                    // Chat: lines of words that move up as it scrolls.
                    let line = y / 12 + scroll;
                    let word = (x - 480) / 23 + line * 7;
                    let shown = !word.is_multiple_of(5) && word % 3 != 1;
                    if y % 12 < 6 && (x - 480) % 23 < 17 && shown {
                        210
                    } else {
                        40
                    }
                };
                frame.bgra[i..i + 3].copy_from_slice(&[value; 3]);
            }
        }
        frame
    }

    #[test]
    fn a_scroll_waits_for_the_screen_gap_a_new_scene_does_not() {
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let saved = thumbnail(&desktop(0, false));
        let scrolled = thumbnail(&desktop(3, false));
        let switched = thumbnail(&desktop(0, true));
        assert!(moved(&saved, &scrolled) && !new_scene(&saved, &scrolled));
        assert!(new_scene(&saved, &switched));

        // Frame at 0 s; a scroll at 5 s, then still.
        let mut scene = Scene::default();
        scene.sampled(&saved, &scrolled, at(5.));
        assert!(
            !scene.due(&saved, &scrolled, at(6.), Some(at(0.))),
            "not yet quiet"
        );
        assert!(
            !scene.due(&saved, &scrolled, at(8.), Some(at(0.))),
            "too soon"
        );
        assert!(scene.due(&saved, &scrolled, at(20.), Some(at(0.))));

        // Switching apps needs only the quiet and SCENE_GAP.
        let mut scene = Scene::default();
        scene.sampled(&saved, &switched, at(1.5));
        assert!(!scene.due(&saved, &switched, at(3.), Some(at(0.))));
        assert!(scene.due(&saved, &switched, at(3.5), Some(at(0.))));

        // Still scrolling: never, however long since the last frame.
        let mut scene = Scene::default();
        scene.sampled(&saved, &scrolled, at(59.));
        assert!(!scene.due(&saved, &scrolled, at(60.), Some(at(0.))));

        // A cursor: never.
        let mut cursor = flat(640, 360, 30);
        cursor.bgra = desktop(0, false).bgra;
        for y in 100..120 {
            for x in 100..120 {
                let i = ((y * 640 + x) * 4) as usize;
                cursor.bgra[i..i + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let pointed = thumbnail(&cursor);
        let mut scene = Scene::default();
        scene.sampled(&saved, &pointed, at(1.));
        assert!(!scene.due(&saved, &pointed, at(60.), Some(at(0.))));
    }

    #[test]
    fn a_scroll_heavy_call_takes_few_screen_frames() {
        // Five minutes of a chat scrolling every 2 to 4 seconds, sampled twice a second,
        // with a switch to another app at 100 s and back at 200 s. The old rule took a
        // frame for nearly every scroll, about 100.
        let start = Instant::now();
        let gaps = [2.0, 3.5, 2.5, 4.0, 3.0];
        let mut scrolls = Vec::new();
        let mut t = 0.;
        for gap in gaps.iter().cycle() {
            t += gap;
            if t > 300. {
                break;
            }
            scrolls.push(t);
        }
        let mut scene = Scene::default();
        let mut previous = thumbnail(&desktop(0, false));
        let mut saved = previous.clone();
        let mut last_frame = Some(start);
        let mut taken = Vec::new();
        for tick in 1..=600 {
            let seconds = f64::from(tick) * 0.5;
            let now = start + Duration::from_secs_f64(seconds);
            let scroll = scrolls.iter().filter(|&&s| s <= seconds).count() as u32;
            let other = (100. ..200.).contains(&seconds);
            let thumb = thumbnail(&desktop(scroll, other));
            scene.sampled(&previous, &thumb, now);
            if scene.due(&saved, &thumb, now, last_frame) {
                taken.push(seconds);
                saved = thumb.clone();
                last_frame = Some(now);
            }
            previous = thumb;
        }
        assert!(scrolls.len() >= 90, "{}", scrolls.len());
        // At most one every SCREEN_GAP (10 s): about 30, not 100.
        assert!(taken.len() <= 32, "{taken:?}");
        // Both switches are frames within a few seconds.
        for switch in [100., 200.] {
            assert!(
                taken.iter().any(|&t| t >= switch && t <= switch + 4.),
                "{switch}: {taken:?}"
            );
        }
    }

    /// Output streaming for two minutes, changing every sample, never settles: it still
    /// gets a frame about every BUSY (30 s), and none before 30 s of it: three in two
    /// minutes.
    #[test]
    fn a_screen_that_never_settles_still_gets_frames() {
        let start = Instant::now();
        let mut scene = Scene::default();
        let mut previous = thumbnail(&desktop(0, false));
        let mut saved = previous.clone();
        let mut last_frame = Some(start);
        let mut taken = Vec::new();
        for tick in 1..=240 {
            let seconds = f64::from(tick) * 0.5;
            let now = start + Duration::from_secs_f64(seconds);
            let thumb = thumbnail(&desktop(tick, false));
            scene.sampled(&previous, &thumb, now);
            if scene.due(&saved, &thumb, now, last_frame) {
                taken.push(seconds);
                saved = thumb.clone();
                last_frame = Some(now);
            }
            previous = thumb;
        }
        assert_eq!(taken.len(), 3, "{taken:?}");
        assert!(taken[0] >= 30. && taken[0] <= 31., "{taken:?}");
        for pair in taken.windows(2) {
            assert!((pair[1] - pair[0] - 30.).abs() <= 1., "{taken:?}");
        }
    }

    #[test]
    fn a_caption_says_each_mark_once() {
        let mark = |by: &str, shape: &str, pin: Option<u32>, x: f64| MarkInfo {
            by: by.into(),
            shape: shape.into(),
            pin,
            text: None,
            bounds: [x, 0.2, x, 0.2],
            colour: "#ff0000".into(),
        };
        let marks = vec![
            mark("Sam", "stroke", None, 0.35),
            mark("Sam", "stroke", None, 0.35),
            mark("Sam", "stroke", None, 0.351),
            mark("Alex", "stroke", None, 0.35),
            mark("Sam", "pin", Some(1), 0.35),
            mark("Sam", "pin", Some(2), 0.35),
        ];
        let text = caption(35, &marks, None, "marks", false);
        assert_eq!(
            text,
            "#35 Sam drew at top centre (35%, 20%); Alex drew at top centre (35%, 20%); \
             Sam dropped pin 1 at top centre (35%, 20%); \
             Sam dropped pin 2 at top centre (35%, 20%)"
        );
    }

    #[test]
    fn the_full_frame_is_the_webp_or_an_older_capture_s_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = |kind: &str| directory.path().join(format!("frame-004.{kind}"));
        assert_eq!(full_in(directory.path(), 4), path("webp"));
        std::fs::write(path("jpg"), b"")?;
        assert_eq!(full_in(directory.path(), 4), path("jpg"));
        assert_eq!(
            crate::frame_files::small_in(directory.path(), 4),
            path("jpg")
        );
        std::fs::write(path("png"), b"")?;
        assert_eq!(full_in(directory.path(), 4), path("png"));
        std::fs::write(path("full.jpg"), b"")?;
        assert_eq!(full_in(directory.path(), 4), path("full.jpg"));
        std::fs::write(path("full.webp"), b"")?;
        assert_eq!(full_in(directory.path(), 4), path("full.webp"));
        Ok(())
    }

    #[test]
    fn crops_keep_context_around_the_marks() {
        let image = RgbaImage::new(1000, 500);
        let crop = crop_around(&image, [0.45, 0.45, 0.55, 0.55]);
        assert_eq!(crop.width(), 400);
        assert_eq!(crop.height(), 200);
        let corner = crop_around(&image, [0.0, 0.0, 0.05, 0.05]);
        assert!(corner.width() <= 1000 && corner.height() <= 500);
    }
}
