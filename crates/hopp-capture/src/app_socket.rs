//! How the `hopp` command reaches the running app: one JSON line there and one back, over a
//! socket only this user can open. The app does the work; this is the wire and what's said
//! on it.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{cards::CardRequest, paths};

/// Nothing asked of the app is long, and a client can't make it hold more than this.
#[cfg(unix)]
const MAX_REQUEST: u64 = 64 * 1024;
/// The app's state lists the whole team.
#[cfg(unix)]
const MAX_REPLY: u64 = 8 * 1024 * 1024;

pub const NOT_RUNNING: &str = "Hopp isn't running";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AppRequest {
    /// The state the app last published.
    Snapshot,
    /// Call this teammate, or invite them when already in a call.
    Call(String),
    JoinRoom(String),
    Accept,
    Decline,
    Leave,
    Microphone,
    Camera,
    Share,
    Draw,
    CaptureStart,
    CaptureStop,
    /// Open the captures window, on this call if one is named.
    CaptureWindow(Option<String>),
    CaptureBookmark(Option<String>),
    Open,
    Settings,
    Card(CardRequest),
    CancelCard(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AppReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

impl AppReply {
    pub fn done() -> Self {
        Self {
            ok: true,
            ..Self::default()
        }
    }

    pub fn state(state: Value) -> Self {
        Self {
            ok: true,
            state: Some(state),
            ..Self::default()
        }
    }

    pub fn refused(why: impl Into<String>) -> Self {
        Self {
            error: Some(why.into()),
            ..Self::default()
        }
    }
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(paths::config_dir()?.join("run/hopp.sock"))
}

/// Ask the running app to do something. Its refusal comes back as the error.
pub fn request(request: &AppRequest) -> Result<AppReply> {
    let reply = exchange(request)?;
    match (&reply.ok, &reply.error) {
        (true, _) => Ok(reply),
        (false, Some(error)) => anyhow::bail!("{error}"),
        (false, None) => anyhow::bail!("Hopp refused that"),
    }
}

/// The running app's state, as its window last published it.
pub fn snapshot() -> Result<Value> {
    match request(&AppRequest::Snapshot)?.state {
        Some(state) => Ok(state),
        None => anyhow::bail!("Hopp is still starting"),
    }
}

#[cfg(unix)]
fn exchange(request: &AppRequest) -> Result<AppReply> {
    unix::exchange(&socket_path()?, request)
}

#[cfg(not(unix))]
fn exchange(_request: &AppRequest) -> Result<AppReply> {
    anyhow::bail!("The hopp command can't reach the app on this system yet")
}

#[cfg(unix)]
pub use unix::serve;

#[cfg(unix)]
mod unix {
    use std::{
        fs,
        io::{self, BufRead, BufReader, Read, Write},
        os::unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        path::Path,
        time::Duration,
    };

    use anyhow::{Context, Result, bail};

    use super::{AppReply, AppRequest, MAX_REPLY, MAX_REQUEST, NOT_RUNNING};
    use crate::paths;

    /// Long enough for the app to take its locks, short enough that a client who says
    /// nothing can't hold up the next one.
    const PATIENCE: Duration = Duration::from_secs(2);

    /// Answer requests one at a time for as long as the app runs.
    pub fn serve(path: &Path, mut answer: impl FnMut(AppRequest) -> AppReply) -> Result<()> {
        let listener = listen(path)?;
        for client in listener.incoming() {
            match client {
                Ok(client) => {
                    if let Err(error) = answer_one(client, &mut answer) {
                        log::warn!("App socket: {error:#}");
                    }
                }
                Err(error) => {
                    log::warn!("App socket: {error}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        Ok(())
    }

    fn listen(path: &Path) -> Result<UnixListener> {
        let directory = path.parent().context("The socket needs a folder")?;
        paths::private_dir(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Making {} private", directory.display()))?;
        match fs::symlink_metadata(path) {
            Ok(found) if found.file_type().is_socket() => {
                if UnixStream::connect(path).is_ok() {
                    bail!("Another Hopp is answering at {}", path.display());
                }
                // Left by an app that didn't get to clean up.
                fs::remove_file(path).with_context(|| format!("Removing {}", path.display()))?;
            }
            Ok(_) => bail!("{} is in the way, and it isn't a socket", path.display()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Looking for an old socket"),
        }
        let listener =
            UnixListener::bind(path).with_context(|| format!("Listening at {}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Making {} private", path.display()))?;
        Ok(listener)
    }

    fn answer_one(
        client: UnixStream,
        answer: &mut impl FnMut(AppRequest) -> AppReply,
    ) -> Result<()> {
        client.set_read_timeout(Some(PATIENCE))?;
        client.set_write_timeout(Some(PATIENCE))?;
        let mut line = String::new();
        let read = BufReader::new((&client).take(MAX_REQUEST)).read_line(&mut line);
        let reply = match read {
            Ok(_) if !line.ends_with('\n') => {
                AppReply::refused("That request was too long, or stopped short")
            }
            Ok(_) => match serde_json::from_str(&line) {
                Ok(request) => answer(request),
                // A newer `hopp` than this app: asking something it doesn't know.
                Err(error) => AppReply::refused(format!("Hopp doesn't understand that: {error}")),
            },
            Err(error) => return Err(error).context("Reading a request"),
        };
        let mut reply = serde_json::to_vec(&reply)?;
        reply.push(b'\n');
        (&client).write_all(&reply).context("Sending a reply")
    }

    pub fn exchange(path: &Path, request: &AppRequest) -> Result<AppReply> {
        check_socket(path)?;
        let mut app = match UnixStream::connect(path) {
            Ok(app) => app,
            // The socket of an app that has since quit.
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => bail!(NOT_RUNNING),
            Err(error) => return Err(error).context("Reaching Hopp"),
        };
        app.set_read_timeout(Some(Duration::from_secs(10)))?;
        app.set_write_timeout(Some(PATIENCE))?;
        let mut message = serde_json::to_vec(request)?;
        message.push(b'\n');
        app.write_all(&message).context("Asking Hopp")?;
        let mut line = String::new();
        BufReader::new(app.take(MAX_REPLY))
            .read_line(&mut line)
            .context("Waiting for Hopp to answer")?;
        serde_json::from_str(&line).context("Hopp's answer made no sense")
    }

    /// A socket this user doesn't own could be anyone listening for what's asked of Hopp.
    fn check_socket(path: &Path) -> Result<()> {
        let found = match fs::symlink_metadata(path) {
            Ok(found) => found,
            Err(error) if error.kind() == io::ErrorKind::NotFound => bail!(NOT_RUNNING),
            Err(error) => return Err(error).context("Looking for Hopp"),
        };
        let home = std::env::home_dir().context("No home directory")?;
        if !found.file_type().is_socket() || found.uid() != fs::metadata(home)?.uid() {
            bail!("{} doesn't belong to you", path.display());
        }
        if found.mode() & 0o077 != 0 {
            bail!("{} isn't private", path.display());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        use std::{os::unix::fs::PermissionsExt, sync::mpsc, thread, time::Instant};

        /// An app that says yes to everything, and tells the test what it was asked.
        fn answering(path: &Path) -> mpsc::Receiver<AppRequest> {
            let (asked, heard) = mpsc::channel();
            let at = path.to_owned();
            thread::spawn(move || {
                let _ = serve(&at, |request| {
                    let _ = asked.send(request.clone());
                    match request {
                        AppRequest::Snapshot => AppReply::state(json!({ "signed_in": true })),
                        AppRequest::Leave => AppReply::refused("You're not in a call"),
                        _ => AppReply::done(),
                    }
                });
            });
            let started = Instant::now();
            while UnixStream::connect(path).is_err() {
                assert!(started.elapsed() < Duration::from_secs(5), "never listened");
                thread::sleep(Duration::from_millis(5));
            }
            heard
        }

        #[test]
        fn one_request_one_reply_and_nobody_else_can_get_in() -> Result<()> {
            let home = tempfile::tempdir()?;
            let path = home.path().join("run/hopp.sock");
            let heard = answering(&path);

            let reply = exchange(&path, &AppRequest::Call("u1".into()))?;
            assert!(reply.ok);
            assert_eq!(heard.recv()?, AppRequest::Call("u1".into()));

            let reply = exchange(&path, &AppRequest::Leave)?;
            assert_eq!(reply.error.as_deref(), Some("You're not in a call"));

            let reply = exchange(&path, &AppRequest::Snapshot)?;
            assert_eq!(reply.state, Some(json!({ "signed_in": true })));

            let socket = fs::symlink_metadata(&path)?;
            assert_eq!(socket.permissions().mode() & 0o777, 0o600);
            let folder = fs::metadata(path.parent().context("folder")?)?;
            assert_eq!(folder.permissions().mode() & 0o777, 0o700);
            Ok(())
        }

        #[test]
        fn nothing_there_means_not_running() -> Result<()> {
            let home = tempfile::tempdir()?;
            let path = home.path().join("run/hopp.sock");
            let missing = exchange(&path, &AppRequest::Snapshot).unwrap_err();
            assert_eq!(missing.to_string(), NOT_RUNNING);

            // An app that died left its socket behind: nothing answers.
            fs::create_dir_all(path.parent().context("folder")?)?;
            drop(UnixListener::bind(&path)?);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            let stale = exchange(&path, &AppRequest::Snapshot).unwrap_err();
            assert_eq!(stale.to_string(), NOT_RUNNING);

            // The next app takes it over.
            let heard = answering(&path);
            assert!(exchange(&path, &AppRequest::Open)?.ok);
            assert_eq!(heard.recv()?, AppRequest::Open);
            Ok(())
        }

        #[test]
        fn a_second_app_leaves_the_first_alone_and_files_are_safe() -> Result<()> {
            let home = tempfile::tempdir()?;
            let path = home.path().join("run/hopp.sock");
            let heard = answering(&path);
            let second = serve(&path, |_| AppReply::done()).unwrap_err();
            assert!(second.to_string().contains("Another Hopp"), "{second:#}");
            assert!(exchange(&path, &AppRequest::Open)?.ok);
            assert_eq!(heard.recv()?, AppRequest::Open);

            let other = home.path().join("other/hopp.sock");
            fs::create_dir_all(other.parent().context("folder")?)?;
            fs::write(&other, "someone's file")?;
            assert!(serve(&other, |_| AppReply::done()).is_err());
            assert_eq!(fs::read_to_string(&other)?, "someone's file");
            Ok(())
        }

        #[test]
        fn a_socket_others_can_reach_is_refused() -> Result<()> {
            let home = tempfile::tempdir()?;
            let path = home.path().join("run/hopp.sock");
            let _heard = answering(&path);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o666))?;
            let open = exchange(&path, &AppRequest::Open).unwrap_err();
            assert!(open.to_string().contains("isn't private"), "{open:#}");
            Ok(())
        }

        #[test]
        fn nonsense_and_silence_cant_stop_the_next_client() -> Result<()> {
            let home = tempfile::tempdir()?;
            let path = home.path().join("run/hopp.sock");
            let heard = answering(&path);

            let mut confused = UnixStream::connect(&path)?;
            confused.write_all(b"{\"type\":\"fly\"}\n")?;
            let mut line = String::new();
            BufReader::new(&confused).read_line(&mut line)?;
            let reply: AppReply = serde_json::from_str(&line)?;
            assert!(!reply.ok);
            assert!(
                reply
                    .error
                    .is_some_and(|error| error.contains("understand")),
                "{line}"
            );

            let mut greedy = UnixStream::connect(&path)?;
            // The app may answer and hang up before the last of this has gone.
            let _ = greedy.write_all(&vec![b'x'; MAX_REQUEST as usize + 10]);
            let mut line = String::new();
            BufReader::new(&greedy).read_line(&mut line)?;
            let reply: AppReply = serde_json::from_str(&line)?;
            assert!(!reply.ok);
            drop(greedy);

            // Connects, then says nothing.
            let silent = UnixStream::connect(&path)?;
            let asked = Instant::now();
            assert!(exchange(&path, &AppRequest::Open)?.ok);
            assert!(asked.elapsed() < PATIENCE + Duration::from_secs(2));
            drop(silent);
            assert_eq!(heard.recv()?, AppRequest::Open);
            Ok(())
        }

        #[test]
        fn requests_read_as_the_window_expects() -> Result<()> {
            for (request, expected) in [
                (AppRequest::Accept, json!({ "type": "accept" })),
                (
                    AppRequest::JoinRoom("r1".into()),
                    json!({ "type": "join_room", "value": "r1" }),
                ),
                (
                    AppRequest::CaptureWindow(None),
                    json!({ "type": "capture_window", "value": null }),
                ),
            ] {
                assert_eq!(serde_json::to_value(&request)?, expected);
                assert_eq!(serde_json::from_value::<AppRequest>(expected)?, request);
            }
            Ok(())
        }
    }
}
