//! Private single-instance IPC for desktop launchers, browser sign-in links, and the
//! Omarchy bar plugin.
//!
//! Every request is one length-prefixed JSON message answered with one status byte.
//! `subscribe` keeps the connection open afterwards and streams length-prefixed JSON
//! state snapshots, the latest immediately and then one per change.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tokio::sync::mpsc::Sender;

use crate::{
    api::{DeepLink, parse_deep_link},
    engine::Command,
};

const MAX_REQUEST: usize = 64 * 1024;

#[derive(Serialize, Deserialize)]
struct LaunchRequest {
    link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<LaunchAction>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LaunchAction {
    RevokeControl,
    Ui(crate::engine::UiRequest),
    Subscribe,
}

enum Outcome {
    Done,
    Subscribe,
}

/// State snapshots for subscribers (the bar plugin). Cloned into the UI, which
/// publishes whenever what it shows changes.
#[derive(Clone, Default)]
pub struct StateFeed(Arc<Mutex<Feed>>);

#[derive(Default)]
struct Feed {
    streams: Vec<UnixStream>,
    latest: Option<Vec<u8>>,
}

fn framed(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

impl StateFeed {
    /// Send `json` to every subscriber unless it is unchanged. Subscribers that
    /// stop reading are dropped rather than allowed to stall the UI.
    pub fn publish(&self, json: &str) {
        let Ok(mut feed) = self.0.lock() else {
            return;
        };
        if feed.latest.as_deref() == Some(json.as_bytes()) {
            return;
        }
        let frame = framed(json.as_bytes());
        feed.streams
            .retain_mut(|stream| stream.write_all(&frame).is_ok());
        feed.latest = Some(json.as_bytes().to_vec());
    }

    fn attach(&self, mut stream: UnixStream) {
        if stream
            .set_write_timeout(Some(Duration::from_millis(100)))
            .is_err()
        {
            return;
        }
        let Ok(mut feed) = self.0.lock() else {
            return;
        };
        if let Some(latest) = &feed.latest
            && stream.write_all(&framed(latest)).is_err()
        {
            return;
        }
        // Bounded: a misbehaving client cannot accumulate connections.
        if feed.streams.len() >= 8 {
            feed.streams.remove(0);
        }
        feed.streams.push(stream);
    }
}

pub struct Instance {
    feed: StateFeed,
    socket: PathBuf,
    socket_inode: u64,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    // Keep the same lock-file inode across launches. Unlinking it while locked
    // would let a second process lock a new inode and create another listener.
    _ownership: File,
}

fn instance_directory() -> Result<PathBuf> {
    let uid = fs::metadata("/proc/self")
        .context("Reading current user identity")?
        .uid();
    let directory = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(base) => {
            let base = PathBuf::from(base);
            if !base.is_absolute() {
                bail!("XDG_RUNTIME_DIR must be an absolute directory");
            }
            let metadata = fs::symlink_metadata(&base).context("Reading user runtime directory")?;
            if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
                bail!("User runtime directory must be private and owned by this user");
            }
            base.join(crate::paths::APP)
        }
        None => PathBuf::from(format!("/tmp/{}-{uid}", crate::paths::APP)),
    };
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("Creating private Hopp runtime directory"),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        bail!("Hopp runtime directory must be private and owned by this user");
    }
    Ok(directory)
}

fn socket_path(directory: &Path, instance: &str) -> Result<PathBuf> {
    if instance.is_empty()
        || instance.len() > 32
        || !instance
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("Instance name must contain 1–32 letters, digits, hyphens or underscores");
    }
    let path = directory.join(format!("{instance}.sock"));
    if path.as_os_str().len() >= 108 {
        bail!("Hopp runtime socket path is too long");
    }
    Ok(path)
}

fn validate_socket(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let uid = fs::metadata("/proc/self")?.uid();
    if !metadata.file_type().is_socket() || metadata.uid() != uid {
        bail!("Hopp instance socket is not owned by this user");
    }
    if metadata.mode() & 0o077 != 0 {
        bail!("Hopp instance socket is not private");
    }
    Ok(())
}

/// Call before starting the engine. Returns false if no instance is running.
/// A browser sign-in token is sent only through the user's private Unix socket.
pub fn forward_existing(instance: &str, link: Option<&str>) -> Result<bool> {
    forward_at(&instance_directory()?, instance, link)
}

