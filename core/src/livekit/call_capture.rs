//! Capture in the Mac app: a transcript of the call, one track per person, saved only on
//! this computer. The recorder and its rules are in `hopp-capture`; this starts and stops
//! it with the call and feeds it. Nothing here can hold up the call: when nothing is being
//! captured each audio or video frame costs one atomic load, and a capture never waits.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hopp_capture::heard::{Heard, StampListener};
use hopp_capture::moments::{
    DrawEvent, DrawingMode, Moments, OwnWindows, PathPoint, Point, Sampler, VideoFrame, AGENT_EDGE,
};
use hopp_capture::recorder::{self, Recorder, Speaker};
use hopp_capture::{config, hooks, liveness, models, store, Tap};
use hopp_marks::colours::{canonical_identity, person};
use hopp_marks::stamps::StampMessage;
use livekit::participant::LocalParticipant;
use livekit::webrtc::prelude::I420Buffer;
use livekit::webrtc::video_frame::{native::VideoFrameBufferExt, VideoBuffer, VideoFormatType};
use socket_lib::CoreParticipantState;

use crate::livekit::audio::LIVEKIT_SAMPLE_RATE;
use crate::livekit::participant::CAPTURE_ATTRIBUTE;
use crate::room_service::{ClientEvent, ClientPoint};

// The recorder counts 10 ms frames at its own rate, so it must be the call's.
const _: () = assert!(LIVEKIT_SAMPLE_RATE as usize == hopp_capture::vad::RATE);

/// The helper is only built and bundled for the Mac.
pub const SUPPORTED: bool = cfg!(target_os = "macos");
const TELLING_THE_CALL: Duration = Duration::from_secs(5);
/// How long quitting waits for a capture's last words. One that takes longer is closed
/// next time as "ended unexpectedly", with everything said up to then.
const LAST_WORDS: Duration = Duration::from_secs(10);

/// Captures finishing after their call. Leaving never waits for them, quitting does.
static FINISHING: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// Who is in the call and who shares, by person, to note the changes between the lines.
type People = BTreeMap<String, Person>;

#[derive(Clone, Debug, PartialEq)]
struct Person {
    name: String,
    sharing: bool,
}

fn name_or(name: &str, placeholder: &str) -> String {
    if name.is_empty() { placeholder } else { name }.to_string()
}

/// The server names a person's other connections "Sam video" and "Sam camera".
pub fn person_name<'a>(identity: &str, name: &'a str) -> &'a str {
    let suffix = match identity.rsplit(':').next() {
        Some("video") => " video",
        Some("camera") => " camera",
        _ => return name,
    };
    name.strip_suffix(suffix).unwrap_or(name)
}

fn people(snapshot: &[CoreParticipantState]) -> People {
    snapshot
        .iter()
        .map(|participant| {
            let placeholder = if participant.identity == "local" {
                "You"
            } else {
                "Someone"
            };
            (
                person(&participant.identity).to_string(),
                Person {
                    name: name_or(&participant.name, placeholder),
                    sharing: participant.is_screensharing,
                },
            )
        })
        .collect()
}

/// Add who's here to who has been. A name that arrives late replaces its placeholder.
fn remember(been_here: &mut BTreeMap<String, String>, here: &People) -> Vec<String> {
    for (id, person) in here {
        been_here.insert(id.clone(), person.name.clone());
    }
    been_here.values().cloned().collect()
}

/// What changed, as lines for the transcript.
fn changes(before: &People, after: &People) -> Vec<String> {
    let mut lines = Vec::new();
    for (id, now) in after {
        let was = before.get(id);
        if was.is_none() {
            lines.push(format!("{} joined", now.name));
        }
        if now.sharing != was.is_some_and(|was| was.sharing) {
            let change = if now.sharing { "started" } else { "stopped" };
            lines.push(format!("{} {change} sharing their screen", now.name));
        }
    }
    for (id, was) in before {
        if !after.contains_key(id) {
            lines.push(format!("{} left", was.name));
        }
    }
    lines
}

/// Everyone's identities for their colours: the snapshot calls you "local".
fn everyone(own: &str, people: &People) -> Vec<String> {
    people
        .keys()
        .map(|id| if id == "local" { own } else { id }.to_string())
        .collect()
}

/// What a data message means to the capture's frames, if anything.
fn draw_event(event: &ClientEvent) -> Option<DrawEvent> {
    let at = |point: &ClientPoint| Point {
        x: point.x,
        y: point.y,
    };
    Some(match event {
        ClientEvent::DrawStart(start) => DrawEvent::DrawStart(PathPoint {
            point: at(&start.point),
            path_id: start.path_id,
        }),
        ClientEvent::DrawAddPoint(point) => DrawEvent::DrawAddPoint(at(point)),
        ClientEvent::DrawEnd(point) => DrawEvent::DrawEnd(at(point)),
        ClientEvent::DrawClearPath { path_id } => DrawEvent::DrawClearPath { path_id: *path_id },
        ClientEvent::DrawClearAllPaths => DrawEvent::DrawClearAllPaths,
        ClientEvent::DrawingMode(crate::room_service::DrawingMode::Draw(settings)) => {
            DrawEvent::DrawingMode(DrawingMode::Draw {
                permanent: settings.permanent,
            })
        }
        ClientEvent::DrawingMode(_) => DrawEvent::DrawingMode(DrawingMode::Off),
        ClientEvent::ClickAnimation(point) => DrawEvent::ClickAnimation(at(point)),
        _ => return None,
    })
}

/// A copy of the shared screen to keep, shrunk to a short side of at most `AGENT_EDGE`.
fn still_of(mut buffer: I420Buffer) -> Option<VideoFrame> {
    let (width, height) = (buffer.width(), buffer.height());
    let pixels = width as usize * height as usize;
    if pixels == 0 || pixels > 7680 * 4320 {
        return None;
    }
    let short = width.min(height);
    let scaled = (short > AGENT_EDGE).then(|| {
        let scale = AGENT_EDGE as f64 / short as f64;
        let even = |side: u32| (((side as f64 * scale) as u32) / 2 * 2).max(2);
        buffer.scale(even(width) as i32, even(height) as i32)
    });
    let buffer = scaled.as_ref().unwrap_or(&buffer);
    let (width, height) = (buffer.width(), buffer.height());
    let mut bgra = vec![0; width as usize * height as usize * 4];
    // libyuv names formats by word, so its ARGB is B, G, R, A in memory.
    buffer.to_argb(
        VideoFormatType::ARGB,
        &mut bgra,
        width * 4,
        width as i32,
        height as i32,
    );
    Some(VideoFrame {
        width,
        height,
        bgra,
    })
}

