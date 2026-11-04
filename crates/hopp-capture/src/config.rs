//! Capture preferences, in `capture.json` in the config folder. Kept apart from the main
//! settings so `hopp capture model use` can change them while Hopp is running; they are
//! read afresh each time a capture starts.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Which model transcribes; see `models::MODELS`.
    pub model: String,
    /// Delete captures older than this many days; keep them forever when unset.
    pub keep_days: Option<u32>,
    /// Transcribe on the GPU (the default): faster and less energy, but on Apple silicon
    /// only smooth with a kernel that lets the firmware preempt compute work. Off: the
    /// CPU, using only idle time.
    pub gpu: bool,
    /// Show what people say while they say it (drafts), where the model and device
    /// allow: Parakeet on the GPU.
    pub drafts: bool,
    /// Capture every call from the moment it connects, on by default. Off: only when started
    /// by hand.
    pub auto: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: super::models::DEFAULT.into(),
            keep_days: None,
            gpu: true,
            drafts: true,
            auto: true,
        }
    }
}

fn path() -> Result<PathBuf> {
    Ok(crate::paths::config_dir()?.join("capture.json"))
}

pub fn load() -> Config {
    path()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(config: &Config) -> Result<()> {
    let path = path()?;
    let directory = path.parent().expect("config file has a directory");
    crate::paths::private_dir(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    crate::paths::make_private(file.as_file())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(config)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
