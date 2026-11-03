//! Splitting one person's audio into utterances, so the model gets whole sentences and
//! never hears silence.
//!
//! Speech is anything well above that person's background level, which is the quietest
//! moment of the last two seconds (pauses between words keep it honest during speech,
//! and a fan or hum raises it within two seconds).

use std::collections::VecDeque;

/// Audio arrives at 48 kHz mono in 10 ms frames.
pub const RATE: usize = 48_000;
pub const FRAME: usize = RATE / 100;

/// Audio kept from just before speech starts, so first syllables aren't clipped.
const PREROLL_FRAMES: usize = 20;
/// Silence that ends an utterance.
const HANGOVER_FRAMES: u32 = 70;
/// Silence kept on the end of an utterance.
const TAIL_FRAMES: u32 = 20;
/// Shorter bursts are coughs and clicks.
const MIN_SPEECH_FRAMES: u32 = 25;
/// After this much, a short pause ends the utterance, so text arrives while someone is
/// still talking and the cut falls between words.
const SOFT_FRAMES: usize = 800;
/// The pause that ends an utterance past [`SOFT_FRAMES`].
const SHORT_PAUSE_FRAMES: u32 = 20;
/// No utterance is longer than this. With no pause before it, the cut goes at the
/// quietest moment of the last [`SPLIT_WINDOW_FRAMES`], and the rest carries on as the
/// next utterance: on a real call 15 of 185 lines ended mid-word at a plain 20 s cut.
const MAX_FRAMES: usize = 2_000;
const SPLIT_WINDOW_FRAMES: usize = 200;
/// How far back the background level looks.
const FLOOR_FRAMES: usize = 200;
const ABOVE_FLOOR_DB: f32 = 12.0;
/// Below this nothing counts as speech, however quiet the background.
const QUIETEST_SPEECH_DB: f32 = -50.0;

#[derive(Debug, PartialEq)]
pub struct Utterance {
    /// When it started, in capture time.
    pub t_ms: i64,
    pub samples: Vec<i16>,
}

impl Utterance {
    pub fn duration_ms(&self) -> i64 {
        (self.samples.len() * 1000 / RATE) as i64
    }
}

struct Current {
    t_ms: i64,
    samples: Vec<i16>,
    /// Per frame: its level, whether it was speech, and where its samples end.
    frames: Vec<(f32, bool, usize)>,
    speech_frames: u32,
    silent_run: u32,
}

impl Current {
    fn push(&mut self, frame: &[i16], db: f32, speech: bool) {
        self.samples.extend_from_slice(frame);
        self.frames.push((db, speech, self.samples.len()));
        if speech {
            self.speech_frames += 1;
            self.silent_run = 0;
        } else {
            self.silent_run += 1;
        }
    }

    /// Split after the quietest of the last frames: this keeps the part before, and
    /// returns the rest as an utterance of its own still in progress.
    fn split(&mut self) -> Current {
        let count = self.frames.len();
        let from = count.saturating_sub(SPLIT_WINDOW_FRAMES).max(1);
        let quietest = (from..count)
            .min_by(|&a, &b| self.frames[a].0.total_cmp(&self.frames[b].0))
            .unwrap_or(count - 1);
        let keep = quietest + 1;
        let cut_sample = self.frames[keep - 1].2;
        let mut rest = Current {
            t_ms: self.t_ms + (cut_sample * 1000 / RATE) as i64,
            samples: Vec::with_capacity(RATE * 4),
            frames: Vec::new(),
            speech_frames: 0,
            silent_run: 0,
        };
        let samples = self.samples.split_off(cut_sample);
        let mut start = 0;
        for (db, speech, end) in self.frames.split_off(keep) {
            let end = end - cut_sample;
            rest.push(&samples[start..end], db, speech);
            start = end;
        }
        self.speech_frames -= rest.speech_frames;
        self.silent_run = 0;
        rest
    }
}

#[derive(Default)]
pub struct Segmenter {
    levels: VecDeque<f32>,
    preroll: VecDeque<(i64, Vec<i16>)>,
    current: Option<Current>,
}

/// RMS level in dBFS.
pub fn level_db(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return -120.0;
    }
    let sum: f64 = frame.iter().map(|&sample| f64::from(sample).powi(2)).sum();
    let rms = (sum / frame.len() as f64).sqrt();
    if rms < 1.0 {
        -120.0
    } else {
        (20.0 * (rms / 32_768.0).log10()) as f32
    }
}

