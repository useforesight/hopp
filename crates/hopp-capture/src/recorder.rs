//! The recorder: one thread per capture that splits each person's audio into
//! utterances, has the helper transcribe them several at a time (see `batch`), and
//! writes the results.
//!
//! Nothing here can hold up a call. Audio arrives through a bounded queue that drops
//! frames when full; if transcription falls behind by more than the backlog limit, the
//! oldest audio is skipped. Both leave a "not transcribed" line, so gaps are visible.

use anyhow::Result;
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use super::{
    batch::{self, Part, Waiting},
    draft::Draft,
    helper::{Helper, Reply},
    models::{self, Model},
    store::{self, Kind, NewLine, Store},
    text,
    vad::{FRAME, RATE, Segmenter},
};

/// About two seconds of 10 ms frames from three people.
const QUEUE: usize = 600;
/// A speaker whose audio stops (muted, left) has their last words flushed after this.
const IDLE: Duration = Duration::from_millis(1_500);
/// How long a stopping capture waits for the last utterances to be transcribed: half the
/// audio still waiting (the slowest model runs four times faster than speech), within
/// these bounds.
const DRAIN_MIN: Duration = Duration::from_secs(30);
const DRAIN_MAX: Duration = Duration::from_secs(300);
/// With nothing to transcribe for this long, the helper exits and gives its memory
/// (gigabytes, for the large models) back; it reloads, in seconds, when someone speaks.
const UNLOAD: Duration = Duration::from_secs(90);
/// The least time a request may take before the helper counts as stuck.
const IN_FLIGHT_MIN: Duration = Duration::from_secs(60);
/// On the CPU the helper only gets time nothing else wants, so a busy machine (a build)
/// can slow it many times over: it gets longer before it counts as stuck.
const IN_FLIGHT_MIN_CPU: Duration = Duration::from_secs(120);
/// After transcription fails, try again after this long, doubling up to the most. A
/// capture never gives up while it runs: speech waits (up to the backlog) meanwhile.
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MOST: Duration = Duration::from_secs(120);
/// Live captures skip audio once transcription is this far behind.
pub const LIVE_BACKLOG_MS: i64 = 10 * 60 * 1000;

#[derive(Debug)]
pub struct Speaker {
    pub id: String,
    pub name: String,
}

pub enum Input {
    Audio {
        speaker: Arc<Speaker>,
        t_ms: i64,
        samples: Vec<i16>,
    },
    Event {
        t_ms: i64,
        text: String,
    },
    Bookmark {
        t_ms: i64,
        who: Option<String>,
        label: String,
    },
    /// A keyframe of the shared screen (see `moments`).
    Frame {
        t_ms: i64,
        who: Option<String>,
        text: String,
    },
    Participants(Vec<String>),
    Downloading(u8),
    Downloaded(Result<PathBuf, String>),
    /// From helper number `.0`; replies from one already replaced are ignored.
    Helper(u64, Reply),
    Stop,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum Phase {
    Starting,
    Downloading {
        percent: u8,
    },
    Loading,
    Ready,
    /// Nobody has spoken for a while; the model is unloaded until someone does.
    Idle,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub id: String,
    pub model: String,
    #[serde(flatten)]
    pub phase: Phase,
    /// Audio waiting to be transcribed.
    pub behind_ms: i64,
    pub lines: u64,
    /// Where transcription runs: the GPU's name, or "CPU".
    pub device: Option<String>,
    /// Time the helper spent on drafts and on final text: the cost of the capture.
    pub draft_ms: u64,
    pub final_ms: u64,
}

pub struct Options {
    pub room: String,
    pub model: &'static Model,
    pub backlog_ms: i64,
    /// Transcribe everything before finishing, however long it takes (files).
    pub drain_all: bool,
    /// Show what people say while they say it, where the model allows (see `draft`).
    pub drafts: bool,
    /// Transcribe on the GPU (see `config::Config::gpu`).
    pub gpu: bool,
    pub store: PathBuf,
    /// Called whenever the status changes.
    pub changed: Arc<dyn Fn() + Send + Sync>,
}

pub struct Recorder {
    input: SyncSender<Input>,
    status: Arc<Mutex<Status>>,
    dropped: Arc<AtomicU64>,
    started: Instant,
    thread: Option<JoinHandle<Result<()>>>,
}

/// Cheap to clone; handed to the audio paths of a call.
#[derive(Clone)]
pub struct Feed {
    input: SyncSender<Input>,
    dropped: Arc<AtomicU64>,
    started: Instant,
}

impl Feed {
    /// A feed whose input a test reads, here or in a client.
    pub fn for_tests() -> (Self, std::sync::mpsc::Receiver<Input>) {
        let (input, receiver) = sync_channel(QUEUE);
        (
            Self {
                input,
                dropped: Default::default(),
                started: Instant::now(),
            },
            receiver,
        )
    }

