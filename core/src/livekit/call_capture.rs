//! Capture in the Mac app: a transcript of the call, one track per person, saved only on
//! this computer. The recorder and its rules are in `hopp-capture`; this starts and stops
//! it with the call and feeds it. Nothing here can hold up the call: when nothing is being
//! captured each 10 ms audio frame costs one atomic load, and a capture never waits.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hopp_capture::recorder::{self, Recorder, Speaker};
use hopp_capture::{config, liveness, models, store, Tap};
use hopp_marks::colours::{canonical_identity, person};
use livekit::participant::LocalParticipant;
use socket_lib::CoreParticipantState;

use crate::livekit::audio::LIVEKIT_SAMPLE_RATE;
use crate::livekit::participant::CAPTURE_ATTRIBUTE;

// The recorder counts 10 ms frames at its own rate, so it must be the call's.
const _: () = assert!(LIVEKIT_SAMPLE_RATE as usize == hopp_capture::vad::RATE);

/// The helper is only built and bundled for the Mac.
const SUPPORTED: bool = cfg!(target_os = "macos");
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
}

/// The capture of the current call, if there is one. Cheap to clone.
#[derive(Clone, Default)]
pub struct CallCapture {
    tap: Tap,
    state: Arc<Mutex<State>>,
}

impl std::fmt::Debug for CallCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallCapture").finish()
    }
}

impl CallCapture {
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

    /// Capture this call if the user chose to capture every call. Off until they do, see
    /// `config::Config::auto`.
    pub async fn auto_start(
        &self,
        room: String,
        local: LocalParticipant,
        snapshot: Vec<CoreParticipantState>,
    ) {
        if !SUPPORTED || !config::load().auto {
            return;
        }
        let capture = self.clone();
        match tokio::task::spawn_blocking(move || capture.begin(room, &snapshot)).await {
            Ok(Ok(id)) => {
                let capture = self.clone();
                tokio::spawn(async move {
                    tell_the_call(local).await;
                    capture.listen(&id);
                });
            }
            Ok(Err(error)) => log::error!("Capture: didn't start: {error:#}"),
            Err(error) => log::error!("Capture: didn't start: {error:?}"),
        }
    }

    /// Open the capture, not yet listening. Returns its id.
    fn begin(&self, room: String, snapshot: &[CoreParticipantState]) -> anyhow::Result<String> {
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
        feed.participants(seen.values().map(|p| p.name.clone()).collect());
        for sharer in seen.values().filter(|p| p.sharing) {
            feed.event(format!("{} is sharing their screen", sharer.name));
        }
        let mut state = self.state();
        state.seen = seen;
        state.recorder = Some(recorder);
        Ok(id)
    }

    /// Start hearing the call, unless it has ended in the meantime.
    fn listen(&self, id: &str) {
        let state = self.state();
        if let Some(recorder) = state.recorder.as_ref().filter(|r| r.id() == id) {
            self.tap.set(Some(recorder.feed()));
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
        feed.participants(now.values().map(|p| p.name.clone()).collect());
        state.seen = now;
    }

    /// Stop capturing. Leaving a call never waits for its transcript: the last words finish
    /// on a thread of their own.
    pub fn stop(&self) {
        let recorder = {
            let mut state = self.state();
            self.tap.set(None);
            state.seen.clear();
            state.recorder.take()
        };
        let Some(recorder) = recorder else {
            return;
        };
        let id = recorder.id();
        let finishing = std::thread::Builder::new()
            .name("hopp-capture-finish".into())
            .spawn(move || {
                if let Err(error) = recorder.finish() {
                    log::error!("Capture {id}: didn't finish cleanly: {error:#}");
                }
                if let Err(error) = hopp_capture::prune() {
                    log::error!("Capture: deleting old captures: {error:#}");
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

/// Lets others in the call show that it's being captured, before a word of it is heard.
/// Best effort, as on Linux: the capture goes ahead either way.
async fn tell_the_call(local: LocalParticipant) {
    let attributes = HashMap::from([(CAPTURE_ATTRIBUTE.to_string(), "1".to_string())]);
    match tokio::time::timeout(TELLING_THE_CALL, local.set_attributes(attributes)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::error!("Capture: telling the call: {error:?}"),
        Err(_) => log::error!("Capture: telling the call timed out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopp_capture::recorder::{Feed, Input};

    fn participant(identity: &str, name: &str, sharing: bool) -> CoreParticipantState {
        CoreParticipantState {
            identity: identity.to_string(),
            name: name.to_string(),
            connected: true,
            muted: false,
            has_camera: false,
            is_screensharing: sharing,
            capturing: false,
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

    /// Everything but LiveKit, in real time: two people's tracks into a saved transcript.
    /// Alex mutes after their first sentence, so their second must not be in it.
    #[test]
    #[ignore = "needs HOPP_CAPTURE_HOME with the model, HOPP_TRANSCRIBE_HELPER, HOPP_TEST_VOICES"]
    fn a_call_goes_from_its_tracks_to_a_transcript() -> anyhow::Result<()> {
        let voices = std::path::PathBuf::from(std::env::var("HOPP_TEST_VOICES")?);
        let load = |file: &str| -> anyhow::Result<Vec<i16>> {
            let (rate, samples) = hopp_capture::wav::read(&std::fs::read(voices.join(file))?)?;
            Ok(hopp_capture::resample::to_call_rate(rate, &samples))
        };
        let (alex, sam) = (load("alex.wav")?, load("sam.wav")?);
        let frame = LIVEKIT_SAMPLE_RATE as usize / 100;
        let you = participant("local", "Alex", false);
        let sam_in_call = participant(SAM, "Sam", false);

        let capture = CallCapture::default();
        let id = capture.begin("Test room".to_string(), std::slice::from_ref(&you))?;
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
        capture.stop();
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

    #[cfg(target_os = "macos")]
    #[test]
    fn calls_are_not_captured_until_the_user_chooses() {
        assert!(!config::Config::default().auto);
    }
}
