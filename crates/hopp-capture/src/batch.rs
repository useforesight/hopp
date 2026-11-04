//! Several utterances to one transcription request, and back.
//!
//! Packing utterances into one request, a second of silence apart, costs less per
//! utterance and is as accurate or better (the model has more context). Whisper always
//! works on a 30-second window, so one short utterance costs as much as a full one.
//! Measured on LibriSpeech (tasks/capture-bench): Parakeet 1.91% word errors at 27×
//! real time batched, against 2.35% at 15× one at a time; whisper.cpp's large-v3-turbo
//! 3.3% at 7.5× against 4.6% at 3×.
//!
//! Parakeet times every word, so each word goes back to the utterance it was spoken
//! in, and one request can hold several people. transcribe.cpp times Whisper by
//! segment only, and a segment can run across two utterances, so with Whisper a
//! request holds one person's speech.

use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{recorder::Speaker, resample, vad::Utterance};

/// Silence between one person's utterances in a request.
pub const GAP_MS: i64 = 1_000;
/// Silence between one person's utterances and the next person's.
const SPEAKER_GAP_MS: i64 = 3_000;
/// Fill a request to about this much (Whisper hears 30 s; keep a margin).
pub const FULL_MS: i64 = 24_000;
/// The most a request may hold.
const LIMIT_MS: i64 = 29_000;
/// Send what's waiting once the oldest utterance has waited this long: how late text
/// may arrive, traded against how often the GPU wakes.
pub const MAX_WAIT: Duration = Duration::from_secs(12);

pub struct Waiting {
    pub speaker: Arc<Speaker>,
    pub utterance: Utterance,
    pub queued: Instant,
    /// The draft line showing it, which its final text replaces.
    pub line: Option<i64>,
}

/// One utterance in a request: where its audio sits, in request milliseconds.
pub struct Part {
    pub speaker: Arc<Speaker>,
    pub utterance: Utterance,
    pub start_ms: i64,
    pub end_ms: i64,
    pub line: Option<i64>,
}

fn span(waiting: &VecDeque<Waiting>) -> i64 {
    waiting
        .iter()
        .map(|item| item.utterance.duration_ms() + GAP_MS)
        .sum::<i64>()
        - GAP_MS
}

/// Whether to send a request now rather than wait for more speech.
///
/// `eager` (Parakeet): at once. Its cost follows the audio's length, about 15× faster
/// than speech even one utterance at a time, so waiting only delays the text; whatever
/// piles up while a request runs goes in the next one. Otherwise (Whisper, which hears
/// 30 s however little is said) once a window is nearly full or the oldest has waited
/// [`MAX_WAIT`].
pub fn due(waiting: &VecDeque<Waiting>, stopping: bool, eager: bool) -> bool {
    !waiting.is_empty()
        && (stopping
            || eager
            || span(waiting) >= FULL_MS
            || waiting
                .front()
                .is_some_and(|first| first.queued.elapsed() >= MAX_WAIT))
}

/// Take utterances from the front that fit in one request, oldest first (at least
/// one; the segmenter caps an utterance at 20 s). With `one_speaker`, only the first
/// person's, up to the next person's turn.
///
/// In the request each person's utterances sit together, in order, and a longer
/// silence separates one person from the next. When a model hears nothing it can write
/// down in an utterance (a short "mm", a cough), it can time the next words early, into
/// that utterance; next to the same person's speech that costs nothing, next to
/// someone else's it gives them the other person's words.
pub fn take(waiting: &mut VecDeque<Waiting>, one_speaker: bool) -> Vec<Part> {
    let mut taken: Vec<Waiting> = Vec::new();
    while let Some(next) = waiting.front() {
        if let Some(first) = taken.first()
            && (length(&taken, Some(next)) > LIMIT_MS
                || (one_speaker && first.speaker.id != next.speaker.id))
        {
            break;
        }
        let Some(item) = waiting.pop_front() else {
            break;
        };
        taken.push(item);
    }
    let mut parts: Vec<Part> = Vec::with_capacity(taken.len());
    for item in grouped(taken) {
        let start_ms = match parts.last() {
            Some(last) if last.speaker.id == item.speaker.id => last.end_ms + GAP_MS,
            Some(last) => last.end_ms + SPEAKER_GAP_MS,
            None => 0,
        };
        parts.push(Part {
            line: item.line,
            end_ms: start_ms + item.utterance.duration_ms(),
            speaker: item.speaker,
            utterance: item.utterance,
            start_ms,
        });
    }
    parts
}

