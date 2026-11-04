//! The capture database: one SQLite file, private to this user, holding every captured
//! call and its transcript. The client is the only writer of transcript lines; the
//! `hopp` command and the MCP server read it, and may set titles and summaries.

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 3;

/// Version 1 to 2: when each line ends.
const MIGRATE_1: &str = "ALTER TABLE lines ADD COLUMN end_ms INTEGER;";
/// Version 2 to 3: a revision for every write, so readers see lines that change
/// (drafts while someone speaks) as well as new ones.
const MIGRATE_2: &str = "
ALTER TABLE lines ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
UPDATE lines SET rev = id;
CREATE INDEX lines_by_rev ON lines(call_id, rev);
";

/// The next revision. Every write to `lines` takes one, inside its own statement, which
/// holds SQLite's write lock, so no two writes share one.
const NEXT_REV: &str = "(SELECT COALESCE(MAX(rev), 0) + 1 FROM lines)";

/// One speaker's lines join into one message across pauses up to this long, unless
/// something else (someone else speaking, an event) comes between them.
pub const MESSAGE_GAP_MS: i64 = 5_000;

const SCHEMA: &str = "
CREATE TABLE calls(
    id TEXT PRIMARY KEY,
    room TEXT NOT NULL,
    title TEXT,
    summary TEXT,
    started_ms INTEGER NOT NULL,
    ended_ms INTEGER,
    participants TEXT NOT NULL DEFAULT '[]',
    model TEXT
);
CREATE TABLE lines(
    id INTEGER PRIMARY KEY,
    call_id TEXT NOT NULL REFERENCES calls(id) ON DELETE CASCADE,
    t_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    speaker_id TEXT,
    speaker TEXT,
    text TEXT NOT NULL,
    end_ms INTEGER,
    rev INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX lines_by_call ON lines(call_id, id);
CREATE INDEX lines_by_rev ON lines(call_id, rev);
CREATE VIRTUAL TABLE lines_fts USING fts5(
    text, speaker, content='lines', content_rowid='id', tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER lines_ai AFTER INSERT ON lines BEGIN
    INSERT INTO lines_fts(rowid, text, speaker) VALUES (new.id, new.text, new.speaker);
END;
CREATE TRIGGER lines_ad AFTER DELETE ON lines BEGIN
    INSERT INTO lines_fts(lines_fts, rowid, text, speaker) VALUES ('delete', old.id, old.text, old.speaker);
END;
CREATE TRIGGER lines_au AFTER UPDATE ON lines BEGIN
    INSERT INTO lines_fts(lines_fts, rowid, text, speaker) VALUES ('delete', old.id, old.text, old.speaker);
    INSERT INTO lines_fts(rowid, text, speaker) VALUES (new.id, new.text, new.speaker);
END;
";

/// What a transcript line records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Something someone said.
    Speech,
    /// Something that happened: a join, a share, control changing hands.
    Event,
    /// A moment someone marked.
    Bookmark,
    /// A still of the shared screen with everyone's ink on it, numbered through the
    /// call; the text starts `#N` (see `moments`).
    Frame,
    /// Audio that was dropped rather than transcribed.
    Gap,
    /// Speech still being transcribed: its text grows and may change until the line
    /// becomes [`Kind::Speech`] (or [`Kind::Retracted`]) under the same id.
    Draft,
    /// A draft that turned out to hold no speech (a cough, a click). Kept so readers
    /// following drafts see it go; never shown in a transcript.
    Retracted,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Speech => "speech",
            Kind::Event => "event",
            Kind::Bookmark => "bookmark",
            Kind::Frame => "frame",
            Kind::Gap => "gap",
            Kind::Draft => "draft",
            Kind::Retracted => "retracted",
        }
    }

    fn parse(text: &str) -> Kind {
        match text {
            "event" => Kind::Event,
            "bookmark" => Kind::Bookmark,
            "frame" => Kind::Frame,
            "gap" => Kind::Gap,
            "draft" => Kind::Draft,
            "retracted" => Kind::Retracted,
            _ => Kind::Speech,
        }
    }

    /// Part of the transcript: not a draft, and not a retracted one.
    pub fn is_final(self) -> bool {
        !matches!(self, Kind::Draft | Kind::Retracted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// Increases through the whole database; use it to ask for lines after this one.
    pub id: i64,
    /// Milliseconds since the capture started.
    pub t_ms: i64,
    /// When speech ended (lines from before Hopp stored it have none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
    /// In a message ([`messages`]): the ids of the lines it joins, when more than one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joins: Vec<i64>,
    /// When the line was last written; see [`Store::changes`].
    #[serde(default)]
    pub rev: i64,
}

/// A line before it is stored.
#[derive(Clone, Debug)]
pub struct NewLine {
    pub t_ms: i64,
    pub end_ms: Option<i64>,
    pub kind: Kind,
    pub speaker_id: Option<String>,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub id: String,
    pub room: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Unix time in milliseconds.
    pub started_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<i64>,
    pub participants: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Transcript lines, not counting events.
    pub lines: i64,
}

impl Call {
    /// The title an agent or person gave it, or where it happened.
    pub fn heading(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| format!("Call in {}", self.room))
    }

    pub fn live(&self) -> bool {
        self.ended_ms.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub call_id: String,
    pub call: String,
    pub line: Line,
    /// The matching text with matches in [brackets].
    pub snippet: String,
}

pub struct Store {
    connection: Connection,
}

/// `~/.local/share/hopp/capture/index.db`.
pub fn default_path() -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?.join("capture/index.db"))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

