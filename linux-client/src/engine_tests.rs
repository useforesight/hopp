//! Ignored tests for the complete worker lifecycle against dedicated local
//! two-user Hopp accounts. Set HOPP_API_TEST_FIXTURE and an isolated temporary
//! XDG_CONFIG_HOME. The lifecycle test never captures hardware. The separate
//! desktop test requires explicit portal consent and optional isolated-editor
//! clipboard consent; it never enables a microphone or camera.
use crate::{
    api::{ApiClient, RoomInfo},
    engine::{Command, Engine, Event},
    settings::Settings,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct User {
    token: String,
    user_id: String,
}
#[derive(Deserialize)]
struct Fixture {
    api_base: String,
    room_id: String,
    publisher: User,
    viewer: User,
}
struct TestEngine(Engine);
impl Drop for TestEngine {
    fn drop(&mut self) {
        let _ = self.0.commands.try_send(Command::Leave);
        let _ = self.0.commands.try_send(Command::Shutdown);
    }
}
fn send(engine: &Engine, command: Command) -> Result<()> {
    engine
        .commands
        .blocking_send(command)
        .context("Engine command channel closed")
}
fn wait(
    engine: &Engine,
    description: &str,
    predicate: impl FnMut(&Event) -> bool,
) -> Result<Event> {
    wait_for(engine, description, Duration::from_secs(20), predicate)
}
fn wait_for(
    engine: &Engine,
    description: &str,
    timeout: Duration,
    mut predicate: impl FnMut(&Event) -> bool,
) -> Result<Event> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        match engine.events.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => {
                if predicate(&event) {
                    return Ok(event);
                }
                if let Event::Error(error) = event {
                    last_error = Some(error);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error).context("Engine event channel closed"),
        }
    }
    bail!(
        "Timed out waiting for {description}; last error: {}",
        last_error.as_deref().unwrap_or("none")
    );
}
fn start(server: &str, user: &User) -> TestEngine {
    TestEngine(Engine::start(Settings {
        server: server.into(),
        token: Some(user.token.clone()),
        ..Default::default()
    }))
}
fn join(engine: &Engine, room: &RoomInfo) -> Result<()> {
    send(engine, Command::JoinRoom(room.clone()))?;
    wait(
        engine,
        "room join",
        |event| matches!(event,Event::Joined(joined) if joined.id==room.id),
    )?;
    Ok(())
}
fn leave(engine: &Engine) -> Result<()> {
    send(engine, Command::Leave)?;
    wait(engine, "room leave", |event| {
        matches!(event, Event::Disconnected(_))
    })?;
    Ok(())
}