/// Utterances with each person's together, people in the order they first spoke.
fn grouped(mut items: Vec<Waiting>) -> Vec<Waiting> {
    let mut order: Vec<String> = Vec::new();
    for item in &items {
        if !order.contains(&item.speaker.id) {
            order.push(item.speaker.id.clone());
        }
    }
    items.sort_by_key(|item| order.iter().position(|id| *id == item.speaker.id));
    items
}

/// How long a request holding these utterances (and `next`) would be.
fn length(items: &[Waiting], next: Option<&Waiting>) -> i64 {
    let mut speakers: Vec<&str> = items.iter().map(|item| item.speaker.id.as_str()).collect();
    speakers.extend(next.map(|item| item.speaker.id.as_str()));
    let count = speakers.len() as i64;
    speakers.sort_unstable();
    speakers.dedup();
    let people = speakers.len() as i64;
    let audio: i64 = items
        .iter()
        .chain(next)
        .map(|item| item.utterance.duration_ms())
        .sum();
    audio + (count - people) * GAP_MS + (people - 1) * SPEAKER_GAP_MS
}

/// The request's audio: each utterance at 16 kHz, a gap of silence between.
pub fn audio(parts: &[Part]) -> Vec<f32> {
    let mut samples = Vec::new();
    for part in parts {
        // Silence up to where this utterance starts (never cutting the previous one).
        let start = (part.start_ms * 16) as usize;
        if samples.len() < start {
            samples.resize(start, 0.0);
        }
        samples.extend(resample::to_16k(&part.utterance.samples));
    }
    samples
}

/// Give each word to the utterance it was spoken in: the one whose span, widened by
/// half a gap each side, contains it; the nearest otherwise.
///
/// Whole words, never tokens: "It's" is " It" + "'s", and the first token after a
/// pause is often timed early, into the silence before it, so tokens assigned one by
/// one split a word between two people. A word is placed by its last token with a
/// letter or digit in it (trailing punctuation is timed late).
pub fn split(parts: &[Part], pieces: &[(i64, String)]) -> Vec<String> {
    let mut texts = vec![String::new(); parts.len()];
    for (at, word) in words(pieces) {
        let owner = parts
            .iter()
            .position(|part| at >= part.start_ms - GAP_MS / 2 && at < part.end_ms + GAP_MS / 2)
            .unwrap_or_else(|| {
                parts
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, part)| {
                        (part.start_ms - at).abs().min((part.end_ms - at).abs())
                    })
                    .map_or(0, |(index, _)| index)
            });
        if let Some(text_for) = texts.get_mut(owner) {
            text_for.push_str(&word);
        }
    }
    texts
}

/// Tokens joined into words, each with the time that places it.
pub(super) fn words(pieces: &[(i64, String)]) -> Vec<(i64, String)> {
    let mut words: Vec<(i64, String, bool)> = Vec::new();
    for (at, token) in pieces {
        let spoken = token.chars().any(char::is_alphanumeric);
        match words.last_mut() {
            Some((placed_at, word, placed_by_speech)) if continues(word, token) => {
                word.push_str(token);
                if spoken {
                    *placed_at = *at;
                    *placed_by_speech = true;
                } else if !*placed_by_speech {
                    *placed_at = *at;
                }
            }
            _ => words.push((*at, token.clone(), spoken)),
        }
    }
    words.into_iter().map(|(at, word, _)| (at, word)).collect()
}