/// A readable id that sorts by time: `c_20261014-140212-ab12`.
pub fn new_call_id(started_ms: i64) -> String {
    let local = chrono::DateTime::from_timestamp_millis(started_ms)
        .unwrap_or_default()
        .with_timezone(&chrono::Local);
    let noise = (std::process::id() as i64 ^ started_ms.wrapping_mul(2654435761)) & 0xffff;
    format!("c_{}-{noise:04x}", local.format("%Y%m%d-%H%M%S"))
}

impl Store {
    /// Open the default database, creating it (private to this user) if needed.
    pub fn open_default() -> Result<Self> {
        Self::open(&default_path()?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::paths::private_dir(parent)?;
        }
        // Create the file ourselves so it is 0600 from the start; SQLite's journal
        // files copy its permissions. Only when it doesn't exist yet: closing any
        // descriptor of a file drops every POSIX lock this process holds on it,
        // SQLite's included. Opening and closing the database here while the recorder
        // had it open let another process (`hopp capture follow`) think it was the last
        // user, checkpoint, and delete the WAL the recorder was still writing to: its
        // lines were lost, and its later checkpoints damaged the database.
        if !path.exists() {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(path)
                .map(drop)
                .or_else(|error| match error.kind() {
                    // Another thread or process created it first: fine.
                    std::io::ErrorKind::AlreadyExists => Ok(()),
                    _ => Err(error),
                })
                .with_context(|| format!("Creating {}", path.display()))?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(&format!(
                    "BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
                ))?;
            }
            1 => {
                connection.execute_batch(&format!(
                    "BEGIN IMMEDIATE; {MIGRATE_1} {MIGRATE_2} \
                     PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
                ))?;
            }
            2 => {
                connection.execute_batch(&format!(
                    "BEGIN IMMEDIATE; {MIGRATE_2} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
                ))?;
            }
            SCHEMA_VERSION => {}
            newer => bail!("The capture database is from a newer Hopp (version {newer})"),
        }
        Ok(Self { connection })
    }

    pub fn begin_call(&self, id: &str, room: &str, started_ms: i64, model: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO calls(id, room, started_ms, model) VALUES (?1, ?2, ?3, ?4)",
            params![id, room, started_ms, model],
        )?;
        Ok(())
    }

    pub fn add_line(&self, call_id: &str, line: &NewLine) -> Result<i64> {
        self.connection.execute(
            &format!(
                "INSERT INTO lines(call_id, t_ms, kind, speaker_id, speaker, text, end_ms, rev)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, {NEXT_REV})"
            ),
            params![
                call_id,
                line.t_ms,
                line.kind.as_str(),
                line.speaker_id,
                line.speaker,
                line.text,
                line.end_ms
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Rewrite a line in place, under the same id: a draft's growing text, or the draft
    /// becoming final. `end_ms` stays as it was when `None`.
    pub fn update_line(&self, id: i64, kind: Kind, text: &str, end_ms: Option<i64>) -> Result<()> {
        self.connection.execute(
            &format!(
                "UPDATE lines SET kind = ?2, text = ?3, end_ms = COALESCE(?4, end_ms),
                 rev = {NEXT_REV} WHERE id = ?1"
            ),
            params![id, kind.as_str(), text, end_ms],
        )?;
        Ok(())
    }

    /// Every line of a call written or rewritten after revision `after` (0 for all), in
    /// the order of those writes, drafts included: how live readers keep up.
    pub fn changes(&self, call_id: &str, after: i64) -> Result<Vec<Line>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {LINE_COLUMNS} FROM lines WHERE call_id = ?1 AND rev > ?2 ORDER BY rev"
        ))?;
        let lines = statement
            .query_map(params![call_id, after], line_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(lines)
    }

    /// The id of a call's newest line, or 0.
    pub fn last_line_id(&self, call_id: &str) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(id), 0) FROM lines WHERE call_id = ?1",
            [call_id],
            |row| row.get(0),
        )?)
    }

    /// The latest revision of a call's lines, or 0.
    pub fn latest_rev(&self, call_id: &str) -> Result<i64> {
        Ok(self.connection.query_row(
            "SELECT COALESCE(MAX(rev), 0) FROM lines WHERE call_id = ?1",
            [call_id],
            |row| row.get(0),
        )?)
    }

    pub fn set_participants(&self, call_id: &str, names: &[String]) -> Result<()> {
        self.connection.execute(
            "UPDATE calls SET participants = ?2 WHERE id = ?1",
            params![call_id, serde_json::to_string(names)?],
        )?;
        Ok(())
    }

    pub fn end_call(&self, call_id: &str, ended_ms: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE calls SET ended_ms = ?2 WHERE id = ?1 AND ended_ms IS NULL",
            params![call_id, ended_ms],
        )?;
        Ok(())
    }

    /// Mark captures left open by a client that stopped without finishing them.
    fn call_from_row(row: &rusqlite::Row) -> rusqlite::Result<Call> {
        let participants: String = row.get(6)?;
        Ok(Call {
            id: row.get(0)?,
            room: row.get(1)?,
            title: row.get(2)?,
            summary: row.get(3)?,
            started_ms: row.get(4)?,
            ended_ms: row.get(5)?,
            participants: serde_json::from_str(&participants).unwrap_or_default(),
            model: row.get(7)?,
            lines: row.get(8)?,
        })
    }

    const CALL_COLUMNS: &str =
        "id, room, title, summary, started_ms, ended_ms, participants, model,
        (SELECT COUNT(*) FROM lines WHERE lines.call_id = calls.id AND kind = 'speech')";

    /// Newest first.
    pub fn calls(&self, limit: usize) -> Result<Vec<Call>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {} FROM calls ORDER BY started_ms DESC LIMIT ?1",
            Self::CALL_COLUMNS
        ))?;
        let calls = statement
            .query_map([limit as i64], Self::call_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(calls)
    }

    /// Calls without an end, newest first: being captured, or orphaned (see `liveness`).
    pub fn live_calls(&self) -> Result<Vec<Call>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {} FROM calls WHERE ended_ms IS NULL ORDER BY started_ms DESC",
            Self::CALL_COLUMNS
        ))?;
        let calls = statement
            .query_map([], Self::call_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(calls)
    }

    /// A call by id, or `latest` (the newest) or `current` (the one still being captured).
    pub fn call(&self, which: &str) -> Result<Option<Call>> {
        let filter = match which {
            "latest" | "last" => "1 ORDER BY started_ms DESC LIMIT 1",
            "current" | "live" => "ended_ms IS NULL ORDER BY started_ms DESC LIMIT 1",
            _ => "id = ?1",
        };
        let sql = format!("SELECT {} FROM calls WHERE {filter}", Self::CALL_COLUMNS);
        let mut statement = self.connection.prepare(&sql)?;
        let call = if filter.contains("?1") {
            statement
                .query_row([which], Self::call_from_row)
                .optional()?
        } else {
            statement.query_row([], Self::call_from_row).optional()?
        };
        Ok(call)
    }

    /// Like [`Store::call`], but an unknown id is an error.
    pub fn require(&self, which: &str) -> Result<Call> {
        match self.call(which)? {
            Some(call) => Ok(call),
            None if which == "current" || which == "live" => bail!("Nothing is being captured"),
            None if which == "latest" || which == "last" => bail!("No captured calls yet"),
            None => bail!("No captured call '{which}'"),
        }
    }

    /// Lines of a call added after line `after` (0 for all), in the order they were
    /// added, drafts included: transcription finishes out of order, so for reading use
    /// [`Store::transcript`], and to follow drafts [`Store::changes`].
    pub fn lines(&self, call_id: &str, after: i64) -> Result<Vec<Line>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {LINE_COLUMNS} FROM lines WHERE call_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let lines = statement
            .query_map(params![call_id, after], line_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(lines)
    }

    /// Every final line of a call in the order things happened: no drafts.
    pub fn transcript(&self, call_id: &str) -> Result<Vec<Line>> {
        let mut lines = self.lines(call_id, 0)?;
        lines.retain(|line| line.kind.is_final());
        sort(&mut lines);
        Ok(lines)
    }

    /// The last `count` final lines of a call, oldest first.
    pub fn tail(&self, call_id: &str, count: usize) -> Result<Vec<Line>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {LINE_COLUMNS} FROM lines
             WHERE call_id = ?1 AND kind NOT IN ('draft', 'retracted') ORDER BY id DESC LIMIT ?2"
        ))?;
        let mut lines: Vec<Line> = statement
            .query_map(params![call_id, count as i64], line_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        lines.reverse();
        Ok(lines)
    }

    /// Full-text search over every transcript, best matches first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let Some(query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare(
            "SELECT lines.id, lines.t_ms, lines.kind, lines.speaker_id, lines.speaker, lines.text,
                    lines.end_ms, lines.rev, lines.call_id,
                    COALESCE(calls.title, 'Call in ' || calls.room),
                    snippet(lines_fts, 0, '[', ']', '…', 16)
             FROM lines_fts
             JOIN lines ON lines.id = lines_fts.rowid
             JOIN calls ON calls.id = lines.call_id
             WHERE lines_fts MATCH ?1 AND lines.kind NOT IN ('draft', 'retracted')
             ORDER BY bm25(lines_fts), lines.id DESC
             LIMIT ?2",
        )?;
        let hits = statement
            .query_map(params![query, limit as i64], |row| {
                Ok(Hit {
                    line: line_from_row(row)?,
                    call_id: row.get(8)?,
                    call: row.get(9)?,
                    snippet: row.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(hits)
    }

    pub fn annotate(
        &self,
        call_id: &str,
        title: Option<&str>,
        summary: Option<&str>,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE calls SET title = COALESCE(?2, title), summary = COALESCE(?3, summary)
             WHERE id = ?1",
            params![call_id, title, summary],
        )?;
        if changed == 0 {
            bail!("No captured call '{call_id}'");
        }
        Ok(())
    }

    pub fn delete(&self, call_id: &str) -> Result<()> {
        if self
            .connection
            .execute("DELETE FROM calls WHERE id = ?1", [call_id])?
            == 0
        {
            bail!("No captured call '{call_id}'");
        }
        Ok(())
    }

    /// Delete finished captures that started before `cutoff_ms`; returns their ids.
    pub fn prune(&self, cutoff_ms: i64) -> Result<Vec<String>> {
        let ids: Vec<String> = self
            .connection
            .prepare("SELECT id FROM calls WHERE ended_ms IS NOT NULL AND started_ms < ?1")?
            .query_map([cutoff_ms], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &ids {
            self.delete(id)?;
        }
        Ok(ids)
    }

    /// The whole call as Markdown, for people and agents.
    pub fn markdown(&self, call: &Call) -> Result<String> {
        let mut out = format!("# {}\n\n", call.heading());
        let mut meta = vec![local_time(call.started_ms, "%a %-d %b %Y, %H:%M")];
        if let Some(ended) = call.ended_ms {
            meta.push(duration(ended - call.started_ms));
        } else {
            meta.push("still capturing".into());
        }
        if !call.participants.is_empty() {
            meta.push(format!("with {}", call.participants.join(", ")));
        }
        meta.push(format!("room {}", call.room));
        out.push_str(&meta.join(" · "));
        out.push_str(&format!("\nCapture id: `{}`\n\n", call.id));
        if let Some(summary) = &call.summary {
            for line in summary.lines() {
                out.push_str(&format!("> {line}\n"));
            }
            out.push('\n');
        }
        for line in messages(&self.transcript(&call.id)?) {
            out.push_str(&format_line(&line));
            out.push('\n');
        }
        Ok(out)
    }
}

/// The columns [`line_from_row`] reads, in its order.
const LINE_COLUMNS: &str = "id, t_ms, kind, speaker_id, speaker, text, end_ms, rev";

fn line_from_row(row: &rusqlite::Row) -> rusqlite::Result<Line> {
    let kind: String = row.get(2)?;
    Ok(Line {
        id: row.get(0)?,
        t_ms: row.get(1)?,
        kind: Kind::parse(&kind),
        speaker_id: row.get(3)?,
        speaker: row.get(4)?,
        text: row.get(5)?,
        end_ms: row.get(6)?,
        joins: Vec::new(),
        rev: row.get(7)?,
    })
}

/// Into the order things happened: by start, then by when they were written.
pub fn sort(lines: &mut [Line]) {
    lines.sort_by_key(|line| (line.t_ms, line.id));
}

/// When a line's speech ended, or a guess from its length for lines stored before
/// Hopp kept end times (about 2.5 words a second).
pub fn end_of(line: &Line) -> i64 {
    line.end_ms.unwrap_or_else(|| {
        let words = line.text.split_whitespace().count() as i64;
        line.t_ms + (words * 400).max(1_000)
    })
}

/// The longest reaction, such as "Yeah" or "Mm-hmm", that may sit out someone else's
/// message rather than split it: on a real call 29 of 185 lines were one word.
const REACTION_WORDS: usize = 3;
const REACTION_MS: i64 = 1_500;

/// A short reaction from someone who isn't speaking: words and time both short.
fn reaction(line: &Line) -> bool {
    line.kind == Kind::Speech
        && line.speaker_id.is_some()
        && line.text.split_whitespace().count() <= REACTION_WORDS
        && end_of(line) - line.t_ms <= REACTION_MS
}

/// Whether `line` is speech from the same person as `message`.
fn same_person(message: &Line, line: &Line) -> bool {
    line.kind == Kind::Speech
        && message.kind == Kind::Speech
        && line.speaker_id.is_some()
        && message.speaker_id == line.speaker_id
}

/// Whether `line` is the same person's speech as `message`, starting no more than
/// [`MESSAGE_GAP_MS`] after `gap_from`.
fn same_speech(message: &Line, line: &Line, gap_from: i64) -> bool {
    same_person(message, line) && line.t_ms - gap_from <= MESSAGE_GAP_MS
}

fn join(message: &mut Line, line: &Line) {
    if message.joins.is_empty() {
        message.joins.push(message.id);
    }
    message.joins.push(line.id);
    message.end_ms = Some(end_of(message).max(end_of(line)));
    let text = line.text.trim();
    if !text.is_empty() {
        if !message.text.is_empty() {
            message.text.push(' ');
        }
        message.text.push_str(text);
    }
}

/// Join `line` onto the last message if it carries it on, or start a new one.
fn add(out: &mut Vec<Line>, line: &Line) {
    match out.last_mut() {
        Some(message) if same_speech(message, line, end_of(message)) => join(message, line),
        _ => out.push(line.clone()),
    }
}

/// Time-ordered lines with each speaker's consecutive lines joined into one message.
/// Voice activity detection cuts at every pause, so one thought arrives as several
/// lines; they join while nothing else comes between and the pause is at most
/// [`MESSAGE_GAP_MS`]. A short reaction from someone else during a message ("Yeah",
/// "Hmm") doesn't come between: it is shown just after the message. The text is not
/// changed, only joined with spaces.
pub fn messages(sorted: &[Line]) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    // Reactions heard during the last message, shown once it ends.
    let mut aside: Vec<Line> = Vec::new();
    for line in sorted {
        if let Some(message) = out.last()
            && reaction(line)
            && message.kind == Kind::Speech
            && message.speaker_id.is_some()
            && message.speaker_id != line.speaker_id
            && line.t_ms < end_of(message)
        {
            aside.push(line.clone());
            continue;
        }
        let carries_on = out
            .last()
            .is_some_and(|message| same_speech(message, line, end_of(message)));
        if carries_on || aside.is_empty() {
            add(&mut out, line);
            continue;
        }
        // The message is over: its reactions follow it as one message, which the
        // next line joins if it is theirs and follows the message closely.
        let ended = out.last().map_or(line.t_ms, end_of);
        flush(&mut out, &mut aside);
        match out.last_mut() {
            Some(reactions) if same_speech(reactions, line, ended) => join(reactions, line),
            _ => out.push(line.clone()),
        }
    }
    flush(&mut out, &mut aside);
    out
}

/// Reactions held during a message, as one message after it (several people's
/// reactions stay apart).
fn flush(out: &mut Vec<Line>, aside: &mut Vec<Line>) {
    let mut first = true;
    for held in aside.drain(..) {
        match out.last_mut() {
            Some(reactions) if !first && same_person(reactions, &held) => join(reactions, &held),
            _ => add(out, &held),
        }
        first = false;
    }
}

/// Turn what someone typed into an FTS5 query: every word must appear, and the last
/// may be the start of a word. Quoting each word keeps FTS5 syntax out of it.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.replace('"', ""))
        .filter(|word| !word.is_empty())
        .collect();
    let (last, rest) = words.split_last()?;
    let mut terms: Vec<String> = rest.iter().map(|word| format!("\"{word}\"")).collect();
    terms.push(format!("\"{last}\"*"));
    Some(terms.join(" "))
}

