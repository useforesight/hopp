//! Drafts: what someone is saying, while they are still saying it.
//!
//! While an utterance is in progress the recorder sends its audio so far to the helper
//! every [`INTERVAL_MS`] of speech, and each reply is a new guess at the whole of it.
//! Guesses differ at their trailing edge, where the model has heard least, so a draft
//! shows only the words that have settled, and never takes a shown word back: people
//! reading live captions find text that changes under them tiring (Google's study of
//! caption stability), so the draft only ever grows. A word settles when two guesses in
//! a row agree on it, or once it ends [`EDGE_MS`] before the audio heard so far. When
//! the speaker pauses, the batch transcription replaces the draft with final text under
//! the same line.

use super::batch;

/// The first guess once this much has been said.
pub const FIRST_MS: i64 = 1_000;
/// A new guess after this much more, early on: the first words need two guesses that
/// agree, and early guesses are cheap.
const EARLY_INTERVAL_MS: i64 = 600;
/// Until this much has been said.
const EARLY_MS: i64 = 2_000;
/// Then a new guess after this much more. Each guess costs about 0.27 s on the M2
/// Max's GPU whatever its length, so this sets what drafts cost while someone talks.
pub const INTERVAL_MS: i64 = 1_500;
/// Words this far from the live edge don't change any more.
const EDGE_MS: i64 = 1_000;

#[derive(Clone, Debug, PartialEq)]
struct Word {
    /// As written, with the space before it.
    text: String,
    /// Lower case, letters, digits and apostrophes only: what two guesses compare.
    key: String,
    /// Where the word starts, in utterance milliseconds.
    at_ms: i64,
}

fn key(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

/// One person's utterance in progress, shown as a line of kind `draft`.
#[derive(Debug)]
pub struct Draft {
    /// The line showing it.
    pub line: i64,
    /// When the utterance began, in capture time: which utterance this draft is for.
    pub t_ms: i64,
    /// How much audio the last guess was asked for, once one was.
    sent_ms: Option<i64>,
    previous: Vec<Word>,
    shown: Vec<Word>,
}

impl Draft {
    pub fn new(line: i64, t_ms: i64) -> Self {
        Self {
            line,
            t_ms,
            sent_ms: None,
            previous: Vec::new(),
            shown: Vec::new(),
        }
    }

    /// Whether `audio_ms` of the utterance is worth a new guess.
    pub fn due(&self, audio_ms: i64) -> bool {
        match self.sent_ms {
            None => audio_ms >= FIRST_MS,
            Some(sent_ms) if sent_ms < EARLY_MS => audio_ms >= sent_ms + EARLY_INTERVAL_MS,
            Some(sent_ms) => audio_ms >= sent_ms + INTERVAL_MS,
        }
    }

    /// How much has been said since the last guess was asked for.
    pub fn waiting_ms(&self, audio_ms: i64) -> i64 {
        audio_ms - self.sent_ms.unwrap_or(0)
    }

    pub fn sent(&mut self, audio_ms: i64) {
        self.sent_ms = Some(audio_ms);
    }

    /// Take a guess at the first `audio_ms` of the utterance (the helper's pieces, in
    /// utterance milliseconds). Returns the text to show when it grew.
    pub fn hear(&mut self, pieces: &[(i64, String)], audio_ms: i64) -> Option<String> {
        let guess: Vec<Word> = batch::words(pieces)
            .into_iter()
            .map(|(at_ms, text)| Word {
                key: key(&text),
                text,
                at_ms,
            })
            .filter(|word| !word.key.is_empty())
            .collect();
        let before = self.shown.len();
        // Grow only from what is shown: if the guess no longer starts with it, wait
        // for a guess that does (the final text will settle it either way).
        let agrees = guess.len() >= before
            && guess
                .iter()
                .zip(&self.shown)
                .all(|(word, shown)| word.key == shown.key);
        if agrees {
            for (index, word) in guess.iter().enumerate().skip(before) {
                let repeated = self
                    .previous
                    .get(index)
                    .is_some_and(|previous| previous.key == word.key);
                let settled = word.at_ms <= audio_ms - EDGE_MS;
                if !(repeated || settled) {
                    break;
                }
                self.shown.push(word.clone());
            }
        }
        self.previous = guess;
        (self.shown.len() > before).then(|| self.text())
    }

    /// The words shown so far.
    pub fn text(&self) -> String {
        self.shown
            .iter()
            .map(|word| word.text.as_str())
            .collect::<String>()
            .trim()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess(words: &[(i64, &str)]) -> Vec<(i64, String)> {
        words
            .iter()
            .map(|(at, text)| (*at, format!(" {text}")))
            .collect()
    }

    #[test]
    fn asks_once_a_second_has_been_said_then_every_interval() {
        let mut draft = Draft::new(1, 0);
        assert!(!draft.due(900));
        assert!(draft.due(FIRST_MS));
        draft.sent(1_000);
        assert!(!draft.due(1_500));
        assert!(draft.due(1_600), "early guesses come quicker");
        draft.sent(2_200);
        assert!(!draft.due(3_600));
        assert!(draft.due(3_700), "then a guess every 1.5 s");
    }

    #[test]
    fn shows_words_once_two_guesses_agree_and_never_takes_them_back() {
        let mut draft = Draft::new(1, 0);
        // The first guess alone settles nothing near the edge.
        assert_eq!(
            draft.hear(&guess(&[(500, "So"), (700, "the"), (900, "cash")]), 1_000),
            None
        );
        // The second agrees on "So the", and hears "cache" where "cash" was.
        assert_eq!(
            draft
                .hear(
                    &guess(&[(500, "So"), (700, "the"), (900, "cache"), (1_400, "is")]),
                    1_600
                )
                .as_deref(),
            Some("So the")
        );
        // A guess that changes a shown word is ignored, though later words agree.
        assert_eq!(
            draft.hear(
                &guess(&[(500, "Sew"), (700, "the"), (900, "cache"), (1_400, "is")]),
                2_200
            ),
            None
        );
        assert_eq!(draft.text(), "So the");
        // Agreement resumes once the guess starts with what is shown.
        assert_eq!(
            draft
                .hear(
                    &guess(&[
                        (500, "So"),
                        (700, "the"),
                        (900, "cache"),
                        (1_400, "is"),
                        (2_500, "warm")
                    ]),
                    2_800
                )
                .as_deref(),
            Some("So the cache is")
        );
    }

    #[test]
    fn words_far_behind_the_edge_show_without_a_second_guess() {
        let mut draft = Draft::new(1, 0);
        let shown = draft.hear(
            &guess(&[(100, "Deploy"), (600, "it"), (2_900, "now")]),
            3_000,
        );
        assert_eq!(
            shown.as_deref(),
            Some("Deploy it"),
            "a second behind the edge"
        );
    }

    #[test]
    fn punctuation_and_case_dont_count_as_disagreeing() {
        let mut draft = Draft::new(1, 0);
        draft.hear(&guess(&[(100, "okay"), (500, "so")]), 1_000);
        let shown = draft.hear(&guess(&[(100, "Okay,"), (500, "so"), (900, "we")]), 1_700);
        assert_eq!(
            shown.as_deref(),
            Some("Okay, so"),
            "the newer spelling is shown"
        );
    }
}
