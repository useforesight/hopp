//! Where a call's frames live and what each one records: `frame-NNN.webp` (at most
//! 1440 px, for agents and the captures window), `frame-NNN.full.webp` (full size) and
//! `frame-NNN.json` ([`FrameInfo`]). Captures from before WebP have `.jpg` and
//! `.full.jpg`, and older still a full-size `.png`; readers take whichever is there. Written by [`super::moments`]; kept apart so readers
//! (the captures window, its screenshot harness) need none of the recorder.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// `~/.local/share/hopp/capture/frames/<id>/`.
pub fn dir(call_id: &str) -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?
        .join("capture/frames")
        .join(call_id))
}

/// A frame's files: `kind` is `full.webp`, `webp` or `json` (see the module for older
/// captures' kinds).
pub fn file(call_id: &str, number: u32, kind: &str) -> Result<PathBuf> {
    Ok(name(&dir(call_id)?, number, kind))
}

pub(crate) fn name(directory: &Path, number: u32, kind: &str) -> PathBuf {
    directory.join(format!("frame-{number:03}.{kind}"))
}

/// A frame's largest copy (`.full.webp`, an older capture's `.full.jpg` or `.png`), or
/// failing those its small one.
pub fn full(call_id: &str, number: u32) -> Result<PathBuf> {
    Ok(full_in(&dir(call_id)?, number))
}

pub(crate) fn full_in(directory: &Path, number: u32) -> PathBuf {
    ["full.webp", "full.jpg", "png"]
        .into_iter()
        .map(|kind| name(directory, number, kind))
        .find(|path| path.exists())
        .unwrap_or_else(|| small_in(directory, number))
}

/// A frame's small copy, at most 1440 px: `.webp`, or an older capture's `.jpg`.
pub fn small(call_id: &str, number: u32) -> Result<PathBuf> {
    Ok(small_in(&dir(call_id)?, number))
}

pub(crate) fn small_in(directory: &Path, number: u32) -> PathBuf {
    let webp = name(directory, number, "webp");
    let jpg = name(directory, number, "jpg");
    if !webp.exists() && jpg.exists() {
        jpg
    } else {
        webp
    }
}

/// The MIME type of a frame file, from its name.
pub fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        _ => "image/jpeg",
    }
}

/// The frame number a frame line's text starts with (`#3 …`).
pub fn number(text: &str) -> Option<u32> {
    let digits: String = text
        .strip_prefix('#')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// Delete a call's frames, if it has any.
pub fn remove(call_id: &str) {
    if let Ok(directory) = dir(call_id) {
        let _ = std::fs::remove_dir_all(directory);
    }
}

/// What a frame's `.json` records.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameInfo {
    pub number: u32,
    pub t_ms: i64,
    pub width: u32,
    pub height: u32,
    /// Why it was taken: `marks`, `bookmark` or `screen`.
    pub trigger: String,
    pub caption: String,
    pub marks: Vec<MarkInfo>,
    /// Everything drawn on the frame, theirs and older marks still showing.
    pub ink: Vec<MarkInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkInfo {
    pub by: String,
    /// `circle`, `underline`, `line`, `stroke` or `pin`.
    pub shape: String,
    /// Pins are numbered through the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<u32>,
    /// What was said over a stamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Fractions of the screen from the top left: `[left, top, right, bottom]`.
    pub bounds: [f64; 4],
    pub colour: String,
}

impl MarkInfo {
    pub fn centre(&self) -> (f64, f64) {
        (
            (self.bounds[0] + self.bounds[2]) / 2.,
            (self.bounds[1] + self.bounds[3]) / 2.,
        )
    }
}

/// Read a frame's metadata.
pub fn info(call_id: &str, number: u32) -> Result<FrameInfo> {
    let path = file(call_id, number, "json")?;
    let bytes = std::fs::read(&path).with_context(|| format!("No frame {number} in {call_id}"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Every frame of a call, in order.
pub fn list(call_id: &str) -> Result<Vec<FrameInfo>> {
    let directory = dir(call_id)?;
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Ok(out);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(bytes) = std::fs::read(&path)
            && let Ok(info) = serde_json::from_slice::<FrameInfo>(&bytes)
        {
            out.push(info);
        }
    }
    out.sort_by_key(|info| info.number);
    Ok(out)
}
