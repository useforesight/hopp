//! Calls and invitations that rang but weren't answered, so none goes unnoticed: the
//! caller gave up, it rang out, it came while you were busy or while Hopp was offline,
//! or another call pushed it aside. As in the Mac app, there is one entry per person,
//! newest first, with how many times they tried. They're kept on disk so a restart
//! doesn't lose them, and go when you call that person back, join a call with them, or
//! dismiss them.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How long a call rings before it counts as missed, as on the Mac. Callers usually give
/// up first; this catches the ones whose cancel never arrives.
pub const RING_TIMEOUT: Duration = Duration::from_secs(70);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missed {
    pub caller_id: String,
    pub name: String,
    /// An invitation to join their call, rather than a call.
    pub invite: bool,
    /// When they last tried, in Unix milliseconds.
    pub at_ms: i64,
    pub count: u32,
}

/// Note another try from `caller_id`: it becomes the newest entry, counting the earlier
/// ones.
pub fn add(list: &mut Vec<Missed>, caller_id: &str, name: &str, invite: bool, at_ms: i64) {
    let previous = list.iter().position(|missed| missed.caller_id == caller_id);
    let (count, at_ms) = match previous.map(|index| list.remove(index)) {
        // The same stale ring, delivered again after a reconnect.
        Some(earlier) if earlier.at_ms == at_ms => (earlier.count, at_ms),
        Some(earlier) => (earlier.count + 1, earlier.at_ms.max(at_ms)),
        None => (1, at_ms),
    };
    list.insert(
        0,
        Missed {
            caller_id: caller_id.to_owned(),
            name: name.to_owned(),
            invite,
            at_ms,
            count,
        },
    );
}

/// Forget `caller_id`'s missed calls. Returns whether there were any.
pub fn remove(list: &mut Vec<Missed>, caller_id: &str) -> bool {
    let before = list.len();
    list.retain(|missed| missed.caller_id != caller_id);
    list.len() != before
}

/// "Missed call · 14:05", "Missed 3 calls · Yesterday 14:05", "Missed invite · 4 Nov".
pub fn describe(missed: &Missed, now_ms: i64) -> String {
    let what = if missed.invite {
        "Missed invite".to_owned()
    } else if missed.count > 1 {
        format!("Missed {} calls", missed.count)
    } else {
        "Missed call".to_owned()
    };
    format!("{what} · {}", when(missed.at_ms, now_ms))
}

fn when(at_ms: i64, now_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    let (Some(at), Some(now)) = (
        Local.timestamp_millis_opt(at_ms).single(),
        Local.timestamp_millis_opt(now_ms).single(),
    ) else {
        return String::new();
    };
    let days = (now.date_naive() - at.date_naive()).num_days();
    match days {
        0 => at.format("%H:%M").to_string(),
        1 => at.format("Yesterday %H:%M").to_string(),
        _ => at.format("%-d %b").to_string(),
    }
}

fn path() -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?.join("missed-calls.json"))
}

/// The saved list; empty if there is none or it can't be read.
pub fn load() -> Vec<Missed> {
    path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(list: &[Missed]) -> Result<()> {
    let path = path()?;
    if let Some(dir) = path.parent() {
        crate::paths::private_dir(dir)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(list)?)
        .with_context(|| format!("Writing {}", temporary.display()))?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_entry_per_person_newest_first_with_a_count() {
        let mut list = Vec::new();
        add(&mut list, "a", "Ada", false, 1_000);
        add(&mut list, "b", "Bo", true, 2_000);
        add(&mut list, "a", "Ada", false, 3_000);
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].caller_id.as_str(), list[0].count), ("a", 2));
        assert_eq!(list[0].at_ms, 3_000);
        // A stale ring reported late doesn't make the entry look older.
        add(&mut list, "a", "Ada", false, 500);
        assert_eq!((list[0].count, list[0].at_ms), (3, 3_000));
        add(&mut list, "a", "Ada", false, 3_000);
        assert_eq!(list[0].count, 3, "the same ring again");
        assert!(remove(&mut list, "a"));
        assert!(!remove(&mut list, "a"));
        assert_eq!(list[0].caller_id, "b");
    }

    #[test]
    fn says_what_was_missed_and_when() {
        use chrono::{Local, TimeZone};
        let now = Local.with_ymd_and_hms(2026, 11, 5, 15, 0, 0).unwrap();
        let at = |day, hour, minute| {
            Local
                .with_ymd_and_hms(2026, 11, day, hour, minute, 0)
                .unwrap()
                .timestamp_millis()
        };
        let missed = |invite, count, at_ms| Missed {
            caller_id: "a".into(),
            name: "Ada".into(),
            invite,
            at_ms,
            count,
        };
        let now = now.timestamp_millis();
        assert_eq!(
            describe(&missed(false, 1, at(5, 14, 5)), now),
            "Missed call · 14:05"
        );
        assert_eq!(
            describe(&missed(false, 3, at(4, 9, 30)), now),
            "Missed 3 calls · Yesterday 09:30"
        );
        assert_eq!(
            describe(&missed(true, 1, at(1, 9, 30)), now),
            "Missed invite · 1 Nov"
        );
    }
}