/// Ask the running instance to do something (peek, settings, call controls).
/// Missing instances return `false` so a keybinding can start Hopp instead.
pub fn request_existing(instance: &str, request: crate::engine::UiRequest) -> Result<bool> {
    forward_request(
        &instance_directory()?,
        instance,
        LaunchRequest {
            link: None,
            action: Some(LaunchAction::Ui(request)),
        },
    )
}

/// The running instance's current state snapshot (the JSON the bar plugin gets), or
/// `None` if nothing is running.
pub fn snapshot(instance: &str) -> Result<Option<String>> {
    snapshot_at(&instance_directory()?, instance)
}

fn snapshot_at(directory: &Path, instance: &str) -> Result<Option<String>> {
    let path = socket_path(directory, instance)?;
    if fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    validate_socket(&path)?;
    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => return Ok(None),
        Err(error) => return Err(error).context("Could not contact the running Hopp"),
    };
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let request = serde_json::to_vec(&LaunchRequest {
        link: None,
        action: Some(LaunchAction::Subscribe),
    })?;
    stream.write_all(&(request.len() as u32).to_be_bytes())?;
    stream.write_all(&request)?;
    let mut acknowledged = [1];
    stream.read_exact(&mut acknowledged)?;
    if acknowledged[0] != 0 {
        bail!("The running Hopp refused the status request");
    }
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .context("The running Hopp has no state to report yet")?;
    let length = u32::from_be_bytes(header) as usize;
    if length > 4 * 1024 * 1024 {
        bail!("Hopp state is too large");
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(Some(String::from_utf8(body)?))
}

/// Watch the running instance's state snapshots (the latest, then one per change) until
/// `done` returns something, or `deadline` passes (`None`). Errors if nothing is
/// running or the instance goes away meanwhile.
pub fn watch<T>(
    instance: &str,
    deadline: Option<std::time::Instant>,
    done: impl FnMut(&str) -> Option<T>,
) -> Result<Option<T>> {
    watch_at(&instance_directory()?, instance, deadline, done)
}