/// One person's audio on its way to the capture.
#[derive(Clone)]
pub struct TrackTap {
    tap: Tap,
    speaker: Arc<Speaker>,
}

impl TrackTap {
    pub fn new(tap: Tap, identity: &str, name: String) -> Self {
        let speaker = Arc::new(Speaker {
            id: canonical_identity(identity).to_string(),
            name,
        });
        Self { tap, speaker }
    }

    pub fn hear(&self, samples: &[i16]) {
        self.tap.audio(&self.speaker, samples);
    }
}

/// Your own voice, after echo cancellation and noise removal: what everyone else hears.
pub struct MicTap {
    track: TrackTap,
    muted: Arc<AtomicBool>,
}

impl MicTap {
    pub fn new(tap: Tap, identity: &str, name: &str, muted: Arc<AtomicBool>) -> Self {
        let track = TrackTap::new(tap, identity, name_or(name, "You"));
        Self { track, muted }
    }

    /// Never while you're muted. The microphone keeps running then, because muting only
    /// mutes the published track, so what you say to the room must stop here.
    pub fn hear(&self, samples: &[i16]) {
        if !self.muted.load(Ordering::Relaxed) {
            self.track.hear(samples);
        }
    }
}

#[derive(Default)]
struct State {
    recorder: Option<Recorder>,
    seen: People,
    /// Everyone who has been in the call, by person, for its heading. `seen` is who's here now.
    been_here: BTreeMap<String, String>,
    own_identity: String,
}

/// The capture's stills of the shared screen. Apart from `State`, so that video and marks
/// don't wait on the people in the call.
#[derive(Default)]
struct Stills {
    /// Off until the call has been told, like the audio.
    watching: AtomicBool,
    moments: Mutex<Option<Moments>>,
    converting: AtomicBool,
    /// Counts changes of the shared screen, to drop a copy of the old one still on its way.
    screen: AtomicU64,
    /// Finds Hopp's windows on the screen you share, for a capture that starts during it.
    own_windows: Mutex<Option<OwnWindows>>,
}

/// Held while a copy of the screen is being converted, however that ends.
struct Converting(Arc<Stills>);

impl Drop for Converting {
    fn drop(&mut self) {
        self.0.converting.store(false, Ordering::Release);
    }
}

/// The capture of the current call, if there is one. Cheap to clone.
#[derive(Clone, Default)]
pub struct CallCapture {
    tap: Tap,
    state: Arc<Mutex<State>>,
    stills: Arc<Stills>,
    changed: Arc<Mutex<Option<Changed>>>,
    /// What the call's capture says it's in: the room's name, or nothing for a call between
    /// two people. The server's name for a room is an id.
    room: Arc<Mutex<String>>,
}

type Changed = Box<dyn Fn(Option<String>) + Send>;

impl std::fmt::Debug for CallCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallCapture").finish()
    }
}