impl Segmenter {
    fn floor_db(&self) -> f32 {
        self.levels
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min)
            .clamp(-90.0, 0.0)
    }

    /// Add one 10 ms frame; returns an utterance when one has just ended.
    pub fn push(&mut self, t_ms: i64, frame: &[i16]) -> Option<Utterance> {
        let db = level_db(frame);
        let floor = if self.levels.is_empty() {
            -90.0
        } else {
            self.floor_db()
        };
        self.levels.push_back(db);
        if self.levels.len() > FLOOR_FRAMES {
            self.levels.pop_front();
        }
        let speech = db > (floor + ABOVE_FLOOR_DB).max(QUIETEST_SPEECH_DB);
        match &mut self.current {
            None => {
                if speech {
                    let start = self.preroll.front().map_or(t_ms, |(t, _)| *t);
                    let mut current = Current {
                        t_ms: start,
                        samples: Vec::with_capacity(RATE * 4),
                        frames: Vec::new(),
                        speech_frames: 0,
                        silent_run: 0,
                    };
                    for (_, earlier) in self.preroll.drain(..) {
                        current.push(&earlier, level_db(&earlier), false);
                    }
                    current.push(frame, db, true);
                    self.current = Some(current);
                } else {
                    self.preroll.push_back((t_ms, frame.to_vec()));
                    if self.preroll.len() > PREROLL_FRAMES {
                        self.preroll.pop_front();
                    }
                }
                None
            }
            Some(current) => {
                current.push(frame, db, speech);
                let long = current.frames.len() >= SOFT_FRAMES;
                if current.silent_run >= HANGOVER_FRAMES
                    || (long && current.silent_run >= SHORT_PAUSE_FRAMES)
                {
                    self.finish()
                } else if current.frames.len() >= MAX_FRAMES {
                    let rest = current.split();
                    let done = self.finish();
                    self.current = Some(rest);
                    done
                } else {
                    None
                }
            }
        }
    }

    /// End any utterance in progress (the person left, or the capture stopped).
    pub fn finish(&mut self) -> Option<Utterance> {
        let mut current = self.current.take()?;
        if current.speech_frames < MIN_SPEECH_FRAMES {
            return None;
        }
        let trailing = current.silent_run.saturating_sub(TAIL_FRAMES) as usize * FRAME;
        current
            .samples
            .truncate(current.samples.len().saturating_sub(trailing));
        Some(Utterance {
            t_ms: current.t_ms,
            samples: current.samples,
        })
    }

    /// The utterance in progress, once it has enough speech to be one: when it began
    /// and its audio so far.
    pub fn in_progress(&self) -> Option<(i64, &[i16])> {
        self.current
            .as_ref()
            .filter(|current| current.speech_frames >= MIN_SPEECH_FRAMES)
            .map(|current| (current.t_ms, current.samples.as_slice()))
    }

    #[cfg(test)]
    pub fn speaking(&self) -> bool {
        self.current.is_some()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 10 ms of a tone at `db` dBFS (or silence below -100).
    pub fn frame(db: f32, phase: &mut f32) -> Vec<i16> {
        let amplitude = if db < -100.0 {
            0.0
        } else {
            32_768.0 * 10f32.powf(db / 20.0) * std::f32::consts::SQRT_2
        };
        (0..FRAME)
            .map(|_| {
                *phase += 2.0 * std::f32::consts::PI * 300.0 / RATE as f32;
                (amplitude * phase.sin()).clamp(-32_768.0, 32_767.0) as i16
            })
            .collect()
    }

    fn feed(segmenter: &mut Segmenter, pattern: &[(f32, usize)]) -> Vec<Utterance> {
        let (mut t, mut phase, mut out) = (0i64, 0f32, Vec::new());
        for &(db, frames) in pattern {
            for _ in 0..frames {
                if let Some(utterance) = segmenter.push(t, &frame(db, &mut phase)) {
                    out.push(utterance);
                }
                t += 10;
            }
        }
        out
    }

    #[test]
    fn measures_levels() {
        let mut phase = 0.0;
        assert!((level_db(&frame(-20.0, &mut phase)) + 20.0).abs() < 0.5);
        assert_eq!(level_db(&[0; FRAME]), -120.0);
    }

    #[test]
    fn finds_an_utterance_with_preroll_and_a_short_tail() {
        let mut segmenter = Segmenter::default();
        let utterances = feed(&mut segmenter, &[(-70.0, 100), (-20.0, 150), (-70.0, 100)]);
        assert_eq!(utterances.len(), 1);
        let utterance = &utterances[0];
        assert_eq!(utterance.t_ms, 800, "starts 200 ms before the speech");
        // 200 ms preroll + 1.5 s speech + 200 ms tail.
        assert_eq!(utterance.duration_ms(), 1_900);
        assert!(!segmenter.speaking());
    }

    #[test]
    fn short_pauses_stay_in_one_utterance_and_clicks_are_dropped() {
        let mut segmenter = Segmenter::default();
        let utterances = feed(
            &mut segmenter,
            &[
                (-70.0, 100),
                (-20.0, 60),
                (-70.0, 40),
                (-20.0, 60),
                (-70.0, 100),
                (-20.0, 5),
                (-70.0, 100),
            ],
        );
        assert_eq!(
            utterances.len(),
            1,
            "a 400 ms pause doesn't split; a click is ignored"
        );
    }

    #[test]
    fn long_speech_is_cut_and_steady_noise_is_not_speech() {
        // Words with 150 ms gaps between them, too short to end an utterance, for 27
        // seconds; one word a little quieter than the rest.
        let mut talk = vec![(-70.0, 50)];
        talk.extend([(-20.0, 40), (-60.0, 15)].repeat(34));
        talk.extend([(-20.0, 40), (-75.0, 15)]);
        talk.extend([(-20.0, 40), (-60.0, 15)].repeat(14));
        let mut segmenter = Segmenter::default();
        let utterances = feed(&mut segmenter, &talk);
        assert_eq!(utterances.len(), 1);
        // 200 ms preroll, then 35 words and their gaps: the cut falls in the quietest
        // gap, after the 35th word, not at 20 s.
        let gap_starts = 200 + 35 * 550 - 150;
        let cut_ms = utterances[0].duration_ms();
        assert!(
            (gap_starts + 10..=gap_starts + 150).contains(&cut_ms),
            "{cut_ms} ms"
        );
        assert!(
            segmenter.speaking(),
            "the rest carries on as the next utterance"
        );
        let rest = segmenter.finish().expect("the rest");
        assert_eq!(
            rest.t_ms,
            utterances[0].t_ms + cut_ms,
            "the rest starts where the cut was"
        );
        let mut fan = Segmenter::default();
        // A fan comes on: the first two seconds look like speech, then it is background.
        let noise = feed(&mut fan, &[(-75.0, 100), (-35.0, 1_000)]);
        assert!(noise.len() <= 1);
        assert!(
            !fan.speaking(),
            "background noise doesn't keep an utterance open"
        );
    }

    #[test]
    fn past_eight_seconds_a_short_pause_ends_the_utterance() {
        // 400 ms words with 250 ms pauses: one utterance until 8 s, then the next
        // pause ends it.
        let mut talk = vec![(-70.0, 50)];
        talk.extend([(-20.0, 40), (-70.0, 25)].repeat(20));
        let mut segmenter = Segmenter::default();
        let utterances = feed(&mut segmenter, &talk);
        assert!(!utterances.is_empty());
        let first = utterances[0].duration_ms();
        // The first pause that reaches 200 ms after the 8 s mark (the tail keeps 200 ms).
        assert!((7_600..=8_650).contains(&first), "{first} ms");
        let short = feed(
            &mut Segmenter::default(),
            &[(-70.0, 50), (-20.0, 40), (-70.0, 25), (-20.0, 40)],
        );
        assert!(short.is_empty(), "under 8 s a 250 ms pause doesn't end it");
    }

    #[test]
    fn an_utterance_is_in_progress_once_it_has_enough_speech() {
        let mut segmenter = Segmenter::default();
        feed(&mut segmenter, &[(-70.0, 50), (-20.0, 10)]);
        assert!(segmenter.in_progress().is_none(), "a click so far");
        feed(&mut segmenter, &[(-20.0, 30)]);
        let (t_ms, samples) = segmenter.in_progress().expect("speech");
        assert_eq!(t_ms, 300, "from the start of the preroll");
        assert_eq!(samples.len(), 60 * FRAME, "preroll and speech so far");
    }

    #[test]
    fn finish_flushes_what_is_in_progress() {
        let mut segmenter = Segmenter::default();
        assert!(feed(&mut segmenter, &[(-70.0, 50), (-20.0, 80)]).is_empty());
        assert!(segmenter.speaking());
        assert!(segmenter.finish().is_some());
        assert!(segmenter.finish().is_none());
    }
}
