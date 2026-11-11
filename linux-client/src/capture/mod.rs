//! Capture: a transcript of a call (and, later, recordings of its shared screens),
//! saved only on this computer.

pub mod asking;
pub mod batch;
pub mod commands;
pub mod config;
pub mod draft;
pub mod follow;
pub mod frame_files;
pub mod helper;
pub mod hooks;
pub mod liveness;
pub mod models;
pub mod moments;
pub mod pause;
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
    os::unix::fs::PermissionsExt,
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
    let Some(days) = config::load().keep_days else {
        return Ok(0);
    };
    let path = store::default_path()?;
    if !path.exists() {
        return Ok(0);
    }
    let cutoff = store::now_ms() - i64::from(days) * 24 * 3600 * 1000;
    let ids = store::Store::open(&path)?.prune(cutoff)?;
    for id in &ids {
        let _ = fs::remove_file(markdown_path(id)?);
        moments::remove(id);
    }
    Ok(ids.len())
}

/// Bytes used by captures and by downloaded models.
pub fn disk_usage() -> (u64, u64) {
    let size = |directory: Option<PathBuf>| {
        directory
            .and_then(|directory| fs::read_dir(directory).ok())
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok()?.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum()
    };
    let data = crate::paths::data_dir().ok();
    (
        size(data.as_ref().map(|data| data.join("capture"))),
        size(data.map(|data| data.join("models"))),
    )
}

/// `~/.local/share/hopp/capture/<id>.md`, written when a capture finishes.
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
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    use std::io::Write;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