fn watch_at<T>(
    directory: &Path,
    instance: &str,
    deadline: Option<std::time::Instant>,
    mut done: impl FnMut(&str) -> Option<T>,
) -> Result<Option<T>> {
    let path = socket_path(directory, instance)?;
    if fs::symlink_metadata(&path).is_err() {
        bail!("Hopp isn't running");
    }
    validate_socket(&path)?;
    let mut stream = UnixStream::connect(&path).context("Could not contact the running Hopp")?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let request = serde_json::to_vec(&LaunchRequest {
        link: None,
        action: Some(LaunchAction::Subscribe),
    })?;
    stream.write_all(&(request.len() as u32).to_be_bytes())?;
    stream.write_all(&request)?;
    let mut acknowledged = [1];
    stream.read_exact(&mut acknowledged)?;
    if acknowledged[0] != 0 {
        bail!("The running Hopp refused the status request");
    }
    loop {
        // Wake at least every second to check the deadline; a snapshot interrupted by
        // one is read on.
        let mut header = [0; 4];
        let mut filled = 0;
        while filled < header.len() {
            if deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline) && filled == 0
            {
                return Ok(None);
            }
            stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            match stream.read(&mut header[filled..]) {
                Ok(0) => bail!("Hopp quit"),
                Ok(read) => filled += read,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let length = u32::from_be_bytes(header) as usize;
        if length > 4 * 1024 * 1024 {
            bail!("Hopp state is too large");
        }
        let mut body = vec![0; length];
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.read_exact(&mut body)?;
        if let Some(value) = done(std::str::from_utf8(&body)?) {
            return Ok(Some(value));
        }
    }
}

/// Send an emergency revoke to the selected running instance without opening
/// or activating a window. Missing instances are a successful no-op for binds.
pub fn revoke_existing(instance: &str) -> Result<bool> {
    forward_request(
        &instance_directory()?,
        instance,
        LaunchRequest {
            link: None,
            action: Some(LaunchAction::RevokeControl),
        },
    )
}

fn forward_at(directory: &Path, instance: &str, link: Option<&str>) -> Result<bool> {
    if link.is_some_and(|link| link.len() > MAX_REQUEST) {
        bail!("Hopp launch request is too large");
    }
    if let Some(link) = link {
        parse_deep_link(link)?;
    }
    forward_request(
        directory,
        instance,
        LaunchRequest {
            link: link.map(str::to_owned),
            action: None,
        },
    )
}

fn forward_request(directory: &Path, instance: &str, request: LaunchRequest) -> Result<bool> {
    let request = serde_json::to_vec(&request)?;
    if request.len() > MAX_REQUEST {
        bail!("Hopp launch request is too large");
    }
    let path = socket_path(directory, instance)?;
    let lock_path = directory.join(format!("{instance}.lock"));
    for attempt in 0..50 {
        if path.exists() {
            validate_socket(&path)?;
        }
        match UnixStream::connect(&path) {
            Ok(mut stream) => {
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                stream.write_all(&(request.len() as u32).to_be_bytes())?;
                stream.write_all(&request)?;
                let mut acknowledged = [1];
                stream
                    .read_exact(&mut acknowledged)
                    .context("Existing Hopp instance did not acknowledge launch")?;
                if acknowledged[0] != 0 {
                    bail!("Existing Hopp instance could not accept the launch request");
                }
                return Ok(true);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                // Another process may hold the ownership lock while creating
                // its listener. Allow it to finish instead of starting twice.
                let starting = File::open(&lock_path)
                    .ok()
                    .is_some_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
                if !starting || attempt == 49 {
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => bail!("Could not contact the existing Hopp instance"),
        }
    }
    Ok(false)
}

pub fn listen(instance: &str, commands: Sender<Command>, server: String) -> Result<Instance> {
    listen_at(&instance_directory()?, instance, commands, server)
}

fn listen_at(
    directory: &Path,
    instance: &str,
    commands: Sender<Command>,
    server: String,
) -> Result<Instance> {
    let path = socket_path(directory, instance)?;
    let lock_path = directory.join(format!("{instance}.lock"));
    if let Ok(metadata) = fs::symlink_metadata(&lock_path)
        && (!metadata.file_type().is_file()
            || metadata.uid() != fs::metadata("/proc/self")?.uid()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1)
    {
        bail!("Hopp instance ownership file is not private");
    }
    let ownership = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .context("Opening Hopp instance ownership file")?;
    ownership
        .try_lock()
        .map_err(|_| anyhow::anyhow!("Another Hopp instance is already starting or running"))?;
    if fs::symlink_metadata(&path).is_ok() {
        validate_socket(&path)?;
        // Holding the exclusive lock makes stale recovery atomic between
        // instances. Never replace a socket with an active legacy listener.
        if UnixStream::connect(&path).is_ok() {
            bail!("Another Hopp instance is already running");
        }
        fs::remove_file(&path).context("Removing stale Hopp instance socket")?;
    }
    let listener = UnixListener::bind(&path).context("Opening private Hopp instance socket")?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let socket_inode = fs::symlink_metadata(&path)?.ino();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let feed = StateFeed::default();
    let worker_feed = feed.clone();
    let worker = thread::Builder::new()
        .name("hopp-launcher".into())
        .spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => match receive_launch(&mut stream, &commands, &server) {
                        Ok(Outcome::Subscribe) => {
                            if stream.write_all(&[0]).is_ok() {
                                worker_feed.attach(stream);
                            }
                        }
                        Ok(Outcome::Done) => {
                            let _ = stream.write_all(&[0]);
                        }
                        Err(_) => {
                            let _ = stream.write_all(&[1]);
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(_) => break,
                }
            }
        })
        .context("Starting Hopp launcher listener")?;
    Ok(Instance {
        feed,
        socket: path,
        socket_inode,
        stop,
        worker: Some(worker),
        _ownership: ownership,
    })
}

fn receive_launch(
    stream: &mut UnixStream,
    commands: &Sender<Command>,
    server: &str,
) -> Result<Outcome> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_REQUEST {
        bail!("Invalid Hopp launch request size");
    }
    let mut request = vec![0; length];
    stream.read_exact(&mut request)?;
    let request: LaunchRequest =
        serde_json::from_slice(&request).context("Invalid Hopp launch request")?;
    if let Some(LaunchAction::RevokeControl) = request.action {
        if request.link.is_some() {
            bail!("An emergency revoke cannot include a sign-in link");
        }
        commands
            .try_send(Command::RevokeControl)
            .map_err(|_| anyhow::anyhow!("Hopp session worker is unavailable"))?;
        return Ok(Outcome::Done);
    }
    if let Some(LaunchAction::Subscribe) = request.action {
        if request.link.is_some() {
            bail!("A subscription cannot include a sign-in link");
        }
        return Ok(Outcome::Subscribe);
    }
    if let Some(LaunchAction::Ui(ui)) = request.action {
        if request.link.is_some() {
            bail!("A window request cannot include a sign-in link");
        }
        commands
            .try_send(Command::Ui(ui))
            .map_err(|_| anyhow::anyhow!("Hopp session worker is unavailable"))?;
        return Ok(Outcome::Done);
    }
    let has_link = request.link.is_some();
    if let Some(link) = request.link {
        let command = match parse_deep_link(&link)? {
            DeepLink::SignIn(token) => Command::SignIn {
                server: crate::settings::load()
                    .map(|settings| settings.server)
                    .unwrap_or_else(|_| server.to_owned()),
                token,
            },
            DeepLink::JoinSession(id) => Command::JoinSession(id),
        };
        commands
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Hopp session worker is unavailable"))?;
    }
    // Activate is best effort after successful token delivery: a full queue
    // should not make the browser repeat an already accepted sign-in token.
    let activated = commands.try_send(Command::Activate);
    if !has_link {
        activated.map_err(|_| anyhow::anyhow!("Hopp session worker is unavailable"))?;
    }
    Ok(Outcome::Done)
}

