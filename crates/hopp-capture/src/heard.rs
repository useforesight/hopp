//! What you say into a stamp, from the capture's transcript as it's written.
//!
//! Hold a stamp down and your lines that start before you let go are the note, as on Linux. A
//! quick click is for typing, so its listener is dropped. It's read on a thread of its own, so a
//! window never waits on the database.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::store::{self, Kind, Line, Store};

/// How often to look for words you've said: they reach the transcript a moment after you say
/// them.
const LISTEN_EVERY: Duration = Duration::from_millis(300);
/// Speech starting this long before you pressed still counts: people start talking as they
/// click.
const LEAD_MS: i64 = 500;
/// After you let go, the stamp finishes once your words have been final this long.
const SETTLED: Duration = Duration::from_millis(600);
/// Nothing heard this long after letting go: it was silence.
const NOTHING_SAID: Duration = Duration::from_secs(4);
/// However far behind transcription is, finish with what's there by this long after.
const GIVE_UP: Duration = Duration::from_secs(12);

/// What's been heard for a stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// All your words so far: final `said`, then `draft` words still being recognised.
    Words { said: String, draft: String },
    /// You've let go or stopped talking and they're written down, or as near as they'll get.
    Finished,
    /// You've said nothing, or the transcript can't be read.
    NothingSaid,
}

/// Your lines so far, and what to make of them.
struct Listening {
    /// Your speaker id in the transcript.
    me: String,
    /// Your lines starting from this capture time count…
    from_ms: i64,
    /// …up to this one, once you've let go.
    until_ms: Option<i64>,
    released: Option<Instant>,
    /// By line id: (final, text).
    lines: BTreeMap<i64, (bool, String)>,
    changed: Instant,
}

impl Listening {
    fn new(me: String, from_ms: i64, now: Instant) -> Self {
        Self {
            me,
            from_ms,
            until_ms: None,
            released: None,
            lines: BTreeMap::new(),
            changed: now,
        }
    }

    fn let_go(&mut self, until_ms: i64, now: Instant) {
        self.until_ms = Some(until_ms);
        self.released = Some(now);
    }

    /// Takes in the lines written since the last look, and says if there's news.
    fn look(&mut self, lines: Vec<Line>, now: Instant) -> Option<Heard> {
        let mut changed = false;
        for line in lines {
            let mine = line.speaker_id.as_deref() == Some(self.me.as_str());
            let held =
                line.t_ms >= self.from_ms && self.until_ms.is_none_or(|until| line.t_ms <= until);
            if !mine || !held {
                continue;
            }
            match line.kind {
                Kind::Speech => {
                    self.lines.insert(line.id, (true, line.text));
                }
                Kind::Draft => {
                    self.lines.insert(line.id, (false, line.text));
                }
                Kind::Retracted => {
                    self.lines.remove(&line.id);
                }
                _ => continue,
            }
            changed = true;
        }
        if changed {
            self.changed = now;
            let (said, draft) = self.words();
            return Some(Heard::Words { said, draft });
        }
        let said = self.lines.values().any(|(done, _)| *done);
        let talking = self.lines.values().any(|(done, _)| !done);
        let since_released = now.duration_since(self.released?);
        if (said && !talking && now.duration_since(self.changed) >= SETTLED)
            || since_released >= GIVE_UP
        {
            return Some(Heard::Finished);
        }
        (!said && !talking && since_released >= NOTHING_SAID).then_some(Heard::NothingSaid)
    }

    fn words(&self) -> (String, String) {
        let joined = |wanted: bool| {
            let pieces: Vec<&str> = self
                .lines
                .values()
                .filter(|(done, _)| *done == wanted)
                .map(|(_, text)| text.trim())
                .filter(|text| !text.is_empty())
                .collect();
            pieces.join(" ")
        };
        (joined(true), joined(false))
    }
}

/// Hears you for a stamp, until it's finished or this is dropped.
pub struct StampListener {
    /// When you let go, in Unix milliseconds.
    let_go: mpsc::Sender<i64>,
}

impl StampListener {
    /// `pressed_ms` is when you pressed, in Unix milliseconds. `me` is your speaker id in the
    /// capture. `on_heard` is called on the listener's thread.
    pub fn start(
        database: PathBuf,
        call_id: String,
        me: String,
        pressed_ms: i64,
        on_heard: impl Fn(Heard) + Send + 'static,
    ) -> Result<Self> {
        let (let_go, letting_go) = mpsc::channel();
        std::thread::Builder::new()
            .name("hopp-stamp-listener".into())
            .spawn(move || {
                let hearing = Hearing {
                    database,
                    call_id,
                    me,
                    pressed_ms,
                    letting_go,
                };
                if let Err(error) = hearing.run(&on_heard) {
                    log::error!("Stamp: couldn't read the transcript: {error:#}");
                    on_heard(Heard::NothingSaid);
                }
            })
            .context("Starting the stamp's listener")?;
        Ok(Self { let_go })
    }

