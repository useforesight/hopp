//! What the captures window shows: every call down the side, and one call as a timeline
//! that stays current as the recorder writes it.
//!
//! Kept apart from any window so its rules have tests. The client's window asks for
//! [`list`], opens a call with [`Reading::open`] and calls [`Reading::poll`] while it's
//! showing. A poll sends only the rows that changed, so a long call stays cheap to follow.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use anyhow::{Result, bail};
use hopp_marks::colours::{Colours, PALETTE, hex};
use serde::Serialize;

use crate::{
    frame_files::{self, FrameInfo, MarkInfo},
    liveness,
    store::{self, Call, Kind, Line, Store},
    timeline::{self, Overview, Row},
};

/// Calls down the side, newest first.
const CALLS_LISTED: usize = 200;

/// A call in the list.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub call: Call,
    /// While searching, who said what matched, with the matches in [brackets].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
}

/// Every call, or those mentioning `query`. A capture whose recorder has gone is closed
/// here, so it doesn't look live for good.
pub fn list(store: &Store, query: &str) -> Result<Vec<Listed>> {
    if let Err(error) = liveness::close_orphans(store) {
        crate::logger::line(format!("Closing abandoned captures: {error:#}"));
    }
    let query = query.trim();
    if query.is_empty() {
        let calls = store.calls(CALLS_LISTED)?;
        return Ok(calls
            .into_iter()
            .map(|call| Listed { call, found: None })
            .collect());
    }
    let mut out: Vec<Listed> = Vec::new();
    for hit in store.search(query, CALLS_LISTED)? {
        if out.iter().any(|listed| listed.call.id == hit.call_id) {
            continue;
        }
        if let Some(call) = store.call(&hit.call_id)? {
            let found = match hit.line.speaker.as_deref().filter(|who| !who.is_empty()) {
                Some(who) => format!("{who}: {}", hit.snippet),
                None => hit.snippet,
            };
            out.push(Listed {
                call,
                found: Some(found),
            });
        }
    }
    Ok(out)
}

/// Delete an ended call, its Markdown copy and its frames.
pub fn delete(store: &Store, call_id: &str) -> Result<()> {
    let call = store.require(call_id)?;
    if call.live() {
        bail!("{} is still being captured; stop it first", call.id);
    }
    store.delete(&call.id)?;
    let _ = std::fs::remove_file(crate::markdown_path(&call.id)?);
    frame_files::remove(&call.id);
    Ok(())
}

/// A call's Markdown copy, written if it isn't there yet.
pub fn markdown_file(store: &Store, call_id: &str) -> Result<PathBuf> {
    let call = store.require(call_id)?;
    let path = crate::markdown_path(&call.id)?;
    if !path.exists() {
        crate::write_private(&path, store.markdown(&call)?.as_bytes())?;
    }
    Ok(path)
}

/// A picture the full-size viewer can step to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Picture {
    pub frame: u32,
    pub title: String,
    pub detail: String,
    pub width: u32,
    pub height: u32,
    pub marks: Vec<MarkInfo>,
}

/// What changed in the call being read.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Update {
    pub call: Call,
    /// The rows from this one on replace those the window has.
    pub from: usize,
    pub rows: Vec<Row>,
    pub overview: Overview,
    /// Everyone's colour by name, as `#rrggbb`.
    pub colours: BTreeMap<String, String>,
    /// Every picture in the call, in timeline order, when they've changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pictures: Option<Vec<Picture>>,
}

/// The call the window is showing.
pub struct Reading {
    call: Call,
    lines: Vec<Line>,
    cursor: i64,
    frames: Vec<FrameInfo>,
    /// Frame lines seen, to know when the frame records need reading again.
    frame_lines: usize,
    rows: Vec<Row>,
    pictures: Vec<Picture>,
}

impl Reading {
    /// Start reading the call `which` names: an id, `last` or `live` (see [`Store::call`]).
    /// The update has all of it.
    pub fn open(store: &Store, which: &str) -> Result<(Self, Update)> {
        let call = store.require(which)?;
        // Lines before frames: a frame's line is written last, so every frame a line names is
        // in the listing, and a frame newer than the lines is listed again when its line comes.
        let changes = store.changes(&call.id, 0)?;
        let mut reading = Self {
            lines: Vec::new(),
            cursor: 0,
            frames: frame_files::list(&call.id)?,
            frame_lines: 0,
            rows: Vec::new(),
            pictures: Vec::new(),
            call,
        };
        merge(&mut reading.lines, &mut reading.cursor, changes);
        reading.frame_lines = frame_lines(&reading.lines);
        let update = reading.update();
        Ok((reading, update))
    }

