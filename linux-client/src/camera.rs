//! V4L2 camera capture, on its own cancellable thread.
use crate::{
    convert::{CAMERA_PREVIEW_EDGE, display_bgra},
    device_health::CameraWatch,
    engine::Event,
    frames::{FrameSlot, VideoFrame},
    wake::Sender,
};
use anyhow::{Context, Result, bail};
use livekit::webrtc::{
    native::yuv_helper,
    video_frame::{I420Buffer, VideoFrame as RtcFrame, VideoRotation},
    video_source::native::NativeVideoSource,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::sync_channel,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use v4l::{
    Device, Format, FourCC,
    buffer::Type,
    io::{mmap::Stream, traits::CaptureStream},
    video::Capture,
};

#[derive(Debug, Clone)]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
}

/// Cameras with a pixel format Hopp reads, in the order to try them when none is
/// chosen: loopback devices first, since an app that processes a camera (Omalive, OBS)
/// publishes its picture there and usually holds the camera itself exclusively.
pub fn devices() -> Vec<CameraDevice> {
    let mut devices: Vec<_> = v4l::context::enum_devices()
        .into_iter()
        .filter_map(|node| {
            let path = node.path().to_string_lossy().into_owned();
            let device = Device::with_path(&path).ok()?;
            let formats = device.enum_formats().ok()?;
            if !formats
                .iter()
                .any(|f| matches!(&f.fourcc.repr, b"MJPG" | b"YUYV" | b"BGR3" | b"RGB3"))
            {
                return None;
            }
            let loopback = device
                .query_caps()
                .is_ok_and(|caps| caps.driver.contains("loopback"));
            Some((
                !loopback,
                node.index(),
                CameraDevice {
                    id: path,
                    name: node.name().unwrap_or_else(|| "Camera".into()),
                },
            ))
        })
        .collect();
    devices.sort_by_key(|(physical, index, _)| (*physical, *index));
    devices.into_iter().map(|(_, _, device)| device).collect()
}

/// Opens a camera at 1280×720 or the nearest size it offers.
fn open(path: &str) -> Result<(Format, Stream<'static>)> {
    let device = Device::with_path(path).context("Opening camera")?;
    let formats = device.enum_formats()?;
    let fourcc = [*b"MJPG", *b"YUYV", *b"BGR3", *b"RGB3"]
        .into_iter()
        .find(|wanted| formats.iter().any(|f| f.fourcc.repr == *wanted))
        .context("Camera has no supported pixel format")?;
    let format = device.set_format(&Format::new(1280, 720, FourCC::new(&fourcc)))?;
    if format.width == 0
        || format.height == 0
        || format.width as usize * format.height as usize > 7680 * 4320
    {
        bail!("Camera returned unsupported dimensions");
    }
    let mut stream = Stream::with_buffers(&device, Type::VideoCapture, 2)?;
    stream.set_timeout(Duration::from_millis(250));
    Ok((format, stream))
}

pub struct CameraCapture {
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    preview: FrameSlot,
}
impl CameraCapture {
    pub fn start(
        selected: Option<String>,
        preview: FrameSlot,
        source: NativeVideoSource,
        events: Sender<Event>,
    ) -> Result<Self> {
        // A chosen camera is the only one tried; otherwise the first that opens, so a
        // camera another app holds is skipped.
        let candidates = match selected {
            Some(path) => vec![path],
            None => devices().into_iter().map(|d| d.id).collect(),
        };
        if candidates.is_empty() {
            bail!("No supported V4L2 camera found");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let worker_failed = failed.clone();
        let worker_stop = stop.clone();
        let worker_preview = preview.clone();
        let (ready_tx, ready_rx) = sync_channel(1);
        let thread = thread::Builder::new()
            .name("hopp-camera".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let mut failure = None;
                    let mut opened = None;
                    for path in &candidates {
                        match open(path) {
                            Ok(camera) => {
                                opened = Some((path.clone(), camera));
                                break;
                            }
                            Err(error) => {
                                crate::logger::line(format!("Camera {path}: {error:#}"));
                                failure = Some(error.context(path.clone()));
                            }
                        }
                    }
                    let Some((path, (mut format, stream))) = opened else {
                        return Err(failure.expect("at least one camera was tried"));
                    };
                    let _ = ready_tx.send(Ok(()));
                    let mut stream = Some(stream);
                    let mut watch = CameraWatch::new(Instant::now());
                    while !worker_stop.load(Ordering::Relaxed) {
                        let Some(live) = stream.as_mut() else { break };
                        let problem = match live.next() {
                            Ok((data, metadata)) => {
                                // Truncated and undecodable frames don't count: a camera
                                // sending only those stalls like one sending nothing.
                                let used = metadata.bytesused as usize;
                                if let Some(frame) =
                                    data.get(..used).and_then(|data| decode(data, &format).ok())
                                {
                                    watch.frame(Instant::now());
                                    publish(frame, &source, &worker_preview, &events);
                                }
                                None
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                                ) =>
                            {
                                watch
                                    .stalled(Instant::now())
                                    .then(|| "stopped sending video".to_owned())
                            }
                            Err(error) => Some(format!("failed: {error}")),
                        };
                        let Some(problem) = problem else { continue };
                        // Closed before reopening: most cameras open only once at a time.
                        drop(stream.take());
                        match reopen(&path, problem, &mut watch, &worker_stop)? {
                            Some((new_format, new_stream)) => {
                                format = new_format;
                                stream = Some(new_stream);
                            }
                            None => break,
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result
                    && !worker_stop.load(Ordering::Acquire)
                {
                    worker_failed.store(true, Ordering::Release);
                    let _ = ready_tx.send(Err(format!("{error:#}")));
                    let _ = events.send(Event::Error(format!(
                        "Your camera stopped working ({error:#}). Turn it on again or choose \
                         another camera in Settings."
                    )));
                    let _ = events.send(Event::Camera(false));
                }
                if let Ok(mut latest) = worker_preview.lock() {
                    *latest = None;
                }
            })?;
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                failed,
                thread: Some(thread),
                preview,
            }),
            result => {
                stop.store(true, Ordering::Relaxed);
                // A device ioctl can hang in a broken driver. Do not block the UI
                // on an unready worker; once it returns it observes the stop flag.
                drop(thread);
                bail!("Could not start camera: {result:?}");
            }
        }
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}
impl Drop for CameraCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Ok(mut latest) = self.preview.lock() {
            *latest = None;
        }
    }
}

