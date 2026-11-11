//! What someone is asking the agent, while they say it, so the call can show at once that
//! the wake word was heard. An agent takes up to a minute to answer; without this, you
//! can't tell a slow answer from a word that was never heard.
//!
//! The recorder passes every draft and final text here. Hearing the wake word
//! ([`DEFAULT_WAKE`]) starts an ask, and that person's words are reported as they grow
//! until they go quiet. If the final text turns out not to have the word after all
//! ("Keepers"), the ask is withdrawn. This only shows what was heard: waking the agent is
//! still the follower's job (`follow`), from final text.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use crate::follow::{DEFAULT_WAKE, contains_phrase};

/// With nothing new from them for this long, their ask is over. Longer than the
/// follower's two seconds, because final text arrives a little after the speech.
const QUIET: Duration = Duration::from_secs(3);
/// The longest an ask goes on: the follower's limit, and time for the last line.
const LIMIT: Duration = Duration::from_secs(25);

#[derive(Debug, PartialEq)]
pub enum Change {
    /// What they have said since the wake word's line began.
    Words(String),
    /// The word wasn't said after all.
    Withdrawn,
}

struct Ask {
    /// Their lines from the one with the word on, in order, with each one's latest text.
    /// A line that never had a draft has no id.
    lines: Vec<(Option<i64>, String)>,
    started: Instant,
    heard: Instant,
}

#[derive(Default)]
pub struct Asks(HashMap<String, Ask>);

impl Asks {
    /// `speaker`'s `line` now reads `text` (empty if it came to nothing). Returns what
    /// to show, if this is part of an ask.
    pub fn hear(
        &mut self,
        speaker: &str,
        line: Option<i64>,
        text: &str,
        now: Instant,
    ) -> Option<Change> {
        if self.0.get(speaker).is_some_and(|ask| {
            now.saturating_duration_since(ask.heard) > QUIET
                || now.saturating_duration_since(ask.started) > LIMIT
        }) {
            self.0.remove(speaker);
        }
        let Some(ask) = self.0.get_mut(speaker) else {
            if !contains_phrase(text, DEFAULT_WAKE) {
                return None;
            }
            self.0.insert(
                speaker.to_owned(),
                Ask {
                    lines: vec![(line, text.to_owned())],
                    started: now,
                    heard: now,
                },
            );
            return Some(Change::Words(text.to_owned()));
        };
        let known = ask
            .lines
            .iter_mut()
            .find(|(id, _)| line.is_some() && *id == line);
        match known {
            Some((_, words)) if words == text => return None,
            Some((_, words)) => *words = text.to_owned(),
            None if text.is_empty() => return None,
            None => ask.lines.push((line, text.to_owned())),
        }
        ask.heard = now;
        if !ask
            .lines
            .iter()
            .any(|(_, words)| contains_phrase(words, DEFAULT_WAKE))
        {
            self.0.remove(speaker);
            return Some(Change::Withdrawn);
        }
        let words: Vec<&str> = ask
            .lines
            .iter()
            .map(|(_, words)| words.as_str())
            .filter(|words| !words.is_empty())
            .collect();
        Some(Change::Words(words.join(" ")))
    }
}

/// `text` word by word, each marked if it is the wake word, for showing it lit up.
pub fn marked(text: &str) -> Vec<(String, bool)> {
    text.split_whitespace()
        .map(|word| (word.to_owned(), contains_phrase(word, DEFAULT_WAKE)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Option<Change> {
        Some(Change::Words(text.into()))
    }

    #[test]
    fn the_words_grow_from_the_wake_word_on() {
        let start = Instant::now();
        let mut asks = Asks::default();
        assert_eq!(asks.hear("kg", Some(1), "So", start), None);
        assert_eq!(
            asks.hear("kg", Some(1), "So, Jeepers", start),
            words("So, Jeepers")
        );
        assert_eq!(asks.hear("kg", Some(1), "So, Jeepers", start), None);
        // A pause after the word starts a new line: it is still the same ask.
        let later = start + Duration::from_millis(1500);
        assert_eq!(
            asks.hear("kg", Some(2), "why's that", later),
            words("So, Jeepers why's that")
        );
        assert_eq!(
            asks.hear("kg", Some(2), "Why's that failing?", later),
            words("So, Jeepers Why's that failing?")
        );
        // Someone else talking isn't part of it.
        assert_eq!(asks.hear("jc", Some(3), "Yeah", later), None);
    }

    #[test]
    fn a_word_the_final_text_drops_is_withdrawn() {
        let start = Instant::now();
        let mut asks = Asks::default();
        assert_eq!(asks.hear("kg", Some(1), "Jeepers", start), words("Jeepers"));
        assert_eq!(
            asks.hear("kg", Some(1), "Keepers of the old API", start),
            Some(Change::Withdrawn)
        );
        assert_eq!(asks.hear("kg", Some(2), "and so on", start), None);
        // So is a draft that came to nothing.
        assert_eq!(asks.hear("kg", Some(3), "Jeepers", start), words("Jeepers"));
        assert_eq!(asks.hear("kg", Some(3), "", start), Some(Change::Withdrawn));
    }

    #[test]
    fn an_ask_ends_once_they_go_quiet() {
        let start = Instant::now();
        let mut asks = Asks::default();
        asks.hear("kg", Some(1), "Jeepers, why?", start);
        let later = start + QUIET + Duration::from_millis(1);
        assert_eq!(asks.hear("kg", Some(2), "Anyway", later), None);
        // And however long they talk, it ends at the limit.
        let mut asks = Asks::default();
        asks.hear("kg", Some(1), "Jeepers", start);
        let mut now = start;
        while now.duration_since(start) <= LIMIT {
            now += Duration::from_secs(2);
            asks.hear("kg", Some(2), &format!("{now:?}"), now);
        }
        assert_eq!(asks.hear("kg", Some(3), "more", now), None);
    }

    #[test]
    fn the_wake_word_is_marked() {
        assert_eq!(
            marked("So, Jeepers, why's that?"),
            [
                ("So,".to_owned(), false),
                ("Jeepers,".to_owned(), true),
                ("why's".to_owned(), false),
                ("that?".to_owned(), false),
            ]
        );
        assert!(!marked("keepers")[0].1);
    }

    #[test]
    fn a_line_with_no_draft_still_counts() {
        let start = Instant::now();
        let mut asks = Asks::default();
        assert_eq!(
            asks.hear("kg", None, "What do you reckon, Jeepers?", start),
            words("What do you reckon, Jeepers?")
        );
        assert_eq!(
            asks.hear("kg", None, "About the port.", start),
            words("What do you reckon, Jeepers? About the port.")
        );
    }
}
