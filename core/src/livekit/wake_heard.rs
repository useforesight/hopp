//! "Heard you" in the Mac app: what the call window shows the moment the capture hears
//! someone say the wake word, and for how long. An agent takes up to a minute to answer;
//! without this a slow answer looks the same as a word that was never heard.
//!
//! Which words are an ask is `hopp_capture::asking`'s business, on the recorder's
//! thread. This keeps what Hopp for Linux keeps in its windows (`ui/heard.rs` there):
//! one ask shows at a time, the latest, and it goes [`HOLD`] after its last word, or at
//! once if the final text turns out not to have the word.

use std::time::{Duration, Instant};

use hopp_capture::asking::marked;
use socket_lib::{WakeHeard, WakeWord};

/// How long the light and the bubble stay after the last word was heard. The same on
/// Hopp for Linux.
pub const HOLD: Duration = Duration::from_millis(3_500);
/// The most words the bubble is given. Earlier ones make way.
const WORDS: usize = 30;
/// What the app calls you, as in the participants snapshot.
pub const YOU: &str = "local";

/// Whose ask the app is showing.
#[derive(Debug, Default)]
pub struct Heard {
    /// Who, and when their words last changed.
    showing: Option<(String, Instant)>,
}

impl Heard {
    /// The capture heard `identity` ask `words`, or with none that they didn't say the
    /// word after all. Returns what to tell the app, if anything: who to show, or `None`
    /// to take it back.
    pub fn hear(
        &mut self,
        identity: &str,
        words: Option<&str>,
        now: Instant,
    ) -> Option<Option<WakeHeard>> {
        let Some(words) = words else {
            // Not the word after all. Someone else's ask stays.
            let theirs = self.showing(now).is_some_and(|who| who == identity);
            return theirs.then(|| {
                self.showing = None;
                None
            });
        };
        self.showing = Some((identity.to_string(), now));
        Some(Some(WakeHeard {
            identity: identity.to_string(),
            words: shown(words),
        }))
    }

    /// [`HOLD`] after some words were heard: whether the app is to take the ask back,
    /// which it isn't if more words came meanwhile.
    pub fn over(&mut self, now: Instant) -> bool {
        let over = self
            .showing
            .as_ref()
            .is_some_and(|(_, updated)| held(*updated, now));
        if over {
            self.showing = None;
        }
        over
    }

    fn showing(&self, now: Instant) -> Option<&str> {
        self.showing
            .as_ref()
            .filter(|(_, updated)| !held(*updated, now))
            .map(|(who, _)| who.as_str())
    }
}

fn held(updated: Instant, now: Instant) -> bool {
    now.saturating_duration_since(updated) >= HOLD
}

/// The words to show, each marked if it is the wake word. A long ask keeps its latest
/// words; the first is then "…".
fn shown(words: &str) -> Vec<WakeWord> {
    let words = marked(words);
    let skipped = words.len().saturating_sub(WORDS);
    let more = (skipped > 0).then(|| WakeWord {
        text: "…".into(),
        wake: false,
    });
    let latest = words
        .into_iter()
        .skip(skipped)
        .map(|(text, wake)| WakeWord { text, wake });
    more.into_iter().chain(latest).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAM: &str = "room:r1:sam";
    const MIRA: &str = "room:r1:mira";

    fn who(told: Option<Option<WakeHeard>>) -> Option<Option<String>> {
        told.map(|heard| heard.map(|heard| heard.identity))
    }

    #[test]
    fn the_words_reach_the_app_with_the_wake_word_marked() {
        let mut heard = Heard::default();
        let told = heard.hear(SAM, Some("So, Jeepers, why?"), Instant::now());
        let told = told.unwrap().unwrap();
        assert_eq!(told.identity, SAM);
        let words: Vec<(&str, bool)> = told
            .words
            .iter()
            .map(|word| (word.text.as_str(), word.wake))
            .collect();
        assert_eq!(words, [("So,", false), ("Jeepers,", true), ("why?", false)]);
    }

    #[test]
    fn an_ask_goes_a_hold_after_its_last_word() {
        let start = Instant::now();
        let mut heard = Heard::default();
        heard.hear(YOU, Some("Jeepers"), start);
        // More words a second later: the first timer finds it still going.
        let later = start + Duration::from_secs(1);
        assert_eq!(
            who(heard.hear(YOU, Some("Jeepers why"), later)),
            Some(Some(YOU.into()))
        );
        assert!(!heard.over(start + HOLD));
        assert!(!heard.over(later + HOLD - Duration::from_millis(1)));
        assert!(heard.over(later + HOLD));
        // Once only.
        assert!(!heard.over(later + HOLD));
    }

    #[test]
    fn a_word_that_wasnt_said_goes_at_once() {
        let start = Instant::now();
        let mut heard = Heard::default();
        heard.hear(SAM, Some("Jeepers"), start);
        assert_eq!(who(heard.hear(SAM, None, start)), Some(None));
        // Nothing left for the timer, or for being told twice.
        assert!(!heard.over(start + HOLD));
        assert_eq!(heard.hear(SAM, None, start), None);
    }

    #[test]
    fn someone_else_asking_takes_over_and_their_ask_stays() {
        let start = Instant::now();
        let mut heard = Heard::default();
        heard.hear(SAM, Some("Jeepers"), start);
        assert_eq!(
            who(heard.hear(MIRA, Some("Jeepers, wait"), start)),
            Some(Some(MIRA.into()))
        );
        // Sam's turns out not to be the word: Mira's ask is the one showing.
        assert_eq!(heard.hear(SAM, None, start), None);
        assert!(heard.over(start + HOLD));
        // Taken back after it has gone anyway: nothing to say.
        heard.hear(SAM, Some("Jeepers"), start);
        assert_eq!(heard.hear(SAM, None, start + HOLD), None);
    }

    #[test]
    fn a_long_ask_keeps_its_latest_words() {
        let long: Vec<String> = (0..WORDS + 5).map(|word| word.to_string()).collect();
        let shown = shown(&long.join(" "));
        assert_eq!(shown.len(), WORDS + 1);
        assert_eq!(shown[0].text, "…");
        assert_eq!(shown[1].text, "5");
        assert_eq!(super::shown("Jeepers, why?").len(), 2);
    }

    #[test]
    fn it_reaches_the_call_window_as_it_reads_it() {
        let told = Heard::default()
            .hear(YOU, Some("Jeepers why"), Instant::now())
            .unwrap();
        assert_eq!(
            serde_json::to_value(told).unwrap(),
            serde_json::json!({
                "identity": "local",
                "words": [
                    { "text": "Jeepers", "wake": true },
                    { "text": "why", "wake": false },
                ],
            })
        );
    }
}
