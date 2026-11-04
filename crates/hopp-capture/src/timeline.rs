//! A captured call as one timeline: what was said, what happened, and the frames, grouped
//! the way a person reads them back.
//!
//! - **Speech** comes as messages (see [`store::messages`]). When the same words reach
//!   two microphones (one person's speakers into the other's mic), the later copy is
//!   dropped and the kept one notes the other mic.
//! - **Drawing frames** taken close together (people drawing back and forth) become one
//!   drawing, shown as its last frame, with every new mark listed once and a crop around
//!   the part of the screen that was drawn on.
//! - **Screen frames** in a run become one burst with a few evenly spaced thumbnails.
//!
//! Pure: the captures window and its tests build it from lines and frame records.

use crate::{
    frame_files::{FrameInfo, MarkInfo},
    store::{Kind, Line},
};

/// Drawing frames this close together (and within [`DRAWING_SPAN_MS`]) are one drawing.
const DRAWING_GAP_MS: i64 = 3_000;
const DRAWING_SPAN_MS: i64 = 10_000;
/// Screen frames this close together (and within [`SCREEN_SPAN_MS`]) are one burst.
const SCREEN_GAP_MS: i64 = 8_000;
const SCREEN_SPAN_MS: i64 = 40_000;
/// Thumbnails shown for one burst.
const BURST_THUMBS: usize = 6;
/// Two people's lines this close in time, with these words, are one voice on two mics.
const ECHO_WINDOW_MS: i64 = 3_000;
const ECHO_SIMILARITY: f64 = 0.6;
/// The shorter copy has at least this share of the longer's letters: a reply that only
/// starts the same way ("I just don't know…, but anyway, cool man") is not an echo.
const ECHO_LENGTHS: f64 = 0.55;

#[derive(Clone, Debug)]
pub enum Row {
    /// A join, a share, control changing hands.
    Event {
        t_ms: i64,
        text: String,
    },
    Bookmark {
        t_ms: i64,
        who: Option<String>,
        text: String,
    },
    /// Audio dropped rather than transcribed.
    Gap {
        t_ms: i64,
        text: String,
    },
    /// One message: a speaker's consecutive lines.
    Said {
        t_ms: i64,
        speaker: String,
        text: String,
        /// Still being transcribed.
        draft: bool,
        /// Someone else's mic picked the same words up; that copy isn't shown.
        also: Option<String>,
    },
    Drawing(Drawing),
    Screen(Burst),
}

impl Row {
    pub fn t_ms(&self) -> i64 {
        match self {
            Row::Event { t_ms, .. }
            | Row::Bookmark { t_ms, .. }
            | Row::Gap { t_ms, .. }
            | Row::Said { t_ms, .. } => *t_ms,
            Row::Drawing(drawing) => drawing.t_ms,
            Row::Screen(burst) => burst.t_ms,
        }
    }

    /// Which filter shows it.
    pub fn filter(&self) -> Filter {
        match self {
            Row::Said { .. } => Filter::Said,
            Row::Drawing(_) => Filter::Drawings,
            Row::Screen(_) => Filter::Screen,
            Row::Event { .. } | Row::Bookmark { .. } | Row::Gap { .. } => Filter::Everything,
        }
    }

