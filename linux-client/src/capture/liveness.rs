//! Whether a capture marked as running still has a recorder behind it.
//!
//! A call row stays "capturing" until its recorder writes an end. If the recorder never
//! gets there (a crash, a thread that died, Hopp killed), the row stays live forever
//! and `hopp capture follow` waits on it for good. So each recorder holds an exclusive
//! lock on `capture/<id>.live` while it runs. The kernel releases a lock when its file
//! closes, however the recorder ends. A live row nobody holds is an orphan: Hopp closes
//! it at start-up, and `follow` and `list` close it when they meet it.

use anyhow::{Context, Result};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

use super::store::{Call, Kind, NewLine, Store};

/// A call this new is never an orphan: its recorder takes the lock before it writes
/// the row, but another process may read the row first.
const GRACE_MS: i64 = 30_000;

fn path(call_id: &str) -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?
        .join("capture")
        .join(format!("{call_id}.live")))
}

/// Held by a running recorder; dropping it says the recorder has stopped.
pub struct Claim {
    path: PathBuf,
    _file: File,
}

impl Drop for Claim {
    fn drop(&mut self) {
        // Remove the file while still holding the lock, so nobody sees it unlocked.
        let _ = fs::remove_file(&self.path);
    }
}

/// Claim `call_id` for this recorder.
pub fn claim(call_id: &str) -> Result<Claim> {
    claim_at(path(call_id)?)
}

fn claim_at(path: PathBuf) -> Result<Claim> {
    if let Some(directory) = path.parent() {
        crate::paths::private_dir(directory)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("Creating {}", path.display()))?;
    file.try_lock()
        .map_err(|error| anyhow::anyhow!("{} is already claimed: {error}", path.display()))?;
    Ok(Claim { path, _file: file })
}

/// Whether a recorder holds `call_id`.
pub fn held(call_id: &str) -> bool {
    path(call_id).is_ok_and(|path| held_at(&path))
}

fn held_at(path: &std::path::Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    // Taking a shared lock fails only while the recorder holds its exclusive one.
    matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock))
}

/// A live call with no recorder behind it.
pub fn orphaned(call: &Call) -> bool {
    call.live() && super::store::now_ms() - call.started_ms > GRACE_MS && !held(&call.id)
}

/// Write the end an orphaned capture never wrote, and its Markdown copy.
pub fn close(store: &Store, call: &Call) -> Result<()> {
    let call = end(store, call)?;
    let markdown = store.markdown(&call)?;
    super::write_private(&super::markdown_path(&call.id)?, markdown.as_bytes())?;
    crate::logger::line(format!(
        "Capture {}: closed; its recorder had stopped without ending it",
        call.id
    ));
    Ok(())
}

/// The database half of [`close`]: a line saying so, then the end.
pub(super) fn end(store: &Store, call: &Call) -> Result<Call> {
    let ended_ms = super::store::now_ms();
    let last_t_ms = store
        .tail(&call.id, 1)?
        .first()
        .map_or(0, |line| line.end_ms.unwrap_or(line.t_ms));
    store.add_line(
        &call.id,
        &NewLine {
            t_ms: last_t_ms,
            kind: Kind::Event,
            speaker_id: None,
            speaker: None,
            text: "Capture ended unexpectedly; nothing after this was transcribed".into(),
            end_ms: None,
        },
    )?;
    store.end_call(&call.id, ended_ms)?;
    store.require(&call.id)
}

/// Close every orphaned capture. Returns how many there were.
pub fn close_orphans(store: &Store) -> Result<usize> {
    let orphans: Vec<Call> = store.live_calls()?.into_iter().filter(orphaned).collect();
    for call in &orphans {
        close(store, call)?;
    }
    Ok(orphans.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_captures_a_crashed_client_left_open() -> Result<()> {
        let (_directory, store) = crate::capture::store::tests::fixture()?;
        let live = store.live_calls()?;
        assert_eq!(live.len(), 1);
        let call = end(&store, &live[0])?;
        assert!(!call.live());
        assert!(store.live_calls()?.is_empty());
        let last = store.transcript("c_2")?.pop().expect("a line");
        assert!(last.text.starts_with("Capture ended unexpectedly"));
        Ok(())
    }

    #[test]
    fn a_claim_is_held_until_dropped() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("c_1.live");
        assert!(!held_at(&path), "nothing claimed yet");
        let claim = claim_at(path.clone())?;
        assert!(held_at(&path));
        assert!(
            claim_at(path.clone()).is_err(),
            "only one recorder per capture"
        );
        drop(claim);
        assert!(!held_at(&path));
        assert!(!path.exists(), "the file goes with the claim");
        Ok(())
    }

    #[test]
    fn only_old_live_calls_without_a_claim_are_orphans() {
        let call = |started_ms, ended_ms| Call {
            id: "c_test_no_claim_file".into(),
            room: String::new(),
            title: None,
            summary: None,
            started_ms,
            ended_ms,
            participants: Vec::new(),
            model: None,
            lines: 0,
        };
        let now = super::super::store::now_ms();
        assert!(orphaned(&call(now - 60_000, None)));
        assert!(!orphaned(&call(now - 1_000, None)), "too new to tell");
        assert!(!orphaned(&call(now - 60_000, Some(now))), "already ended");
    }
}
