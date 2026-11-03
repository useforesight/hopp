use std::{
    os::fd::OwnedFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
};
use pipewire::{self as pw, properties::properties, spa};
use serde::Deserialize;

use crate::{
    convert::{ScreenSlot, packed_i420},
    engine::Event,
    wake::Sender,
};

#[derive(Debug, Clone)]
pub struct CaptureTarget {
    pub output_name: Option<String>,
    pub logical_width: u32,
    pub logical_height: u32,
    /// Top-left of the shared monitor in Hyprland's global logical layout.
    pub logical_origin: Option<(f64, f64)>,
}

/// The pointer's global logical position, read straight from Hyprland's IPC socket
/// (cheap enough to poll while sharing, unlike spawning `hyprctl`).
pub fn cursor_position() -> Option<(f64, f64)> {
    use std::io::{Read, Write};
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let path = std::path::Path::new(&runtime)
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    let mut socket = std::os::unix::net::UnixStream::connect(path).ok()?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_millis(50)))
        .ok()?;
    socket.write_all(b"j/cursorpos").ok()?;
    let mut reply = Vec::with_capacity(64);
    socket.take(4096).read_to_end(&mut reply).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&reply).ok()?;
    Some((value["x"].as_f64()?, value["y"].as_f64()?))
}

/// A global pointer position as a point on the shared monitor, or `None` when the
/// pointer is on another output (nothing is published then).
pub fn cursor_on_target(
    target: &CaptureTarget,
    position: (f64, f64),
) -> Option<crate::protocol::Point> {
    target.output_name.as_ref()?;
    let (left, top) = target.logical_origin?;
    let (width, height) = (
        f64::from(target.logical_width),
        f64::from(target.logical_height),
    );
    if width <= 0. || height <= 0. {
        return None;
    }
    let point = crate::protocol::Point {
        x: (position.0 - left) / width,
        y: (position.1 - top) / height,
    };
    point.valid().then_some(point)
}

pub struct Capture {
    session: Option<PortalSession>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pointer: Option<std::thread::JoinHandle<()>>,
    closed: tokio::task::JoinHandle<()>,
    pub target: CaptureTarget,
    /// Latest host pointer position on the shared monitor, when it moved.
    pub cursor: Arc<std::sync::Mutex<Option<crate::protocol::Point>>>,
}

/// Viewers follow the sharer's pointer; poll it at the video frame rate on a thread of
/// its own, so a slow compositor reply never holds up the session.
const POINTER_INTERVAL: Duration = Duration::from_millis(33);

fn watch_pointer(
    target: CaptureTarget,
    cursor: Arc<std::sync::Mutex<Option<crate::protocol::Point>>>,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
) {
    let mut last = None;
    while !stop.load(Ordering::Acquire) {
        if let Some(point) =
            cursor_position().and_then(|position| cursor_on_target(&target, position))
            && last.is_none_or(|last: crate::protocol::Point| {
                (last.x - point.x).abs() > 0.0005 || (last.y - point.y).abs() > 0.0005
            })
        {
            last = Some(point);
            if let Ok(mut slot) = cursor.lock() {
                *slot = Some(point);
            }
            events.wake();
        }
        std::thread::sleep(POINTER_INTERVAL);
    }
}

/// The session is closed when its owner is dropped. Hyprland portal 1.4.1
/// blocks its D-Bus loop while selecting a source, so cancellation must drain
/// that response and check its flag before starting a stream. Session.Close
/// alone cannot dismiss that backend's synchronous chooser.
struct PortalSession(Arc<Session<Screencast>>);

impl std::ops::Deref for PortalSession {
    type Target = Session<Screencast>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        let session = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}

impl Capture {
    #[cfg(test)]
    pub async fn start(
        frames: ScreenSlot,
        events: Sender<Event>,
        window_only: bool,
    ) -> Result<Self> {
        Self::start_cancellable(
            frames,
            events,
            window_only,
            Arc::new(AtomicBool::new(false)),
        )
        .await
    }