    /// Order among rows at the same second: what happened, then what it showed, then
    /// what was said about it.
    fn rank(&self) -> u8 {
        match self {
            Row::Event { .. } | Row::Gap { .. } => 0,
            Row::Bookmark { .. } => 1,
            Row::Screen(_) => 2,
            Row::Drawing(_) => 3,
            Row::Said { .. } => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Everything,
    Said,
    Drawings,
    Screen,
}

#[derive(Clone, Debug)]
pub struct Drawing {
    pub t_ms: i64,
    pub end_ms: i64,
    /// The frame that shows it all: the last one, with every mark still on it.
    pub frame: u32,
    pub frames: usize,
    /// The frame's size in pixels.
    pub width: u32,
    pub height: u32,
    /// Each new mark once, in the order made.
    pub marks: Vec<MarkInfo>,
    /// The part of the frame to show: `[left, top, width, height]` as fractions.
    pub crop: [f64; 4],
}

impl Drawing {
    /// Everyone who drew, in the order they first did.
    pub fn people(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for mark in &self.marks {
            if !out.contains(&mark.by) {
                out.push(mark.by.clone());
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Burst {
    pub t_ms: i64,
    pub end_ms: i64,
    pub count: usize,
    /// `(frame number, t_ms)`, evenly spaced through the burst, last included.
    pub thumbs: Vec<(u32, i64)>,
}

/// Build the timeline from a call's lines (time-ordered, as [`store::sort`] leaves them)
/// and its frame records.
pub fn build(lines: &[Line], frames: &[FrameInfo]) -> Vec<Row> {
    let mut rows = Vec::new();
    let messages = crate::store::messages(lines);
    let echoes = echoes(&messages);
    for (index, line) in messages.iter().enumerate() {
        let speaker = || line.speaker.clone().unwrap_or_else(|| "Someone".into());
        match line.kind {
            Kind::Speech | Kind::Draft => {
                if echoes.dropped.contains(&index) {
                    continue;
                }
                rows.push(Row::Said {
                    t_ms: line.t_ms,
                    speaker: speaker(),
                    text: line.text.trim().to_owned(),
                    draft: line.kind == Kind::Draft,
                    also: echoes
                        .kept
                        .iter()
                        .find(|(kept, _)| *kept == index)
                        .map(|(_, other)| other.clone()),
                });
            }
            Kind::Event => rows.push(Row::Event {
                t_ms: line.t_ms,
                text: line.text.clone(),
            }),
            Kind::Bookmark => rows.push(Row::Bookmark {
                t_ms: line.t_ms,
                who: line.speaker.clone(),
                text: line.text.clone(),
            }),
            Kind::Gap => rows.push(Row::Gap {
                t_ms: line.t_ms,
                text: line.text.clone(),
            }),
            // Frames come from their records below; retracted drafts are never shown.
            Kind::Frame | Kind::Retracted => {}
        }
    }
    rows.extend(drawings(frames).into_iter().map(Row::Drawing));
    rows.extend(bursts(frames).into_iter().map(Row::Screen));
    rows.sort_by_key(|row| (row.t_ms() / 1_000, row.rank(), row.t_ms()));
    rows
}

struct Echoes {
    dropped: Vec<usize>,
    /// `(kept message, whose mic also had it)`.
    kept: Vec<(usize, String)>,
}

/// One voice on two mics: two people's speech within [`ECHO_WINDOW_MS`], nearly the same
/// words and length. The first to say it is the speaker; the other mic heard it a moment
/// later, often garbled. At the same moment, the longer reading is kept.
fn echoes(messages: &[Line]) -> Echoes {
    let mut out = Echoes {
        dropped: Vec::new(),
        kept: Vec::new(),
    };
    let speech = |line: &Line| line.kind == Kind::Speech;
    for (i, a) in messages.iter().enumerate() {
        if !speech(a) || out.dropped.contains(&i) {
            continue;
        }
        for (j, b) in messages.iter().enumerate().skip(i + 1) {
            if b.t_ms - a.t_ms > ECHO_WINDOW_MS {
                break;
            }
            if !speech(b) || b.speaker == a.speaker || out.dropped.contains(&j) {
                continue;
            }
            let (short, long) = {
                let (x, y) = (a.text.chars().count(), b.text.chars().count());
                (x.min(y) as f64, x.max(y).max(1) as f64)
            };
            if short / long < ECHO_LENGTHS || similarity(&a.text, &b.text) < ECHO_SIMILARITY {
                continue;
            }
            let a_first = a.t_ms < b.t_ms || (a.t_ms == b.t_ms && a.text.len() >= b.text.len());
            let (keep, drop) = if a_first { (i, j) } else { (j, i) };
            let other = messages[drop].speaker.clone().unwrap_or_default();
            out.dropped.push(drop);
            out.kept.push((keep, other));
            break;
        }
    }
    out
}

/// How alike two lines read, 0 to 1: shared letter pairs, ignoring case and punctuation
/// (the Sørensen–Dice coefficient over bigrams).
fn similarity(a: &str, b: &str) -> f64 {
    fn pairs(text: &str) -> Vec<(char, char)> {
        let letters: Vec<char> = text
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .flat_map(char::to_lowercase)
            .collect();
        letters.windows(2).map(|w| (w[0], w[1])).collect()
    }
    let (x, mut y) = (pairs(a), pairs(b));
    if x.is_empty() || y.is_empty() {
        return 0.;
    }
    let total = x.len() + y.len();
    let mut shared = 0;
    for pair in &x {
        if let Some(found) = y.iter().position(|other| other == pair) {
            y.swap_remove(found);
            shared += 1;
        }
    }
    2. * shared as f64 / total as f64
}

fn drawings(frames: &[FrameInfo]) -> Vec<Drawing> {
    let mut groups: Vec<Vec<&FrameInfo>> = Vec::new();
    for frame in frames.iter().filter(|frame| frame.trigger != "screen") {
        match groups.last_mut() {
            Some(group)
                if frame.t_ms - group.last().map_or(0, |last| last.t_ms) <= DRAWING_GAP_MS
                    && frame.t_ms - group[0].t_ms <= DRAWING_SPAN_MS =>
            {
                group.push(frame)
            }
            _ => groups.push(vec![frame]),
        }
    }
    groups
        .into_iter()
        .map(|group| {
            let last = group[group.len() - 1];
            let mut marks: Vec<MarkInfo> = Vec::new();
            for frame in &group {
                for mark in &frame.marks {
                    if !marks.iter().any(|seen| same_mark(seen, mark)) {
                        marks.push(mark.clone());
                    }
                }
            }
            Drawing {
                t_ms: group[0].t_ms,
                end_ms: last.t_ms,
                frame: last.number,
                frames: group.len(),
                width: last.width,
                height: last.height,
                crop: crop(&marks, last.width, last.height),
                marks,
            }
        })
        .collect()
}

fn same_mark(a: &MarkInfo, b: &MarkInfo) -> bool {
    let near = a
        .bounds
        .iter()
        .zip(&b.bounds)
        .all(|(x, y)| (x - y).abs() < 0.005);
    a.by == b.by && a.shape == b.shape && a.pin == b.pin && near
}

/// The part of the screen drawn on: padded, about 16:9, at least a third of the width,
/// and inside the frame. The whole frame when nothing was drawn (a bookmark).
pub fn crop(marks: &[MarkInfo], width: u32, height: u32) -> [f64; 4] {
    if marks.is_empty() || width == 0 || height == 0 {
        return [0., 0., 1., 1.];
    }
    let (mut x0, mut y0, mut x1, mut y1) = (1f64, 1f64, 0f64, 0f64);
    for mark in marks {
        let [l, t, r, b] = mark.bounds;
        x0 = x0.min(l.min(r));
        y0 = y0.min(t.min(b));
        x1 = x1.max(l.max(r));
        y1 = y1.max(t.max(b));
    }
    let aspect = f64::from(width) / f64::from(height);
    let pad = 0.05;
    let mut w = (x1 - x0 + pad * 2.).clamp(0.34, 1.);
    let mut h = (y1 - y0 + pad * 2. * aspect).clamp(0.3, 1.);
    // Grow the short side towards 16:9; when the frame can't hold that (marks right
    // across a wide screen), stop at its edge and keep every mark rather than the shape.
    let want = 16. / 9.;
    if w * aspect / h < want {
        w = (h * want / aspect).min(1.);
    } else {
        h = (w * aspect / want).min(1.);
    }
    let left = ((x0 + x1) / 2. - w / 2.).clamp(0., (1. - w).max(0.));
    let top = ((y0 + y1) / 2. - h / 2.).clamp(0., (1. - h).max(0.));
    [left, top, w, h]
}

fn bursts(frames: &[FrameInfo]) -> Vec<Burst> {
    let mut groups: Vec<Vec<&FrameInfo>> = Vec::new();
    for frame in frames.iter().filter(|frame| frame.trigger == "screen") {
        match groups.last_mut() {
            Some(group)
                if frame.t_ms - group.last().map_or(0, |last| last.t_ms) <= SCREEN_GAP_MS
                    && frame.t_ms - group[0].t_ms <= SCREEN_SPAN_MS =>
            {
                group.push(frame)
            }
            _ => groups.push(vec![frame]),
        }
    }
    groups
        .into_iter()
        .map(|group| {
            let step = group.len().div_ceil(BURST_THUMBS).max(1);
            let mut thumbs: Vec<(u32, i64)> = group
                .iter()
                .step_by(step)
                .map(|frame| (frame.number, frame.t_ms))
                .collect();
            let last = group[group.len() - 1];
            if thumbs.last().map(|(number, _)| *number) != Some(last.number) {
                if thumbs.len() >= BURST_THUMBS {
                    thumbs.pop();
                }
                thumbs.push((last.number, last.t_ms));
            }
            Burst {
                t_ms: group[0].t_ms,
                end_ms: last.t_ms,
                count: group.len(),
                thumbs,
            }
        })
        .collect()
}

/// The scrubber across the top of a call: who spoke when, drawings and screen bursts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overview {
    pub duration_ms: i64,
    /// `(speaker, t_ms, end_ms)`.
    pub speech: Vec<(String, i64, i64)>,
    /// `(t_ms, first person who drew)`.
    pub drawings: Vec<(i64, String)>,
    /// `(t_ms, end_ms)`.
    pub screens: Vec<(i64, i64)>,
}

pub fn overview(rows: &[Row], lines: &[Line], duration_ms: i64) -> Overview {
    let mut out = Overview {
        duration_ms: duration_ms.max(1),
        ..Default::default()
    };
    for row in rows {
        match row {
            Row::Drawing(drawing) => out.drawings.push((
                drawing.t_ms,
                drawing
                    .marks
                    .first()
                    .map(|m| m.by.clone())
                    .unwrap_or_default(),
            )),
            Row::Screen(burst) => out.screens.push((burst.t_ms, burst.end_ms)),
            _ => {}
        }
    }
    for line in lines.iter().filter(|line| line.kind == Kind::Speech) {
        let end = crate::store::end_of(line).max(line.t_ms + 400);
        out.speech
            .push((line.speaker.clone().unwrap_or_default(), line.t_ms, end));
        out.duration_ms = out.duration_ms.max(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(id: i64, t_ms: i64, who: &str, text: &str) -> Line {
        Line {
            id,
            t_ms,
            end_ms: Some(t_ms + 1_500),
            kind: Kind::Speech,
            speaker_id: Some(who.to_lowercase()),
            speaker: Some(who.into()),
            text: text.into(),
            joins: Vec::new(),
            rev: id,
        }
    }

    fn mark(by: &str, shape: &str, bounds: [f64; 4]) -> MarkInfo {
        MarkInfo {
            by: by.into(),
            shape: shape.into(),
            pin: None,
            text: None,
            bounds,
            colour: "#0a84ff".into(),
        }
    }

    fn frame(number: u32, t_ms: i64, trigger: &str, marks: Vec<MarkInfo>) -> FrameInfo {
        FrameInfo {
            number,
            t_ms,
            width: 3412,
            height: 1440,
            trigger: trigger.into(),
            caption: String::new(),
            marks,
            ink: Vec::new(),
        }
    }

    /// As on real calls: Sam's mic picked up Alex a moment later, slightly garbled.
    #[test]
    fn the_same_words_on_two_mics_show_once_as_the_first_speakers() {
        let lines = vec![
            said(1, 29_000, "Alex", "It's not like 4"),
            said(2, 30_000, "Sam", "It was not like 4"),
            said(3, 37_000, "Sam", "Oh, I don't know what I mean."),
        ];
        let rows = build(&lines, &[]);
        let shown: Vec<_> = rows
            .iter()
            .map(|row| match row {
                Row::Said {
                    speaker,
                    text,
                    also,
                    ..
                } => (speaker.as_str(), text.as_str(), also.clone()),
                _ => panic!("only speech"),
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("Alex", "It's not like 4", Some("Sam".to_string())),
                ("Sam", "Oh, I don't know what I mean.", None),
            ]
        );
    }

    /// A reply that starts the same way but says more is a reply, not an echo.
    #[test]
    fn a_reply_that_only_starts_the_same_is_kept() {
        let lines = vec![
            said(
                1,
                296_000,
                "Alex",
                "I just don't know what the thing is for",
            ),
            said(
                2,
                297_000,
                "Sam",
                "I just don't know what the thing is, but anyway, cool man. Gotta get some dinner as well.",
            ),
        ];
        assert_eq!(build(&lines, &[]).len(), 2);
    }

    /// Drawing back and forth for half a minute becomes a few drawings, not a frame a
    /// second; each mark is listed once though it shows on every later frame.
    #[test]
    fn drawing_frames_close_together_are_one_drawing() {
        let line = mark("Sam", "line", [0.3, 0.5, 0.4, 0.7]);
        let pin = mark("Sam", "pin", [0.2, 0.4, 0.2, 0.4]);
        let frames = vec![
            frame(3, 17_000, "marks", vec![line.clone()]),
            frame(4, 18_500, "marks", vec![line.clone(), pin.clone()]),
            frame(5, 20_000, "marks", vec![pin.clone()]),
            frame(
                6,
                40_000,
                "marks",
                vec![mark("Alex", "underline", [0.5, 0.6, 0.7, 0.62])],
            ),
        ];
        let rows = build(&[], &frames);
        let drawings: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Drawing(drawing) => Some((drawing.frame, drawing.frames, drawing.marks.len())),
                _ => None,
            })
            .collect();
        assert_eq!(drawings, [(5, 3, 2), (6, 1, 1)]);
    }

    /// A long scroll is a few bursts with at most six thumbnails each, the last frame
    /// always among them.
    #[test]
    fn a_run_of_screen_frames_is_a_burst_with_its_last_frame_shown() {
        let frames: Vec<_> = (0..14)
            .map(|i| frame(42 + i, 95_000 + i64::from(i) * 2_700, "screen", Vec::new()))
            .collect();
        let rows = build(&[], &frames);
        let Row::Screen(burst) = &rows[0] else {
            panic!("a burst")
        };
        assert_eq!(burst.count, 14);
        assert!(burst.thumbs.len() <= BURST_THUMBS);
        assert_eq!(burst.thumbs.last(), Some(&(55, 95_000 + 13 * 2_700)));
    }

    /// The crop holds every mark, keeps a 16:9 shape and stays inside the frame.
    #[test]
    fn the_crop_holds_the_marks_and_stays_inside() {
        let marks = vec![
            mark("Sam", "pin", [0.95, 0.1, 0.95, 0.1]),
            mark("Sam", "line", [0.8, 0.3, 0.9, 0.5]),
        ];
        let [left, top, w, h] = crop(&marks, 3412, 1440);
        assert!(left >= 0. && top >= 0. && left + w <= 1.0001 && top + h <= 1.0001);
        assert!(left <= 0.8 && left + w >= 0.95 && top <= 0.1 && top + h >= 0.5);
        let shape = w * 3412. / (h * 1440.);
        assert!((shape - 16. / 9.).abs() < 0.02, "shape {shape}");
    }

    /// Marks right across a wide screen: the crop keeps them all, inside the frame,
    /// even though it can't be 16:9 (a real call panicked here).
    #[test]
    fn marks_across_the_whole_screen_are_all_kept() {
        let marks = vec![
            mark("Alex", "underline", [0.25, 0.44, 0.64, 0.48]),
            mark("Sam", "line", [0.68, 0.6, 0.83, 0.84]),
            mark("Sam", "stroke", [0.46, 0.05, 0.89, 0.51]),
        ];
        let [left, top, w, h] = crop(&marks, 3412, 1440);
        assert!(left >= 0. && top >= 0. && left + w <= 1.0001 && top + h <= 1.0001);
        assert!(left <= 0.25 && left + w >= 0.89 && top <= 0.05 && top + h >= 0.84);
    }

    /// Something said goes after the frame that shows what was talked about.
    #[test]
    fn rows_at_the_same_second_put_the_picture_first() {
        let lines = vec![said(1, 59_400, "Sam", "Look at this")];
        let frames = vec![frame(
            32,
            59_100,
            "marks",
            vec![mark("Sam", "pin", [0.2, 0.4, 0.2, 0.4])],
        )];
        let rows = build(&lines, &frames);
        assert!(matches!(rows[0], Row::Drawing(_)));
        assert!(matches!(rows[1], Row::Said { .. }));
    }
}
