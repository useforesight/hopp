use super::mixer::{MixerHandle, OutputHeartbeat, SharedProcessor};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

const OUTPUT_STALL_TIMEOUT: Duration = Duration::from_secs(3);
const OUTPUT_MONITOR_INTERVAL: Duration = Duration::from_secs(1);
const OUTPUT_RECONNECT_SPACING: Duration = Duration::from_secs(5);
const MAX_OUTPUT_RECONNECTS_PER_STALL: u32 = 5;

#[derive(Error, Debug)]
pub enum PlayerError {
    #[error("Failed to create mixer: {0}")]
    Mixer(String),
    #[error("Failed to start device monitor: {0}")]
    DeviceMonitor(String),
}

struct PlayerInner {
    mixer: MixerHandle,
    processor: SharedProcessor,
    _device_monitor: super::device_monitor::DeviceMonitor,
    output_monitor_stop: Arc<AtomicBool>,
}

impl Drop for PlayerInner {
    fn drop(&mut self) {
        self.output_monitor_stop.store(true, Ordering::Relaxed);
    }
}

fn spawn_output_monitor(
    heartbeat: Arc<OutputHeartbeat>,
    stop_flag: Arc<AtomicBool>,
    proxy: EventLoopProxy<UserEvent>,
) {
    std::thread::spawn(move || {
        let mut reconnects_this_stall = 0;
        let mut time_since_reconnect = OUTPUT_RECONNECT_SPACING;
        while !stop_flag.load(Ordering::Relaxed) {
            std::thread::sleep(OUTPUT_MONITOR_INTERVAL);
            time_since_reconnect += OUTPUT_MONITOR_INTERVAL;
            if heartbeat.time_since_last_callback() < OUTPUT_STALL_TIMEOUT {
                reconnects_this_stall = 0;
                continue;
            }
            if reconnects_this_stall >= MAX_OUTPUT_RECONNECTS_PER_STALL
                || time_since_reconnect < OUTPUT_RECONNECT_SPACING
            {
                continue;
            }
            reconnects_this_stall += 1;
            time_since_reconnect = Duration::ZERO;
            let _ = proxy.send_event(UserEvent::AudioOutputStalled);
        }
    });
}

pub struct Player {
    inner: Option<PlayerInner>,
    event_loop_proxy: EventLoopProxy<UserEvent>,
}

impl Player {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            inner: None,
            event_loop_proxy: proxy,
        }
    }

    #[allow(unused_variables)]
    pub fn start(&mut self) -> Result<(), PlayerError> {
        log::info!("Player::start");
        let (mixer, processor) = MixerHandle::new().map_err(PlayerError::Mixer)?;

        let device_monitor = super::device_monitor::DeviceMonitor::new(
            super::device_monitor::DeviceKind::Output,
            self.event_loop_proxy.clone(),
        )
        .map_err(|e| PlayerError::DeviceMonitor(e.to_string()))?;

        let output_monitor_stop = Arc::new(AtomicBool::new(false));
        spawn_output_monitor(
            mixer.output_heartbeat(),
            output_monitor_stop.clone(),
            self.event_loop_proxy.clone(),
        );

        self.inner = Some(PlayerInner {
            mixer,
            processor,
            _device_monitor: device_monitor,
            output_monitor_stop,
        });
        Ok(())
    }

    pub fn stop(&mut self) {
        log::info!("Player::stop");
        self.inner = None;
    }

    pub fn mixer(&self) -> Option<&MixerHandle> {
        self.inner.as_ref().map(|i| &i.mixer)
    }

    pub fn processor(&self) -> Option<SharedProcessor> {
        self.inner.as_ref().map(|i| i.processor.clone())
    }
}
