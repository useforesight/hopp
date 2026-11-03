//! Run the long-lived client in a systemd scope of its own, with a memory cap.
//!
//! Started from the Omarchy bar (Quickshell) or a Hyprland binding, the client would
//! otherwise live inside Hyprland's own service. systemd stops a service when the kernel
//! kills any of its processes for memory, so a runaway client took the whole desktop
//! session down with it. In its own scope, only the client is stopped (and the bar
//! plugin starts it again).
use std::{
    ffi::OsString, io::IsTerminal, os::unix::process::CommandExt, path::Path, process::Command,
};

/// Set on the re-executed client so it doesn't try again.
const MARKER: &str = "HOPP_SCOPED";
/// Above this the kernel reclaims the client's memory hard and slows it down…
const MEMORY_HIGH: &str = "3G";
/// …and at this it is killed. Normal use, sharing or viewing a 5K screen, is well below.
const MEMORY_MAX: &str = "4G";
/// The caps above count RAM only; without this a runaway client could keep spilling
/// into zram and the disk swapfile long after hitting them.
const MEMORY_SWAP_MAX: &str = "1G";

/// Re-execute this process inside `app-hopp-<pid>.scope` if it isn't already in
/// one. Returns only if that isn't possible (no systemd user manager), leaving the client
/// where it is.
pub fn enter() {
    if std::env::var_os(MARKER).is_some() || in_own_scope(&own_cgroup()) || !user_manager() {
        return;
    }
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new("systemd-run");
    command.args(arguments(std::process::id()));
    // Started from the bar or a launcher, stderr is /dev/null and a panic or the call log
    // would vanish; send them to the journal (`journalctl --user -t hopp`).
    if !std::io::stderr().is_terminal() && on_path("systemd-cat") {
        command.args(["systemd-cat", "--identifier=hopp"]);
    }
    let error = command
        .arg(program)
        .args(std::env::args_os().skip(1))
        .env(MARKER, "1")
        .exec();
    eprintln!("hopp: running without a memory limit: systemd-run failed: {error}");
}

/// `systemd-run` arguments that start a helper in a scope of its own, so its memory
/// never counts against the client's limits and its CPU time only comes from idle
/// cores. GPU memory counts as memory here: on Apple silicon it is system RAM, charged
/// to whoever allocates it, so a transcription model can be gigabytes. There is no
/// soft limit (throttling GPU buffers only stalls the helper), just a hard one against
/// runaways. `None` without a systemd user manager; append the program and its
/// arguments to what's returned.
pub fn isolated(name: &str, memory_max: u64) -> Option<Command> {
    if !user_manager() {
        return None;
    }
    let mut command = Command::new("systemd-run");
    command.args(helper_arguments(name, std::process::id(), memory_max));
    Some(command)
}

fn helper_arguments(name: &str, pid: u32, memory_max: u64) -> Vec<String> {
    // Unique per start: a restarted helper must not collide with the one being collected.
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default();
    vec![
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        "--slice=app.slice".into(),
        format!("--unit=app-hopp-{name}-{pid}-{started}"),
        format!("--description=Hopp {name}"),
        "--property".into(),
        format!("MemoryMax={memory_max}"),
        "--property".into(),
        "MemorySwapMax=0".into(),
        "--property".into(),
        "CPUWeight=idle".into(),
        "--property".into(),
        "IOWeight=1".into(),
        "--".into(),
    ]
}

fn arguments(pid: u32) -> Vec<OsString> {
    [
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        "--slice=app.slice".into(),
        format!("--unit=app-hopp-{pid}"),
        // Fixed, so a sign-in link's token never reaches the unit name or the journal.
        "--description=Hopp".into(),
        "--property".into(),
        format!("MemoryHigh={MEMORY_HIGH}"),
        "--property".into(),
        format!("MemoryMax={MEMORY_MAX}"),
        "--property".into(),
        format!("MemorySwapMax={MEMORY_SWAP_MAX}"),
        "--".into(),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn own_cgroup() -> String {
    std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default()
}

fn in_own_scope(cgroup: &str) -> bool {
    cgroup.contains("/app-hopp-")
}

/// systemd-run --user needs the user manager; without it exec would replace this process
/// with a failing systemd-run instead of starting Hopp.
fn user_manager() -> bool {
    std::env::var_os("XDG_RUNTIME_DIR")
        .is_some_and(|runtime| Path::new(&runtime).join("systemd/private").exists())
        && on_path("systemd-run")
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_its_own_scope_but_not_the_compositors_service() {
        let hyprland = "0::/user.slice/user-1000.slice/user@1000.service/session.slice/wayland-wm@hyprland.desktop.service\n";
        let own =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-hopp-4242.scope\n";
        assert!(!in_own_scope(hyprland));
        assert!(in_own_scope(own));
    }

    #[test]
    fn helpers_get_their_own_capped_idle_scope() {
        let arguments = helper_arguments("transcribe", 4242, 6 << 30);
        assert!(
            arguments
                .iter()
                .any(|a| a.starts_with("--unit=app-hopp-transcribe-4242-"))
        );
        assert!(arguments.contains(&format!("MemoryMax={}", 6u64 << 30)));
        assert!(arguments.contains(&"CPUWeight=idle".to_owned()));
        assert_eq!(arguments.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn scope_is_capped_and_never_named_after_the_command_line() {
        let arguments: Vec<String> = arguments(4242)
            .into_iter()
            .map(|argument| argument.into_string().unwrap())
            .collect();
        assert!(arguments.contains(&"--unit=app-hopp-4242".into()));
        assert!(arguments.contains(&"--description=Hopp".into()));
        assert!(arguments.contains(&format!("MemoryMax={MEMORY_MAX}")));
        assert!(arguments.contains(&format!("MemoryHigh={MEMORY_HIGH}")));
        assert!(arguments.contains(&format!("MemorySwapMax={MEMORY_SWAP_MAX}")));
        assert_eq!(arguments.last().map(String::as_str), Some("--"));
    }
}