    pub fn now_ms(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }

    /// Never blocks: when the recorder is behind, the frame is dropped and counted.
    pub fn audio(&self, speaker: &Arc<Speaker>, samples: &[i16]) {
        let input = Input::Audio {
            speaker: speaker.clone(),
            t_ms: self.now_ms(),
            samples: samples.to_vec(),
        };
        if let Err(TrySendError::Full(_)) = self.input.try_send(input) {
            self.dropped
                .fetch_add(samples.len() as u64, Ordering::Relaxed);
        }
    }

    pub fn event(&self, text: impl Into<String>) {
        let _ = self.input.try_send(Input::Event {
            t_ms: self.now_ms(),
            text: text.into(),
        });
    }

    pub fn bookmark(&self, who: Option<String>, label: impl Into<String>) {
        let _ = self.input.try_send(Input::Bookmark {
            t_ms: self.now_ms(),
            who,
            label: label.into(),
        });
    }

    /// A keyframe's line. Waits for room in the queue: frames are rare and must not be
    /// lost, so call it only off the engine thread (the moments thread does).
    pub fn frame(&self, t_ms: i64, who: Option<String>, text: String) {
        let _ = self.input.send(Input::Frame { t_ms, who, text });
    }

    pub fn participants(&self, names: Vec<String>) {
        let _ = self.input.try_send(Input::Participants(names));
    }
}

impl Recorder {
    pub fn start(options: Options) -> Result<Self> {
        let started_ms = store::now_ms();
        let id = store::new_call_id(started_ms);
        // Claimed before the row exists, so nobody takes the new call for an orphan.
        let claim = super::liveness::claim(&id)?;
        let store = Store::open(&options.store)?;
        store.begin_call(&id, &options.room, started_ms, options.model.name)?;
        let (input, receiver) = sync_channel(QUEUE);
        let status = Arc::new(Mutex::new(Status {
            id: id.clone(),
            model: options.model.name.into(),
            phase: Phase::Starting,
            behind_ms: 0,
            lines: 0,
            device: None,
            draft_ms: 0,
            final_ms: 0,
        }));
        let dropped = Arc::new(AtomicU64::new(0));
        let cpu = !options.gpu;
        let worker = Worker {
            id: id.clone(),
            _claim: claim,
            store,
            options,
            input: input.clone(),
            status: status.clone(),
            dropped: dropped.clone(),
            speakers: HashMap::new(),
            pending: VecDeque::new(),
            in_flight: None,
            helper: None,
            failures: 0,
            retry_at: None,
            retry_delay: RETRY_FIRST,
            last_chance: true,
            generation: 0,
            cpu,
            words: false,
            drafts: false,
            last_work: Instant::now(),
            stopping: None,
            reported_drops: 0,
            last_t_ms: std::cell::Cell::new(0),
        };
        let thread = std::thread::Builder::new()
            .name("hopp-capture".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker.run(receiver)));
                let result = match result {
                    Ok(result) => result,
                    Err(panic) => {
                        let message = panic
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "no message".into());
                        Err(anyhow::anyhow!("the capture thread panicked: {message}"))
                    }
                };
                match &result {
                    Ok(()) => crate::logger::line(format!("Capture {id}: finished")),
                    Err(error) => crate::logger::line(format!("Capture {id}: stopped: {error:#}")),
                }
                result
            })?;
        Ok(Self {
            input,
            status,
            dropped,
            started: Instant::now(),
            thread: Some(thread),
        })
    }

    pub fn feed(&self) -> Feed {
        Feed {
            input: self.input.clone(),
            dropped: self.dropped.clone(),
            started: self.started,
        }
    }

    /// Queue an input, waiting for room: for feeding a file faster than real time.
    pub fn send(&self, input: Input) {
        let _ = self.input.send(input);
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    pub fn id(&self) -> String {
        self.status().id
    }

    /// Stop and wait for the last words to be transcribed. Returns the final status.
    pub fn finish(mut self) -> Result<Status> {
        let _ = self.input.send(Input::Stop);
        let result = match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow::anyhow!("The capture thread panicked")),
            None => Ok(()),
        };
        result.map(|()| self.status())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = self.input.send(Input::Stop);
            let _ = thread.join();
        }
    }
}