    pub async fn start_cancellable(
        frames: ScreenSlot,
        events: Sender<Event>,
        window_only: bool,
        canceled: Arc<AtomicBool>,
    ) -> Result<Self> {
        let portal = Screencast::new().await?;
        let session = PortalSession(Arc::new(portal.create_session(Default::default()).await?));
        ensure_not_canceled(&canceled)?;
        let request = portal
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_sources(if window_only {
                        ashpd::enumflags2::BitFlags::from(SourceType::Window)
                    } else {
                        ashpd::enumflags2::BitFlags::from(SourceType::Monitor)
                    })
                    .set_multiple(false)
                    .set_cursor_mode(CursorMode::Embedded)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await
            .and_then(|request| request.response());
        if let Err(error) = request {
            let _ = session.close().await;
            return Err(error.into());
        }
        ensure_not_canceled(&canceled)?;
        let result = portal.start(&session, None, Default::default()).await;
        let response = match result.and_then(|request| request.response()) {
            Ok(response) => response,
            Err(error) => {
                let _ = session.close().await;
                return Err(error.into());
            }
        };
        let stream = response.streams().first().context("No source selected")?;
        ensure_not_canceled(&canceled)?;
        let node = stream.pipe_wire_node_id();
        let target = capture_target(stream, window_only);
        let fd = match portal
            .open_pipe_wire_remote(&session, Default::default())
            .await
        {
            Ok(fd) => fd,
            Err(error) => {
                let _ = session.close().await;
                return Err(error.into());
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        ensure_not_canceled(&canceled)?;
        let worker_stop = stop.clone();
        let worker_events = events.clone();
        let worker = std::thread::spawn(move || {
            if let Err(error) = stream_frames(fd, node, frames, worker_stop, worker_events.clone())
            {
                let _ = worker_events.send(Event::Error(format!("Screen capture: {error:#}")));
                let _ = worker_events.send(Event::CaptureClosed);
            }
        });
        let cursor = Arc::new(std::sync::Mutex::new(None));
        let pointer = target.output_name.is_some().then(|| {
            let (target, cursor, stop, events) =
                (target.clone(), cursor.clone(), stop.clone(), events.clone());
            std::thread::spawn(move || watch_pointer(target, cursor, stop, events))
        });
        let closure_session = session.0.clone();
        let closure_stop = stop.clone();
        let closed = tokio::spawn(async move {
            use futures_util::StreamExt;
            let closure = closure_session.receive_closed().await;
            if let Ok(mut closure) = closure
                && closure.next().await.is_some()
            {
                closure_stop.store(true, Ordering::Release);
                let _ = events.send(Event::CaptureClosed);
            }
        });
        Ok(Self {
            session: Some(session),
            stop,
            worker: Some(worker),
            pointer,
            closed,
            target,
            cursor,
        })
    }

    pub async fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.closed.abort();
        if let Some(session) = self.session.take() {
            let _ = session.close().await;
        }
        let workers = [self.worker.take(), self.pointer.take()];
        let _ = tokio::task::spawn_blocking(move || {
            for worker in workers.into_iter().flatten() {
                let _ = worker.join();
            }
        })
        .await;
    }
}

fn ensure_not_canceled(canceled: &AtomicBool) -> Result<()> {
    if canceled.load(Ordering::Acquire) {
        bail!("Sharing canceled");
    }
    Ok(())
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.closed.abort();
        self.session.take();
    }
}

#[derive(Deserialize)]
struct Monitor {
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
    #[serde(default)]
    transform: u32,
}

fn capture_target(stream: &ashpd::desktop::screencast::Stream, window_only: bool) -> CaptureTarget {
    let (w, h) = stream.size().unwrap_or((0, 0));
    let mut target = CaptureTarget {
        output_name: None,
        logical_width: w.max(0) as u32,
        logical_height: h.max(0) as u32,
        logical_origin: None,
    };
    if window_only
        || matches!(
            stream.source_type(),
            Some(SourceType::Window | SourceType::Virtual)
        )
    {
        return target;
    }
    let monitors: Vec<Monitor> = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice(&output.stdout).ok())
        .unwrap_or_default();
    let monitor = identify_monitor(&monitors, stream.id(), stream.position());
    if let Some(monitor) = monitor
        && monitor.scale.is_finite()
        && monitor.scale > 0.0
    {
        target.output_name = Some(monitor.name.clone());
        target.logical_origin = Some((f64::from(monitor.x), f64::from(monitor.y)));
        let (width, height) = if monitor.transform % 2 == 1 {
            (monitor.height, monitor.width)
        } else {
            (monitor.width, monitor.height)
        };
        target.logical_width = (f64::from(width) / monitor.scale).round() as u32;
        target.logical_height = (f64::from(height) / monitor.scale).round() as u32;
    }
    target
}

