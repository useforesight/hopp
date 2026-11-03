//! Conversions between capture, codec and display pixel layouts, done once per frame
//! on the thread that produces it.
use anyhow::{Result, bail};
use livekit::webrtc::{
    native::yuv_helper,
    prelude::I420Buffer,
    video_frame::{VideoBuffer, VideoFormatType, native::VideoFrameBufferExt},
};
use std::sync::{Arc, Mutex};

use crate::frames::VideoFrame;

/// Camera tiles are at most 160 logical pixels at up to 2× scale, square-cropped from
/// 16:9, so a short side of 360 is plenty; the call window never needs the full 720p.
pub const CAMERA_PREVIEW_EDGE: u32 = 360;

/// Convert a decoded frame to BGRA for display, first shrinking it so its short side is
/// at most `short_edge` (keeping the aspect ratio). Smaller frames cost less to convert,
/// upload and composite every frame.
pub fn display_bgra(buffer: &mut I420Buffer, short_edge: Option<u32>) -> Option<VideoFrame> {
    let (width, height) = (buffer.width(), buffer.height());
    let pixels = width as usize * height as usize;
    if pixels == 0 || pixels > 7680 * 4320 {
        return None;
    }
    let short = width.min(height);
    let scaled = match short_edge {
        Some(edge) if short > edge => {
            let scale = edge as f64 / short as f64;
            let even = |side: u32| (((side as f64 * scale) as u32) / 2 * 2).max(2);
            Some(buffer.scale(even(width) as i32, even(height) as i32))
        }
        _ => None,
    };
    let buffer = scaled.as_ref().unwrap_or(buffer);
    let (width, height) = (buffer.width(), buffer.height());
    let mut bgra = vec![0; width as usize * height as usize * 4];
    buffer.to_argb(
        VideoFormatType::ARGB,
        &mut bgra,
        width * 4,
        width as i32,
        height as i32,
    );
    Some(VideoFrame {
        width,
        height,
        bgra,
    })
}

/// A captured screen frame, already in the encoder's I420 layout (1.5 bytes a pixel
/// instead of 4, and converted once, on the capture thread).
pub struct ScreenFrame {
    pub width: u32,
    pub height: u32,
    pub buffer: I420Buffer,
}

pub type ScreenSlot = Arc<Mutex<Option<ScreenFrame>>>;

pub fn screen_slot() -> ScreenSlot {
    Arc::new(Mutex::new(None))
}

impl ScreenFrame {
    /// Tightly packed BGRA, as produced by synthetic test patterns.
    #[cfg(test)]
    pub fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Result<Self> {
        packed_i420(width, height, width as i32 * 4, 0, bgra, false)
    }
}

/// PipeWire frames can have row padding and a nonzero chunk offset. Never
/// interpret the entire mapped allocation as tightly packed image pixels.
/// `red_first` selects RGBx/RGBA byte order; otherwise BGRx/BGRA. Alpha is ignored.
pub fn packed_i420(
    width: u32,
    height: u32,
    stride: i32,
    offset: u32,
    data: &[u8],
    red_first: bool,
) -> Result<ScreenFrame> {
    let row = (width as usize).saturating_mul(4);
    let pixels = (width as usize).saturating_mul(height as usize);
    if pixels == 0 || pixels > 7680 * 4320 || stride < 0 || (stride as usize) < row {
        bail!("Unsupported capture dimensions or stride");
    }
    let stride = stride as usize;
    let offset = offset as usize;
    let required = offset
        .checked_add((height as usize - 1).saturating_mul(stride))
        .and_then(|end| end.checked_add(row));
    if required.is_none_or(|end| end > data.len()) {
        bail!("Truncated PipeWire frame");
    }
    // libyuv wants a whole stride for the last row too; the last row's padding may be
    // missing from the mapping, so repack in that (rare) case.
    let packed;
    let (source, source_stride) = if offset + stride * height as usize <= data.len() {
        (&data[offset..], stride)
    } else {
        packed = (0..height as usize)
            .flat_map(|y| &data[offset + y * stride..offset + y * stride + row])
            .copied()
            .collect::<Vec<u8>>();
        (&packed[..], row)
    };
    let mut buffer = I420Buffer::new(width, height);
    let (stride_y, stride_u, stride_v) = buffer.strides();
    let (y, u, v) = buffer.data_mut();
    let convert = if red_first {
        yuv_helper::abgr_to_i420
    } else {
        yuv_helper::argb_to_i420
    };
    convert(
        source,
        source_stride as u32,
        y,
        stride_y,
        u,
        stride_u,
        v,
        stride_v,
        width as i32,
        height as i32,
    );
    Ok(ScreenFrame {
        width,
        height,
        buffer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_capture_converts_only_pixels_and_checks_truncation() {
        // Two rows of one pixel: red then blue, two bytes of offset and two of padding.
        let data = [0, 0, 255, 0, 0, 0, 99, 99, 0, 0, 255, 0, 99, 99];
        let frame = packed_i420(1, 2, 6, 2, &data, true).unwrap();
        let (y, _, _) = frame.buffer.data();
        // BT.601 luma: red ≈ 82, blue ≈ 41.
        assert!(y[0].abs_diff(82) <= 2 && y[1].abs_diff(41) <= 2, "{y:?}");
        // The same bytes read as BGRx swap red and blue.
        let swapped = packed_i420(1, 2, 6, 2, &data, false).unwrap();
        assert!(swapped.buffer.data().0[0].abs_diff(41) <= 2);
        // The last row may lack its padding; that still converts.
        let repacked = packed_i420(1, 2, 6, 2, &data[..12], true).unwrap();
        assert_eq!(repacked.buffer.data().0[..2], frame.buffer.data().0[..2]);
        assert!(packed_i420(1, 2, 6, 2, &data[..11], false).is_err());
        assert!(packed_i420(1, 2, -4, 0, &data, false).is_err());
    }

    #[test]
    fn camera_previews_shrink_to_the_tile_size_keeping_their_shape() {
        let frame =
            display_bgra(&mut I420Buffer::new(1280, 720), Some(CAMERA_PREVIEW_EDGE)).unwrap();
        assert_eq!((frame.width, frame.height), (640, 360));
        assert_eq!(frame.bgra.len(), 640 * 360 * 4);
        let portrait =
            display_bgra(&mut I420Buffer::new(720, 1280), Some(CAMERA_PREVIEW_EDGE)).unwrap();
        assert_eq!((portrait.width, portrait.height), (360, 640));
        // Already small, or the shared screen (no limit): untouched.
        let small =
            display_bgra(&mut I420Buffer::new(320, 180), Some(CAMERA_PREVIEW_EDGE)).unwrap();
        assert_eq!((small.width, small.height), (320, 180));
        let screen = display_bgra(&mut I420Buffer::new(4096, 1728), None).unwrap();
        assert_eq!((screen.width, screen.height), (4096, 1728));
    }
}