struct SpeakerState {
    speaker: Arc<Speaker>,
    segmenter: Segmenter,
    /// Capture time of the next frame, from when its audio last arrived.
    next_t_ms: i64,
    last_heard: Instant,
    /// The line showing what they are saying now (see `draft`).
    draft: Option<Draft>,
}

struct InFlight {
    id: u64,
    /// Utterances to transcribe for good; none for a draft.
    parts: Vec<Part>,
    sent: Instant,
    /// A guess at someone's utterance in progress: whose, its line, and how much audio.
    draft: Option<(String, i64, i64)>,
}

impl InFlight {
    /// How long the helper may take before it counts as stuck: a minute, plus four
    /// times the audio (Whisper large-v3 on a busy CPU runs about as fast as speech).
    fn limit(&self, cpu: bool) -> Duration {
        let audio_ms = self
            .parts
            .iter()
            .map(|part| part.utterance.duration_ms())
            .sum::<i64>()
            .max(0) as u64;
        if cpu {
            IN_FLIGHT_MIN_CPU + Duration::from_millis(audio_ms * 10)
        } else {
            IN_FLIGHT_MIN + Duration::from_millis(audio_ms * 4)
        }
    }
}

struct Worker {
    id: String,
    /// Held while this worker runs; see `liveness`.
    _claim: super::liveness::Claim,
    store: Store,
    options: Options,
    input: SyncSender<Input>,
    status: Arc<Mutex<Status>>,
    dropped: Arc<AtomicU64>,
    speakers: HashMap<String, SpeakerState>,
    pending: VecDeque<Waiting>,
    in_flight: Option<InFlight>,
    helper: Option<Helper>,
    /// Helper failures since the last transcript: the first restarts it, the second
    /// moves to the CPU (or, on the CPU, waits and tries again).
    failures: u32,
    /// When to try again after transcription failed, and the wait after that.
    retry_at: Option<Instant>,
    retry_delay: Duration,
    /// Whether a stopping capture may still try once more after a failure.
    last_chance: bool,
    /// Counts helper starts, so a replaced helper's last words are ignored.
    generation: u64,
    /// On the CPU: chosen, or fallen back to after the GPU failed.
    cpu: bool,
    /// The helper times each word, so a request can mix people (see `batch::take`).
    words: bool,
    /// Show what people say while they say it (see `draft`).
    drafts: bool,
    /// When the helper last had something to do.
    last_work: Instant,
    /// When stopping, the deadline for the last transcriptions.
    stopping: Option<Instant>,
    reported_drops: u64,
    /// The latest time any line was written at.
    last_t_ms: std::cell::Cell<i64>,
}

impl Worker {
    fn update(&self, change: impl FnOnce(&mut Status)) {
        if let Ok(mut status) = self.status.lock() {
            change(&mut status);
        }
        (self.options.changed)();
    }

    fn phase(&self) -> Phase {
        self.status
            .lock()
            .map(|status| status.phase.clone())
            .unwrap_or(Phase::Starting)
    }