/// An explicit name takes precedence over geometry. Ambiguous mirrored
/// outputs must never silently become a remote-input destination.
fn identify_monitor<'a>(
    monitors: &'a [Monitor],
    id: Option<&str>,
    position: Option<(i32, i32)>,
) -> Option<&'a Monitor> {
    if let Some(monitor) = id.and_then(|id| monitors.iter().find(|m| m.name == id)) {
        return Some(monitor);
    }
    if let Some(position) = position {
        let mut matching = monitors.iter().filter(|m| position == (m.x, m.y));
        if let Some(first) = matching.next() {
            return matching.next().is_none().then_some(first);
        }
    }
    (monitors.len() == 1).then(|| &monitors[0])
}

struct StreamData {
    format: spa::param::video::VideoInfoRaw,
    frames: ScreenSlot,
    events: Sender<Event>,
    last: Option<std::time::Instant>,
}

fn stream_frames(
    fd: OwnedFd,
    node: u32,
    frames: ScreenSlot,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopBox::new(None)?;
    let context = pw::context::ContextBox::new(mainloop.loop_(), None)?;
    let core = context.connect_fd(fd, None)?;
    let stream = pw::stream::StreamBox::new(
        &core,
        "Hopp screen sharing",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let failure_stop = stop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(StreamData {
            format: Default::default(),
            frames,
            events,
            last: None,
        })
        .state_changed(move |_, state, _, new| {
            if let pw::stream::StreamState::Error(error) = new {
                failure_stop.store(true, Ordering::Release);
                let _ = state
                    .events
                    .send(Event::Error(format!("PipeWire: {error}")));
                let _ = state.events.send(Event::CaptureClosed);
            }
        })
        .param_changed(|_, state, id, parameter| {
            if id == spa::param::ParamType::Format.as_raw()
                && let Some(parameter) = parameter
                && let Err(error) = state.format.parse(parameter)
            {
                let _ = state
                    .events
                    .send(Event::Error(format!("Capture format: {error}")));
            }
        })
        .process(|stream, state| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            // The encoder sends at most 30 fps; converting faster monitors' extra frames
            // would only be thrown away. The margin keeps a jittery 30 Hz source at 30.
            if state
                .last
                .is_some_and(|last| last.elapsed() < Duration::from_millis(25))
            {
                return;
            }
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            let size = state.format.size();
            let stride = data.chunk().stride();
            let offset = data.chunk().offset();
            let format = state.format.format();
            let Some(bytes) = data.data() else {
                return;
            };
            let red_first = format == spa::param::video::VideoFormat::RGBx
                || format == spa::param::video::VideoFormat::RGBA;
            if let Ok(frame) =
                packed_i420(size.width, size.height, stride, offset, bytes, red_first)
                && let Ok(mut slot) = state.frames.lock()
            {
                *slot = Some(frame);
                drop(slot);
                state.last = Some(std::time::Instant::now());
                state.events.wake();
            }
        })
        .register()?;
    let format = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBA
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: 1920,
                height: 1080
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 7680,
                height: 4320
            }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: 30, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: 30, denom: 1 }
        )
    );
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format),
    )?
    .0
    .into_inner();
    let mut parameters =
        [spa::pod::Pod::from_bytes(&bytes).context("Invalid capture format specification")?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut parameters,
    )?;
    while !stop.load(Ordering::Acquire) {
        if mainloop
            .loop_()
            .iterate(pw::loop_::Timeout::Finite(Duration::from_millis(30)))
            < 0
        {
            bail!("PipeWire capture loop stopped");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_cursor_is_published_only_on_the_shared_monitor() {
        // 3456×2160 at scale 2 placed right of a 1920-wide output.
        let target = CaptureTarget {
            output_name: Some("eDP-1".into()),
            logical_width: 1728,
            logical_height: 1080,
            logical_origin: Some((1920., 0.)),
        };
        let point = cursor_on_target(&target, (1920. + 864., 270.)).unwrap();
        assert!((point.x - 0.5).abs() < 1e-9 && (point.y - 0.25).abs() < 1e-9);
        assert!(
            cursor_on_target(&target, (100., 100.)).is_none(),
            "other output"
        );
        let window = CaptureTarget {
            output_name: None,
            logical_origin: None,
            ..target
        };
        assert!(
            cursor_on_target(&window, (2000., 100.)).is_none(),
            "window shares have no geometry"
        );
    }

    #[test]
    fn system_pipewire_initialization_and_loop_do_not_call_webrtc_lazy_stubs() -> Result<()> {
        // LiveKit's static WebRTC archive also exports weak pw_* wrappers.
        // These must be namespaced at link time: an uninitialized wrapper
        // otherwise shadows the real library and calls a null function pointer.
        // This opens no daemon connection, portal, camera, microphone or display.
        pw::init();
        let _loop = pw::main_loop::MainLoopBox::new(None)?;
        Ok(())
    }
    #[test]
    fn named_output_wins_and_mirrored_geometry_is_not_guessed() {
        let monitors = vec![
            Monitor {
                name: "DP-1".into(),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                transform: 0,
            },
            Monitor {
                name: "DP-2".into(),
                x: 0,
                y: 0,
                width: 5120,
                height: 2160,
                scale: 1.6,
                transform: 0,
            },
        ];
        assert_eq!(
            identify_monitor(&monitors, Some("DP-2"), Some((0, 0)))
                .unwrap()
                .name,
            "DP-2"
        );
        assert!(identify_monitor(&monitors, None, Some((0, 0))).is_none());
        assert!(identify_monitor(&monitors, None, None).is_none());
        assert_eq!(
            identify_monitor(&monitors[..1], None, None).unwrap().name,
            "DP-1"
        );
    }

    /// Requires an unlocked desktop and an explicit choice in the real portal.
    /// Prints frame dimensions only; it never saves captured screen contents.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "opens the real screen-sharing chooser; requires HOPP_PORTAL_TEST=window or monitor"]
    async fn portal_pipewire_capture_stops_cleanly() -> Result<()> {
        let mode =
            std::env::var("HOPP_PORTAL_TEST").context("Set HOPP_PORTAL_TEST=window or monitor")?;
        if !matches!(mode.as_str(), "window" | "monitor") {
            bail!("Choose window or monitor");
        }
        let lock = std::process::Command::new("hyprctl")
            .args(["-j", "locked"])
            .output()?;
        let lock: serde_json::Value = serde_json::from_slice(&lock.stdout)?;
        if lock["locked"] != false {
            bail!("Unlock the desktop before the portal test");
        }
        let frames = crate::convert::screen_slot();
        let (tx, rx) = crate::wake::detached();
        let mut capture = tokio::time::timeout(
            Duration::from_secs(60),
            Capture::start(frames.clone(), tx, mode == "window"),
        )
        .await
        .context("Screen chooser timed out")??;
        let frame = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                while let Ok(event) = rx.try_recv() {
                    if let Event::Error(error) = event {
                        bail!("{error}");
                    }
                }
                if let Some(frame) = frames.lock().ok().and_then(|mut slot| slot.take()) {
                    break Ok::<_, anyhow::Error>(frame);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .context("PipeWire did not deliver a frame")??;
        assert_eq!(
            frame.buffer.data().0.len(),
            frame.width as usize * frame.height as usize
        );
        println!("Portal delivered {}×{} as I420", frame.width, frame.height);
        let started = std::time::Instant::now();
        capture.stop().await;
        assert!(capture.worker.is_none());
        assert!(capture.session.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "Capture did not stop promptly"
        );
        if let Ok(mut slot) = frames.lock() {
            *slot = None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            frames.lock().unwrap().is_none(),
            "A stopped capture refilled its frame slot"
        );
        Ok(())
    }
}
