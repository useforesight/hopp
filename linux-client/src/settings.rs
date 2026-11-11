use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

#[derive(Clone, Deserialize, Serialize)]
pub struct Settings {
    pub server: String,
    pub token: Option<String>,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default)]
    pub camera_device: Option<String>,
    #[serde(default)]
    pub face_size: FaceSize,
    #[serde(default)]
    pub face_layout: FaceLayout,
    /// Unmute as soon as a call connects. Off by default: you join muted.
    #[serde(default)]
    pub start_microphone: bool,
    /// Turn the camera on as soon as a call connects.
    #[serde(default)]
    pub start_camera: bool,
    /// Show you as Away while the desktop is locked, except during a call.
    #[serde(default = "yes")]
    pub auto_away: bool,
    /// Play sound with reactions (the reactions tray's switch).
    #[serde(default = "yes")]
    pub reaction_sounds: bool,
}

fn yes() -> bool {
    true
}

/// Tile size in the call window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Arrangement of the call window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceLayout {
    #[default]
    Column,
    Row,
    Bubbles,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: crate::api::DEFAULT_API_BASE.into(),
            token: None,
            input_device: None,
            output_device: None,
            camera_device: None,
            face_size: FaceSize::default(),
            face_layout: FaceLayout::default(),
            start_microphone: false,
            start_camera: false,
            auto_away: true,
            reaction_sounds: true,
        }
    }
}

fn directory() -> Result<PathBuf> {
    crate::paths::config_dir()
}

pub fn load() -> Result<Settings> {
    let path = directory()?.join("settings.json");
    if !path.exists() {
        return Ok(Settings::default());
    }
    serde_json::from_slice(&fs::read(path)?).context("Reading saved Hopp settings")
}

pub fn save(settings: &Settings) -> Result<()> {
    let dir = directory()?;
    crate::paths::private_dir(&dir)?;
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(settings)?)?;
    file.as_file().sync_all()?;
    file.persist(dir.join("settings.json"))?;
    Ok(())
}