impl CallCapture {
    /// Be told which capture is recording, whenever that changes.
    pub fn on_change(&self, changed: impl Fn(Option<String>) + Send + 'static) {
        *self.changed.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(changed));
    }

    pub fn set_room(&self, name: Option<String>) {
        *self.room.lock().unwrap_or_else(PoisonError::into_inner) = name.unwrap_or_default();
    }

    fn announce(&self, id: Option<String>) {
        let changed = self.changed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(changed) = changed.as_ref() {
            changed(id);
        }
    }

    pub fn tap(&self) -> Tap {
        self.tap.clone()
    }

    pub fn track_tap(&self, identity: &str, name: &str) -> TrackTap {
        TrackTap::new(self.tap(), identity, name_or(name, "Someone"))
    }

    pub fn is_capturing(&self) -> bool {
        self.state().recorder.is_some()
    }

    /// A capture must never take the call down with it.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn moments(&self) -> MutexGuard<'_, Option<Moments>> {
        let moments = self.stills.moments.lock();
        moments.unwrap_or_else(PoisonError::into_inner)
    }

    fn watching(&self) -> bool {
        self.stills.watching.load(Ordering::Relaxed)
    }

    /// A frame of the shared screen, once it's on its way to the call. Called for every
    /// frame, and copies at most two a second: the shrinking and converting happen on a
    /// thread of their own, and a frame that would have to wait is skipped.
    pub fn offer_screen(&self, frame: &impl VideoBuffer) {
        if !self.watching() {
            return;
        }
        let screen = self.stills.screen.load(Ordering::Acquire);
        let Some(sampler) = self.sampler_if_wanted() else {
            return;
        };
        let copy = frame.to_i420();
        let converting = Converting(self.stills.clone());
        let thread = std::thread::Builder::new()
            .name("hopp-capture-still".into())
            .spawn(move || {
                let converting = converting;
                if let Some(still) = still_of(copy) {
                    if converting.0.screen.load(Ordering::Acquire) == screen {
                        sampler.sample(still);
                    }
                }
            });
        if let Err(error) = thread {
            log::error!("Capture: couldn't copy the screen on a new thread: {error}");
        }
    }

    /// Hear what you say for a stamp you pressed at `pressed_ms`, if the call is being captured.
    pub fn listen_for_stamp(
        &self,
        pressed_ms: i64,
        on_heard: impl Fn(Heard) + Send + 'static,
    ) -> Option<StampListener> {
        if !self.watching() {
            return None;
        }
        let (call_id, me) = {
            let state = self.state();
            (
                state.recorder.as_ref()?.id(),
                canonical_identity(&state.own_identity).to_string(),
            )
        };
        let listener = store::default_path()
            .and_then(|database| StampListener::start(database, call_id, me, pressed_ms, on_heard));
        match listener {
            Ok(listener) => Some(listener),
            Err(error) => {
                log::error!("Capture: a stamp can't hear you: {error:#}");
                None
            }
        }
    }

    /// The shared screen started, stopped or changed hands. Say so before its first frame,
    /// which is the only one if the screen stays still. `own_windows` finds Hopp's windows
    /// on it, when it's your whole display.
    pub fn screen_changed(&self, own_windows: Option<OwnWindows>) {
        self.stills.screen.fetch_add(1, Ordering::AcqRel);
        *self
            .stills
            .own_windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = own_windows.clone();
        if let Some(moments) = self.moments().as_mut() {
            moments.screen_changed(own_windows);
        }
    }

    fn sampler_if_wanted(&self) -> Option<Sampler> {
        // A mark may be holding this while the frames thread catches up.
        let mut moments = match self.stills.moments.try_lock() {
            Ok(moments) => moments,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let moments = moments.as_mut().filter(|moments| moments.wants_sample())?;
        if self.stills.converting.swap(true, Ordering::AcqRel) {
            return None;
        }
        moments.sampling();
        Some(moments.sampler())
    }

    /// Ink or a pin, from anyone. Marks can wait for room in the frames thread's queue, so
    /// this isn't for the video paths.
    pub fn mark(&self, owner: &str, name: &str, event: &ClientEvent) {
        if !self.watching() {
            return;
        }
        let Some(event) = draw_event(event) else {
            return;
        };
        let name = name_or(person_name(owner, name), "Someone");
        if let Some(moments) = self.moments().as_mut() {
            moments.draw(owner, &name, &event);
        }
    }

    pub fn own_mark(&self, local: &LocalParticipant, event: &ClientEvent) {
        if self.watching() {
            let name = name_or(&local.name(), "You");
            self.mark(local.identity().as_str(), &name, event);
        }
    }

    pub fn stamp(&self, owner: &str, name: &str, message: &StampMessage) {
        if !self.watching() {
            return;
        }
        let name = name_or(person_name(owner, name), "Someone");
        if let Some(moments) = self.moments().as_mut() {
            moments.stamp(owner, &name, message);
        }
    }

    pub fn own_stamp(&self, local: &LocalParticipant, message: &StampMessage) {
        if self.watching() {
            let name = name_or(&local.name(), "You");
            self.stamp(local.identity().as_str(), &name, message);
        }
    }

    /// Mark this moment in the transcript and on a frame. False if nothing's being captured.
    pub fn bookmark(&self, label: Option<String>) -> bool {
        // Not with `state` held: the stills thread may be busy, and others need it meanwhile.
        let (feed, who) = {
            let state = self.state();
            let Some(recorder) = state.recorder.as_ref() else {
                return false;
            };
            let who = state.seen.get("local").map(|you| you.name.clone());
            (recorder.feed(), who)
        };
        let label = label.unwrap_or_else(|| "Bookmark".to_string());
        if let Some(moments) = self.moments().as_mut() {
            moments.bookmark(who.clone(), &label);
        }
        feed.bookmark(who, label);
        true
    }

    /// Start capturing this call and tell the call, unless it already is.
    pub async fn start(&self, local: LocalParticipant, snapshot: Vec<CoreParticipantState>) {
        if !SUPPORTED || self.is_capturing() {
            return;
        }
        let room = self
            .room
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let capture = self.clone();
        let own_identity = local.identity().as_str().to_string();
        match tokio::task::spawn_blocking(move || capture.begin(room, &own_identity, &snapshot))
            .await
        {
            Ok(Ok(id)) => {
                let capture = self.clone();
                tokio::spawn(async move {
                    tell_the_call(local, true).await;
                    capture.listen(&id);
                });
            }
            Ok(Err(error)) => log::error!("Capture: didn't start: {error:#}"),
            Err(error) => log::error!("Capture: didn't start: {error:?}"),
        }
    }

    /// Open the capture, not yet listening. Returns its id.
    fn begin(
        &self,
        room: String,
        own_identity: &str,
        snapshot: &[CoreParticipantState],
    ) -> anyhow::Result<String> {
        let chosen = config::load();
        let model = models::find(&chosen.model).or_else(|_| models::find(models::DEFAULT))?;
        let store = store::default_path()?;
        // A capture left open by a core that crashed or was killed.
        if let Err(error) =
            store::Store::open(&store).and_then(|store| liveness::close_orphans(&store))
        {
            log::error!("Capture: closing abandoned captures: {error:#}");
        }
        let recorder = Recorder::start(recorder::Options {
            room,
            model,
            backlog_ms: recorder::LIVE_BACKLOG_MS,
            drain_all: false,
            drafts: chosen.drafts,
            gpu: chosen.gpu,
            store,
            changed: Arc::new(|| {}),
        })?;
        let id = recorder.id();
        log::info!("Capture {id}: started");
        let feed = recorder.feed();
        let seen = people(snapshot);
        let mut been_here = BTreeMap::new();
        let participants = remember(&mut been_here, &seen);
        feed.participants(participants.clone());
        for sharer in seen.values().filter(|p| p.sharing) {
            feed.event(format!("{} is sharing their screen", sharer.name));
        }
        // The transcript matters more than its frames.
        match Moments::start(&id, feed) {
            Ok(mut moments) => {
                moments.people(everyone(own_identity, &seen));
                let own_windows = self.stills.own_windows.lock();
                let own_windows = own_windows.unwrap_or_else(PoisonError::into_inner).clone();
                if own_windows.is_some() {
                    moments.screen_changed(own_windows);
                }
                *self.moments() = Some(moments);
            }
            Err(error) => log::error!("Capture {id}: no screen frames: {error:#}"),
        }
        let mut state = self.state();
        state.seen = seen;
        state.been_here = been_here;
        state.own_identity = own_identity.to_string();
        state.recorder = Some(recorder);
        drop(state);
        self.announce(Some(id.clone()));
        // From here, because the recorder hasn't necessarily written them down yet.
        let mut payload = hooks::payload("capture-started", &id);
        payload["participants"] = participants.into();
        hooks::run("capture-started", payload);
        Ok(id)
    }

    /// Start hearing the call, unless it has ended in the meantime.
    fn listen(&self, id: &str) {
        let state = self.state();
        if let Some(recorder) = state.recorder.as_ref().filter(|r| r.id() == id) {
            self.tap.set(Some(recorder.feed()));
            self.stills.watching.store(true, Ordering::Relaxed);
        }
    }

    /// Note who joined, left and started or stopped sharing since the last snapshot.
    pub fn observe(&self, snapshot: &[CoreParticipantState]) {
        let mut state = self.state();
        let Some(feed) = state.recorder.as_ref().map(Recorder::feed) else {
            return;
        };
        let now = people(snapshot);
        if now == state.seen {
            return;
        }
        for line in changes(&state.seen, &now) {
            feed.event(line);
        }
        feed.participants(remember(&mut state.been_here, &now));
        if let Some(moments) = self.moments().as_mut() {
            moments.people(everyone(&state.own_identity, &now));
        }
        state.seen = now;
    }

    /// A full reconnect comes back without the attribute, while the capture goes on. And a
    /// stop told while the connection was down never got through, so others still see it.
    pub fn tell_the_call_again(&self, local: LocalParticipant) {
        let capturing = self.is_capturing();
        let they_think_so = local
            .attributes()
            .get(CAPTURE_ATTRIBUTE)
            .is_some_and(|value| !value.is_empty());
        if capturing || they_think_so {
            tokio::spawn(tell_the_call(local, capturing));
        }
    }

    /// Stop capturing because you said so, and tell the call. Only starting by hand, or the
    /// next call, captures again: a reconnect leaves it stopped.
    pub fn stop_by_hand(&self, local: Option<LocalParticipant>) {
        if !self.is_capturing() {
            return;
        }
        self.stop();
        if let Some(local) = local {
            tokio::spawn(tell_the_call(local, false));
        }
    }

    /// Stop capturing. Leaving a call never waits for its transcript: the last words finish
    /// on a thread of their own.
    pub fn stop(&self) {
        let mut state = self.state();
        self.tap.set(None);
        self.stills.watching.store(false, Ordering::Relaxed);
        state.seen.clear();
        state.been_here.clear();
        let moments = self.moments().take();
        let Some(recorder) = state.recorder.take() else {
            return;
        };
        let id = recorder.id();
        drop(state);
        self.announce(None);
        let finishing = std::thread::Builder::new()
            .name("hopp-capture-finish".into())
            .spawn(move || {
                // Its last frames go into the transcript before the recorder finishes.
                if let Some(moments) = moments {
                    moments.finish();
                }
                if let Err(error) = recorder.finish() {
                    log::error!("Capture {id}: didn't finish cleanly: {error:#}");
                }
                hooks::run("capture-complete", hooks::payload("capture-complete", &id));
                if let Err(error) = hopp_capture::prune() {
                    log::error!("Capture: deleting old captures: {error:#}");
                }
                // This call's own copy, when the disk was full, or an earlier one's.
                if let Err(error) = hopp_capture::write_missing_markdown() {
                    log::error!("Capture: writing missing Markdown copies: {error:#}");
                }
            });
        match finishing {
            Ok(thread) => {
                let mut finishing = FINISHING.lock().unwrap_or_else(PoisonError::into_inner);
                finishing.retain(|thread| !thread.is_finished());
                finishing.push(thread);
            }
            Err(error) => log::error!("Capture: couldn't finish on a new thread: {error}"),
        }
    }
}