/// `[mm:ss]` or `[h:mm:ss]` since the capture started.
pub fn clock(t_ms: i64) -> String {
    let seconds = t_ms.max(0) / 1000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

pub fn duration(ms: i64) -> String {
    let minutes = (ms.max(0) + 30_000) / 60_000;
    if minutes >= 60 {
        format!("{} h {} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

pub fn local_time(ms: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&chrono::Local)
        .format(format)
        .to_string()
}

/// One transcript line as text: `[00:12] Mira: …`.
pub fn format_line(line: &Line) -> String {
    let at = clock(line.t_ms);
    match line.kind {
        Kind::Speech => format!(
            "[{at}] {}: {}",
            line.speaker.as_deref().unwrap_or("Someone"),
            line.text
        ),
        Kind::Event => format!("[{at}] — {}", line.text),
        Kind::Bookmark => match &line.speaker {
            Some(who) => format!("[{at}] Bookmark ({who}): {}", line.text),
            None => format!("[{at}] Bookmark: {}", line.text),
        },
        Kind::Frame => format!("[{at}] Frame {}", line.text),
        Kind::Gap => format!("[{at}] (not transcribed: {})", line.text),
        Kind::Draft => format!(
            "[{at}] {}: {} …",
            line.speaker.as_deref().unwrap_or("Someone"),
            line.text
        ),
        Kind::Retracted => format!("[{at}] (withdrawn: no speech)"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn speech(t_ms: i64, who: &str, text: &str) -> NewLine {
        NewLine {
            end_ms: None,
            t_ms,
            kind: Kind::Speech,
            speaker_id: Some(who.to_lowercase()),
            speaker: Some(who.into()),
            text: text.into(),
        }
    }

    /// Run in a child process by the test below: read the database, then close it,
    /// as `hopp capture show` does.
    #[test]
    #[ignore]
    fn other_process_reads_and_closes() -> Result<()> {
        let Some(path) = std::env::var_os("HOPP_STORE_PROBE") else {
            return Ok(());
        };
        let store = Store::open(Path::new(&path))?;
        store.calls(10)?;
        Ok(())
    }

    /// Opening the store again in a process that already has it open must not drop
    /// that process's SQLite locks: if it does, another process closing the database
    /// deletes the WAL still in use, and every line written since is lost.
    #[test]
    fn a_second_open_keeps_the_wal_of_the_first() -> Result<()> {
        let (directory, store) = fixture()?;
        let path = directory.path().join("capture/index.db");
        let wal = directory.path().join("capture/index.db-wal");
        store.add_line("c_2", &speech(3_000, "Mira", "Before the second open."))?;
        assert!(wal.exists());
        drop(Store::open(&path)?);
        let status = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "capture::store::tests::other_process_reads_and_closes",
                "--ignored",
                "--quiet",
            ])
            .env("HOPP_STORE_PROBE", &path)
            .stdout(std::process::Stdio::null())
            .status()?;
        assert!(status.success());
        assert!(wal.exists(), "another process deleted the WAL in use");
        store.add_line("c_2", &speech(4_000, "Mira", "After it."))?;
        let reader = Store::open(&path)?;
        assert_eq!(
            reader
                .transcript("c_2")?
                .last()
                .map(|line| line.text.as_str()),
            Some("After it.")
        );
        Ok(())
    }

    /// A draft is one line from first word to final text: it grows under one id, each
    /// write gets a new revision, and only its final form is part of the transcript.
    #[test]
    fn a_draft_grows_in_place_then_becomes_final() -> Result<()> {
        let (_directory, store) = fixture()?;
        let start = store.latest_rev("c_2")?;
        let draft = NewLine {
            kind: Kind::Draft,
            text: String::new(),
            ..speech(5_000, "Mira", "")
        };
        let id = store.add_line("c_2", &draft)?;
        store.update_line(id, Kind::Draft, "So the cache", None)?;
        let changes = store.changes("c_2", start)?;
        assert_eq!(changes.len(), 1, "one line, at its latest");
        assert_eq!((changes[0].id, changes[0].kind), (id, Kind::Draft));
        assert_eq!(changes[0].text, "So the cache");
        assert!(changes[0].rev > start + 1, "each write took a revision");
        assert!(
            store.transcript("c_2")?.iter().all(|line| line.id != id),
            "drafts aren't transcript"
        );
        assert!(store.tail("c_2", 50)?.iter().all(|line| line.id != id));
        let before = store.latest_rev("c_2")?;
        store.update_line(id, Kind::Speech, "So the cache is warm.", Some(7_000))?;
        let last = store.changes("c_2", before)?;
        assert_eq!(last.len(), 1);
        assert_eq!(
            (last[0].id, last[0].kind, last[0].end_ms),
            (id, Kind::Speech, Some(7_000))
        );
        assert!(store.transcript("c_2")?.iter().any(|line| line.id == id));
        assert_eq!(store.search("cache warm", 5)?.len(), 1);
        // A draft that held nothing is retracted, still under its id.
        let noise = store.add_line("c_2", &draft)?;
        store.update_line(noise, Kind::Retracted, "", None)?;
        assert!(store.transcript("c_2")?.iter().all(|line| line.id != noise));
        assert_eq!(
            store.changes("c_2", 0)?.last().map(|line| line.kind),
            Some(Kind::Retracted)
        );
        Ok(())
    }

    #[test]
    fn drafts_in_progress_are_not_found_by_search() -> Result<()> {
        let (_directory, store) = fixture()?;
        let id = store.add_line(
            "c_2",
            &NewLine {
                kind: Kind::Draft,
                ..speech(5_000, "Mira", "")
            },
        )?;
        store.update_line(id, Kind::Draft, "zeppelin", None)?;
        assert!(store.search("zeppelin", 5)?.is_empty());
        Ok(())
    }

    /// A database from before drafts opens, and its lines keep their order for readers
    /// that follow by revision.
    #[test]
    fn upgrades_a_version_2_database() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.db");
        {
            let connection = Connection::open(&path)?;
            let old = SCHEMA
                .replace(",\n    rev INTEGER NOT NULL DEFAULT 0", "")
                .replace("CREATE INDEX lines_by_rev ON lines(call_id, rev);\n", "");
            assert!(!old.contains("rev"), "the version 2 schema");
            connection.execute_batch(&format!(
                "{old} PRAGMA user_version = 2;
                 INSERT INTO calls(id, room, started_ms) VALUES ('c_old', 'Core', 1);
                 INSERT INTO lines(call_id, t_ms, kind, text) VALUES ('c_old', 0, 'event', 'a');
                 INSERT INTO lines(call_id, t_ms, kind, text) VALUES ('c_old', 5, 'event', 'b');"
            ))?;
        }
        let store = Store::open(&path)?;
        let lines = store.changes("c_old", 0)?;
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.id, line.rev))
                .collect::<Vec<_>>(),
            [(1, 1), (2, 2)]
        );
        let id = store.add_line("c_old", &speech(9, "Mira", "New."))?;
        assert_eq!(store.changes("c_old", 2)?[0].id, id);
        drop(store);
        // Opening again doesn't migrate again.
        assert_eq!(Store::open(&path)?.latest_rev("c_old")?, 3);
        Ok(())
    }

    pub fn fixture() -> Result<(tempfile::TempDir, Store)> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(&directory.path().join("capture/index.db"))?;
        store.begin_call("c_1", "Core", 1_000, "base.en")?;
        store.add_line(
            "c_1",
            &speech(1_000, "Mira", "The flood starts on a profile switch."),
        )?;
        store.add_line(
            "c_1",
            &NewLine {
                t_ms: 2_000,
                end_ms: None,
                kind: Kind::Event,
                speaker_id: None,
                speaker: None,
                text: "Mira started sharing DP-1".into(),
            },
        )?;
        store.add_line(
            "c_1",
            &speech(3_000, "You", "What does cpal do after an xrun?"),
        )?;
        store.set_participants("c_1", &["Mira".into()])?;
        store.end_call("c_1", 61_000)?;
        store.begin_call("c_2", "Core", 100_000, "base.en")?;
        store.add_line("c_2", &speech(500, "Jonas", "TURN still needs port 3478."))?;
        Ok((directory, store))
    }

    #[test]
    fn creates_a_private_database_and_reopens_it() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let (directory, store) = fixture()?;
        drop(store);
        let path = directory.path().join("capture/index.db");
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())?
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let store = Store::open(&path)?;
        assert_eq!(store.calls(10)?.len(), 2);
        Ok(())
    }

    #[test]
    fn finds_calls_by_id_latest_and_current() -> Result<()> {
        let (_directory, store) = fixture()?;
        assert_eq!(store.require("latest")?.id, "c_2");
        assert_eq!(store.require("current")?.id, "c_2");
        let first = store.require("c_1")?;
        assert_eq!(first.lines, 2, "events are not counted as lines");
        assert_eq!(first.participants, vec!["Mira".to_string()]);
        assert!(!first.live());
        assert!(store.require("c_9").is_err());
        store.end_call("c_2", 200_000)?;
        assert!(store.call("current")?.is_none());
        Ok(())
    }

    #[test]
    fn searches_words_and_prefixes_without_fts_syntax() -> Result<()> {
        let (_directory, store) = fixture()?;
        let hits = store.search("xrun", 10)?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].call_id, "c_1");
        assert!(hits[0].snippet.contains("[xrun]"));
        assert_eq!(store.search("prof", 10)?.len(), 1, "last word is a prefix");
        assert_eq!(store.search("cpal xrun", 10)?.len(), 1);
        assert!(
            store.search("cpal 3478", 10)?.is_empty(),
            "every word must match"
        );
        assert!(
            store.search("\") OR (", 10).is_ok(),
            "syntax is never passed through"
        );
        assert!(store.search("   ", 10)?.is_empty());
        Ok(())
    }

    #[test]
    fn lines_after_a_cursor_tail_and_cascade_on_delete() -> Result<()> {
        let (_directory, store) = fixture()?;
        let all = store.lines("c_1", 0)?;
        assert_eq!(all.len(), 3);
        assert_eq!(store.lines("c_1", all[0].id)?.len(), 2);
        assert_eq!(store.tail("c_1", 2)?, all[1..].to_vec());
        store.delete("c_1")?;
        assert!(store.lines("c_1", 0)?.is_empty());
        assert!(
            store.search("xrun", 10)?.is_empty(),
            "search index follows deletes"
        );
        assert!(store.delete("c_1").is_err());
        Ok(())
    }

    #[test]
    fn annotates_and_exports_markdown() -> Result<()> {
        let (_directory, store) = fixture()?;
        store.annotate("c_1", Some("Fix the POLLERR flood"), None)?;
        store.annotate("c_1", None, Some("Latch the first failure."))?;
        let call = store.require("c_1")?;
        assert_eq!(call.heading(), "Fix the POLLERR flood");
        let markdown = store.markdown(&call)?;
        assert!(markdown.starts_with("# Fix the POLLERR flood\n"));
        assert!(markdown.contains("> Latch the first failure.\n"));
        assert!(markdown.contains("[00:01] Mira: The flood starts"));
        assert!(markdown.contains("[00:02] — Mira started sharing DP-1"));
        assert!(markdown.contains("with Mira"));
        assert!(store.annotate("c_9", Some("x"), None).is_err());
        Ok(())
    }

    #[test]
    fn prunes_only_finished_captures_older_than_the_cutoff() -> Result<()> {
        let (_directory, store) = fixture()?;
        assert_eq!(store.prune(50_000)?, vec!["c_1".to_owned()]);
        assert!(store.call("c_1")?.is_none());
        assert!(store.prune(i64::MAX)?.is_empty(), "c_2 is still live");
        Ok(())
    }

    #[test]
    fn formats_times() {
        assert_eq!(clock(61_500), "01:01");
        assert_eq!(clock(3_725_000), "1:02:05");
        assert_eq!(duration(48 * 60_000), "48 min");
        assert_eq!(duration(72 * 60_000), "1 h 12 min");
        assert!(new_call_id(now_ms()).starts_with("c_20"));
    }

    fn line(id: i64, t_ms: i64, end_ms: Option<i64>, who: Option<&str>, text: &str) -> Line {
        Line {
            id,
            t_ms,
            end_ms,
            kind: if who.is_some() {
                Kind::Speech
            } else {
                Kind::Event
            },
            speaker_id: who.map(str::to_lowercase),
            speaker: who.map(str::to_owned),
            text: text.into(),
            joins: Vec::new(),
            rev: id,
        }
    }

    /// The shape of a real call: one explanation cut at every pause, the other person
    /// interjecting, a screen share starting, lines written out of order.
    #[test]
    fn a_speakers_lines_join_into_messages_in_time_order() {
        let mut lines = vec![
            line(
                1,
                59_900,
                Some(61_800),
                Some("Alex"),
                "But it's loaded that big,",
            ),
            line(
                2,
                62_300,
                Some(65_400),
                Some("Alex"),
                "whisper model. It's running on the GPU.",
            ),
            line(4, 87_000, Some(88_000), Some("Sam"), "That's a good dog."),
            line(
                3,
                66_100,
                Some(86_200),
                Some("Alex"),
                "It's like super low latency",
            ),
            line(
                5,
                88_300,
                Some(91_200),
                Some("Alex"),
                "and it tested it all",
            ),
            line(6, 120_000, Some(121_000), Some("Alex"), "Oh dear."),
            line(7, 121_500, None, None, "Sam started sharing their screen"),
            line(8, 122_000, Some(123_000), Some("Alex"), "Wow."),
        ];
        sort(&mut lines);
        let messages = messages(&lines);
        let shown: Vec<_> = messages
            .iter()
            .map(|message| (message.t_ms, message.text.as_str(), message.joins.clone()))
            .collect();
        assert_eq!(
            shown,
            [
                (
                    59_900,
                    "But it's loaded that big, whisper model. It's running on the GPU. \
                     It's like super low latency",
                    vec![1, 2, 3]
                ),
                (87_000, "That's a good dog.", vec![]),
                (88_300, "and it tested it all", vec![]),
                (120_000, "Oh dear.", vec![]),
                (121_500, "Sam started sharing their screen", vec![]),
                (122_000, "Wow.", vec![]),
            ],
            "split around the other speaker and the event; 29 s of silence ends a message"
        );
        assert_eq!(messages[0].end_ms, Some(86_200));
    }

    /// As on real calls: Lee explains across two lines while Alex says "Hmm" and
    /// "Yeah"; Alex then answers.
    #[test]
    fn short_reactions_dont_split_someone_elses_message() {
        let lines = [
            line(1, 231_553, Some(251_553), Some("Lee"), "New browser stuff"),
            line(2, 237_272, Some(237_982), Some("Alex"), "Hmm"),
            line(3, 248_762, Some(249_552), Some("Alex"), "Yeah"),
            line(
                4,
                251_553,
                Some(256_603),
                Some("Lee"),
                "All monitoring related stuff.",
            ),
            line(
                5,
                258_229,
                Some(264_029),
                Some("Alex"),
                "I mean if they're low risk",
            ),
            line(6, 262_343, Some(263_053), Some("Lee"), "Mm-hmm"),
            line(
                7,
                290_000,
                Some(296_000),
                Some("Lee"),
                "A longer answer than a reaction.",
            ),
        ];
        let shown: Vec<_> = messages(&lines)
            .into_iter()
            .map(|message| (message.speaker.unwrap_or_default(), message.text))
            .collect();
        let shown: Vec<_> = shown
            .iter()
            .map(|(who, text)| (who.as_str(), text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Lee", "New browser stuff All monitoring related stuff."),
                ("Alex", "Hmm Yeah I mean if they're low risk"),
                ("Lee", "Mm-hmm"),
                ("Lee", "A longer answer than a reaction.")
            ]
            .map(|(who, text)| (who, text))
        );
    }

    #[test]
    fn lines_without_end_times_join_by_estimated_length() {
        let lines = [
            line(1, 0, None, Some("Mira"), "One two three four five"),
            line(2, 6_000, None, Some("Mira"), "six"),
            line(3, 20_000, None, Some("Mira"), "seven"),
        ];
        let messages = messages(&lines);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "One two three four five six");
    }

    #[test]
    fn the_transcript_is_in_time_order_and_markdown_shows_messages() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(&directory.path().join("capture/index.db"))?;
        store.begin_call("c_1", "Core", 1_000, "base.en")?;
        for (t_ms, text) in [
            (9_000, "said last"),
            (1_000, "said first,"),
            (2_500, "then this"),
        ] {
            store.add_line(
                "c_1",
                &NewLine {
                    end_ms: Some(t_ms + 1_000),
                    ..speech(t_ms, "Mira", text)
                },
            )?;
        }
        let order: Vec<_> = store
            .transcript("c_1")?
            .into_iter()
            .map(|l| l.t_ms)
            .collect();
        assert_eq!(order, [1_000, 2_500, 9_000]);
        let call = store.require("c_1")?;
        let markdown = store.markdown(&call)?;
        assert!(markdown.contains("[00:01] Mira: said first, then this\n[00:09] Mira: said last"));
        Ok(())
    }

    #[test]
    fn a_version_1_database_gains_end_times() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("index.db");
        {
            let old = Connection::open(&path)?;
            let v1 = SCHEMA
                .replace(
                    ",\n    end_ms INTEGER,\n    rev INTEGER NOT NULL DEFAULT 0\n",
                    "\n",
                )
                .replace("CREATE INDEX lines_by_rev ON lines(call_id, rev);\n", "");
            assert!(
                !v1.contains("end_ms") && !v1.contains("rev"),
                "the version 1 schema"
            );
            old.execute_batch(&format!("{v1} PRAGMA user_version = 1;"))?;
            old.execute(
                "INSERT INTO calls(id, room, started_ms) VALUES ('c_1', 'Core', 0)",
                [],
            )?;
            old.execute(
                "INSERT INTO lines(call_id, t_ms, kind, text) VALUES ('c_1', 5, 'speech', 'hi')",
                [],
            )?;
        }
        let store = Store::open(&path)?;
        let lines = store.transcript("c_1")?;
        assert_eq!((lines[0].text.as_str(), lines[0].end_ms), ("hi", None));
        store.add_line(
            "c_1",
            &NewLine {
                end_ms: Some(900),
                ..speech(10, "Mira", "again")
            },
        )?;
        assert_eq!(store.transcript("c_1")?[1].end_ms, Some(900));
        assert_eq!(
            store.latest_rev("c_1")?,
            2,
            "and revisions, through version 2"
        );
        Ok(())
    }
}