    fn line(&self, kind: Kind, t_ms: i64, speaker: Option<&Speaker>, text: String) {
        self.line_until(kind, t_ms, None, speaker, text);
    }

    fn line_until(
        &self,
        kind: Kind,
        t_ms: i64,
        end_ms: Option<i64>,
        speaker: Option<&Speaker>,
        text: String,
    ) {
        self.last_t_ms
            .set(self.last_t_ms.get().max(end_ms.unwrap_or(t_ms)));
        let line = NewLine {
            t_ms,
            end_ms,
            kind,
            speaker_id: speaker.map(|speaker| speaker.id.clone()),
            speaker: speaker.map(|speaker| speaker.name.clone()),
            text,
        };
        if let Err(error) = self.store.add_line(&self.id, &line) {
            crate::logger::line(format!("Capture: could not save a line: {error}"));
        }
    }

    fn start_helper(&mut self, path: PathBuf) {
        let input = self.input.clone();
        self.generation += 1;
        let generation = self.generation;
        let english_only = self.options.model.english_only();
        let limit = self.options.model.memory_limit(!self.cpu);
        match Helper::spawn(&path, english_only, self.cpu, limit, move |reply| {
            let _ = input.send(Input::Helper(generation, reply));
        }) {
            Ok(helper) => {
                self.helper = Some(helper);
                self.update(|status| status.phase = Phase::Loading);
            }
            Err(error) => self.fail(format!("{error:#}")),
        }
    }

    /// Transcription is unavailable for now: say so, and try again later. Speech keeps
    /// queueing meanwhile and is transcribed once a helper works again.
    fn fail(&mut self, message: String) {
        let wait = self.retry_delay;
        crate::logger::line(format!(
            "Capture {}: transcription unavailable ({message}); trying again in {} s",
            self.id,
            wait.as_secs()
        ));
        self.helper = None;
        if let Some(in_flight) = self.in_flight.take() {
            self.requeue(in_flight.parts);
        }
        self.retry_at = Some(Instant::now() + wait);
        self.retry_delay = (wait * 2).min(RETRY_MOST);
        let message = format!("{message}; trying again in {} s", wait.as_secs());
        self.update(|status| status.phase = Phase::Failed { message });
    }

    /// Once the wait after a failure is over, start again on the chosen device.
    fn retry(&mut self) {
        if self
            .retry_at
            .is_none_or(|retry_at| Instant::now() < retry_at)
        {
            return;
        }
        crate::logger::line(format!("Capture {}: trying transcription again", self.id));
        self.retry_at = None;
        self.failures = 0;
        self.cpu = !self.options.gpu;
        self.fetch_model();
    }

    fn fetch_model(&mut self) {
        let model = self.options.model;
        if models::installed(model) {
            match models::path(model) {
                Ok(path) => self.start_helper(path),
                Err(error) => self.fail(format!("{error:#}")),
            }
            return;
        }
        self.update(|status| status.phase = Phase::Downloading { percent: 0 });
        let input = self.input.clone();
        let spawned = std::thread::Builder::new()
            .name("hopp-model".into())
            .spawn(move || {
                let mut last = 0;
                let result = models::download(model, |done, total| {
                    let percent = (done * 100 / total.max(1)) as u8;
                    if percent != last {
                        last = percent;
                        let _ = input.try_send(Input::Downloading(percent));
                    }
                });
                let _ = input.send(Input::Downloaded(
                    result.map_err(|error| format!("{error:#}")),
                ));
            });
        if let Err(error) = spawned {
            self.fail(error.to_string());
        }
    }

