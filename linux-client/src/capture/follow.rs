//! Following a capture as it is written: new lines as they arrive, or (for agents) held
//! back until someone asks for the agent by name, or a script decides it's time.
//!
//! The name can come anywhere in what someone says: "Claude, why is that failing?" or
//! "…what do you reckon, Claude?". Hearing it starts a request, and the follower wakes
//! once that person has finished: they go quiet, someone else starts talking, or
//! [`TURN_LIMIT`] passes. The agent gets their words as the request ([`Follower::asked`])
//! with the lines that led up to it, and is told how much of the call it hasn't seen
//! ([`brief`]).

use anyhow::{Context, Result};
use std::{
    collections::{HashSet, VecDeque},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use super::store::{self, Kind, Line, Store};

/// How often the database is checked for new lines. WAL makes this cheap.
pub const POLL: Duration = Duration::from_millis(300);
/// The longest a `--wake-if` script may take to decide.
const WAKE_SCRIPT_LIMIT: Duration = Duration::from_secs(10);
/// After the wake phrase, how long the person must be quiet for their turn to be over.
/// Their drafts count as talking, so a pause to think doesn't cut them off while they
/// still have a line being transcribed.
const TURN_QUIET: Duration = Duration::from_millis(2_000);
/// The longest the rest of a request is waited for once the wake phrase is heard.
const TURN_LIMIT: Duration = Duration::from_secs(20);

#[derive(Clone, Default)]
pub struct Wake {
    /// Wake when a spoken line contains this, ignoring case and punctuation: any of its
    /// `|`-separated alternatives, or a known mishearing of one (`SOUND_ALIKES`).
    pub phrase: Option<String>,
    /// Wake when this executable, given the held lines as JSON lines on stdin, exits 0.
    pub script: Option<PathBuf>,
}

impl Wake {
    pub fn enabled(&self) -> bool {
        self.phrase.is_some() || self.script.is_some()
    }
}

/// Someone said the wake phrase and may not have finished.
struct Asking {
    /// The line with the wake phrase.
    line: i64,
    t_ms: i64,
    speaker: Option<String>,
    reason: String,
    heard: Instant,
    /// When they were last heard, as a draft or a final line.
    last: Instant,
    /// Their drafts not yet final: they are still talking.
    open: HashSet<i64>,
    /// Someone else has started talking since.
    interrupted: bool,
}

pub struct Follower {
    call_id: String,
    /// The revision read up to (see `Store::changes`).
    cursor: i64,
    /// Pass on drafts as they grow, and retracted ones, not only final lines.
    drafts: bool,
    held: VecDeque<Line>,
    context: usize,
    wake: Wake,
    asking: Option<Asking>,
    /// What the person who woke the follower asked, once they finished.
    asked: Option<Line>,
    quiet: Duration,
    /// End the capture if its recorder has died (see `liveness`).
    watch_orphans: bool,
}

pub struct Poll {
    /// Lines that arrived since the last poll.
    pub new: Vec<Line>,
    /// Why the follower woke, if it did.
    pub woke: Option<String>,
    /// The capture has finished; there will be no more lines.
    pub ended: bool,
}

impl Follower {
    /// Follow `call_id` from revision `after`. In wake mode the last `context` final
    /// lines are kept so an agent that wakes gets what led up to it.
    pub fn new(
        store: &Store,
        call_id: &str,
        after: i64,
        context: usize,
        wake: Wake,
    ) -> Result<Self> {
        let mut held = VecDeque::new();
        if wake.enabled() {
            held.extend(
                store
                    .tail(call_id, context)?
                    .into_iter()
                    .filter(|line| line.rev <= after),
            );
        }
        Ok(Self {
            call_id: call_id.into(),
            cursor: after,
            drafts: false,
            held,
            context,
            wake,
            asking: None,
            asked: None,
            quiet: TURN_QUIET,
            watch_orphans: true,
        })
    }

    /// Also pass on drafts: lines of kind `draft` as they grow (the same id each time)
    /// and `retracted` when one held no speech. A draft's final form arrives under its id
    /// as kind `speech`.
    pub fn with_drafts(mut self) -> Self {
        self.drafts = true;
        self
    }

    /// Without the orphan check, for tests whose calls have no recorder.
    #[cfg(test)]
    fn without_orphan_check(mut self) -> Self {
        self.watch_orphans = false;
        self
    }

    /// A shorter quiet for tests.
    #[cfg(test)]
    fn quiet_after(mut self, quiet: Duration) -> Self {
        self.quiet = quiet;
        self
    }

    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// The lines held for context, oldest first.
    pub fn held(&self) -> Vec<Line> {
        self.held.iter().cloned().collect()
    }

    /// What the person who said the wake phrase asked: everything they said in that
    /// turn, from the start of the message with the phrase in it.
    pub fn asked(&self) -> Option<&Line> {
        self.asked.as_ref()
    }

    pub fn poll(&mut self, store: &Store) -> Result<Poll> {
        let call = store.require(&self.call_id)?;
        // A capture whose recorder died never ends by itself; end it here.
        let ended = if self.watch_orphans && super::liveness::orphaned(&call) {
            super::liveness::close(store, &call)?;
            true
        } else {
            !call.live()
        };
        let mut new = store.changes(&self.call_id, self.cursor)?;
        if let Some(last) = new.last() {
            self.cursor = last.rev;
        }
        let mut woke = None;
        if self.wake.enabled() {
            self.listen(&new);
            let over = self.asking.as_ref().is_some_and(|asking| {
                ended
                    || asking.interrupted
                    || (asking.open.is_empty() && asking.last.elapsed() >= self.quiet)
                    || asking.heard.elapsed() >= TURN_LIMIT
            });
            if over && let Some(asking) = self.asking.take() {
                self.asked = self.request(asking.line);
                woke = Some(asking.reason);
            }
            if woke.is_none()
                && self.asking.is_none()
                && new.iter().any(|line| line.kind.is_final())
                && let Some(script) = &self.wake.script
                && run_wake_script(script, &self.held)?
            {
                woke = Some(format!("{} said so", script.display()));
            }
        }
        if !self.drafts {
            new.retain(|line| line.kind.is_final());
        }
        Ok(Poll { new, woke, ended })
    }

    /// Hold new final lines for context, notice the wake phrase, and keep track of
    /// whether whoever said it is still talking. Wake phrases and context use final lines
    /// only, as they are what was said; drafts only show someone is talking.
    fn listen(&mut self, new: &[Line]) {
        let now = Instant::now();
        for line in new {
            if let Some(asking) = &mut self.asking {
                if line.speaker_id.is_some() && line.speaker_id == asking.speaker {
                    asking.last = now;
                    if line.kind == Kind::Draft {
                        asking.open.insert(line.id);
                    } else {
                        asking.open.remove(&line.id);
                    }
                } else if line.kind == Kind::Speech
                    && line.speaker_id.is_some()
                    && line.t_ms > asking.t_ms
                    && !store::reaction(line)
                {
                    asking.interrupted = true;
                }
            }
            if !line.kind.is_final() {
                continue;
            }
            self.held.push_back(line.clone());
            // While someone is asking, their words stay held whatever the context size.
            if self.asking.is_none() && self.held.len() > self.context.max(1) {
                self.held.pop_front();
            }
            if self.asking.is_none()
                && let Some(phrase) = &self.wake.phrase
                && line.kind == Kind::Speech
                && contains_phrase(&line.text, phrase)
            {
                self.asking = Some(Asking {
                    line: line.id,
                    t_ms: line.t_ms,
                    speaker: line.speaker_id.clone(),
                    reason: format!("phrase \"{phrase}\""),
                    heard: now,
                    last: now,
                    open: HashSet::new(),
                    interrupted: false,
                });
            }
        }
    }

    /// The message holding line `id`, with what its speaker said after it until
    /// someone else spoke (not counting short reactions).
    fn request(&self, id: i64) -> Option<Line> {
        let mut lines = self.held();
        store::sort(&mut lines);
        let messages = store::messages(&lines);
        let start = messages
            .iter()
            .position(|message| message.id == id || message.joins.contains(&id))?;
        let mut request = messages[start].clone();
        for later in &messages[start + 1..] {
            let theirs = later.kind == Kind::Speech && later.speaker_id == request.speaker_id;
            if theirs {
                if request.joins.is_empty() {
                    request.joins.push(request.id);
                }
                request.joins.extend(later.joins.iter().copied());
                if later.joins.is_empty() {
                    request.joins.push(later.id);
                }
                request.end_ms = Some(store::end_of(&request).max(store::end_of(later)));
                request.text = format!("{} {}", request.text.trim(), later.text.trim());
            } else if later.kind == Kind::Speech && !store::reaction(later) {
                break;
            }
        }
        Some(request)
    }

    /// Poll until something happens: in wake mode until it wakes, otherwise until any
    /// new line arrives; either way until the capture ends or `limit` passes. Someone
    /// who has said the wake phrase is heard out even past `limit` (up to
    /// [`TURN_LIMIT`]).
    pub fn wait(&mut self, store: &Store, limit: Duration) -> Result<Poll> {
        let deadline = Instant::now() + limit;
        let mut arrived = Vec::new();
        loop {
            let mut poll = self.poll(store)?;
            arrived.append(&mut poll.new);
            let done = poll.woke.is_some()
                || poll.ended
                || (!self.wake.enabled() && !arrived.is_empty())
                || (Instant::now() >= deadline && self.asking.is_none());
            if done {
                poll.new = arrived;
                return Ok(poll);
            }
            std::thread::sleep(POLL);
        }
    }

    /// How much of the call came before the held lines: final lines, and the frames
    /// among them by number.
    pub fn earlier(&self, store: &Store) -> Result<(usize, Vec<u32>)> {
        let held: HashSet<i64> = self.held.iter().map(|line| line.id).collect();
        let Some(first) = self.held.iter().map(|line| line.t_ms).min() else {
            return Ok((0, Vec::new()));
        };
        let earlier: Vec<Line> = store
            .transcript(&self.call_id)?
            .into_iter()
            .filter(|line| line.t_ms < first && !held.contains(&line.id))
            .collect();
        let frames = earlier
            .iter()
            .filter(|line| line.kind == Kind::Frame)
            .filter_map(|line| super::frame_files::number(&line.text))
            .collect();
        Ok((earlier.len(), frames))
    }
}

/// Where a woken agent can look further back.
pub enum Tools {
    /// Hopp's MCP tools.
    Mcp,
    /// `hopp capture` commands.
    Cli,
}

/// `#4`, or `#1–#12`.
pub fn frame_range(frames: &[u32]) -> Option<String> {
    let first = frames.iter().min()?;
    let last = frames.iter().max()?;
    Some(if first == last {
        format!("#{first}")
    } else {
        format!("#{first}–#{last}")
    })
}

/// What a woken agent is told to do: who asked what, that the lines it has are only the
/// latest part of the call, and how to look further back and at the screen before
/// answering.
pub fn brief(
    call_id: &str,
    asked: Option<&Line>,
    earlier_lines: usize,
    earlier_frames: &[u32],
    tools: Tools,
) -> String {
    let mut out = match asked {
        Some(line) => format!(
            "{} asked you this at {}: \"{}\"",
            line.speaker.as_deref().unwrap_or("Someone"),
            store::clock(line.t_ms),
            line.text.trim()
        ),
        None => "You were woken by the wake script.".into(),
    };
    out.push_str("\n\nThe lines you have are only the latest part of the call");
    match (earlier_lines, frame_range(earlier_frames)) {
        (0, _) => out.push('.'),
        (lines, None) => out.push_str(&format!("; {lines} earlier lines aren't shown.")),
        (lines, Some(frames)) => out.push_str(&format!(
            "; {lines} earlier lines, with frames {frames}, aren't shown."
        )),
    }
    let (read, look, then) = match tools {
        Tools::Mcp => (
            format!("capture_transcript (id \"{call_id}\") or capture_search"),
            "capture_frame (zoom shows a marked area in detail)".to_owned(),
            "Then answer with notify_user, and use request_user_decision before acting on \
             it. Call capture_follow again with after_line to keep listening."
                .to_owned(),
        ),
        Tools::Cli => (
            format!("`hopp capture show {call_id}` or `hopp capture search WORDS`"),
            match super::frame_files::dir(call_id) {
                Ok(dir) => format!("the frame images in {}", dir.display()),
                Err(_) => "the frame images".to_owned(),
            },
            "Then answer, and run this again to keep listening.".to_owned(),
        ),
    };
    out.push_str(&format!(
        " Before you answer, work out what they mean and what they're looking at: read \
         back through the call with {read}, and look at the shared screen with {look}. \
         Trust the screen over the transcript where they disagree. {then} If they were \
         talking about you rather than to you, answer nothing."
    ));
    out
}

/// Lower-case letters and digits, single spaces: "Hey, Claude!" → "hey claude".
fn normalise(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_lowercase().next().unwrap_or(character)
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Other ways speech recognition writes what people call an agent. Parakeet wrote
/// "hey Claude" as "Heckord" and "Hey Cord" (2 November 2026); Tuple keeps a list like
/// this for "Pi". A list rather than a rule that sounds words out: "hey, could you"
/// sounds out much like "hey Claude", and would wake an agent all the time. The bare
/// name leaves out "cloud", which comes up all the time in talk about software.
const SOUND_ALIKES: &[(&str, &[&str])] = &[
    (
        "hey claude",
        &[
            "heckord",
            "hey cord",
            "hey clod",
            "hey cloud",
            "hey claud",
            "hey clawed",
        ],
    ),
    ("claude", &["claud", "clawed", "clod"]),
];

/// The phrases that wake for `phrase`: each of its `|`-separated alternatives, and the
/// known mishearings of any of them.
fn alternatives(phrase: &str) -> Vec<String> {
    let mut phrases: Vec<String> = Vec::new();
    for alternative in phrase.split('|').map(normalise) {
        if alternative.is_empty() || phrases.contains(&alternative) {
            continue;
        }
        if let Some((_, alikes)) = SOUND_ALIKES.iter().find(|(known, _)| *known == alternative) {
            phrases.extend(alikes.iter().map(|alike| (*alike).to_owned()));
        }
        phrases.push(alternative);
    }
    phrases
}

pub fn contains_phrase(text: &str, phrase: &str) -> bool {
    let text = format!(" {} ", normalise(text));
    alternatives(phrase)
        .iter()
        .any(|phrase| text.contains(&format!(" {phrase} ")))
}

fn run_wake_script(script: &PathBuf, held: &VecDeque<Line>) -> Result<bool> {
    let mut child = Command::new(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .with_context(|| format!("Running {}", script.display()))?;
    if let Some(mut stdin) = child.stdin.take() {
        for line in held {
            // A script that exits without reading everything is fine.
            if writeln!(stdin, "{}", serde_json::to_string(line)?).is_err() {
                break;
            }
        }
    }
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if started.elapsed() > WAKE_SCRIPT_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::store::tests::{fixture, speech};

    #[test]
    fn wake_phrases_ignore_case_punctuation_and_word_fragments() {
        assert!(contains_phrase(
            "Hey, Claude! What does cpal do?",
            "hey claude"
        ));
        assert!(contains_phrase("so hey claude", "Hey Claude"));
        assert!(!contains_phrase("they claudette", "hey claude"));
        assert!(!contains_phrase("anything", "  "));
    }

    /// What Parakeet wrote for "hey Claude" in the Scrawl tests, and what must not wake.
    #[test]
    fn known_mishearings_wake_and_near_misses_dont() {
        for heard in [
            "Heckord, can you check the PR?",
            "Hey Cord, what's next?",
            "hey, clod.",
        ] {
            assert!(contains_phrase(heard, "hey claude"), "{heard}");
        }
        for not in [
            "Hey, could you share your screen?",
            "heck, alright then",
            "the cord is unplugged",
            "move it to the cloud",
        ] {
            assert!(!contains_phrase(not, "hey claude"), "{not}");
        }
        assert!(!contains_phrase("deploy it to the cloud", "claude"));
        assert!(contains_phrase("Claud, are you there", "claude"));
    }

    #[test]
    fn several_phrases_separated_by_a_bar() {
        assert!(contains_phrase(
            "ok computer, run it",
            "hey claude | ok computer"
        ));
        assert!(contains_phrase("heckord", "ok computer|hey claude"));
        assert!(!contains_phrase("hello", "hey claude|ok computer"));
        assert!(!contains_phrase("anything", " | "));
    }

    #[test]
    fn streams_new_lines_and_notices_the_end() -> Result<()> {
        let (_directory, store) = fixture()?;
        let last = store.tail("c_2", 1)?[0].id;
        let mut follower =
            Follower::new(&store, "c_2", last, 0, Wake::default())?.without_orphan_check();
        assert!(follower.poll(&store)?.new.is_empty());
        store.add_line("c_2", &speech(1_000, "Mira", "Opening the port now."))?;
        let poll = follower.wait(&store, Duration::from_secs(2))?;
        assert_eq!(poll.new.len(), 1);
        assert!(!poll.ended);
        store.end_call("c_2", 200_000)?;
        assert!(follower.poll(&store)?.ended);
        Ok(())
    }

    /// Without drafts a follower sees a line once, when it is final; with them, every
    /// version under the same id. Wake phrases never fire on a draft.
    #[test]
    fn drafts_reach_only_followers_that_ask_and_never_wake() -> Result<()> {
        use crate::capture::store::{Kind, NewLine};
        let (_directory, store) = fixture()?;
        let start = store.latest_rev("c_2")?;
        let mut plain =
            Follower::new(&store, "c_2", start, 0, Wake::default())?.without_orphan_check();
        let mut live = Follower::new(&store, "c_2", start, 0, Wake::default())?
            .without_orphan_check()
            .with_drafts();
        let wake = Wake {
            phrase: Some("hey claude".into()),
            script: None,
        };
        let mut waiting = Follower::new(&store, "c_2", start, 5, wake)?
            .without_orphan_check()
            .quiet_after(Duration::ZERO);
        let id = store.add_line(
            "c_2",
            &NewLine {
                kind: Kind::Draft,
                ..speech(5_000, "Mira", "")
            },
        )?;
        store.update_line(id, Kind::Draft, "Hey Claude", None)?;
        assert!(plain.poll(&store)?.new.is_empty());
        let seen = live.poll(&store)?.new;
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].id, seen[0].kind), (id, Kind::Draft));
        assert!(waiting.poll(&store)?.woke.is_none(), "not on a draft");
        store.update_line(id, Kind::Speech, "Hey Claude, what's next?", Some(6_000))?;
        let final_line = plain.poll(&store)?.new;
        assert_eq!(final_line.len(), 1);
        assert_eq!((final_line[0].id, final_line[0].kind), (id, Kind::Speech));
        assert_eq!(live.poll(&store)?.new[0].kind, Kind::Speech);
        assert!(waiting.poll(&store)?.woke.is_some(), "on the final line");
        Ok(())
    }

    #[test]
    fn holds_context_until_the_wake_phrase() -> Result<()> {
        let (_directory, store) = fixture()?;
        let last = store.tail("c_2", 1)?[0].id;
        let wake = Wake {
            phrase: Some("hey claude".into()),
            script: None,
        };
        let mut follower = Follower::new(&store, "c_2", last, 3, wake)?
            .without_orphan_check()
            .quiet_after(QUIET);
        assert_eq!(follower.held().len(), 1, "starts with recent context");
        store.add_line("c_2", &speech(1_000, "Mira", "One."))?;
        store.add_line("c_2", &speech(2_000, "Mira", "Two."))?;
        let poll = follower.poll(&store)?;
        assert!(poll.woke.is_none());
        store.add_line("c_2", &speech(3_000, "You", "Hey Claude, which port?"))?;
        let poll = follower.wait(&store, Duration::from_secs(2))?;
        assert!(poll.woke.is_some());
        let held = follower.held();
        assert_eq!(held.len(), 3, "context is capped");
        assert_eq!(held.last().unwrap().text, "Hey Claude, which port?");
        assert_eq!(follower.asked().unwrap().text, "Hey Claude, which port?");
        Ok(())
    }

    const QUIET: Duration = Duration::from_millis(200);

    fn calling(store: &Store, phrase: &str) -> Result<Follower> {
        let last = store.latest_rev("c_2")?;
        let wake = Wake {
            phrase: Some(phrase.into()),
            script: None,
        };
        Ok(Follower::new(store, "c_2", last, 10, wake)?
            .without_orphan_check()
            .quiet_after(QUIET))
    }

    /// The name starts a request; the follower wakes once they've finished, with all of
    /// it, so the agent hears the question and not only its name.
    #[test]
    fn hears_the_whole_request_after_the_name() -> Result<()> {
        let (_directory, store) = fixture()?;
        let mut follower = calling(&store, "claude")?;
        store.add_line("c_2", &speech(10_000, "Mira", "Claude,"))?;
        assert!(follower.poll(&store)?.woke.is_none(), "still talking");
        store.add_line("c_2", &speech(11_000, "Mira", "why does the port close?"))?;
        let poll = follower.wait(&store, Duration::from_secs(2))?;
        assert!(poll.woke.is_some());
        assert_eq!(
            follower.asked().unwrap().text,
            "Claude, why does the port close?"
        );
        Ok(())
    }

    /// The name at the end of a question brings in the rest of their message.
    #[test]
    fn the_name_can_come_last() -> Result<()> {
        let (_directory, store) = fixture()?;
        let mut follower = calling(&store, "claude")?;
        store.add_line(
            "c_2",
            &speech(10_000, "Mira", "So the socket closes early."),
        )?;
        store.add_line(
            "c_2",
            &speech(12_000, "Mira", "What do you reckon, Claude?"),
        )?;
        let poll = follower.wait(&store, Duration::from_secs(2))?;
        assert!(poll.woke.is_some());
        assert_eq!(
            follower.asked().unwrap().text,
            "So the socket closes early. What do you reckon, Claude?"
        );
        Ok(())
    }

    /// A line still being said keeps the request open past the quiet; someone else
    /// talking ends it at once, but a "yeah" from them doesn't.
    #[test]
    fn a_turn_ends_when_they_stop_or_someone_else_starts() -> Result<()> {
        use crate::capture::store::{Kind, NewLine};
        let (_directory, store) = fixture()?;
        let mut follower = calling(&store, "claude")?.quiet_after(Duration::ZERO);
        store.add_line("c_2", &speech(10_000, "Mira", "Claude, look at this"))?;
        let draft = store.add_line(
            "c_2",
            &NewLine {
                kind: Kind::Draft,
                ..speech(12_000, "Mira", "and the")
            },
        )?;
        assert!(follower.poll(&store)?.woke.is_none(), "a draft is talking");
        store.add_line("c_2", &speech(12_500, "Joe", "Yeah"))?;
        assert!(follower.poll(&store)?.woke.is_none(), "a reaction");
        store.update_line(draft, Kind::Speech, "and the log.", Some(13_000))?;
        assert!(follower.poll(&store)?.woke.is_some());
        assert_eq!(
            follower.asked().unwrap().text,
            "Claude, look at this and the log."
        );

        let mut follower = calling(&store, "claude")?.quiet_after(Duration::from_secs(60));
        store.add_line("c_2", &speech(20_000, "Mira", "Claude, can you check it"))?;
        assert!(follower.poll(&store)?.woke.is_none());
        store.add_line("c_2", &speech(21_000, "Joe", "I think it's the timeout."))?;
        assert!(follower.poll(&store)?.woke.is_some(), "Joe took over");
        assert_eq!(follower.asked().unwrap().text, "Claude, can you check it");
        Ok(())
    }

    #[test]
    fn the_brief_says_what_was_asked_and_what_is_not_shown() -> Result<()> {
        let (_directory, store) = fixture()?;
        let mut follower = calling(&store, "claude")?.quiet_after(Duration::ZERO);
        store.add_line("c_2", &speech(10_000, "Mira", "Claude, which port?"))?;
        assert!(follower.poll(&store)?.woke.is_some());
        let brief = brief("c_2", follower.asked(), 12, &[3, 1, 7], Tools::Mcp);
        assert!(brief.starts_with("Mira asked you this at 00:10: \"Claude, which port?\""));
        assert!(brief.contains("12 earlier lines, with frames #1–#7, aren't shown"));
        assert!(brief.contains("capture_frame"));
        Ok(())
    }

    #[test]
    fn a_script_can_decide_to_wake() -> Result<()> {
        let (directory, store) = fixture()?;
        let script = directory.path().join("wake-on-port.sh");
        std::fs::write(&script, "#!/bin/sh\ngrep -q port\n")?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))?;
        let wake = Wake {
            phrase: None,
            script: Some(script),
        };
        let last = store.tail("c_2", 1)?[0].id;
        let mut follower =
            Follower::new(&store, "c_2", last, 5, wake.clone())?.without_orphan_check();
        store.add_line("c_2", &speech(1_000, "Mira", "Nothing yet."))?;
        assert!(
            follower.poll(&store)?.woke.is_some(),
            "held context mentions a port"
        );
        let mut quiet = Follower::new(&store, "c_1", 0, 5, wake)?.without_orphan_check();
        assert!(quiet.poll(&store)?.woke.is_none());
        Ok(())
    }
}