/// Whether a token carries on the word before it: no space before it, and either
/// punctuation, an apostrophe, or the rest of a word in a script that spaces its words.
/// (Chinese or Japanese tokens have no spaces at all; each stays a word of its own.)
fn continues(word: &str, token: &str) -> bool {
    let Some(first) = token.chars().next() else {
        return true;
    };
    if first.is_whitespace() {
        return false;
    }
    let spaced = |c: char| c.is_ascii_alphanumeric() || ('\u{c0}'..='\u{24f}').contains(&c);
    !first.is_alphanumeric() || (spaced(first) && word.chars().last().is_some_and(spaced))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(seconds: &[i64]) -> VecDeque<Waiting> {
        seconds
            .iter()
            .map(|seconds| Waiting {
                speaker: Arc::new(Speaker {
                    id: "mira".into(),
                    name: "Mira".into(),
                }),
                utterance: Utterance {
                    t_ms: 0,
                    samples: vec![0; *seconds as usize * crate::vad::RATE],
                },
                queued: Instant::now(),
                line: None,
            })
            .collect()
    }

    #[test]
    fn waits_for_a_full_window_or_the_deadline() {
        assert!(!due(&waiting(&[3, 4]), false, false), "7 s: wait for more");
        assert!(
            due(&waiting(&[3, 4]), true, false),
            "stopping sends everything"
        );
        assert!(
            due(&waiting(&[10, 10, 4]), false, false),
            "26 s with gaps is full"
        );
        assert!(!due(&waiting(&[]), true, false));
        let mut old = waiting(&[2]);
        old[0].queued = Instant::now() - MAX_WAIT;
        assert!(due(&old, false, false), "the oldest has waited long enough");
        assert!(due(&waiting(&[2]), false, true), "eager sends at once");
        assert!(!due(&waiting(&[]), false, true));
    }

    #[test]
    fn packs_utterances_a_gap_apart_up_to_the_limit() {
        let mut queue = waiting(&[10, 10, 10]);
        let parts = take(&mut queue, false);
        assert_eq!(parts.len(), 2, "a third would pass 29 s");
        assert_eq!((parts[1].start_ms, parts[1].end_ms), (11_000, 21_000));
        assert_eq!(queue.len(), 1);
        let samples = audio(&parts);
        assert_eq!(samples.len(), 21 * 16_000);
        let mut long = waiting(&[20]);
        assert_eq!(take(&mut long, false).len(), 1, "one utterance always fits");
    }

    #[test]
    fn words_go_back_to_the_utterance_they_were_spoken_in() {
        let parts = take(&mut waiting(&[5, 5]), false);
        let pieces = [
            (800, " Mister".to_owned()),
            (4_900, " Quilter".to_owned()),
            (5_400, ".".to_owned()),
            (6_200, " He".to_owned()),
            (11_900, " laments".to_owned()),
            (40_000, " later".to_owned()),
        ];
        let texts = split(&parts, &pieces);
        assert_eq!(texts[0], " Mister Quilter.");
        assert_eq!(texts[1], " He laments later");
    }

    /// From a real call: a short noise from one person, then another's sentence whose
    /// first token was timed into the first person's slot. "It's" stays whole, with
    /// the person who said it.
    #[test]
    fn a_word_is_never_split_between_two_people() {
        let parts = take(&mut waiting(&[1, 5]), false);
        let pieces = [
            (900, " It".to_owned()),
            (2_150, "'s".to_owned()),
            (2_400, " better".to_owned()),
            (2_900, " than".to_owned()),
            (3_300, " Tuple".to_owned()),
            (3_900, ".".to_owned()),
        ];
        let texts = split(&parts, &pieces);
        assert_eq!(texts[0], "");
        assert_eq!(texts[1], " It's better than Tuple.");
    }

    #[test]
    fn each_persons_utterances_sit_together_a_longer_gap_from_the_next_persons() {
        let mut queue = waiting(&[2, 1, 3, 2]);
        for (item, who) in queue.iter_mut().zip(["alice", "bob", "alice", "bob"]) {
            item.speaker = Arc::new(Speaker {
                id: who.into(),
                name: who.into(),
            });
        }
        let parts = take(&mut queue, false);
        let layout: Vec<_> = parts
            .iter()
            .map(|part| (part.speaker.id.as_str(), part.start_ms, part.end_ms))
            .collect();
        assert_eq!(
            layout,
            [
                ("alice", 0, 2_000),
                ("alice", 3_000, 6_000),
                ("bob", 9_000, 10_000),
                ("bob", 11_000, 13_000),
            ]
        );
    }

    #[test]
    fn without_word_timings_a_request_holds_one_persons_turn() {
        let mut queue = waiting(&[2, 1, 3, 2]);
        for (item, who) in queue.iter_mut().zip(["alice", "alice", "bob", "alice"]) {
            item.speaker = Arc::new(Speaker {
                id: who.into(),
                name: who.into(),
            });
        }
        let parts = take(&mut queue, true);
        assert_eq!(parts.len(), 2, "Alice's first turn only");
        assert!(parts.iter().all(|part| part.speaker.id == "alice"));
        assert_eq!(take(&mut queue, true)[0].speaker.id, "bob");
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn words_are_whole_words() {
        let pieces: Vec<_> = [" Don", "'t", " re", "write", " it", ",", " 好", "的"]
            .iter()
            .enumerate()
            .map(|(index, token)| (index as i64 * 100, (*token).to_owned()))
            .collect();
        let words: Vec<_> = words(&pieces).into_iter().map(|(_, word)| word).collect();
        assert_eq!(words, [" Don't", " rewrite", " it,", " 好", "的"]);
    }
}