/// Whether a call is captured with nobody asking: unless the user turned off capturing every
/// call, see `config::Config::auto`.
pub fn wanted_by_default() -> bool {
    SUPPORTED && config::load().auto
}

/// Give captures finishing their last words a moment before the process goes.
pub fn wait_for_finishing() {
    let deadline = Instant::now() + LAST_WORDS;
    while Instant::now() < deadline {
        let finishing = FINISHING.lock().unwrap_or_else(PoisonError::into_inner);
        if finishing.iter().all(JoinHandle::is_finished) {
            return;
        }
        drop(finishing);
        std::thread::sleep(Duration::from_millis(50));
    }
    log::error!("Capture: quitting before its last words finished");
}

/// Lets others in the call show that it's being captured, before a word of it is heard, or
/// that it no longer is. Best effort, as on Linux: the capture goes ahead either way.
async fn tell_the_call(local: LocalParticipant, capturing: bool) {
    let value = if capturing { "1" } else { "" };
    let attributes = HashMap::from([(CAPTURE_ATTRIBUTE.to_string(), value.to_string())]);
    match tokio::time::timeout(TELLING_THE_CALL, local.set_attributes(attributes)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::error!("Capture: telling the call: {error:?}"),
        Err(_) => log::error!("Capture: telling the call timed out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_service::DrawPathPoint;
    use hopp_capture::moments;
    use hopp_capture::recorder::{Feed, Input};
    use livekit::webrtc::prelude::NV12Buffer;
    use std::collections::BTreeSet;

    fn participant(identity: &str, name: &str, sharing: bool) -> CoreParticipantState {
        CoreParticipantState {
            identity: identity.to_string(),
            name: name.to_string(),
            connected: true,
            muted: false,
            has_camera: false,
            is_screensharing: sharing,
            capturing: false,
            is_speaking: false,
        }
    }

    const SAM: &str = "room:r1:user-sam:audio";

    fn tap_with_feed() -> (Tap, std::sync::mpsc::Receiver<Input>) {
        let (feed, input) = Feed::for_tests();
        let tap = Tap::default();
        tap.set(Some(feed));
        (tap, input)
    }

    fn audio_from(input: &std::sync::mpsc::Receiver<Input>) -> Vec<(String, String, usize)> {
        input
            .try_iter()
            .filter_map(|input| match input {
                Input::Audio {
                    speaker, samples, ..
                } => Some((speaker.id.clone(), speaker.name.clone(), samples.len())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn stopping_by_hand_with_nothing_capturing_does_nothing() {
        let capture = CallCapture::default();
        capture.stop_by_hand(None);
        assert!(!capture.is_capturing());
    }

    #[test]
    fn a_muted_mic_never_reaches_the_transcript() {
        let (tap, input) = tap_with_feed();
        let muted = Arc::new(AtomicBool::new(true));
        let mic = MicTap::new(tap, SAM, "Sam", muted.clone());
        mic.hear(&[1; 160]);
        assert!(audio_from(&input).is_empty(), "said while muted");
        muted.store(false, Ordering::Relaxed);
        mic.hear(&[1; 160]);
        muted.store(true, Ordering::Relaxed);
        mic.hear(&[1; 160]);
        assert_eq!(
            audio_from(&input).len(),
            1,
            "only what was said while unmuted"
        );
    }

    #[test]
    fn a_track_goes_to_the_person_not_their_connection() {
        let (tap, input) = tap_with_feed();
        TrackTap::new(tap, SAM, "Sam".into()).hear(&[1; 160]);
        assert_eq!(
            audio_from(&input),
            vec![("room:r1:user-sam".to_string(), "Sam".to_string(), 160)]
        );
    }

    #[test]
    fn nothing_is_heard_without_a_capture() {
        let (feed, input) = Feed::for_tests();
        let tap = Tap::default();
        TrackTap::new(tap.clone(), SAM, "Sam".into()).hear(&[1; 160]);
        tap.set(Some(feed));
        tap.set(None);
        TrackTap::new(tap, SAM, "Sam".into()).hear(&[1; 160]);
        assert!(audio_from(&input).is_empty());
    }

    #[test]
    fn a_track_without_a_name_is_still_someone() {
        let (tap, input) = tap_with_feed();
        let capture = CallCapture {
            tap,
            ..Default::default()
        };
        capture.track_tap(SAM, "").hear(&[1; 160]);
        assert_eq!(audio_from(&input)[0].1, "Someone");
    }

    #[test]
    fn notes_joins_leaves_and_shares() {
        let start = people(&[participant("local", "Mira", false)]);
        let joined = people(&[
            participant("local", "Mira", false),
            participant(SAM, "Sam", false),
        ]);
        assert_eq!(changes(&start, &joined), vec!["Sam joined"]);
        let sharing = people(&[
            participant("local", "Mira", false),
            participant(SAM, "Sam", true),
        ]);
        assert_eq!(
            changes(&joined, &sharing),
            vec!["Sam started sharing their screen"]
        );
        assert_eq!(
            changes(&sharing, &joined),
            vec!["Sam stopped sharing their screen"]
        );
        assert_eq!(changes(&joined, &start), vec!["Sam left"]);
        assert!(changes(&joined, &joined).is_empty());
    }

    #[test]
    fn someone_joining_while_sharing_is_noted_both_ways() {
        let start = people(&[participant("local", "Mira", false)]);
        let after = people(&[
            participant("local", "Mira", false),
            participant(SAM, "Sam", true),
        ]);
        assert_eq!(
            changes(&start, &after),
            vec!["Sam joined", "Sam started sharing their screen"]
        );
    }

    #[test]
    fn you_are_you_when_the_room_has_no_name_for_you() {
        let seen = people(&[participant("local", "", false)]);
        assert_eq!(seen["local"].name, "You");
    }

    #[test]
    fn a_capture_that_ended_while_telling_the_call_stays_deaf() {
        let capture = CallCapture::default();
        capture.listen("c_gone");
        assert!(capture.tap().feed().is_none());
    }

    #[test]
    fn with_nothing_finishing_quitting_does_not_wait() {
        let started = Instant::now();
        wait_for_finishing();
        assert!(started.elapsed() < LAST_WORDS);
    }

    #[test]
    fn a_nameless_person_still_gets_a_line() {
        let start = people(&[]);
        let after = people(&[participant(SAM, "", false)]);
        assert_eq!(changes(&start, &after), vec!["Someone joined"]);
    }

    #[test]
    fn a_person_is_not_their_connection() {
        assert_eq!(person_name("room:r1:u1:video", "Sam video"), "Sam");
        assert_eq!(person_name("room:r1:u1:camera", "Sam camera"), "Sam");
        assert_eq!(person_name("room:r1:u1:audio", "Sam video"), "Sam video");
        assert_eq!(person_name("local", "Sam"), "Sam");
    }

    /// YUV for a slide in one colour.
    const RED: [u8; 3] = [81, 90, 240];
    const BLUE: [u8; 3] = [41, 240, 110];

    fn slide(width: u32, height: u32, [y, u, v]: [u8; 3]) -> I420Buffer {
        let mut buffer = I420Buffer::new(width, height);
        let (luma, blue_chroma, red_chroma) = buffer.data_mut();
        luma.fill(y);
        blue_chroma.fill(u);
        red_chroma.fill(v);
        buffer
    }

    #[test]
    fn a_still_is_in_the_order_the_frames_expect() {
        let still = still_of(slide(64, 36, RED)).unwrap();
        assert_eq!((still.width, still.height), (64, 36));
        let [blue, green, red, alpha] = still.bgra[..4] else {
            panic!("no pixels");
        };
        assert!(red > 200 && green < 60 && blue < 60, "{blue} {green} {red}");
        assert_eq!(alpha, 255);
    }

    #[test]
    fn a_still_is_no_bigger_than_agents_need() {
        let big = still_of(slide(2560, 1600, RED)).unwrap();
        assert_eq!((big.width, big.height), (2304, 1440));
        assert_eq!(big.bgra.len(), 2304 * 1440 * 4);
        let smaller = still_of(slide(1920, 1080, RED)).unwrap();
        assert_eq!((smaller.width, smaller.height), (1920, 1080));
    }

    fn capture_with_moments(name: &str) -> (CallCapture, std::path::PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("hopp-call-capture-{name}-{}", std::process::id()));
        let (feed, _input) = Feed::for_tests();
        let capture = CallCapture::default();
        *capture.moments() = Some(Moments::start_in(directory.clone(), feed).unwrap());
        (capture, directory)
    }

    fn wants_a_frame(capture: &CallCapture) -> bool {
        capture.moments().as_ref().unwrap().wants_sample()
    }

    #[test]
    fn the_screen_is_left_alone_until_the_call_has_been_told() {
        let (capture, directory) = capture_with_moments("told");
        capture.offer_screen(&slide(64, 36, RED));
        assert!(wants_a_frame(&capture), "copied before the call was told");
        capture.mark(
            SAM,
            "Sam",
            &ClientEvent::ClickAnimation(ClientPoint { x: 0.5, y: 0.5 }),
        );
        capture.stills.watching.store(true, Ordering::Relaxed);
        capture.offer_screen(&slide(64, 36, RED));
        assert!(!wants_a_frame(&capture), "the first frame wasn't copied");
        capture.moments().take().unwrap().finish();
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_finished_mark_asks_for_a_frame_straight_away() {
        let (capture, directory) = capture_with_moments("urgent");
        capture.stills.watching.store(true, Ordering::Relaxed);
        capture.offer_screen(&slide(64, 36, RED));
        assert!(!wants_a_frame(&capture));
        capture.mark(
            SAM,
            "Sam",
            &ClientEvent::ClickAnimation(ClientPoint { x: 0.5, y: 0.5 }),
        );
        assert!(wants_a_frame(&capture));
        capture.moments().take().unwrap().finish();
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn only_marks_are_marks() {
        let point = || ClientPoint { x: 0.25, y: 0.75 };
        let at = Point { x: 0.25, y: 0.75 };
        let start = PathPoint {
            point: at,
            path_id: 4,
        };
        let cases = [
            (
                ClientEvent::DrawStart(DrawPathPoint {
                    point: point(),
                    path_id: 4,
                }),
                Some(DrawEvent::DrawStart(start)),
            ),
            (
                ClientEvent::DrawAddPoint(point()),
                Some(DrawEvent::DrawAddPoint(at)),
            ),
            (ClientEvent::DrawEnd(point()), Some(DrawEvent::DrawEnd(at))),
            (
                ClientEvent::DrawClearPath { path_id: 4 },
                Some(DrawEvent::DrawClearPath { path_id: 4 }),
            ),
            (
                ClientEvent::DrawClearAllPaths,
                Some(DrawEvent::DrawClearAllPaths),
            ),
            (
                ClientEvent::DrawingMode(crate::room_service::DrawingMode::Draw(
                    crate::room_service::DrawSettings { permanent: true },
                )),
                Some(DrawEvent::DrawingMode(DrawingMode::Draw {
                    permanent: true,
                })),
            ),
            (
                ClientEvent::DrawingMode(crate::room_service::DrawingMode::Disabled),
                Some(DrawEvent::DrawingMode(DrawingMode::Off)),
            ),
            (
                ClientEvent::ClickAnimation(point()),
                Some(DrawEvent::ClickAnimation(at)),
            ),
            (ClientEvent::MouseMove(point()), None),
        ];
        for (event, expected) in cases {
            assert_eq!(draw_event(&event), expected, "{event:?}");
        }
    }

    #[test]
    fn your_colour_goes_by_your_user_id() {
        let seen = people(&[
            participant("local", "Alex", false),
            participant(SAM, "Sam", false),
        ]);
        assert_eq!(
            everyone("room:r1:user-alex:audio", &seen),
            vec!["room:r1:user-alex:audio", "user-sam"]
        );
    }

    #[test]
    fn a_call_keeps_the_names_of_everyone_who_was_in_it() {
        let mut been_here = BTreeMap::new();
        let both = people(&[
            participant("local", "", false),
            participant(SAM, "Sam", false),
        ]);
        assert_eq!(remember(&mut been_here, &both), vec!["You", "Sam"]);

        let sam_left = people(&[participant("local", "Alex", false)]);
        assert_eq!(remember(&mut been_here, &sam_left), vec!["Alex", "Sam"]);
    }

    /// A still screen sends one frame, so it must be told about the share first.
    #[test]
    fn marks_on_a_still_screen_still_get_a_frame() -> anyhow::Result<()> {
        let (capture, directory) = capture_with_moments("still");
        capture.stills.watching.store(true, Ordering::Relaxed);
        capture.screen_changed(None);
        capture.offer_screen(&slide(800, 500, RED));
        std::thread::sleep(Duration::from_millis(300));
        capture.mark(
            SAM,
            "Sam",
            &ClientEvent::ClickAnimation(ClientPoint { x: 0.5, y: 0.5 }),
        );
        std::thread::sleep(Duration::from_millis(2_100));
        capture.moments().take().unwrap().finish();
        let frame: moments::FrameInfo =
            serde_json::from_slice(&std::fs::read(directory.join("frame-002.json"))?)?;
        assert_eq!(frame.marks.len(), 1);
        std::fs::remove_dir_all(directory)?;
        Ok(())
    }

    /// A share that ends while its last frame is being converted.
    #[test]
    fn a_copy_of_the_old_screen_is_dropped() {
        let (capture, directory) = capture_with_moments("old");
        capture.stills.watching.store(true, Ordering::Relaxed);
        for _ in 0..50 {
            capture.moments().as_mut().unwrap().screen_changed(None);
            capture.offer_screen(&slide(2560, 1440, RED));
            capture.screen_changed(None);
            while capture.stills.converting.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
        capture.moments().take().unwrap().finish();
        let frames = std::fs::read_dir(&directory)
            .map(|files| files.count())
            .unwrap_or(0);
        let _ = std::fs::remove_dir_all(directory);
        assert_eq!(frames, 0);
    }

    /// What the stills cost the thread that makes the video, on the largest screens.
    #[test]
    #[ignore = "a measurement, it asserts nothing"]
    fn what_a_still_costs_the_video_thread() {
        let (width, height) = (5120, 2880);
        let mut screen = NV12Buffer::new(width, height);
        let (luma, chroma) = screen.data_mut();
        luma.fill(120);
        chroma.fill(128);
        let (capture, directory) = capture_with_moments("cost");
        let mut idle = Vec::new();
        for _ in 0..1_000 {
            let started = Instant::now();
            capture.offer_screen(&screen);
            idle.push(started.elapsed());
        }
        capture.stills.watching.store(true, Ordering::Relaxed);
        let mut copying = Vec::new();
        for _ in 0..8 {
            capture.moments().as_mut().unwrap().screen_changed(None);
            std::thread::sleep(Duration::from_millis(120));
            let started = Instant::now();
            capture.offer_screen(&screen);
            copying.push(started.elapsed());
        }
        let started = Instant::now();
        let still = still_of(screen.to_i420()).unwrap();
        let converting = started.elapsed();
        idle.sort();
        copying.sort();
        println!(
            "not capturing: worst {:?}; a copy of {width}x{height}: median {:?}, worst {:?}; \
             then {converting:?} off that thread for {}x{}",
            idle[idle.len() - 1],
            copying[copying.len() / 2],
            copying[copying.len() - 1],
            still.width,
            still.height,
        );
        capture.moments().take().unwrap().finish();
        let _ = std::fs::remove_dir_all(directory);
    }

    fn test_voices() -> anyhow::Result<(Vec<i16>, Vec<i16>)> {
        let voices = std::path::PathBuf::from(std::env::var("HOPP_TEST_VOICES")?);
        let load = |file: &str| -> anyhow::Result<Vec<i16>> {
            let (rate, samples) = hopp_capture::wav::read(&std::fs::read(voices.join(file))?)?;
            Ok(hopp_capture::resample::to_call_rate(rate, &samples))
        };
        Ok((load("alex.wav")?, load("sam.wav")?))
    }

    fn near(found: [f64; 4], drawn: [f64; 4]) -> bool {
        found.iter().zip(drawn).all(|(a, b)| (a - b).abs() <= 0.01)
    }

    /// The guide's scripted call without LiveKit or a real screen: two voices in real time,
    /// one share, ink and pins from both people, a bookmark and a finished stamp.
    #[test]
    #[ignore = "needs HOPP_CAPTURE_HOME with the model, HOPP_TRANSCRIBE_HELPER, HOPP_TEST_VOICES"]
    fn a_scripted_call_keeps_every_mark_on_a_frame() -> anyhow::Result<()> {
        const ALEX: &str = "room:r1:user-alex:audio";
        let (alex, sam) = test_voices()?;
        let frame = LIVEKIT_SAMPLE_RATE as usize / 100;
        let you = |sharing: bool| participant("local", "Alex", sharing);
        let them = participant(SAM, "Sam", false);

        let capture = CallCapture::default();
        let id = capture.begin("Test room".to_string(), ALEX, &[you(false), them.clone()])?;
        capture.listen(&id);
        let muted = Arc::new(AtomicBool::new(false));
        let mic = MicTap::new(capture.tap(), ALEX, "Alex", muted);
        let remote = capture.track_tap(SAM, "Sam");
        let (first, second) = (slide(1920, 1080, RED), slide(1920, 1080, BLUE));
        let circle = |step: usize| ClientPoint {
            x: 0.30 + 0.05 * (step as f64 / 60. * std::f64::consts::TAU).cos(),
            y: 0.40 + 0.09 * (step as f64 / 60. * std::f64::consts::TAU).sin(),
        };

        let mut voices = alex.chunks(frame).zip(sam.chunks(frame));
        let started = Instant::now();
        // The share goes on a little after the last words, as the new slide settles.
        for index in 0..2_500 {
            if let Some((alex, sam)) = voices.next() {
                mic.hear(alex);
                remote.hear(sam);
            }
            match index {
                50 => {
                    capture.screen_changed(None);
                    capture.observe(&[you(true), them.clone()]);
                }
                200 => capture.mark(
                    SAM,
                    "Sam",
                    &ClientEvent::DrawStart(DrawPathPoint {
                        point: circle(0),
                        path_id: 1,
                    }),
                ),
                201..=259 => {
                    capture.mark(SAM, "Sam", &ClientEvent::DrawAddPoint(circle(index - 200)))
                }
                260 => capture.mark(SAM, "Sam", &ClientEvent::DrawEnd(circle(0))),
                600 => capture.mark(
                    ALEX,
                    "Alex",
                    &ClientEvent::DrawStart(DrawPathPoint {
                        point: ClientPoint { x: 0.50, y: 0.70 },
                        path_id: 1,
                    }),
                ),
                601..=639 => capture.mark(
                    ALEX,
                    "Alex",
                    &ClientEvent::DrawAddPoint(ClientPoint {
                        x: 0.50 + 0.25 * (index - 600) as f64 / 40.,
                        y: 0.70,
                    }),
                ),
                640 => capture.mark(
                    ALEX,
                    "Alex",
                    &ClientEvent::DrawEnd(ClientPoint { x: 0.75, y: 0.70 }),
                ),
                900 => assert!(capture.bookmark(Some("The totals".to_string()))),
                1100 => {
                    let place = StampMessage::StampPlace {
                        id: 9,
                        x: 0.70,
                        y: 0.20,
                    };
                    capture.stamp(SAM, "Sam", &place);
                    // Everyone's ripple where a stamp lands.
                    capture.mark(
                        SAM,
                        "Sam",
                        &ClientEvent::ClickAnimation(ClientPoint { x: 0.70, y: 0.20 }),
                    );
                }
                1150 => capture.stamp(
                    SAM,
                    "Sam",
                    &StampMessage::StampText {
                        id: 9,
                        text: "check this total".to_string(),
                        draft: String::new(),
                        done: true,
                    },
                ),
                1400 => capture.mark(
                    ALEX,
                    "Alex",
                    &ClientEvent::ClickAnimation(ClientPoint { x: 0.50, y: 0.60 }),
                ),
                _ => {}
            }
            if index % 3 == 0 && index >= 50 {
                capture.offer_screen(if index < 1700 { &first } else { &second });
            }
            let next = Duration::from_millis(10) * (index as u32 + 1);
            std::thread::sleep(next.saturating_sub(started.elapsed()));
        }
        capture.screen_changed(None);
        capture.observe(&[you(false), them]);
        capture.stop();
        assert!(!capture.bookmark(None), "bookmarked after the capture");
        wait_for_finishing();

        let transcript = store::Store::open_default()?.transcript(&id)?;
        let text = transcript
            .iter()
            .map(store::format_line)
            .collect::<Vec<_>>()
            .join("\n");
        println!("{id}\n{text}");
        let frames = moments::list(&id)?;
        for frame in &frames {
            println!("{frame:?}");
        }

        let speakers: BTreeSet<_> = transcript
            .iter()
            .filter(|line| matches!(line.kind, store::Kind::Speech))
            .filter_map(|line| line.speaker.as_deref())
            .collect();
        assert_eq!(speakers, BTreeSet::from(["Alex", "Sam"]), "{text}");

        let on_a_frame = |by: &str, shape: &str, bounds: [f64; 4]| {
            frames.iter().find(|frame| {
                frame
                    .marks
                    .iter()
                    .any(|mark| mark.by == by && mark.shape == shape && near(mark.bounds, bounds))
            })
        };
        let circled = on_a_frame("Sam", "circle", [0.25, 0.31, 0.35, 0.49]);
        assert!(circled.is_some(), "the circle: {frames:?}");
        assert!(
            on_a_frame("Alex", "underline", [0.50, 0.70, 0.75, 0.70]).is_some(),
            "the underline: {frames:?}"
        );
        assert!(
            on_a_frame("Alex", "pin", [0.50, 0.60, 0.50, 0.60]).is_some(),
            "the pin: {frames:?}"
        );
        let bookmark = frames.iter().find(|frame| frame.trigger == "bookmark");
        assert!(bookmark.is_some(), "the bookmark: {frames:?}");
        assert!(text.contains("The totals"), "{text}");

        let stamped = on_a_frame("Sam", "stamp", [0.70, 0.20, 0.70, 0.20]);
        let stamped = stamped.expect("the stamp");
        assert!(
            stamped
                .marks
                .iter()
                .any(|mark| mark.text.as_deref() == Some("check this total")),
            "{stamped:?}"
        );
        let pins: Vec<_> = frames
            .iter()
            .flat_map(|frame| &frame.marks)
            .filter_map(|mark| mark.pin)
            .collect();
        assert_eq!(pins, [1, 2], "a stamp's ripple isn't a pin of its own");

        // The share's change of slide, and the ink in the picture itself.
        assert!(
            frames
                .iter()
                .filter(|frame| frame.trigger == "screen")
                .count()
                >= 2,
            "the first view and the new slide: {frames:?}"
        );
        let circled = circled.unwrap();
        let image = moments::load_image(&moments::full(&id, circled.number)?)?;
        let (width, height) = (f64::from(image.width()), f64::from(image.height()));
        let pixel = |x: f64, y: f64| image.get_pixel((width * x) as u32, (height * y) as u32).0;
        let [red, green, blue, _] = pixel(0.1, 0.1);
        assert!(red > 200 && green < 60 && blue < 60, "the slide is red");
        let on_the_line = pixel(0.35, 0.40);
        assert_ne!(
            on_the_line,
            pixel(0.1, 0.1),
            "no ink in frame {}",
            circled.number
        );
        Ok(())
    }

    /// Everything but LiveKit, in real time: two people's tracks into a saved transcript.
    /// Alex mutes after their first sentence, so their second must not be in it.
    #[test]
    #[ignore = "needs HOPP_CAPTURE_HOME with the model, HOPP_TRANSCRIBE_HELPER, HOPP_TEST_VOICES"]
    fn a_call_goes_from_its_tracks_to_a_transcript() -> anyhow::Result<()> {
        let (alex, sam) = test_voices()?;
        let frame = LIVEKIT_SAMPLE_RATE as usize / 100;
        let you = participant("local", "Alex", false);
        let sam_in_call = participant(SAM, "Sam", false);

        let capture = CallCapture::default();
        let id = capture.begin(
            "Test room".to_string(),
            "room:r1:user-alex:audio",
            std::slice::from_ref(&you),
        )?;
        let muted = Arc::new(AtomicBool::new(false));
        let mic = MicTap::new(
            capture.tap(),
            "room:r1:user-alex:audio",
            "Alex",
            muted.clone(),
        );
        let remote = capture.track_tap(SAM, "Sam");

        mic.hear(&alex[..frame]);
        assert!(
            capture.tap().feed().is_none(),
            "heard before telling the call"
        );
        capture.listen(&id);
        capture.observe(&[you.clone(), sam_in_call]);

        let started = Instant::now();
        for (index, (alex, sam)) in alex.chunks(frame).zip(sam.chunks(frame)).enumerate() {
            // Alex's second sentence starts at 10 s.
            muted.store(index >= 800, Ordering::Relaxed);
            mic.hear(alex);
            remote.hear(sam);
            let next = Duration::from_millis(10) * (index as u32 + 1);
            std::thread::sleep(next.saturating_sub(started.elapsed()));
        }
        capture.observe(std::slice::from_ref(&you));
        capture.stop_by_hand(None);
        assert!(!capture.is_capturing());
        assert!(!capture.is_capturing());
        assert!(capture.tap().feed().is_none());
        wait_for_finishing();

        let transcript = store::Store::open_default()?.transcript(&id)?;
        let text = transcript
            .iter()
            .map(store::format_line)
            .collect::<Vec<_>>()
            .join("\n");
        println!("{text}");
        assert!(text.contains("Sam joined"), "{text}");
        assert!(text.contains("Sam left"), "{text}");
        let said = |who: &str, words: &str| {
            transcript.iter().any(|line| {
                line.speaker.as_deref() == Some(who) && line.text.to_lowercase().contains(words)
            })
        };
        assert!(said("Alex", "discount"), "{text}");
        assert!(said("Sam", "lunch"), "{text}");
        assert!(
            !text.to_lowercase().contains("probably"),
            "said while muted: {text}"
        );
        Ok(())
    }

    /// A stamp held down, through the real transcriber: your words reach it by your speaker id,
    /// and those of whoever else is talking don't. You're muted until you press.
    #[test]
    #[ignore = "needs HOPP_CAPTURE_HOME with the model, HOPP_TRANSCRIBE_HELPER, HOPP_TEST_VOICES"]
    fn a_held_stamp_hears_you_and_not_them() -> anyhow::Result<()> {
        const ALEX: &str = "room:r1:user-alex:audio";
        let (alex, sam) = test_voices()?;
        let frame = LIVEKIT_SAMPLE_RATE as usize / 100;
        let people = [
            participant("local", "Alex", false),
            participant(SAM, "Sam", false),
        ];

        let capture = CallCapture::default();
        let id = capture.begin("Test room".to_string(), ALEX, &people)?;
        capture.listen(&id);
        let muted = Arc::new(AtomicBool::new(true));
        let mic = MicTap::new(capture.tap(), ALEX, "Alex", muted.clone());
        let remote = capture.track_tap(SAM, "Sam");

        let (told, heard) = std::sync::mpsc::channel();
        let listener = capture
            .listen_for_stamp(store::now_ms(), move |news| {
                let _ = told.send(news);
            })
            .expect("a capture is running");
        muted.store(false, Ordering::Relaxed);

        let started = Instant::now();
        let mut news = Vec::new();
        for (index, (alex, sam)) in alex.chunks(frame).zip(sam.chunks(frame)).enumerate() {
            // Alex's first sentence is over by 8 s, and they let go.
            if index == 800 {
                muted.store(true, Ordering::Relaxed);
                listener.let_go();
            }
            mic.hear(alex);
            remote.hear(sam);
            news.extend(heard.try_iter());
            let next = Duration::from_millis(10) * (index as u32 + 1);
            std::thread::sleep(next.saturating_sub(started.elapsed()));
        }
        while let Ok(more) = heard.recv_timeout(Duration::from_secs(15)) {
            news.push(more);
        }
        println!("{news:#?}");
        capture.stop_by_hand(None);
        wait_for_finishing();

        assert_eq!(news.last(), Some(&Heard::Finished), "{news:?}");
        let words = news.iter().rev().find_map(|heard| match heard {
            Heard::Words { said, .. } => Some(said.to_lowercase()),
            _ => None,
        });
        let words = words.unwrap_or_default();
        assert!(words.contains("discount"), "{words}");
        assert!(!words.contains("lunch"), "{words}");
        assert!(!words.contains("probably"), "said once you let go: {words}");
        Ok(())
    }

    #[test]
    fn calls_are_captured_unless_the_user_turns_it_off() {
        assert!(config::Config::default().auto);
    }
}