#[test]
#[ignore = "requires a local Hopp/LiveKit server, HOPP_API_TEST_FIXTURE and isolated temporary XDG_CONFIG_HOME"]
fn real_engine_join_cancel_and_presence_cleanup() -> Result<()> {
    let config=PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").context("Set XDG_CONFIG_HOME to a dedicated temporary directory; engine authentication saves private settings")?);
    ensure!(
        config.starts_with(std::env::temp_dir()) && config != std::env::temp_dir(),
        "Engine tests require a dedicated temporary XDG_CONFIG_HOME"
    );
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(
        std::env::var("HOPP_API_TEST_FIXTURE")
            .context("Set HOPP_API_TEST_FIXTURE to the private two-user local API fixture")?,
    )?)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let api = ApiClient::new(&fixture.api_base, Some(fixture.publisher.token.clone()))?;
    let room = runtime
        .block_on(api.rooms())?
        .into_iter()
        .find(|room| room.id == fixture.room_id)
        .context("Fixture room no longer exists")?;
    let publisher = start(&fixture.api_base, &fixture.publisher);
    let viewer = start(&fixture.api_base, &fixture.viewer);
    wait(&publisher.0, "publisher authentication", |event| {
        matches!(event, Event::LoggedIn(_))
    })?;
    wait(&viewer.0, "viewer authentication", |event| {
        matches!(event, Event::LoggedIn(_))
    })?;
    join(&publisher.0, &room)?;
    send(&viewer.0, Command::JoinRoom(room.clone()))?;
    // Initial peer snapshots may arrive before Joined; retain both observations
    // so event ordering does not make a valid join look like a missing peer.
    let mut joined = false;
    let mut publisher_present = false;
    wait(&viewer.0, "joined room with publisher", |event| {
        match event {
            Event::Joined(joined_room) if joined_room.id == room.id => joined = true,
            Event::Participants(peers) => {
                publisher_present = peers
                    .iter()
                    .any(|peer| peer.identity.contains(&fixture.publisher.user_id));
            }
            _ => {}
        }
        joined && publisher_present
    })?;
    wait(
        &publisher.0,
        "viewer participant",
        |event| matches!(event,Event::Participants(peers) if peers.iter().any(|peer|peer.identity.contains(&fixture.viewer.user_id))),
    )?;
    ensure!(
        publisher.0.shared.local_camera.lock().unwrap().is_none(),
        "Engine opened a hardware capture unexpectedly"
    );

    // This test-only notification reaches the actual desktop privacy handler.
    // It changes no compositor state and prevents a portal from appearing.
    send(&publisher.0, Command::TestDesktopLock(true))?;
    wait(&publisher.0, "desktop privacy notification", |event| {
        matches!(event, Event::DesktopLocked)
    })?;
    send(&publisher.0, Command::Share { window_only: false })?;
    wait(
        &publisher.0,
        "locked sharing rejection",
        |event| matches!(event,Event::Error(error) if error.to_lowercase().contains("unlock")),
    )?;

    leave(&viewer.0)?;
    viewer.0.events.try_iter().for_each(drop);
    send(&viewer.0, Command::JoinRoom(room.clone()))?;
    wait(
        &viewer.0,
        "cancellable join start",
        |event| matches!(event,Event::Status(status) if status.starts_with("Connecting to ")),
    )?;
    // Cancel while the asynchronous HTTP/LiveKit join is still pending.
    send(&viewer.0, Command::Leave)?;
    wait(&viewer.0, "cancelled join leave", |event| {
        matches!(event, Event::Disconnected(_))
    })?;
    join(&viewer.0, &room)?;
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        match viewer.0.events.recv_timeout(Duration::from_millis(25)) {
            Ok(Event::Disconnected(reason)) => {
                bail!("Old join disconnected the newer session: {reason}")
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error).context("Engine event channel closed after reconnect"),
        }
    }
    leave(&viewer.0)?;
    leave(&publisher.0)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut empty = false;
    while Instant::now() < deadline {
        let presence = runtime.block_on(api.rooms_presence())?;
        if presence.values().all(|users| {
            !users.contains(&fixture.publisher.user_id) && !users.contains(&fixture.viewer.user_id)
        }) {
            empty = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    ensure!(
        empty,
        "Engine leave did not clear both users from backend room presence"
    );
    for engine in [&publisher.0, &viewer.0] {
        ensure!(
            engine.shared.remote.lock().unwrap().is_none()
                && engine.shared.local_camera.lock().unwrap().is_none()
                && engine.shared.cameras.lock().unwrap().is_empty(),
            "Leaving retained media previews"
        );
    }
    println!(
        "ENGINE PASS: authenticated two users, participants, locked-share rejection, cancelled join followed by clean reconnect, both room-presence records removed."
    );
    Ok(())
}

fn wait_ready(engine: &Engine) -> Result<()> {
    let mut authenticated = false;
    let mut unlocked = false;
    wait(
        engine,
        "authenticated engine on unlocked Hyprland",
        |event| {
            match event {
                Event::LoggedIn(_) => authenticated = true,
                Event::DesktopUnlocked => unlocked = true,
                Event::DesktopLocked => unlocked = false,
                _ => {}
            }
            authenticated && unlocked
        },
    )?;
    Ok(())
}

#[derive(Debug)]
struct ScreenObservation {
    source: [u32; 2],
    decoded: [u32; 2],
}

fn screen_observation(publisher: &Engine, viewer: &Engine) -> Result<Option<ScreenObservation>> {
    let published = *publisher
        .shared
        .published
        .lock()
        .map_err(|_| anyhow::anyhow!("Published size lock poisoned"))?;
    let remote = viewer
        .shared
        .remote
        .lock()
        .map_err(|_| anyhow::anyhow!("Remote frame lock poisoned"))?;
    let (Some(source), Some(remote)) = (published, remote.as_ref()) else {
        return Ok(None);
    };
    ensure!(
        source[0] > 0 && source[1] > 0 && remote.width > 0 && remote.height > 0,
        "Capture has zero dimensions"
    );
    ensure!(
        remote.bgra.len() == remote.width as usize * remote.height as usize * 4,
        "Invalid packed BGRA frame"
    );
    Ok(Some(ScreenObservation {
        source,
        decoded: [remote.width, remote.height],
    }))
}

fn wait_screen(publisher: &Engine, viewer: &Engine) -> Result<ScreenObservation> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(observed) = screen_observation(publisher, viewer)? {
            let [source_width, source_height] = observed.source;
            let [decoded_width, decoded_height] = observed.decoded;
            ensure!(
                decoded_width <= 4096
                    && decoded_height <= 4096
                    && u64::from(decoded_width) * u64::from(decoded_height) <= 9_437_184,
                "Decoded screen exceeds native H.264 bounds: {observed:?}"
            );
            ensure!(
                decoded_width <= source_width && decoded_height <= source_height,
                "Decoded screen unexpectedly exceeds its capture source: {observed:?}"
            );
            let source_aspect = f64::from(source_width) / f64::from(source_height);
            let decoded_aspect = f64::from(decoded_width) / f64::from(decoded_height);
            ensure!(
                (decoded_aspect / source_aspect - 1.).abs() < 0.01,
                "Screen scaling distorted source aspect: {observed:?}"
            );
            // A share starts small and steps up with the bandwidth estimate; on localhost
            // it reaches the full encoder-safe size within seconds.
            if observed.decoded
                == <[u32; 2]>::from(crate::media::h264_screen_size(source_width, source_height))
            {
                return Ok(observed);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("The decoded screen did not reach full size within twenty seconds")
}

fn editor_focus() -> Result<PathBuf> {
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR").context("Private desktop runtime required")?,
    )
    .canonicalize()?;
    ensure!(
        runtime.starts_with(std::env::temp_dir()) && runtime != std::env::temp_dir(),
        "Clipboard test needs a private runtime under /tmp"
    );
    ensure!(
        !std::path::Path::new("/dev/input").exists() && !std::path::Path::new("/run/user").exists(),
        "Physical input and actual desktop sockets must be hidden before clipboard injection"
    );
    let output = std::process::Command::new("hyprctl")
        .args(["-j", "locked"])
        .output()?;
    ensure!(
        output.status.success(),
        "Could not check compositor lock state"
    );
    let locked: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    ensure!(
        locked["locked"].as_bool() == Some(false),
        "Clipboard injection requires an unlocked isolated compositor"
    );
    let output = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()?;
    ensure!(
        output.status.success(),
        "Could not check isolated editor focus"
    );
    let window: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    ensure!(
        window["class"].as_str() == Some("hopp-native-editor"),
        "Clipboard injection requires focus on the isolated Hopp native editor"
    );
    Ok(runtime.join("hopp-editor.txt"))
}

fn editor_text(path: &std::path::Path, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if std::fs::read_to_string(path).is_ok_and(|value| value == expected) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    bail!("Isolated native editor did not reach the expected text")
}

fn select_editor_text(viewer: &Engine) -> Result<()> {
    use crate::protocol::{InputEvent, Keystroke};
    for down in [true, false] {
        editor_focus()?;
        send(
            viewer,
            Command::SendInput(InputEvent::Keystroke(Keystroke {
                key: vec!["a".into()],
                down,
                ctrl: down,
                shift: false,
                alt: false,
                meta: false,
            })),
        )?;
    }
    Ok(())
}

fn clipboard_roundtrip(publisher: &Engine, viewer: &Engine) -> Result<()> {
    use crate::protocol::{ClipboardPaste, InputEvent};
    let state = editor_focus()?;
    let nonce = format!(
        "Hopp native clipboard {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    select_editor_text(viewer)?;
    for packet in crate::clipboard::packets(nonce.as_bytes())? {
        editor_focus()?;
        send(
            viewer,
            Command::SendInput(InputEvent::PasteFromClipboard(ClipboardPaste {
                data: Some(packet),
            })),
        )?;
    }
    wait(
        publisher,
        "native paste confirmation",
        |event| matches!(event, Event::Status(status) if status == "Pasted to the shared screen"),
    )?;
    editor_text(&state, &nonce)?;
    for is_copy in [true, false] {
        // A sentinel makes stale clipboard contents unable to pass the copy test.
        crate::clipboard::write(b"isolated stale clipboard sentinel")?;
        select_editor_text(viewer)?;
        editor_focus()?;
        send(viewer, Command::CopyClipboard(is_copy))?;
        wait(
            viewer,
            "native clipboard reply",
            |event| matches!(event, Event::Status(status) if status == "Copied from the shared screen"),
        )?;
        ensure!(
            crate::clipboard::read()? == nonce.as_bytes(),
            "Native copy/cut did not return the editor's text"
        );
        editor_text(&state, if is_copy { &nonce } else { "" })?;
    }
    Ok(())
}

#[test]
#[ignore = "opens a real portal chooser; needs HOPP_DESKTOP_TEST=monitor|window, localhost fixture, private config and unlocked isolated Hyprland"]
fn real_engine_portal_share_control_drawing_and_cleanup() -> Result<()> {
    use crate::protocol::{InputEvent, PathPoint, Point, canonical_identity};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mode = std::env::var("HOPP_DESKTOP_TEST")
        .context("Explicitly set HOPP_DESKTOP_TEST=monitor or window before opening a chooser")?;
    ensure!(
        mode == "monitor" || mode == "window",
        "HOPP_DESKTOP_TEST must be monitor or window"
    );
    let window_only = mode == "window";
    let clipboard = std::env::var("HOPP_CLIPBOARD_TEST").is_ok_and(|value| value == "1");
    ensure!(
        !clipboard || !window_only,
        "Clipboard acceptance requires a shared monitor"
    );
    if clipboard {
        editor_focus()?;
    }
    let config = PathBuf::from(
        std::env::var_os("XDG_CONFIG_HOME").context("Set a private temporary XDG_CONFIG_HOME")?,
    );
    ensure!(
        !std::fs::symlink_metadata(&config)?.file_type().is_symlink(),
        "Test config must not be a symlink"
    );
    let config = config.canonicalize()?;
    ensure!(
        config.starts_with(std::env::temp_dir()) && config != std::env::temp_dir(),
        "Test config must be a dedicated temporary directory"
    );
    let metadata = std::fs::metadata(&config)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == std::fs::metadata("/proc/self")?.uid()
            && metadata.permissions().mode() & 0o077 == 0,
        "Test config must be private and user owned"
    );
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(
        std::env::var("HOPP_API_TEST_FIXTURE").context("Set private two-user local fixture")?,
    )?)?;
    ensure!(
        fixture.api_base.starts_with("http://127.0.0.1:"),
        "Portal integration requires localhost accounts"
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let api = ApiClient::new(&fixture.api_base, Some(fixture.publisher.token.clone()))?;
    let room = runtime
        .block_on(api.rooms())?
        .into_iter()
        .find(|room| room.id == fixture.room_id)
        .context("Fixture room missing")?;
    let publisher = start(&fixture.api_base, &fixture.publisher);
    let viewer = start(&fixture.api_base, &fixture.viewer);
    let result = (|| -> Result<()> {
        wait_ready(&publisher.0)?;
        wait_ready(&viewer.0)?;
        join(&publisher.0, &room)?;
        send(&viewer.0, Command::JoinRoom(room.clone()))?;
        let mut joined = false;
        let mut publisher_present = false;
        wait(&viewer.0, "viewer joined with publisher", |event| {
            match event {
                Event::Joined(joined_room) if joined_room.id == room.id => joined = true,
                Event::Participants(peers) => {
                    publisher_present = peers
                        .iter()
                        .any(|peer| peer.identity.contains(&fixture.publisher.user_id))
                }
                _ => {}
            }
            joined && publisher_present
        })?;
        let peers = wait(
            &publisher.0,
            "viewer participant",
            |event| matches!(event, Event::Participants(peers) if peers.iter().any(|peer| peer.identity.contains(&fixture.viewer.user_id))),
        )?;
        let Event::Participants(peers) = peers else {
            unreachable!()
        };
        let viewer_identity = peers
            .into_iter()
            .find(|peer| peer.identity.contains(&fixture.viewer.user_id))
            .context("Viewer identity missing")?
            .identity;
        ensure!(
            publisher.0.shared.local_camera.lock().unwrap().is_none()
                && viewer.0.shared.local_camera.lock().unwrap().is_none(),
            "Test unexpectedly opened a camera"
        );
        println!(
            "Select the controlled {mode} in the native portal chooser; no desktop clicks are automated by this test."
        );
        send(&publisher.0, Command::Share { window_only })?;
        let sharing = wait_for(
            &publisher.0,
            "portal source selection",
            Duration::from_secs(90),
            |event| matches!(event, Event::Sharing { active: true, .. }),
        )?;
        let Event::Sharing {
            target: Some(target),
            ..
        } = sharing
        else {
            bail!("Portal supplied no capture target")
        };
        ensure!(
            window_only || target.output_name.is_some(),
            "Monitor cannot be identified for scoped remote control"
        );
        ensure!(
            !window_only || target.output_name.is_none(),
            "Window share unexpectedly became a monitor input target"
        );
        wait(
            &viewer.0,
            "remote screen publication",
            |event| matches!(event, Event::RemoteShare { identity, active: true } if identity.contains(&fixture.publisher.user_id)),
        )?;
        let observed = wait_screen(&publisher.0, &viewer.0)?;
        if let Ok(expected) = std::env::var("HOPP_DESKTOP_EXPECT_SIZE") {
            let (width, height) = expected
                .split_once('x')
                .context("Expected source size must be WIDTHxHEIGHT")?;
            ensure!(
                observed.source == [width.parse::<u32>()?, height.parse::<u32>()?],
                "Portal selected an unexpected source: {observed:?}"
            );
        }
        println!(
            "PORTAL FRAMES: {}x{} captured -> {}x{} decoded",
            observed.source[0], observed.source[1], observed.decoded[0], observed.decoded[1],
        );
        // Annotations work before any input-control grant.
        let path_id = 84;
        for event in [
            InputEvent::DrawStart(PathPoint {
                point: Point { x: 0.3, y: 0.3 },
                path_id,
            }),
            InputEvent::DrawAddPoint(Point { x: 0.6, y: 0.5 }),
            InputEvent::DrawEnd(Point { x: 0.8, y: 0.7 }),
        ] {
            send(&viewer.0, Command::SendInput(event))?;
        }
        let mut started = false;
        let mut added = false;
        let mut ended = false;
        wait(&publisher.0, "authenticated annotation packets", |event| {
            if let Event::Drawing { sender, event } = event
                && canonical_identity(sender) == canonical_identity(&viewer_identity)
            {
                match event {
                    InputEvent::DrawStart(start) if start.path_id == path_id => started = true,
                    InputEvent::DrawAddPoint(point) if point.x == 0.6 && point.y == 0.5 => {
                        added = true
                    }
                    InputEvent::DrawEnd(point) if point.x == 0.8 && point.y == 0.7 => ended = true,
                    _ => {}
                }
            }
            started && added && ended
        })?;
        send(
            &viewer.0,
            Command::SendInput(InputEvent::DrawClearPath { path_id }),
        )?;
        wait(
            &publisher.0,
            "annotation clear",
            |event| matches!(event, Event::Drawing { sender, event: InputEvent::DrawClearPath { path_id: cleared } } if canonical_identity(sender) == canonical_identity(&viewer_identity) && *cleared == path_id),
        )?;
        if !window_only {
            send(&publisher.0, Command::GrantControl(viewer_identity.clone()))?;
            wait(
                &publisher.0,
                "host granted control",
                |event| matches!(event, Event::ControlGranted(Some(identity)) if canonical_identity(identity) == canonical_identity(&viewer_identity)),
            )?;
            wait(&viewer.0, "viewer received control", |event| {
                matches!(event, Event::PeerControl { enabled: true })
            })?;
            if clipboard {
                clipboard_roundtrip(&publisher.0, &viewer.0)?;
            }
            send(&viewer.0, Command::RevokeControl)?;
            wait(&viewer.0, "viewer released control", |event| {
                matches!(event, Event::PeerControl { enabled: false })
            })?;
            wait(&publisher.0, "remote release revoked host grant", |event| {
                matches!(event, Event::ControlGranted(None))
            })?;
        }
        send(&publisher.0, Command::StopShare)?;
        wait(&publisher.0, "local screen stopped", |event| {
            matches!(event, Event::Sharing { active: false, .. })
        })?;
        wait(
            &viewer.0,
            "remote screen stopped",
            |event| matches!(event, Event::RemoteShare { identity, active: false } if identity.contains(&fixture.publisher.user_id)),
        )?;
        ensure!(
            viewer.0.shared.remote.lock().unwrap().is_none(),
            "StopShare retained a media preview"
        );
        Ok(())
    })();
    // Leave both real workers even after a chooser cancellation or assertion.
    let mut cleanup_error = None;
    for engine in [&viewer.0, &publisher.0] {
        if let Err(error) = leave(engine) {
            cleanup_error.get_or_insert(error);
        }
    }
    let clean = runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while tokio::time::Instant::now() < deadline {
            let presence =
                tokio::time::timeout(Duration::from_secs(2), api.rooms_presence()).await??;
            if presence.values().all(|users| {
                !users.contains(&fixture.publisher.user_id)
                    && !users.contains(&fixture.viewer.user_id)
            }) {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("Both workers did not clear backend room presence")
    });
    result?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    clean?;
    println!(
        "PORTAL ENGINE PASS: real chooser, PipeWire BGRA, native decoded screen, annotations, scoped grant/release when monitor, preview and backend-presence cleanup; clipboard={clipboard}."
    );
    Ok(())
}
