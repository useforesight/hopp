//! Capture's log lines, under a target the Mac app's log filter lets through.

pub fn line(text: impl Into<String>) {
    log::info!(target: "hopp_core::capture", "{}", text.into());
}