impl Instance {
    /// Where the UI publishes state for subscribers.
    pub fn feed(&self) -> StateFeed {
        self.feed.clone()
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if fs::symlink_metadata(&self.socket).is_ok_and(|metadata| {
            metadata.file_type().is_socket() && metadata.ino() == self.socket_inode
        }) {
            let _ = fs::remove_file(&self.socket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emergency_revoke_only_targets_selected_instance_without_activation() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let (other_commands, mut other_receive) = tokio::sync::mpsc::channel(8);
        let instance = listen_at(temporary.path(), "fixture", commands, String::new())?;
        let _other = listen_at(temporary.path(), "other", other_commands, String::new())?;
        assert!(forward_request(
            temporary.path(),
            "fixture",
            LaunchRequest {
                link: None,
                action: Some(LaunchAction::RevokeControl),
            }
        )?);
        assert!(matches!(receive.try_recv()?, Command::RevokeControl));
        assert!(
            receive.try_recv().is_err(),
            "revoke must not activate or sign in"
        );
        assert!(other_receive.try_recv().is_err());
        assert!(
            forward_request(
                temporary.path(),
                "fixture",
                LaunchRequest {
                    link: Some("hopp:///authenticate?token=fixture".into()),
                    action: Some(LaunchAction::RevokeControl),
                }
            )
            .is_err()
        );
        assert!(receive.try_recv().is_err());
        drop(instance);
        assert!(!forward_request(
            temporary.path(),
            "fixture",
            LaunchRequest {
                link: None,
                action: Some(LaunchAction::RevokeControl),
            }
        )?);
        Ok(())
    }

    #[test]
    fn window_requests_reach_only_the_selected_instance_and_never_sign_in() -> Result<()> {
        use crate::engine::UiRequest;
        let temporary = tempfile::tempdir()?;
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let (other_commands, mut other_receive) = tokio::sync::mpsc::channel(8);
        let _instance = listen_at(temporary.path(), "fixture", commands, String::new())?;
        let _other = listen_at(temporary.path(), "other", other_commands, String::new())?;
        for request in [
            UiRequest::Peek,
            UiRequest::Settings,
            UiRequest::Call("mira".into()),
        ] {
            assert!(forward_request(
                temporary.path(),
                "fixture",
                LaunchRequest {
                    link: None,
                    action: Some(LaunchAction::Ui(request.clone())),
                }
            )?);
            assert!(matches!(receive.try_recv()?, Command::Ui(got) if got == request));
            assert!(receive.try_recv().is_err(), "no activation or sign-in");
        }
        assert!(
            forward_request(
                temporary.path(),
                "fixture",
                LaunchRequest {
                    link: Some("hopp:///authenticate?token=fixture".into()),
                    action: Some(LaunchAction::Ui(UiRequest::Peek)),
                }
            )
            .is_err()
        );
        assert!(receive.try_recv().is_err());
        assert!(other_receive.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn subscribers_get_the_latest_state_then_only_changes() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let instance = listen_at(temporary.path(), "fixture", commands, String::new())?;
        let feed = instance.feed();
        feed.publish(r#"{"n":1}"#);
        let mut stream = UnixStream::connect(socket_path(temporary.path(), "fixture")?)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let request = serde_json::to_vec(&LaunchRequest {
            link: None,
            action: Some(LaunchAction::Subscribe),
        })?;
        stream.write_all(&(request.len() as u32).to_be_bytes())?;
        stream.write_all(&request)?;
        let read_frame = |stream: &mut UnixStream| -> Result<String> {
            let mut header = [0; 4];
            stream.read_exact(&mut header)?;
            let mut body = vec![0; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut body)?;
            Ok(String::from_utf8(body)?)
        };
        let mut status = [1];
        stream.read_exact(&mut status)?;
        assert_eq!(status, [0]);
        assert_eq!(read_frame(&mut stream)?, r#"{"n":1}"#);
        // Wait until the listener thread has attached the stream.
        for _ in 0..100 {
            if feed.0.lock().unwrap().streams.len() == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        feed.publish(r#"{"n":1}"#);
        feed.publish(r#"{"n":2}"#);
        assert_eq!(
            read_frame(&mut stream)?,
            r#"{"n":2}"#,
            "unchanged state is not resent"
        );
        assert!(receive.try_recv().is_err(), "subscribing sends no commands");
        Ok(())
    }

    #[test]
    fn status_reads_one_snapshot_and_reports_nothing_running() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        assert_eq!(snapshot_at(temporary.path(), "fixture")?, None);
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let instance = listen_at(temporary.path(), "fixture", commands, String::new())?;
        instance.feed().publish(r#"{"version":1}"#);
        assert_eq!(
            snapshot_at(temporary.path(), "fixture")?.as_deref(),
            Some(r#"{"version":1}"#)
        );
        assert!(receive.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn forwards_activation_and_recovers_stale_socket() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let directory = temporary.path();
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let instance = listen_at(
            directory,
            "fixture",
            commands.clone(),
            "http://localhost:18826".into(),
        )?;
        assert!(forward_at(directory, "fixture", None)?);
        assert!(matches!(receive.try_recv()?, Command::Activate));
        assert!(listen_at(directory, "fixture", commands.clone(), String::new()).is_err());
        assert_eq!(
            fs::metadata(directory.join("fixture.sock"))?.mode() & 0o777,
            0o600
        );
        drop(instance);
        assert!(!directory.join("fixture.sock").exists());
        let stale = UnixListener::bind(directory.join("fixture.sock"))?;
        fs::set_permissions(
            directory.join("fixture.sock"),
            fs::Permissions::from_mode(0o600),
        )?;
        drop(stale);
        let recovered = listen_at(directory, "fixture", commands, String::new())?;
        assert!(forward_at(directory, "fixture", None)?);
        drop(recovered);
        Ok(())
    }

    #[test]
    fn rejects_other_routes_and_never_overwrites_regular_files() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let _instance = listen_at(temporary.path(), "fixture", commands.clone(), String::new())?;
        assert!(
            forward_at(
                temporary.path(),
                "fixture",
                Some("hopp:///slack-join?token=example")
            )
            .is_err()
        );
        assert!(receive.try_recv().is_err());
        assert!(forward_at(temporary.path(), "../escape", None).is_err());
        let path = temporary.path().join("regular.sock");
        fs::write(&path, "retain")?;
        assert!(listen_at(temporary.path(), "regular", commands, String::new()).is_err());
        assert_eq!(fs::read_to_string(path)?, "retain");
        Ok(())
    }

    #[test]
    fn delivers_valid_sign_in_and_rejects_unbounded_packets() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let (commands, mut receive) = tokio::sync::mpsc::channel(8);
        let _instance = listen_at(
            temporary.path(),
            "fixture",
            commands,
            "http://localhost:18826".into(),
        )?;
        assert!(forward_at(
            temporary.path(),
            "fixture",
            Some("hopp:///authenticate?token=example.fixture.token")
        )?);
        match receive.try_recv()? {
            Command::SignIn { server, token } => {
                assert!(!server.is_empty());
                assert_eq!(token, "example.fixture.token");
            }
            _ => bail!("Expected native sign-in command"),
        }
        assert!(matches!(receive.try_recv()?, Command::Activate));
        let mut stream = UnixStream::connect(temporary.path().join("fixture.sock"))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(&u32::MAX.to_be_bytes())?;
        let mut ack = [0];
        stream.read_exact(&mut ack)?;
        assert_eq!(ack, [1]);
        assert!(receive.try_recv().is_err());
        assert!(forward_at(temporary.path(), "fixture", None)?);
        Ok(())
    }
}