    /// You let go: lines starting from now on aren't for this stamp.
    pub fn let_go(&self) {
        // It's gone if it's finished already, and that's fine.
        let _ = self.let_go.send(store::now_ms());
    }
}

struct Hearing {
    database: PathBuf,
    call_id: String,
    me: String,
    pressed_ms: i64,
    letting_go: Receiver<i64>,
}

impl Hearing {
    fn run(self, on_heard: &impl Fn(Heard)) -> Result<()> {
        let store = Store::open(&self.database)?;
        let call = store.require(&self.call_id)?;
        let mut rev = store.latest_rev(&self.call_id)?;
        let from_ms = self.pressed_ms - call.started_ms - LEAD_MS;
        let mut listening = Listening::new(self.me, from_ms, Instant::now());
        loop {
            match self.letting_go.recv_timeout(LISTEN_EVERY) {
                Ok(let_go_ms) => {
                    listening.let_go(let_go_ms - call.started_ms, Instant::now());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            let lines = store.changes(&self.call_id, rev).unwrap_or_else(|error| {
                log::warn!("Stamp: couldn't read the transcript this time: {error:#}");
                Vec::new()
            });
            rev = lines.iter().map(|line| line.rev).fold(rev, i64::max);
            let Some(heard) = listening.look(lines, Instant::now()) else {
                continue;
            };
            let finished = matches!(heard, Heard::Finished | Heard::NothingSaid);
            on_heard(heard);
            if finished {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewLine;

    const ME: &str = "room:r1:user-sam";

    fn line(id: i64, t_ms: i64, kind: Kind, who: &str, text: &str) -> Line {
        Line {
            id,
            t_ms,
            end_ms: None,
            kind,
            speaker_id: Some(who.into()),
            speaker: None,
            text: text.into(),
            joins: Vec::new(),
            rev: id,
        }
    }

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    fn words(said: &str, draft: &str) -> Option<Heard> {
        Some(Heard::Words {
            said: said.into(),
            draft: draft.into(),
        })
    }

    #[test]
    fn your_words_fill_it_as_they_are_recognised() {
        let start = Instant::now();
        let mut listening = Listening::new(ME.into(), 1_000, start);
        let mut look = |lines, ms| listening.look(lines, after(start, ms));

        assert_eq!(look(vec![], 100), None);
        assert_eq!(
            look(vec![line(1, 1_200, Kind::Draft, ME, "change")], 200),
            words("", "change")
        );
        assert_eq!(
            look(vec![line(1, 1_200, Kind::Draft, ME, "change this")], 300),
            words("", "change this")
        );
        assert_eq!(
            look(vec![line(1, 1_200, Kind::Speech, ME, "Change this.")], 400),
            words("Change this.", "")
        );
        assert_eq!(
            look(vec![line(2, 3_000, Kind::Draft, ME, " please ")], 500),
            words("Change this.", "please")
        );
        assert_eq!(
            look(vec![line(2, 3_000, Kind::Retracted, ME, "")], 600),
            words("Change this.", ""),
            "a draft that was only a cough goes"
        );
        assert_eq!(look(vec![], 60_000), None, "you're still holding it");
    }

    #[test]
    fn only_your_lines_from_while_you_held_it_count() {
        let start = Instant::now();
        let mut listening = Listening::new(ME.into(), 1_000, start);
        let lines = vec![
            line(1, 999, Kind::Speech, ME, "before"),
            line(2, 1_500, Kind::Speech, "room:r1:user-mira", "someone else"),
            line(3, 1_500, Kind::Event, ME, "an event"),
            line(4, 1_500, Kind::Speech, ME, "held"),
            line(5, 2_001, Kind::Speech, ME, "after"),
        ];
        listening.let_go(2_000, start);
        assert_eq!(listening.look(lines, start), words("held", ""));
    }

    #[test]
    fn it_finishes_once_your_words_have_settled() {
        let start = Instant::now();
        let mut listening = Listening::new(ME.into(), 0, start);
        listening.look(
            vec![line(1, 10, Kind::Speech, ME, "Done")],
            after(start, 100),
        );
        assert_eq!(listening.look(vec![], after(start, 2_000)), None, "held");

        listening.let_go(3_000, after(start, 3_000));
        assert_eq!(
            listening.look(
                vec![line(2, 2_900, Kind::Draft, ME, "and")],
                after(start, 3_100)
            ),
            words("Done", "and")
        );
        assert_eq!(
            listening.look(vec![], after(start, 4_000)),
            None,
            "still being recognised"
        );
        assert_eq!(
            listening.look(
                vec![line(2, 2_900, Kind::Speech, ME, "and more")],
                after(start, 4_100)
            ),
            words("Done and more", "")
        );
        assert_eq!(listening.look(vec![], after(start, 4_600)), None);
        assert_eq!(
            listening.look(vec![], after(start, 4_700)),
            Some(Heard::Finished)
        );
    }

    #[test]
    fn it_gives_up_on_slow_transcription_with_what_is_there() {
        let start = Instant::now();
        let mut listening = Listening::new(ME.into(), 0, start);
        listening.look(vec![line(1, 10, Kind::Draft, ME, "slow")], start);
        listening.let_go(1_000, start);
        assert_eq!(listening.look(vec![], after(start, 11_999)), None);
        assert_eq!(
            listening.look(vec![], after(start, 12_000)),
            Some(Heard::Finished)
        );
    }

    #[test]
    fn silence_leaves_it_to_type_into() {
        let start = Instant::now();
        let mut listening = Listening::new(ME.into(), 0, start);
        listening.let_go(1_000, start);
        assert_eq!(listening.look(vec![], after(start, 3_999)), None);
        assert_eq!(
            listening.look(vec![], after(start, 4_000)),
            Some(Heard::NothingSaid)
        );
    }

    fn fixture(started_ms: i64) -> Result<(tempfile::TempDir, PathBuf, Store)> {
        let directory = tempfile::tempdir()?;
        let database = directory.path().join("capture/index.db");
        let store = Store::open(&database)?;
        store.begin_call("c", "Core", started_ms, "base.en")?;
        Ok((directory, database, store))
    }

    fn spoken(t_ms: i64, kind: Kind, text: &str) -> NewLine {
        NewLine {
            t_ms,
            end_ms: None,
            kind,
            speaker_id: Some(ME.into()),
            speaker: Some("Sam".into()),
            text: text.into(),
        }
    }

    const PATIENCE: Duration = Duration::from_secs(10);

    #[test]
    fn it_hears_you_from_the_transcript_until_you_let_go() -> Result<()> {
        let started_ms = store::now_ms() - 10_000;
        let (_directory, database, store) = fixture(started_ms)?;
        store.add_line("c", &spoken(1_000, Kind::Speech, "long ago"))?;
        let pressed_ms = store::now_ms();
        let (told, heard) = mpsc::channel();
        let listener =
            StampListener::start(database, "c".into(), ME.into(), pressed_ms, move |h| {
                let _ = told.send(h);
            })?;
        // It reads where the transcript has got to as it starts.
        std::thread::sleep(Duration::from_millis(150));

        let held = pressed_ms - started_ms;
        let line = store.add_line("c", &spoken(held - 400, Kind::Draft, "over"))?;
        assert_eq!(heard.recv_timeout(PATIENCE)?, words("", "over").unwrap());
        store.update_line(line, Kind::Speech, "Over here.", None)?;
        assert_eq!(
            heard.recv_timeout(PATIENCE)?,
            words("Over here.", "").unwrap()
        );

        listener.let_go();
        assert_eq!(heard.recv_timeout(PATIENCE)?, Heard::Finished);
        assert!(
            heard.recv_timeout(PATIENCE).is_err(),
            "it stops, and lets go of what it was told to tell"
        );
        Ok(())
    }

    #[test]
    fn dropping_it_stops_it() -> Result<()> {
        let (_directory, database, _store) = fixture(store::now_ms())?;
        let (told, heard) = mpsc::channel();
        let listener =
            StampListener::start(database, "c".into(), ME.into(), store::now_ms(), move |h| {
                let _ = told.send(h);
            })?;
        drop(listener);
        assert!(matches!(
            heard.recv_timeout(PATIENCE),
            Err(RecvTimeoutError::Disconnected)
        ));
        Ok(())
    }

    #[test]
    fn a_call_it_cannot_find_leaves_it_to_type_into() -> Result<()> {
        let (_directory, database, _store) = fixture(store::now_ms())?;
        let (told, heard) = mpsc::channel();
        let _listener = StampListener::start(
            database,
            "no such call".into(),
            ME.into(),
            store::now_ms(),
            move |h| {
                let _ = told.send(h);
            },
        )?;
        assert_eq!(heard.recv_timeout(PATIENCE)?, Heard::NothingSaid);
        Ok(())
    }
}