    fn audio(&mut self, speaker: Arc<Speaker>, t_ms: i64, samples: Vec<i16>) {
        let state = self
            .speakers
            .entry(speaker.id.clone())
            .or_insert_with(|| SpeakerState {
                speaker: speaker.clone(),
                segmenter: Segmenter::default(),
                next_t_ms: t_ms,
                last_heard: Instant::now(),
                draft: None,
            });
        // Frames arrive in bursts; keep this speaker's clock running from its own audio
        // unless it fell behind real time (a gap in the stream).
        let mut t = if t_ms - state.next_t_ms > 200 {
            t_ms
        } else {
            state.next_t_ms
        };
        state.last_heard = Instant::now();
        for frame in samples.chunks(FRAME) {
            if let Some(utterance) = state.segmenter.push(t, frame) {
                // The draft shown for this utterance becomes its line.
                let line = state
                    .draft
                    .take()
                    .filter(|draft| draft.t_ms == utterance.t_ms)
                    .map(|draft| draft.line);
                self.pending.push_back(Waiting {
                    speaker: state.speaker.clone(),
                    utterance,
                    queued: Instant::now(),
                    line,
                });
            }
            t += (frame.len() * 1000 / RATE) as i64;
        }
        state.next_t_ms = t;
        // Speech that has begun gets its line at once, where it was said, so the
        // transcript never has to make room for it later.
        if self.drafts
            && state.draft.is_none()
            && let Some((t_ms, _)) = state.segmenter.in_progress()
        {
            let line = NewLine {
                t_ms,
                end_ms: None,
                kind: Kind::Draft,
                speaker_id: Some(state.speaker.id.clone()),
                speaker: Some(state.speaker.name.clone()),
                text: String::new(),
            };
            match self.store.add_line(&self.id, &line) {
                Ok(id) => state.draft = Some(Draft::new(id, t_ms)),
                Err(error) => {
                    crate::logger::line(format!("Capture: could not save a draft: {error}"))
                }
            }
        }
    }

    fn flush_idle(&mut self, all: bool) {
        for state in self.speakers.values_mut() {
            if (all || state.last_heard.elapsed() > IDLE)
                && let Some(utterance) = state.segmenter.finish()
            {
                let line = state
                    .draft
                    .take()
                    .filter(|draft| draft.t_ms == utterance.t_ms)
                    .map(|draft| draft.line);
                self.pending.push_back(Waiting {
                    speaker: state.speaker.clone(),
                    utterance,
                    queued: Instant::now(),
                    line,
                });
            }
        }
    }

    fn behind_ms(&self) -> i64 {
        let waiting = self.pending.iter().map(|item| &item.utterance);
        let sent = self
            .in_flight
            .iter()
            .flat_map(|in_flight| in_flight.parts.iter().map(|part| &part.utterance));
        waiting
            .chain(sent)
            .map(|utterance| utterance.duration_ms())
            .sum()
    }

    fn keep_up(&mut self) {
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > self.reported_drops {
            let ms = ((dropped - self.reported_drops) * 1000 / RATE as u64) as i64;
            self.reported_drops = dropped;
            if ms >= 100 {
                let t = self
                    .speakers
                    .values()
                    .map(|state| state.next_t_ms)
                    .max()
                    .unwrap_or(0);
                self.line(
                    Kind::Gap,
                    t,
                    None,
                    format!(
                        "{:.1} s of audio dropped while Hopp was busy",
                        ms as f64 / 1000.0
                    ),
                );
            }
        }
        while self.behind_ms() > self.options.backlog_ms {
            let Some(item) = self.pending.pop_front() else {
                break;
            };
            self.settle(
                item.line,
                Kind::Gap,
                item.utterance.t_ms,
                None,
                &item.speaker,
                format!(
                    "{:.0} s skipped to catch up",
                    item.utterance.duration_ms() as f64 / 1000.0
                ),
            );
        }
    }

    /// Put utterances back at the front, in order, to be sent again.
    fn requeue(&mut self, parts: Vec<Part>) {
        for part in parts.into_iter().rev() {
            self.pending.push_front(Waiting {
                speaker: part.speaker,
                utterance: part.utterance,
                queued: Instant::now() - batch::MAX_WAIT,
                line: part.line,
            });
        }
    }

