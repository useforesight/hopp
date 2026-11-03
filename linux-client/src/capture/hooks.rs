//! Hooks: your own programs, run when a capture starts or finishes.
//!
//! Put executables at `~/.config/hopp/hooks/<event>` or in `<event>.d/`. Each gets the
//! event as JSON on stdin, runs in the background, and is stopped after ten minutes.
//! Events: `capture-started`, `capture-complete`.

use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use super::store::{self, Store};

const LIMIT: Duration = Duration::from_secs(600);

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn hooks_in(directory: &Path, event: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory.join(format!("{event}.d")))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| executable(path))
        .collect();
    found.sort();
    let single = directory.join(event);
    if executable(&single) {
        found.insert(0, single);
    }
    found
}

/// What hooks are told about a capture.
pub fn payload(event: &str, call_id: &str) -> Value {
    let call = Store::open_default().and_then(|store| store.require(call_id));
    let mut value = json!({
        "event": event,
        "id": call_id,
        "db": store::default_path().ok(),
    });
    if let Ok(call) = call {
        value["room"] = call.room.clone().into();
        value["title"] = call.heading().into();
        value["participants"] = call.participants.clone().into();
        value["started_ms"] = call.started_ms.into();
        value["ended_ms"] = call.ended_ms.into();
    }
    if event == "capture-complete" {
        value["markdown"] = super::markdown_path(call_id)
            .ok()
            .map(|path| path.display().to_string())
            .into();
    }
    value
}

/// Run every hook for `event` in the background.
pub fn run(event: &'static str, payload: Value) {
    let Ok(directory) = crate::paths::config_dir().map(|config| config.join("hooks")) else {
        return;
    };
    run_in(&directory, event, payload);
}

fn run_in(
    directory: &Path,
    event: &'static str,
    payload: Value,
) -> Vec<std::thread::JoinHandle<()>> {
    hooks_in(directory, event)
        .into_iter()
        .filter_map(|hook| {
            let payload = payload.to_string();
            std::thread::Builder::new()
                .name("hopp-hook".into())
                .spawn(move || run_one(&hook, &payload))
                .ok()
        })
        .collect()
}

fn run_one(hook: &Path, payload: &str) {
    let name = hook
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut child = match Command::new(hook)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            crate::logger::line(format!("Hook {name} could not start: {error}"));
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    // Keep what it prints, briefly, in the log.
    for (stream, label) in [
        (
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
            "out",
        ),
        (
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
            "err",
        ),
    ] {
        if let Some(stream) = stream {
            let name = name.clone();
            let _ = std::thread::Builder::new()
                .name("hopp-hook-log".into())
                .spawn(move || {
                    for line in BufReader::new(stream)
                        .lines()
                        .map_while(Result::ok)
                        .take(20)
                    {
                        crate::logger::line(format!("Hook {name} ({label}): {line}"));
                    }
                });
        }
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    crate::logger::line(format!("Hook {name} exited with {status}"));
                }
                return;
            }
            Ok(None) if started.elapsed() > LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                crate::logger::line(format!("Hook {name} ran for ten minutes and was stopped"));
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(path: &Path, body: &str) -> anyhow::Result<()> {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        Ok(())
    }

    #[test]
    fn runs_the_hook_and_its_directory_in_order_with_json_on_stdin() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let out = directory.path().join("out");
        script(
            &directory.path().join("capture-complete"),
            &format!("cat > {}/single.json", directory.path().display()),
        )?;
        std::fs::create_dir(directory.path().join("capture-complete.d"))?;
        script(
            &directory.path().join("capture-complete.d/10-notes"),
            &format!("cat > {}", out.display()),
        )?;
        std::fs::write(
            directory.path().join("capture-complete.d/README"),
            "not run",
        )?;
        let hooks = hooks_in(directory.path(), "capture-complete");
        assert_eq!(hooks.len(), 2, "non-executables are skipped");
        for handle in run_in(directory.path(), "capture-complete", json!({"id": "c_1"})) {
            handle.join().unwrap();
        }
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&out)?)?;
        assert_eq!(written["id"], "c_1");
        assert!(directory.path().join("single.json").exists());
        assert!(hooks_in(directory.path(), "capture-started").is_empty());
        Ok(())
    }
}
