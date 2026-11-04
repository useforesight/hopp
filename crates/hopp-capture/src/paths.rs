//! Where Capture keeps its files.
//!
//! On the Mac everything is under `~/Library/Application Support/com.hopp.app`, beside the
//! app's own state. On Linux the XDG base directories, as Hopp for Linux does.
//! `HOPP_CAPTURE_HOME` replaces both, for tests and scripts that must never touch real
//! captures.

use anyhow::{Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

const OVERRIDE: &str = "HOPP_CAPTURE_HOME";

fn home() -> Result<PathBuf> {
    std::env::home_dir().context("No home directory")
}

fn override_dir() -> Option<PathBuf> {
    std::env::var_os(OVERRIDE)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(target_os = "macos")]
fn app_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library/Application Support/com.hopp.app"))
}

#[cfg(not(target_os = "macos"))]
fn xdg(variable: &str, fallback: &str) -> Result<PathBuf> {
    match std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Some(base) => Ok(base.join("hopp")),
        None => Ok(home()?.join(fallback).join("hopp")),
    }
}

/// Settings and hooks.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(directory) = override_dir() {
        return Ok(directory);
    }
    #[cfg(target_os = "macos")]
    return app_dir();
    #[cfg(not(target_os = "macos"))]
    return xdg("XDG_CONFIG_HOME", ".config");
}

/// Captures and transcription models.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(directory) = override_dir() {
        return Ok(directory);
    }
    #[cfg(target_os = "macos")]
    return app_dir();
    #[cfg(not(target_os = "macos"))]
    return xdg("XDG_DATA_HOME", ".local/share");
}

/// Create `path` and any missing parents, private to this user.
pub fn private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("Creating {}", path.display()))
}

/// Options that create a file only this user can read.
pub fn private_file_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Make an open file readable only by this user.
pub fn make_private(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}
