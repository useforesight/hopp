//! Observe Hyprland's actual compositor lock state without acquiring a lock.
//! ext-session-lock controls a lock; hyprland-lock-notify observes one. logind's
//! LockedHint can remain false while Hyprland is locked, so it is not used.
use crate::{engine::Event, wake::Sender};
use anyhow::{Context, Result, bail};
use std::{
    io::ErrorKind,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::io::unix::AsyncFd;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    protocol::{wl_callback, wl_registry},
};
use wayland_protocols_hyprland::lock_notify::v1::client::{
    hyprland_lock_notification_v1::{self, HyprlandLockNotificationV1},
    hyprland_lock_notifier_v1::HyprlandLockNotifierV1,
};

#[derive(Clone, Copy)]
enum Barrier {
    Registry,
    InitialNotification,
}

struct State {
    events: Sender<Event>,
    manager: Option<HyprlandLockNotifierV1>,
    notification: Option<HyprlandLockNotificationV1>,
    manager_global: Option<u32>,
    observed: Option<bool>,
    initial_signal_received: bool,
    ready: bool,
    error: Option<String>,
    initial_ready: Option<tokio::sync::oneshot::Sender<()>>,
    privacy_locked: Arc<AtomicBool>,
}
impl State {
    fn observe(&mut self, locked: bool) {
        self.privacy_locked.store(locked, Ordering::Release);
        if self.observed == Some(locked) {
            return;
        }
        self.observed = Some(locked);
        let _ = self.events.send(if locked {
            Event::DesktopLocked
        } else {
            Event::DesktopUnlocked
        });
    }
}

/// Spawn this once on the engine's Tokio runtime and keep its JoinHandle.
/// Aborting the task drops the pending read guard and closes its own Wayland
/// connection. It does not affect the compositor's lock or any other client.
/// Initialization and connection failures conservatively emit DesktopLocked.
/// The caller should also start with sharing/control disabled until the first
/// DesktopUnlocked notification establishes the initial unlocked state.
#[cfg(test)]
pub async fn watch(events: Sender<Event>) -> Result<()> {
    watch_state(events, Arc::new(AtomicBool::new(true))).await
}

pub async fn watch_state(events: Sender<Event>, privacy_locked: Arc<AtomicBool>) -> Result<()> {
    watch_ready(events, None, privacy_locked).await
}

async fn watch_ready(
    events: Sender<Event>,
    initial_ready: Option<tokio::sync::oneshot::Sender<()>>,
    privacy_locked: Arc<AtomicBool>,
) -> Result<()> {
    let mut state = State {
        events,
        manager: None,
        notification: None,
        manager_global: None,
        observed: None,
        initial_signal_received: false,
        ready: false,
        error: None,
        initial_ready,
        privacy_locked,
    };
    state.observe(true);
    let result = watch_wayland(&mut state).await;
    if result.is_err() {
        state.observe(true);
    }
    result
}

async fn watch_wayland(state: &mut State) -> Result<()> {
    let connection = Connection::connect_to_env().context("Connecting compositor lock observer")?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, Barrier::Registry);
    let backend = connection.backend();
    let fd = AsyncFd::new(backend.poll_fd()).context("Watching compositor socket")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        queue
            .dispatch_pending(state)
            .context("Dispatching compositor lock notification")?;
        if let Some(error) = state.error.take() {
            bail!(error);
        }
        connection
            .flush()
            .context("Flushing compositor lock subscription")?;
        let Some(read_guard) = queue.prepare_read() else {
            continue;
        };
        let mut readable = tokio::select! {
            readable=fd.readable()=>readable.context("Reading compositor lock socket")?,
            _=tokio::time::sleep_until(deadline),if !state.ready=>bail!("Compositor lock subscription timed out"),
        };
        readable.clear_ready();
        match read_guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error).context("Compositor lock connection ended"),
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "hyprland_lock_notifier_v1" && version >= 1 => {
                let manager: HyprlandLockNotifierV1 = registry.bind(name, 1, qh, ());
                state.notification = Some(manager.get_lock_notification(qh, ()));
                state.manager = Some(manager);
                state.manager_global = Some(name);
            }
            wl_registry::Event::GlobalRemove { name } if state.manager_global == Some(name) => {
                state.observe(true);
                state.error = Some("Compositor lock notification interface disappeared".into());
            }
            _ => {}
        }
    }
}
delegate_noop!(State: ignore HyprlandLockNotifierV1);