    pub fn call_id(&self) -> &str {
        &self.call.id
    }

    /// The call has been deleted since it was opened: by the CLI, or by "delete after".
    pub fn is_gone(&self, store: &Store) -> Result<bool> {
        Ok(store.call(&self.call.id)?.is_none())
    }

    /// What's changed since the last update, if anything. Ended calls are polled too: an
    /// agent can give one a title or summary at any time.
    pub fn poll(&mut self, store: &Store) -> Result<Option<Update>> {
        let fresh = store.require(&self.call.id)?;
        let changed = store.changes(&self.call.id, self.cursor)?;
        if changed.is_empty() && fresh == self.call {
            return Ok(None);
        }
        self.call = fresh;
        merge(&mut self.lines, &mut self.cursor, changed);
        let seen = frame_lines(&self.lines);
        if seen != self.frame_lines {
            self.frame_lines = seen;
            self.frames = frame_files::list(&self.call.id)?;
        }
        Ok(Some(self.update()))
    }

    /// A frame's picture. Only frames this call recorded: the window can't name a file.
    pub fn picture_path(&self, number: u32, full: bool) -> Result<PathBuf> {
        if !self.frames.iter().any(|frame| frame.number == number) {
            bail!("No frame {number} in {}", self.call.id);
        }
        if full {
            frame_files::full(&self.call.id, number)
        } else {
            frame_files::small(&self.call.id, number)
        }
    }

    fn update(&mut self) -> Update {
        let rows = timeline::build(&self.lines, &self.frames);
        let from = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(old, new)| old == new)
            .count();
        let duration = self
            .call
            .ended_ms
            .unwrap_or_else(store::now_ms)
            .saturating_sub(self.call.started_ms);
        let overview = timeline::overview(&rows, &self.lines, duration);
        let colours = colours(&self.call, &self.lines, &self.frames);
        let pictures = pictures(&rows, &self.frames);
        let new_pictures =
            (self.rows.is_empty() || pictures != self.pictures).then(|| pictures.clone());
        self.pictures = pictures;
        let update = Update {
            call: self.call.clone(),
            from,
            rows: rows[from..].to_vec(),
            overview,
            colours,
            pictures: new_pictures,
        };
        self.rows = rows;
        update
    }
}

fn frame_lines(lines: &[Line]) -> usize {
    lines.iter().filter(|line| line.kind == Kind::Frame).count()
}

/// Take in lines written since `cursor` (a revision): new ones, and drafts that grew,
/// became final or were retracted, each in place under its id. A line keeps the place
/// it had when its speech began, so nothing already shown moves.
fn merge(lines: &mut Vec<Line>, cursor: &mut i64, changed: Vec<Line>) {
    for line in changed {
        *cursor = (*cursor).max(line.rev);
        let existing = lines.iter().position(|shown| shown.id == line.id);
        match (existing, line.kind) {
            (Some(index), Kind::Retracted) => {
                lines.remove(index);
            }
            (None, Kind::Retracted) => {}
            (Some(index), _) => lines[index] = line,
            (None, _) => lines.push(line),
        }
    }
    store::sort(lines);
}

/// Everyone's colour: the colour their marks were drawn in when the frames say, so a
/// person's words and ink match; then the colour their cursor had, going by the user ids
/// of who spoke; then whatever is free. Nobody shares one until the palette runs out.
fn colours(call: &Call, lines: &[Line], frames: &[FrameInfo]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for frame in frames {
        for mark in frame.marks.iter().chain(&frame.ink) {
            out.insert(mark.by.clone(), mark.colour.clone());
        }
    }
    let spoke = lines
        .iter()
        .filter_map(|line| Some((line.speaker.as_deref()?, line.speaker_id.as_deref()?)));
    let cursors = Colours::for_call(spoke.clone().map(|(_, id)| id));
    let mut taken: BTreeSet<String> = out.values().cloned().collect();
    for (name, id) in spoke {
        if !out.contains_key(name) {
            let colour = cursors.get(id).map(hex).unwrap_or_default();
            if !colour.is_empty() && taken.insert(colour.clone()) {
                out.insert(name.to_owned(), colour);
            }
        }
    }
    let speakers = lines.iter().filter_map(|line| line.speaker.as_deref());
    let names = call.participants.iter().map(String::as_str).chain(speakers);
    for name in names {
        if !out.contains_key(name) {
            let free = PALETTE.iter().map(|colour| hex(*colour));
            let colour = free
                .clone()
                .find(|colour| !taken.contains(colour))
                .unwrap_or_else(|| hex(PALETTE[taken.len() % PALETTE.len()]));
            taken.insert(colour.clone());
            out.insert(name.to_owned(), colour);
        }
    }
    out
}

