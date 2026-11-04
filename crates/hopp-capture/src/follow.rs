//! Following a capture as it is written: new lines as they arrive, or (for agents) held
//! back until someone says a wake phrase or a script decides it's time.

use anyhow::{Context, Result};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::store::{Kind, Line, Store};

/// How often the database is checked for new lines. WAL makes this cheap.
pub const POLL: Duration = Duration::from_millis(300);
/// The longest a `--wake-if` script may take to decide.
const WAKE_SCRIPT_LIMIT: Duration = Duration::from_secs(10);

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

pub struct Follower {
    call_id: String,
    /// The revision read up to (see `Store::changes`).
    cursor: i64,
    /// Pass on drafts as they grow, and retracted ones, not only final lines.
    drafts: bool,
    held: VecDeque<Line>,
    context: usize,
    wake: Wake,
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

    pub fn cursor(&self) -> i64 {
        self.cursor
    }

    /// The lines held for context, oldest first.
    pub fn held(&self) -> Vec<Line> {
        self.held.iter().cloned().collect()
    }

    pub fn poll(&mut self, store: &Store) -> Result<Poll> {
        let call = store.require(&self.call_id)?;
        // A capture whose recorder died never ends by itself; end it here.
        let ended = if self.watch_orphans && crate::liveness::orphaned(&call) {
            crate::liveness::close(store, &call)?;
            true
        } else {
            !call.live()
        };
        let mut new = store.changes(&self.call_id, self.cursor)?;
        if let Some(last) = new.last() {
            self.cursor = last.rev;
        }
        if !self.drafts {
            new.retain(|line| line.kind.is_final());
        }
        let mut woke = None;
        if self.wake.enabled() {
            // Wake phrases and context use final lines only, as they are what was said.
            for line in new.iter().filter(|line| line.kind.is_final()) {
                self.held.push_back(line.clone());
                if self.held.len() > self.context.max(1) {
                    self.held.pop_front();
                }
                if woke.is_none()
                    && let Some(phrase) = &self.wake.phrase
                    && line.kind == Kind::Speech
                    && contains_phrase(&line.text, phrase)
                {
                    woke = Some(format!("phrase \"{phrase}\""));
                }
            }
            if woke.is_none()
                && new.iter().any(|line| line.kind.is_final())
                && let Some(script) = &self.wake.script
                && run_wake_script(script, &self.held)?
            {
                woke = Some(format!("{} said so", script.display()));
            }
        }
        Ok(Poll { new, woke, ended })
    }

    /// Poll until something happens: in wake mode until it wakes, otherwise until any
    /// new line arrives; either way until the capture ends or `limit` passes.
    pub fn wait(&mut self, store: &Store, limit: Duration) -> Result<Poll> {
        let deadline = Instant::now() + limit;
        let mut arrived = Vec::new();
        loop {
            let mut poll = self.poll(store)?;
            arrived.append(&mut poll.new);
            let done = poll.woke.is_some()
                || poll.ended
                || (!self.wake.enabled() && !arrived.is_empty())
                || Instant::now() >= deadline;
            if done {
                poll.new = arrived;
                return Ok(poll);
            }
            std::thread::sleep(POLL);
        }
    }
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
    use crate::store::tests::{fixture, speech};

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
        use crate::store::{Kind, NewLine};
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
        let mut waiting = Follower::new(&store, "c_2", start, 5, wake)?.without_orphan_check();
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
        let mut follower = Follower::new(&store, "c_2", last, 3, wake)?.without_orphan_check();
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
        Ok(())
    }

    #[cfg(unix)]
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
