//! Hooks: your own programs, run when a capture starts or finishes.
//!
//! Put executables at `<config>/hooks/<event>` or in `<event>.d/`, where the config folder is
//! `~/Library/Application Support/com.hopp.app/` on the Mac. Each gets the event as JSON on
//! stdin, runs in the background, and is stopped after ten minutes or 16 MiB of output. Events:
//! `capture-started`, `capture-complete`.
//!
//! What a hook prints goes to `<config>/hooks/logs/<event>/<name>.log`, replaced on each run
//! and readable only by you. It never goes in Hopp's own log: a hook is likely to print what
//! people said, and that log can leave the computer with a bug report.

use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Seek, Write},
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

use crate::store::{self, Store};

/// Where a hook is stopped.
#[derive(Clone, Copy)]
struct Limits {
    time: Duration,
    output_bytes: u64,
}

const LIMITS: Limits = Limits {
    time: Duration::from_secs(600),
    output_bytes: 16 * 1024 * 1024,
};

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> bool {
    false
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
        value["markdown"] = crate::markdown_path(call_id)
            .ok()
            .map(|path| path.display().to_string())
            .into();
    }
    value
}

/// Run every hook for `event`. They have all started when this returns, so they run even if
/// the app is quitting, and are watched in the background.
pub fn run(event: &'static str, payload: Value) {
    let Ok(directory) = crate::paths::config_dir().map(|config| config.join("hooks")) else {
        return;
    };
    run_in(&directory, event, &payload, LIMITS);
}

fn run_in(
    directory: &Path,
    event: &str,
    payload: &Value,
    limits: Limits,
) -> Vec<std::thread::JoinHandle<()>> {
    let logs = directory.join("logs").join(event);
    let payload = payload.to_string();
    hooks_in(directory, event)
        .into_iter()
        .filter_map(|hook| {
            let name = hook.file_name()?.to_string_lossy().into_owned();
            let running = match start(&hook, &name, &logs, &payload) {
                Ok(running) => running,
                Err(error) => {
                    crate::logger::line(format!("Hook {name} could not start: {error:#}"));
                    return None;
                }
            };
            std::thread::Builder::new()
                .name("hopp-hook".into())
                .spawn(move || running.watch(limits))
                .ok()
        })
        .collect()
}

struct Running {
    name: String,
    child: Child,
    output: File,
}