    /// Unload the model after a quiet spell; load it again when there's speech.
    fn rest_or_wake(&mut self) {
        let phase = self.phase();
        let speaking = self.speakers.values().any(|state| state.draft.is_some());
        if phase == Phase::Idle && (!self.pending.is_empty() || speaking) {
            self.fetch_model();
        } else if phase == Phase::Ready
            && self.stopping.is_none()
            && self.in_flight.is_none()
            && self.pending.is_empty()
            && self.last_work.elapsed() > UNLOAD
        {
            self.helper = None;
            self.update(|status| status.phase = Phase::Idle);
        }
    }

    fn dispatch(&mut self) {
        if self.in_flight.is_some() || self.phase() != Phase::Ready {
            return;
        }
        if !batch::due(&self.pending, self.stopping.is_some(), self.words) {
            // Finished speech always goes first; a draft only when nothing is waiting.
            if self.drafts && self.pending.is_empty() && self.stopping.is_none() {
                self.dispatch_draft();
            }
            return;
        }
        let Some(helper) = self.helper.as_mut() else {
            return;
        };
        let parts = batch::take(&mut self.pending, !self.words);
        self.last_work = Instant::now();
        match helper.send(&batch::audio(&parts)) {
            Ok(id) => {
                self.in_flight = Some(InFlight {
                    id,
                    parts,
                    sent: Instant::now(),
                    draft: None,
                })
            }
            // The helper died mid-write; its Exited reply restarts it.
            Err(_) => self.requeue(parts),
        }
    }

    /// A request that never comes back would stop all transcription: nothing else is
    /// sent while one is in flight. Restart the helper and send its audio again.
    fn watch_in_flight(&mut self) {
        let Some(in_flight) = &self.in_flight else {
            return;
        };
        let limit = in_flight.limit(self.cpu);
        if in_flight.sent.elapsed() <= limit {
            return;
        }
        crate::logger::line(format!(
            "Capture {}: no transcript after {} s; restarting the helper",
            self.id,
            limit.as_secs()
        ));
        // A replaced helper's late replies are ignored (`generation`), so this is the
        // same as the helper exiting: requeue, restart, then fall back.
        self.reply(Reply::Exited);
    }

    /// Ask for a new guess at whichever utterance in progress has gone longest without
    /// one.
    fn dispatch_draft(&mut self) {
        let chosen = self
            .speakers
            .iter()
            .filter_map(|(id, state)| {
                let draft = state.draft.as_ref()?;
                let (t_ms, samples) = state.segmenter.in_progress()?;
                let audio_ms = (samples.len() * 1000 / RATE) as i64;
                (draft.t_ms == t_ms && draft.due(audio_ms))
                    .then(|| (id.clone(), draft.waiting_ms(audio_ms)))
            })
            .max_by_key(|(_, waiting_ms)| *waiting_ms)
            .map(|(id, _)| id);
        let Some(speaker_id) = chosen else {
            return;
        };
        let (Some(helper), Some(state)) =
            (self.helper.as_mut(), self.speakers.get_mut(&speaker_id))
        else {
            return;
        };
        let (Some(draft), Some((_, samples))) =
            (state.draft.as_mut(), state.segmenter.in_progress())
        else {
            return;
        };
        let audio_ms = (samples.len() * 1000 / RATE) as i64;
        let audio = super::resample::to_16k(samples);
        let line = draft.line;
        if let Ok(id) = helper.send(&audio) {
            draft.sent(audio_ms);
            self.last_work = Instant::now();
            self.in_flight = Some(InFlight {
                id,
                parts: Vec::new(),
                sent: Instant::now(),
                draft: Some((speaker_id, line, audio_ms)),
            });
        }
    }

    /// A guess at `speaker_id`'s utterance in progress came back: grow its draft if it
    /// is still that utterance's.
    fn draft_heard(
        &mut self,
        speaker_id: &str,
        line: i64,
        audio_ms: i64,
        pieces: &[(i64, String)],
    ) {
        let Some(draft) = self
            .speakers
            .get_mut(speaker_id)
            .and_then(|state| state.draft.as_mut())
            .filter(|draft| draft.line == line)
        else {
            // They stopped speaking meanwhile: the final text is on its way.
            return;
        };
        if let Some(text) = draft.hear(pieces, audio_ms)
            && let Err(error) = self.store.update_line(line, Kind::Draft, &text, None)
        {
            crate::logger::line(format!("Capture: could not save a draft: {error}"));
        }
    }