impl Dispatch<HyprlandLockNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &HyprlandLockNotificationV1,
        event: hyprland_lock_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            hyprland_lock_notification_v1::Event::Locked => {
                state.initial_signal_received = true;
                state.observe(true);
            }
            hyprland_lock_notification_v1::Event::Unlocked => {
                state.initial_signal_received = true;
                state.observe(false);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, Barrier> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        barrier: &Barrier,
        connection: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            match barrier {
                Barrier::Registry => {
                    if state.manager.is_none() {
                        state.error =
                            Some("Hyprland compositor does not expose lock notifications".into());
                        return;
                    }
                    // The notification request was made while reading registry
                    // events. This second barrier waits for its initial response.
                    connection.display().sync(qh, Barrier::InitialNotification);
                }
                Barrier::InitialNotification => {
                    if !state.initial_signal_received {
                        state.observe(false);
                    }
                    state.ready = true;
                    if let Some(ready) = state.initial_ready.take() {
                        let _ = ready.send(());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_unknown_and_repeated_lock_do_not_reenable_control() {
        let (events, rx) = crate::wake::detached();
        let mut state = State {
            events,
            manager: None,
            notification: None,
            manager_global: None,
            observed: None,
            initial_signal_received: false,
            ready: false,
            error: None,
            initial_ready: None,
            privacy_locked: Arc::new(AtomicBool::new(true)),
        };
        state.observe(true);
        state.observe(true);
        state.observe(false);
        state.observe(false);
        state.observe(true);
        assert!(matches!(rx.try_recv().unwrap(), Event::DesktopLocked));
        assert!(matches!(rx.try_recv().unwrap(), Event::DesktopUnlocked));
        assert!(matches!(rx.try_recv().unwrap(), Event::DesktopLocked));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    #[ignore = "requires a running Hyprland session; observes only, does not lock or unlock"]
    async fn compositor_initial_state_and_cancel() -> Result<()> {
        let output = std::process::Command::new("hyprctl")
            .args(["-j", "locked"])
            .output()?;
        let expected = serde_json::from_slice::<serde_json::Value>(&output.stdout)?["locked"]
            .as_bool()
            .context("hyprctl did not return its lock state")?;
        let (events, rx) = crate::wake::detached();
        let (ready, initial_ready) = tokio::sync::oneshot::channel();
        let privacy_locked = Arc::new(AtomicBool::new(true));
        let task = tokio::spawn(watch_ready(events, Some(ready), privacy_locked.clone()));
        tokio::time::timeout(Duration::from_secs(5), initial_ready)
            .await
            .context("Compositor did not finish subscribing")??;
        let mut last = None;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            for event in rx.try_iter() {
                match event {
                    Event::DesktopLocked => last = Some(true),
                    Event::DesktopUnlocked => last = Some(false),
                    _ => {}
                }
            }
            if last == Some(expected) && !task.is_finished() {
                break;
            }
        }
        assert_eq!(last, Some(expected));
        assert_eq!(privacy_locked.load(Ordering::Acquire), expected);
        assert!(
            !task.is_finished(),
            "Compositor notification subscription ended"
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        // The convenience observer also starts conservatively and can be
        // cancelled without completing initialization or leaving a reader.
        let (events, rx) = crate::wake::detached();
        let task = tokio::spawn(watch(events));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(matches!(rx.try_recv()?, Event::DesktopLocked));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        Ok(())
    }
}