fn start(hook: &Path, name: &str, logs: &Path, payload: &str) -> anyhow::Result<Running> {
    crate::paths::private_dir(logs)?;
    // Appending, so that emptying the file below doesn't leave the hook writing at its old
    // offset, into a hole as long as what it had printed.
    let output = crate::paths::private_file_options()
        .create(true)
        .append(true)
        .open(logs.join(format!("{name}.log")))?;
    crate::paths::make_private(&output)?;
    output.set_len(0)?;
    // A file rather than a pipe, so a hook that never reads can't hold up a capture ending.
    let mut stdin = tempfile::tempfile()?;
    stdin.write_all(payload.as_bytes())?;
    stdin.rewind()?;
    let mut command = Command::new(hook);
    command
        .stdin(stdin)
        .stdout(output.try_clone()?)
        .stderr(output.try_clone()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(Running {
        name: name.into(),
        child: command.spawn()?,
        output,
    })
}

impl Running {
    fn watch(mut self, limits: Limits) {
        let started = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        let name = &self.name;
                        crate::logger::line(format!("Hook {name} exited with {status}"));
                    }
                    return;
                }
                Ok(None) if started.elapsed() > limits.time => {
                    self.stop();
                    let name = &self.name;
                    crate::logger::line(format!("Hook {name} ran too long and was stopped"));
                    return;
                }
                Ok(None) if self.printed() > limits.output_bytes => {
                    self.stop();
                    let _ = self.output.set_len(0);
                    let _ = writeln!(self.output, "Stopped: this hook printed too much.");
                    let name = &self.name;
                    crate::logger::line(format!("Hook {name} printed too much and was stopped"));
                    return;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(_) => return,
            }
        }
    }

    fn printed(&self) -> u64 {
        self.output.metadata().map_or(0, |metadata| metadata.len())
    }

    /// Stop the hook and anything it started. It leads its own process group; until it's
    /// waited for below, its id can't have gone to anything else.
    #[cfg(unix)]
    fn stop(&mut self) {
        // SAFETY: only signals a process group; no memory is involved.
        unsafe { libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL) };
        let _ = self.child.wait();
    }

    #[cfg(not(unix))]
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
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
        for handle in run_in(
            directory.path(),
            "capture-complete",
            &json!({"id": "c_1"}),
            LIMITS,
        ) {
            handle.join().unwrap();
        }
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&out)?)?;
        assert_eq!(written["id"], "c_1");
        assert!(directory.path().join("single.json").exists());
        assert!(hooks_in(directory.path(), "capture-started").is_empty());
        Ok(())
    }

    fn run_to_the_end(directory: &Path, limits: Limits) {
        for handle in run_in(directory, "capture-complete", &json!({}), limits) {
            handle.join().unwrap();
        }
    }

    fn log_of(directory: &Path) -> PathBuf {
        directory.join("logs/capture-complete/capture-complete.log")
    }

    #[test]
    fn a_hook_can_print_as_much_as_it_likes_into_a_file_of_its_own() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let hook = directory.path().join("capture-complete");
        let finished = directory.path().join("finished");
        // Reading only twenty lines of a pipe once killed a hook like this partway through.
        script(
            &hook,
            &format!(
                "seq 1 5000\necho complaint >&2\ntouch {}",
                finished.display()
            ),
        )?;
        run_to_the_end(directory.path(), LIMITS);
        assert!(finished.exists());
        let log = log_of(directory.path());
        let printed = std::fs::read_to_string(&log)?;
        assert_eq!(printed.lines().count(), 5001);
        assert!(printed.contains("complaint"));
        assert_eq!(std::fs::metadata(&log)?.permissions().mode() & 0o777, 0o600);

        script(&hook, "echo again")?;
        run_to_the_end(directory.path(), LIMITS);
        assert_eq!(std::fs::read_to_string(&log)?, "again\n", "replaced");
        Ok(())
    }

    #[test]
    fn a_hook_past_its_time_is_stopped_with_what_it_started() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let helper_pid = directory.path().join("helper.pid");
        script(
            &directory.path().join("capture-complete"),
            &format!("sleep 300 &\necho $! > {}\nwait", helper_pid.display()),
        )?;
        let started = Instant::now();
        run_to_the_end(
            directory.path(),
            Limits {
                time: Duration::from_millis(500),
                ..LIMITS
            },
        );
        assert!(started.elapsed() < Duration::from_secs(30));
        let helper: libc::pid_t = std::fs::read_to_string(&helper_pid)?.trim().parse()?;
        // Once its parent has gone, init has to reap it, so give that a moment.
        let deadline = Instant::now() + Duration::from_secs(10);
        // SAFETY: signal 0 only asks whether the process exists.
        while unsafe { libc::kill(helper, 0) } == 0 {
            assert!(Instant::now() < deadline, "its helper is still running");
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    #[test]
    fn a_hook_flooding_its_log_is_stopped_and_the_log_emptied() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        script(&directory.path().join("capture-complete"), "exec yes")?;
        run_to_the_end(
            directory.path(),
            Limits {
                output_bytes: 1024 * 1024,
                ..LIMITS
            },
        );
        assert_eq!(
            std::fs::read_to_string(log_of(directory.path()))?,
            "Stopped: this hook printed too much.\n"
        );
        Ok(())
    }

    #[test]
    fn a_hook_that_never_reads_its_input_holds_nothing_up() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        script(&directory.path().join("capture-complete"), "sleep 300")?;
        let started = Instant::now();
        let handles = run_in(
            directory.path(),
            "capture-complete",
            &json!({ "title": "x".repeat(1024 * 1024) }),
            Limits {
                time: Duration::from_millis(300),
                ..LIMITS
            },
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        for handle in handles {
            handle.join().unwrap();
        }
        Ok(())
    }
}
