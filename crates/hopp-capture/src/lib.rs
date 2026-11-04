//! Capture: a transcript of a call (and, later, recordings of its shared screens),
//! saved only on this computer.
//!
//! Started in Hopp for Linux and shared with the Mac app. Nothing here knows about
//! LiveKit or either client's windows: a client feeds [`Tap`] one track per person.

pub mod batch;
pub mod browse;
pub mod commands;
pub mod config;
pub mod draft;
pub mod follow;
pub mod frame_files;
pub mod helper;
pub mod ink_raster;
pub mod liveness;
pub mod logger;
pub mod models;
pub mod moments;
pub mod paths;
pub mod recorder;
pub mod resample;
pub mod store;
pub mod text;
pub mod timeline;
pub mod vad;
pub mod wav;

use anyhow::Result;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Where the call's audio paths send audio while a capture is running. Shared by the
/// engine (which switches it on and off) and every audio task of a call; when nothing
/// is being captured it costs one atomic load per 10 ms frame.
#[derive(Clone, Default)]
pub struct Tap(Arc<TapState>);

#[derive(Default)]
struct TapState {
    on: AtomicBool,
    feed: Mutex<Option<recorder::Feed>>,
}

impl Tap {
    pub fn set(&self, feed: Option<recorder::Feed>) {
        let on = feed.is_some();
        if let Ok(mut current) = self.0.feed.lock() {
            *current = feed;
        }
        self.0.on.store(on, Ordering::Release);
    }

    /// Called from audio paths: never waits, so a capture can't make a call stutter.
    pub fn audio(&self, speaker: &Arc<recorder::Speaker>, samples: &[i16]) {
        if !self.0.on.load(Ordering::Acquire) {
            return;
        }
        if let Ok(feed) = self.0.feed.try_lock()
            && let Some(feed) = feed.as_ref()
        {
            feed.audio(speaker, samples);
        }
    }

    pub fn feed(&self) -> Option<recorder::Feed> {
        self.0.feed.lock().ok().and_then(|feed| feed.clone())
    }
}

/// Delete captures older than the `keep_days` setting, with their Markdown copies.
pub fn prune() -> Result<usize> {
    if config::load().keep_days.is_none() {
        return Ok(0);
    }
    let path = store::default_path()?;
    if !path.exists() {
        return Ok(0);
    }
    prune_in(&store::Store::open(&path)?)
}

/// [`prune`], for a process that already has the store open.
pub fn prune_in(store: &store::Store) -> Result<usize> {
    let Some(days) = config::load().keep_days else {
        return Ok(0);
    };
    let cutoff = store::now_ms() - i64::from(days) * 24 * 3600 * 1000;
    let mut deleted = 0;
    for id in store.expired(cutoff)? {
        // The files first. Stopping halfway leaves the row, so the next prune finishes the
        // job; the other way round, nothing would ever lead back to these private files.
        match fs::remove_file(markdown_path(&id)?) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                logger::line(format!("Pruning {id}: {error}"));
                continue;
            }
            _ => moments::remove(&id),
        }
        // Another process's prune can get there first.
        match store.delete(&id) {
            Ok(()) => deleted += 1,
            Err(error) => logger::line(format!("Pruning {id}: {error:#}")),
        }
    }
    Ok(deleted)
}

/// Bytes used by captures, with their pictures, and by downloaded models.
pub fn disk_usage() -> (u64, u64) {
    let size =
        |directory: Option<PathBuf>| directory.map_or(0, |directory| folder_size(&directory));
    let data = crate::paths::data_dir().ok();
    (
        size(data.as_ref().map(|data| data.join("capture"))),
        size(data.map(|data| data.join("models"))),
    )
}

fn folder_size(directory: &Path) -> u64 {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            Some(if metadata.is_dir() {
                folder_size(&entry.path())
            } else {
                metadata.len()
            })
        })
        .sum()
}

/// `<data>/capture/<id>.md`, written when a capture finishes.
pub fn markdown_path(call_id: &str) -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?
        .join("capture")
        .join(format!("{call_id}.md")))
}

/// Replace `path` atomically with a file only this user can read.
pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    crate::paths::private_dir(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    crate::paths::make_private(file.as_file())?;
    use std::io::Write;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_counts_what_is_in_its_folders() -> Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("index.db"), [0; 100])?;
        let frames = directory.path().join("frames").join("c_1");
        fs::create_dir_all(&frames)?;
        fs::write(frames.join("frame-001.webp"), [0; 20])?;
        fs::write(frames.join("frame-001.full.webp"), [0; 3])?;
        assert_eq!(folder_size(directory.path()), 123);
        assert_eq!(folder_size(&directory.path().join("nothing here")), 0);
        Ok(())
    }
}