/// Every picture in the call, in timeline order.
fn pictures(rows: &[Row], frames: &[FrameInfo]) -> Vec<Picture> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            Row::Drawing(drawing) => out.push(Picture {
                frame: drawing.frame,
                title: format!("Drawing · {}", store::clock(drawing.t_ms)),
                detail: if drawing.frames > 1 {
                    format!(
                        "{} · drawn over {} frames",
                        summary(&drawing.marks),
                        drawing.frames
                    )
                } else {
                    summary(&drawing.marks)
                },
                width: drawing.width,
                height: drawing.height,
                marks: drawing.marks.clone(),
            }),
            Row::Screen(burst) => {
                let shown = frames.iter().filter(|frame| {
                    frame.trigger == "screen"
                        && frame.t_ms >= burst.t_ms
                        && frame.t_ms <= burst.end_ms
                });
                for frame in shown {
                    out.push(Picture {
                        frame: frame.number,
                        title: format!("Screen · {}", store::clock(frame.t_ms)),
                        detail: format!("Frame {}", frame.number),
                        width: frame.width,
                        height: frame.height,
                        marks: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// "Alex: 2 drawings · Sam: 3 pins, 1 line".
fn summary(marks: &[MarkInfo]) -> String {
    let mut people: Vec<(String, Vec<(String, usize)>)> = Vec::new();
    for mark in marks {
        let shape = match mark.shape.as_str() {
            "stroke" => "drawing",
            other => other,
        }
        .to_owned();
        let person = match people.iter_mut().find(|(who, _)| *who == mark.by) {
            Some(person) => person,
            None => {
                people.push((mark.by.clone(), Vec::new()));
                people.last_mut().expect("just pushed")
            }
        };
        match person.1.iter_mut().find(|(seen, _)| *seen == shape) {
            Some((_, count)) => *count += 1,
            None => person.1.push((shape, 1)),
        }
    }
    people
        .into_iter()
        .map(|(who, shapes)| {
            let shapes: Vec<String> = shapes
                .into_iter()
                .map(|(shape, count)| {
                    format!("{count} {shape}{}", if count == 1 { "" } else { "s" })
                })
                .collect();
            let first_name = who.split_whitespace().next().unwrap_or(&who);
            format!("{first_name}: {}", shapes.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewLine;

    fn line(id: i64, rev: i64, t_ms: i64, kind: Kind, text: &str) -> Line {
        Line {
            id,
            t_ms,
            end_ms: None,
            kind,
            speaker_id: Some("mira".into()),
            speaker: Some("Mira".into()),
            text: text.into(),
            joins: Vec::new(),
            rev,
        }
    }

    fn mark(by: &str, shape: &str, colour: &str) -> MarkInfo {
        MarkInfo {
            by: by.into(),
            shape: shape.into(),
            pin: None,
            text: None,
            bounds: [0.; 4],
            colour: colour.into(),
        }
    }

    fn spoken(at: i64, who: &str, id: &str, kind: Kind, text: &str) -> NewLine {
        NewLine {
            t_ms: at,
            end_ms: Some(at + 1_000),
            kind,
            speaker_id: Some(id.into()),
            speaker: Some(who.into()),
            text: text.into(),
        }
    }

    fn store(directory: &std::path::Path) -> Store {
        Store::open(&directory.join("index.db")).unwrap()
    }

    fn folder(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hopp-browse-{name}-{}-{}",
            std::process::id(),
            store::now_ms()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn rows_after(old: &[Row], update: &Update) -> Vec<Row> {
        let mut rows = old[..update.from].to_vec();
        rows.extend(update.rows.iter().cloned());
        rows
    }

    /// A draft stays where its speech began while it grows and becomes final, and a
    /// retracted one disappears; lines that finish later still go where they were said.
    #[test]
    fn drafts_change_in_place_and_nothing_shown_moves() {
        let (mut lines, mut cursor) = (Vec::new(), 0);
        merge(
            &mut lines,
            &mut cursor,
            vec![
                line(1, 1, 0, Kind::Speech, "First."),
                line(2, 2, 5_000, Kind::Draft, ""),
                line(3, 3, 7_000, Kind::Draft, ""),
            ],
        );
        merge(
            &mut lines,
            &mut cursor,
            vec![
                line(2, 4, 5_000, Kind::Draft, "So the"),
                line(4, 5, 2_000, Kind::Speech, "Said earlier, written late."),
                line(3, 6, 7_000, Kind::Retracted, ""),
                line(2, 7, 5_000, Kind::Speech, "So the cache is warm."),
            ],
        );
        assert_eq!(cursor, 7);
        let shown: Vec<_> = lines
            .iter()
            .map(|line| (line.id, line.kind, line.text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                (1, Kind::Speech, "First."),
                (4, Kind::Speech, "Said earlier, written late."),
                (2, Kind::Speech, "So the cache is warm."),
            ]
        );
    }

    #[test]
    fn marks_are_summed_up_per_person() {
        let marks = [
            mark("Alex Morgan", "stroke", "#0a84ff"),
            mark("Sam Taylor", "pin", "#ff453a"),
            mark("Alex Morgan", "stroke", "#0a84ff"),
            mark("Sam Taylor", "line", "#ff453a"),
            mark("Sam Taylor", "pin", "#ff453a"),
        ];
        assert_eq!(summary(&marks), "Alex: 2 drawings · Sam: 2 pins, 1 line");
    }

    /// While someone talks only the end of the call changes, so that's all that's sent.
    #[test]
    fn a_poll_sends_only_the_rows_that_changed() {
        let directory = folder("poll");
        let store = store(&directory);
        store.begin_call("c_poll", "Room", 1_000, "model").unwrap();
        let earlier = spoken(0, "Mira", "mira", Kind::Speech, "First thing.");
        store.add_line("c_poll", &earlier).unwrap();
        let other = spoken(9_000, "Lee", "lee", Kind::Speech, "Second thing.");
        store.add_line("c_poll", &other).unwrap();
        let draft = spoken(18_000, "Mira", "mira", Kind::Draft, "So the");
        let draft = store.add_line("c_poll", &draft).unwrap();

        let (mut reading, first) = Reading::open(&store, "c_poll").unwrap();
        assert_eq!(first.from, 0);
        assert_eq!(first.rows.len(), 3);
        assert!(reading.poll(&store).unwrap().is_none(), "nothing changed");

        store
            .update_line(draft, Kind::Draft, "So the cache", None)
            .unwrap();
        let grown = reading.poll(&store).unwrap().expect("the draft grew");
        assert_eq!(grown.from, 2, "the earlier rows are left alone");
        assert_eq!(grown.rows.len(), 1);
        let rows = rows_after(&first.rows, &grown);
        assert!(matches!(
            &rows[2],
            Row::Said { text, draft: true, .. } if text == "So the cache"
        ));
        assert!(grown.pictures.is_none(), "no picture changed");

        store
            .update_line(draft, Kind::Speech, "So the cache is warm.", None)
            .unwrap();
        let done = reading.poll(&store).unwrap().expect("the draft finished");
        let rows = rows_after(&rows, &done);
        assert!(matches!(
            &rows[2],
            Row::Said { text, draft: false, .. } if text == "So the cache is warm."
        ));
        assert_eq!(rows.len(), 3);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_call_deleted_elsewhere_is_gone() {
        let directory = folder("gone");
        let store = store(&directory);
        store.begin_call("c_gone", "Room", 1_000, "model").unwrap();
        store.end_call("c_gone", 2_000).unwrap();
        let (reading, _) = Reading::open(&store, "c_gone").unwrap();
        assert!(!reading.is_gone(&store).unwrap());

        delete(&store, "c_gone").unwrap();
        assert!(reading.is_gone(&store).unwrap());

        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Lines written after the call's end, or a title from an agent, still reach the window.
    #[test]
    fn a_call_that_ended_can_still_change() {
        let directory = folder("ended");
        let store = store(&directory);
        store.begin_call("c_ended", "Room", 1_000, "model").unwrap();
        let said = spoken(0, "Mira", "mira", Kind::Speech, "One.");
        store.add_line("c_ended", &said).unwrap();
        let (mut reading, first) = Reading::open(&store, "c_ended").unwrap();
        assert!(first.call.live());

        store.end_call("c_ended", 61_000).unwrap();
        let ended = reading.poll(&store).unwrap().expect("it ended");
        assert!(!ended.call.live());
        assert_eq!(ended.overview.duration_ms, 60_000);
        assert!(reading.poll(&store).unwrap().is_none());

        store
            .annotate("c_ended", Some("Cache review"), None)
            .unwrap();
        let named = reading.poll(&store).unwrap().expect("an agent named it");
        assert_eq!(named.call.title.as_deref(), Some("Cache review"));
        assert!(named.rows.is_empty(), "the timeline hasn't changed");

        std::fs::remove_dir_all(directory).unwrap();
    }

    /// The window can ask for a frame by number and only by number.
    #[test]
    fn only_a_frame_the_call_recorded_has_a_path() {
        let directory = folder("frames");
        let store = store(&directory);
        store
            .begin_call("c_frames", "Room", 1_000, "model")
            .unwrap();
        let (mut reading, _) = Reading::open(&store, "c_frames").unwrap();
        assert!(reading.picture_path(1, false).is_err());
        reading.frames.push(FrameInfo {
            number: 1,
            t_ms: 0,
            width: 16,
            height: 9,
            trigger: "screen".into(),
            caption: String::new(),
            marks: Vec::new(),
            ink: Vec::new(),
        });
        assert!(reading.picture_path(1, false).is_ok());
        assert!(reading.picture_path(2, true).is_err());

        std::fs::remove_dir_all(directory).unwrap();
    }

    fn frame_with(marks: Vec<MarkInfo>) -> FrameInfo {
        FrameInfo {
            number: 1,
            t_ms: 0,
            width: 16,
            height: 9,
            trigger: "marks".into(),
            caption: String::new(),
            marks,
            ink: Vec::new(),
        }
    }

    fn call_with(participants: &[&str]) -> Call {
        Call {
            id: "c".into(),
            room: "Room".into(),
            title: None,
            summary: None,
            started_ms: 0,
            ended_ms: None,
            participants: participants.iter().map(|name| name.to_string()).collect(),
            model: None,
            lines: 0,
        }
    }

    /// Someone's words take the colour of their ink, and whoever hasn't drawn gets a colour
    /// nobody else has.
    #[test]
    fn everyone_gets_their_own_colour_ink_first() {
        let lines = vec![
            line(1, 1, 0, Kind::Speech, "Hello."),
            Line {
                speaker_id: Some("room:r:lee:audio".into()),
                speaker: Some("Lee".into()),
                ..line(2, 2, 1_000, Kind::Speech, "Hi.")
            },
        ];
        let inked = hex(PALETTE[3]);
        let frames = [frame_with(vec![mark("Mira", "stroke", &inked)])];
        let colours = colours(&call_with(&["Mira", "Lee", "Bo"]), &lines, &frames);
        assert_eq!(colours["Mira"], inked);
        let mut all: Vec<&String> = colours.values().collect();
        assert_eq!(all.len(), 3);
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 3, "{colours:?}");
    }

    /// A person's colour here is the one their cursor had, going by their user id, whichever
    /// connection they're named by and whatever order they spoke in.
    #[test]
    fn someone_who_never_drew_has_their_cursors_colour() {
        let lines = vec![Line {
            speaker_id: Some("room:r:lee:audio".into()),
            speaker: Some("Lee".into()),
            ..line(1, 1, 0, Kind::Speech, "Hi.")
        }];
        let colours = colours(&call_with(&["Lee"]), &lines, &[]);
        let cursor = Colours::for_call(["room:r:lee:video"]).get("lee").map(hex);
        assert_eq!(colours.get("Lee"), cursor.as_ref());
    }

    #[test]
    fn more_people_than_colours_still_get_one() {
        let names: Vec<String> = (0..12).map(|n| format!("Person {n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let colours = colours(&call_with(&names), &[], &[]);
        assert_eq!(colours.len(), 12);
        assert!(colours.values().all(|colour| colour.starts_with('#')));
    }
}