    /// Write a line's final form: the draft it replaces, or a new line.
    fn settle(
        &self,
        line: Option<i64>,
        kind: Kind,
        t_ms: i64,
        end_ms: Option<i64>,
        speaker: &Speaker,
        text: String,
    ) {
        match line {
            Some(id) => {
                self.last_t_ms
                    .set(self.last_t_ms.get().max(end_ms.unwrap_or(t_ms)));
                if let Err(error) = self.store.update_line(id, kind, &text, end_ms) {
                    crate::logger::line(format!("Capture: could not save a line: {error}"));
                }
            }
            None => self.line_until(kind, t_ms, end_ms, Some(speaker), text),
        }
    }

    /// The GPU failed: carry on with the CPU.
    fn fall_back(&mut self, message: String) {
        if self.cpu {
            self.fail(message);
        } else {
            crate::logger::line(format!(
                "Capture {}: GPU failed ({message}); using the CPU",
                self.id
            ));
            self.cpu = true;
            self.failures = 0;
            self.helper = None;
            self.fetch_model();
        }
    }

    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Ready { device, words } => {
                self.last_work = Instant::now();
                self.words = words;
                // Drafts cost a pass each: only with a model that times words (Parakeet;
                // Whisper hears a 30 s window however short), on the GPU, and live.
                self.drafts = self.options.drafts && words && device != "CPU";
                self.update(|status| {
                    status.phase = Phase::Ready;
                    status.device = Some(device);
                });
            }
            Reply::Transcript { id, pieces, ms } => {
                let Some(in_flight) = self.in_flight.take_if(|in_flight| in_flight.id == id) else {
                    return;
                };
                // It works: failures start counting again from here.
                self.failures = 0;
                self.retry_delay = RETRY_FIRST;
                if let Some((speaker_id, line, audio_ms)) = in_flight.draft {
                    self.update(|status| status.draft_ms += ms);
                    self.draft_heard(&speaker_id, line, audio_ms, &pieces);
                    return;
                }
                self.update(|status| status.final_ms += ms);
                let texts = batch::split(&in_flight.parts, &pieces);
                let mut written = 0;
                for (part, heard) in in_flight.parts.iter().zip(texts) {
                    let start = part.utterance.t_ms;
                    let end = start + part.utterance.duration_ms();
                    match text::clean(&heard, part.utterance.duration_ms()) {
                        Some(said) => {
                            self.settle(
                                part.line,
                                Kind::Speech,
                                start,
                                Some(end),
                                &part.speaker,
                                said,
                            );
                            written += 1;
                        }
                        // Nothing said after all: the draft goes.
                        None => {
                            if let Some(id) = part.line
                                && let Err(error) =
                                    self.store.update_line(id, Kind::Retracted, "", None)
                            {
                                crate::logger::line(format!(
                                    "Capture: could not retract a draft: {error}"
                                ));
                            }
                        }
                    }
                }
                if written > 0 {
                    self.update(|status| status.lines += written);
                }
            }
            Reply::Failed(message) => self.fall_back(message),
            Reply::Exited => {
                if matches!(self.phase(), Phase::Failed { .. }) {
                    return;
                }
                if let Some(in_flight) = self.in_flight.take() {
                    self.requeue(in_flight.parts);
                }
                self.helper = None;
                self.failures += 1;
                if self.failures == 1 {
                    crate::logger::line(format!(
                        "Capture {}: transcription helper stopped; restarting it",
                        self.id
                    ));
                    self.fetch_model();
                } else {
                    self.fall_back("The transcription helper keeps stopping".into());
                }
            }
        }
    }

    fn run(mut self, receiver: Receiver<Input>) -> Result<()> {
        self.line(Kind::Event, 0, None, "Capture started".into());
        self.fetch_model();
        loop {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(Input::Audio {
                    speaker,
                    t_ms,
                    samples,
                }) => {
                    if self.stopping.is_none() {
                        self.audio(speaker, t_ms, samples);
                    }
                }
                Ok(Input::Event { t_ms, text }) => self.line(Kind::Event, t_ms, None, text),
                Ok(Input::Bookmark { t_ms, who, label }) => {
                    let who = who.map(|name| Speaker {
                        id: String::new(),
                        name,
                    });
                    self.line(Kind::Bookmark, t_ms, who.as_ref(), label);
                }
                Ok(Input::Frame { t_ms, who, text }) => {
                    let who = who.map(|name| Speaker {
                        id: String::new(),
                        name,
                    });
                    self.line(Kind::Frame, t_ms, who.as_ref(), text);
                }
                Ok(Input::Participants(names)) => {
                    let _ = self.store.set_participants(&self.id, &names);
                }
                Ok(Input::Downloading(percent)) => {
                    self.update(|status| status.phase = Phase::Downloading { percent })
                }
                Ok(Input::Downloaded(Ok(path))) => self.start_helper(path),
                Ok(Input::Downloaded(Err(message))) => self.fail(message),
                Ok(Input::Helper(generation, reply)) => {
                    if generation == self.generation {
                        self.reply(reply);
                    }
                }
                Ok(Input::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    if self.stopping.is_none() {
                        self.flush_idle(true);

                        let wait = if self.options.drain_all {
                            Duration::from_secs(24 * 3600)
                        } else {
                            Duration::from_millis(self.behind_ms().max(0) as u64 / 2)
                                .clamp(DRAIN_MIN, DRAIN_MAX)
                        };
                        self.stopping = Some(Instant::now() + wait);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.stopping.is_none() {
                self.flush_idle(false);
            }
            self.watch_in_flight();
            self.retry();
            self.keep_up();
            self.rest_or_wake();
            self.dispatch();
            let behind = self.behind_ms();
            if self
                .status
                .lock()
                .is_ok_and(|status| status.behind_ms != behind)
            {
                self.update(|status| status.behind_ms = behind);
            }
            if let Some(deadline) = self.stopping {
                let failed = matches!(self.phase(), Phase::Failed { .. });
                let drained = self.pending.is_empty() && self.in_flight.is_none();
                if failed && !drained && self.last_chance {
                    // Stopping while transcription is down: one more try, now, for what
                    // is left, rather than waiting out the retry.
                    self.last_chance = false;
                    self.retry_at = Some(Instant::now());
                } else if drained || failed || Instant::now() > deadline {
                    break;
                }
            }
        }
        let left: Vec<_> = self
            .pending
            .drain(..)
            .map(|item| (item.speaker, item.utterance, item.line))
            .chain(
                self.in_flight
                    .take()
                    .into_iter()
                    .flat_map(|in_flight| in_flight.parts)
                    .map(|part| (part.speaker, part.utterance, part.line)),
            )
            .collect();
        for (speaker, utterance, line) in left {
            self.settle(
                line,
                Kind::Gap,
                utterance.t_ms,
                None,
                &speaker,
                format!(
                    "{:.0} s not transcribed",
                    utterance.duration_ms() as f64 / 1000.0
                ),
            );
        }
        // Drafts whose speech never reached the transcriber (none should be left: the
        // stop flushes every utterance in progress).
        for state in self.speakers.values_mut() {
            if let Some(draft) = state.draft.take() {
                let _ = self
                    .store
                    .update_line(draft.line, Kind::Retracted, "", None);
            }
        }
        self.helper = None;
        let ended_ms = store::now_ms();
        let started_ms = self.store.require(&self.id)?.started_ms;
        let end_t_ms = (ended_ms - started_ms).max(self.last_t_ms.get());
        self.line(Kind::Event, end_t_ms, None, "Capture ended".into());
        self.store.end_call(&self.id, ended_ms)?;
        let call = self.store.require(&self.id)?;
        let markdown = self.store.markdown(&call)?;
        super::write_private(&super::markdown_path(&self.id)?, markdown.as_bytes())?;
        (self.options.changed)();
        Ok(())
    }
}
