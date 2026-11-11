//! Off the record: saying "tinfoil hat" or "off the record" stops the capture keeping
//! anything for two minutes, on every client in the call at once.
//!
//! Whoever's capture hears the phrase tells the call with a participant attribute
//! ([`ATTRIBUTE`]), which the server passes to everyone, late joiners included. Every
//! client takes the newest pause it has seen from anyone, so either side can add time
//! (say it again, or press the button) or go back on the record early, and all of them
//! follow. Hopp for Linux and the Mac app each have a copy of this file and must agree.
//!
//! While off the record a capture still listens, in memory, so that it can hear the
//! phrase said again. It writes nothing down: no words, no frames of the shared screen,
//! nothing for an agent following the call. The transcript gets one line when it
//! starts and one when it ends.
use crate::follow::contains_phrase;

/// What to say, and how speech recognition tends to write it.
pub const PHRASES: &str =
    "tinfoil hat|tin foil hat|tinfoil hats|tin foil hats|tinfoil hot|off the record";
/// How long a pause lasts, and how much each press for more time adds.
pub const LENGTH_MS: i64 = 2 * 60 * 1000;
/// The most that can be left at once, however often more time is asked for. Also what
/// a computer with the wrong time can cost the call.
pub const MOST_MS: i64 = 10 * 60 * 1000;
/// Hearing the phrase with this much left or more changes nothing: every capture in
/// the call hears it, a moment apart.
const HEARD_ALREADY_MS: i64 = LENGTH_MS - 15_000;
/// Participant attribute on the audio connection: [`Pause::encode`]. Empty or missing
/// if this participant never paused the call.
pub const ATTRIBUTE: &str = "hopp.capture.pause";

/// The transcript's lines for the start and the end.
pub const STARTED: &str = "Off the record";
pub const ENDED: &str = "Back on the record";

pub fn said(text: &str) -> bool {
    contains_phrase(text, PHRASES)
}

/// One participant's word on the pause. The greatest one in the call is in force: the
/// highest `seq`, and of two with the same `seq` (two people at the same moment) the
/// one that keeps the call off the record longer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pause {
    /// One more than the newest this client had seen when it spoke.
    pub seq: u64,
    /// When the call goes back on the record, in Unix milliseconds. Zero: it already has.
    pub until_ms: i64,
}

impl Pause {
    /// `"<seq>:<until_ms>"`.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.seq, self.until_ms)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let (seq, until_ms) = value.split_once(':')?;
        Some(Self {
            seq: seq.parse().ok()?,
            until_ms: until_ms.parse().ok().filter(|until_ms| *until_ms >= 0)?,
        })
    }

    fn next(latest: Option<Self>, until_ms: i64) -> Self {
        Self {
            seq: latest.map_or(1, |latest| latest.seq.saturating_add(1)),
            until_ms,
        }
    }

    /// Someone said the phrase: off the record for two minutes from now, unless there
    /// is more than that left already. `None` if another capture got there first.
    pub fn said(latest: Option<Self>, now_ms: i64) -> Option<Self> {
        let left = latest
            .and_then(|latest| latest.left_ms(now_ms))
            .unwrap_or(0);
        (left < HEARD_ALREADY_MS).then(|| Self::next(latest, now_ms + LENGTH_MS.max(left)))
    }

    /// Someone pressed for more time: two minutes on top of what is left.
    pub fn more(latest: Option<Self>, now_ms: i64) -> Self {
        let left = latest
            .and_then(|latest| latest.left_ms(now_ms))
            .unwrap_or(0);
        Self::next(latest, now_ms + (left + LENGTH_MS).min(MOST_MS))
    }

    /// Go back on the record now.
    pub fn end(latest: Option<Self>) -> Self {
        Self::next(latest, 0)
    }

    /// How long is left on this computer's clock, if any; never more than
    /// [`MOST_MS`]. Work it out once, when the pause is first seen, and count down
    /// from there.
    pub fn left_ms(&self, now_ms: i64) -> Option<i64> {
        let left = (self.until_ms - now_ms).min(MOST_MS);
        (left > 0).then_some(left)
    }
}

/// The pause in force among everyone's.
pub fn latest(pauses: impl IntoIterator<Item = Pause>) -> Option<Pause> {
    pauses.into_iter().max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hears_either_phrase_however_it_is_written() {
        for text in [
            "Right, tinfoil hat moment.",
            "No no, still tin-foil hat",
            "Tin foil hats on",
            "this bit is off the record",
            "Off the record, though.",
        ] {
            assert!(said(text), "{text}");
        }
        for text in ["Put your hat on", "for the record", "a record of it", ""] {
            assert!(!said(text), "{text}");
        }
    }

    #[test]
    fn what_one_client_writes_another_reads() {
        let pause = Pause::more(None, 1_000);
        assert_eq!(pause.encode(), "1:121000");
        assert_eq!(Pause::parse(&pause.encode()), Some(pause));
        for value in ["", "1", "1:", ":5", "x:5", "1:-5", "1:2:3"] {
            assert_eq!(Pause::parse(value), None, "{value}");
        }
    }

    #[test]
    fn the_newest_word_wins_whoever_said_it() {
        let first = Pause::more(None, 0);
        let more = Pause::more(Some(first), 60_000);
        assert_eq!(more.until_ms, 240_000);
        assert_eq!(latest([first, more]), Some(more));
        let back = Pause::end(Some(more));
        assert_eq!(latest([more, back, first]), Some(back));
        assert_eq!(back.left_ms(60_000), None);
        assert_eq!(latest([]), None);
    }

    #[test]
    fn at_the_same_moment_staying_off_the_record_wins() {
        let first = Pause::more(None, 0);
        let more = Pause::more(Some(first), 100_000);
        let back = Pause::end(Some(first));
        assert_eq!(latest([back, more]), Some(more));
        assert_eq!(latest([more, back]), Some(more));
    }

    #[test]
    fn every_capture_hearing_the_phrase_starts_one_pause() {
        let first = Pause::said(None, 0).unwrap();
        assert_eq!(first.until_ms, LENGTH_MS);
        // Another client's capture hears the same words two seconds later.
        assert_eq!(Pause::said(Some(first), 2_000), None);
        // Said again a minute in: two minutes from then.
        let again = Pause::said(Some(first), 60_000).unwrap();
        assert_eq!((again.seq, again.until_ms), (2, 180_000));
        // Never shortens what a press for more time bought.
        let more = Pause::more(Some(first), 0);
        assert_eq!(Pause::said(Some(more), 30_000), None);
        // After it ran out or was ended, it starts over.
        assert!(Pause::said(Some(first), 500_000).is_some());
        assert!(Pause::said(Some(Pause::end(Some(first))), 1_000).is_some());
    }

    #[test]
    fn time_left_has_a_ceiling() {
        let pause = Pause::more(None, 0);
        assert_eq!(pause.left_ms(0), Some(LENGTH_MS));
        assert_eq!(pause.left_ms(119_000), Some(1_000));
        assert_eq!(pause.left_ms(120_000), None);
        // Their clock is an hour ahead of ours.
        assert_eq!(pause.left_ms(-3_600_000), Some(MOST_MS));
        let mut most = pause;
        for _ in 0..10 {
            most = Pause::more(Some(most), 0);
        }
        assert_eq!(most.left_ms(0), Some(MOST_MS));
    }
}
