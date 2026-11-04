//! Cleaning up what the model writes before it goes in a transcript.

/// Utterances shorter than this that come back as stock phrases are almost always
/// a model inventing text for noise.
const SHORT_MS: i64 = 1_500;
const STOCK_PHRASES: [&str; 9] = [
    "thank you",
    "thanks for watching",
    "thank you for watching",
    "thank you very much",
    "you",
    "bye",
    "okay",
    "subtitles by the amara org community",
    "please subscribe",
];

/// Lower-case words only, for comparing against stock phrases.
fn bare(text: &str) -> String {
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

/// Drop `[BLANK_AUDIO]`, `(music)`, `*laughs*`, `♪` and the like that Whisper writes
/// for non-speech, and tidy the spacing.
fn strip_annotations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut depth, mut starred) = (0usize, false);
    for character in text.chars() {
        match character {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            '*' => starred = !starred,
            '♪' | '♫' => {}
            _ if depth == 0 && !starred => out.push(character),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What to keep of an utterance `audio_ms` long that the model heard as `text`, if
/// anything.
pub fn clean(text: &str, audio_ms: i64) -> Option<String> {
    let text = strip_annotations(text);
    let words = bare(&text);
    let stock = audio_ms < SHORT_MS && STOCK_PHRASES.contains(&words.as_str());
    (!words.is_empty() && !stock).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_non_speech_annotations() {
        assert_eq!(strip_annotations(" [BLANK_AUDIO] "), "");
        assert_eq!(
            strip_annotations("(music) Okay, let's go ♪"),
            "Okay, let's go"
        );
        assert_eq!(
            strip_annotations("Line 1250 [inaudible] in media.rs"),
            "Line 1250 in media.rs"
        );
        assert_eq!(strip_annotations("*laughs* fine"), "fine");
    }

    #[test]
    fn drops_stock_phrases_only_on_short_audio() {
        assert_eq!(clean("Thank you.", 800), None);
        assert_eq!(clean(" Thank you.", 4_000).as_deref(), Some("Thank you."));
        assert_eq!(
            clean("Thank you for the fix.", 800).as_deref(),
            Some("Thank you for the fix.")
        );
        assert_eq!(clean(" ... ", 4_000), None);
        assert_eq!(clean("[BLANK_AUDIO]", 4_000), None);
    }
}