/// Send a frame to the call and a small copy to your own tile.
fn publish(
    frame: VideoFrame,
    source: &NativeVideoSource,
    preview: &FrameSlot,
    events: &Sender<Event>,
) {
    let mut buffer = I420Buffer::new(frame.width, frame.height);
    let (sy, su, sv) = buffer.strides();
    let (y, u, v) = buffer.data_mut();
    yuv_helper::argb_to_i420(
        &frame.bgra,
        frame.width * 4,
        y,
        sy,
        u,
        su,
        v,
        sv,
        frame.width as i32,
        frame.height as i32,
    );
    let small = display_bgra(&mut buffer, Some(CAMERA_PREVIEW_EDGE));
    source.capture_frame(&RtcFrame::new(VideoRotation::VideoRotation0, buffer));
    if let Ok(mut latest) = preview.lock() {
        *latest = small;
    }
    events.wake();
}

/// Reopen a camera that stalled or failed, waiting longer before each try, as choosing it
/// again by hand would. `Ok(None)` when the camera is being turned off meanwhile; an error
/// once the tries are used up, which turns the camera off.
fn reopen(
    path: &str,
    problem: String,
    watch: &mut CameraWatch,
    stop: &AtomicBool,
) -> Result<Option<(Format, Stream<'static>)>> {
    let mut last_error = None;
    loop {
        let Some(delay) = watch.retry() else {
            let detail = last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default();
            bail!("it {problem}, and reopening it didn't help{detail}");
        };
        crate::logger::line(format!("Camera {path} {problem}; reopening in {delay:?}"));
        let until = Instant::now() + delay;
        while Instant::now() < until {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(50));
        }
        match open(path) {
            Ok(camera) => {
                watch.reopened(Instant::now());
                crate::logger::line(format!("Camera {path} reopened"));
                return Ok(Some(camera));
            }
            Err(error) => {
                crate::logger::line(format!("Camera {path} didn't reopen: {error:#}"));
                last_error = Some(format!("{error:#}"));
            }
        }
    }
}

fn decode(data: &[u8], format: &Format) -> Result<VideoFrame> {
    let width = format.width;
    let height = format.height;
    let pixels = width as usize * height as usize;
    let mut bgra = Vec::with_capacity(pixels * 4);
    match &format.fourcc.repr {
        b"MJPG" => {
            let image =
                image::load_from_memory_with_format(data, image::ImageFormat::Jpeg)?.to_rgb8();
            let (width, height) = image.dimensions();
            if width as usize * height as usize > 7680 * 4320 {
                bail!("Camera JPEG exceeds dimension limit");
            }
            for pixel in image.pixels() {
                bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
            return Ok(VideoFrame {
                width,
                height,
                bgra,
            });
        }
        b"YUYV" => {
            if !width.is_multiple_of(2) {
                bail!("Camera YUYV width must be even");
            }
            let stride = (format.stride as usize).max(width as usize * 2);
            if data.len() < stride * height as usize {
                bail!("Truncated YUYV frame");
            }
            for row in data[..stride * height as usize].chunks_exact(stride) {
                for group in row[..width as usize * 2].as_chunks::<4>().0 {
                    let u = group[1] as i32 - 128;
                    let v = group[3] as i32 - 128;
                    for yy in [group[0], group[2]] {
                        let y = (yy as i32 - 16).max(0);
                        let r = ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
                        let g = ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
                        let b = ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8;
                        bgra.extend_from_slice(&[b, g, r, 255]);
                    }
                }
            }
        }
        b"BGR3" | b"RGB3" => {
            let stride = (format.stride as usize).max(width as usize * 3);
            if data.len() < stride * height as usize {
                bail!("Truncated RGB camera frame");
            }
            for row in data[..stride * height as usize].chunks_exact(stride) {
                for p in row[..width as usize * 3].as_chunks::<3>().0 {
                    if format.fourcc.repr == *b"BGR3" {
                        bgra.extend_from_slice(&[p[0], p[1], p[2], 255]);
                    } else {
                        bgra.extend_from_slice(&[p[2], p[1], p[0], 255]);
                    }
                }
            }
        }
        _ => bail!("Unsupported camera pixel format: {}", format.fourcc),
    }
    Ok(VideoFrame {
        width,
        height,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yuyv_capture_respects_row_padding_and_limited_range() {
        let mut format = Format::new(2, 2, FourCC::new(b"YUYV"));
        format.stride = 8;
        let data = [
            16, 128, 235, 128, 0, 0, 0, 0, 235, 128, 16, 128, 255, 255, 255, 255,
        ];
        let frame = decode(&data, &format).unwrap();
        assert_eq!(
            frame.bgra,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255
            ]
        );
        assert!(decode(&data[..11], &format).is_err());
    }
}
