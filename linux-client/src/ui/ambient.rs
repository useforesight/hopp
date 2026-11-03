//! The shared screen, stretched and blurred, behind the sharp copy: the way TV fills the
//! sides of a portrait video.
//!
//! The viewer stretches this tiny copy over the whole window. GPUI samples images
//! linearly, so a 48-pixel-wide copy blurred a few cells wide becomes a soft wash of the
//! screen's colours, laid out where they are on the screen, for almost no cost. The pixels
//! carry their own transparency, so the desktop behind the (transparent) viewer window,
//! frosted by Hyprland's blur, shows through.

/// Width of the ambient copy in pixels; height follows the screen's aspect ratio. Small,
/// so stretching it is itself most of the blur and rebuilding it is cheap.
pub const WIDTH: u32 = 48;
/// How much of the band is the shared screen; the rest is the frosted desktop behind.
pub const OPACITY: f32 = 0.6;
/// Keeps the bands quieter than the screen itself.
const DIM: f32 = 0.7;
/// Samples per cell edge; enough for a stable average without touching every pixel.
const SAMPLES: u32 = 8;
/// Box blur radius in cells.
const BLUR: i64 = 2;

/// Downsample tightly packed BGRA into a small, blurred, dimmed, translucent BGRA image.
pub fn ambient(width: u32, height: u32, bgra: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if width == 0 || height == 0 || bgra.len() < width as usize * height as usize * 4 {
        return None;
    }
    let w = WIDTH.min(width);
    let h = ((height as f32 * w as f32 / width as f32).round() as u32).clamp(1, 96);
    let mut cells = vec![[0f32; 3]; (w * h) as usize];
    for cy in 0..h {
        let (y0, y1) = span(cy, h, height);
        for cx in 0..w {
            let (x0, x1) = span(cx, w, width);
            let (mut sum, mut count) = ([0f32; 3], 0f32);
            for y in (y0..y1).step_by(((y1 - y0) / SAMPLES).max(1) as usize) {
                for x in (x0..x1).step_by(((x1 - x0) / SAMPLES).max(1) as usize) {
                    let i = (y as usize * width as usize + x as usize) * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += f32::from(bgra[i + channel]);
                    }
                    count += 1.;
                }
            }
            cells[(cy * w + cx) as usize] = sum.map(|total| total / count.max(1.));
        }
    }
    // Three box passes approximate a Gaussian.
    for _ in 0..3 {
        cells = box_blur(&cells, w, h);
    }
    let alpha = (OPACITY * 255.).round() as u8;
    let pixels = cells
        .iter()
        .flat_map(|[b, g, r]| {
            [
                (b * DIM).round() as u8,
                (g * DIM).round() as u8,
                (r * DIM).round() as u8,
                alpha,
            ]
        })
        .collect();
    Some((w, h, pixels))
}

/// Source pixel range covered by output cell `index` of `cells`.
fn span(index: u32, cells: u32, total: u32) -> (u32, u32) {
    let start = (index as u64 * total as u64 / cells as u64) as u32;
    let end = ((index as u64 + 1) * total as u64 / cells as u64) as u32;
    (start, end.max(start + 1).min(total))
}

fn box_blur(cells: &[[f32; 3]], w: u32, h: u32) -> Vec<[f32; 3]> {
    let at = |x: i64, y: i64| {
        cells[(y.clamp(0, h as i64 - 1) * w as i64 + x.clamp(0, w as i64 - 1)) as usize]
    };
    let mut out = Vec::with_capacity(cells.len());
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let mut sum = [0f32; 3];
            for dy in -BLUR..=BLUR {
                for dx in -BLUR..=BLUR {
                    let cell = at(x + dx, y + dy);
                    for channel in 0..3 {
                        sum[channel] += cell[channel];
                    }
                }
            }
            out.push(sum.map(|total| total / ((2 * BLUR + 1) * (2 * BLUR + 1)) as f32));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_aspect_ratio_and_its_own_transparency() {
        let frame = vec![200u8; 2560 * 1600 * 4];
        let (w, h, pixels) = ambient(2560, 1600, &frame).unwrap();
        assert_eq!((w, h), (48, 30));
        assert_eq!(pixels.len(), 48 * 30 * 4);
        let expected = (200. * DIM).round() as u8;
        let alpha = (OPACITY * 255.).round() as u8;
        assert!(
            pixels
                .chunks(4)
                .all(|p| p[..3] == [expected; 3] && p[3] == alpha)
        );
    }

    #[test]
    fn softens_hard_edges_between_regions() {
        // Left half black, right half white: the middle should become a gradient.
        let (width, height) = (640, 400);
        let mut frame = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in width / 2..width {
                frame[(y * width + x) * 4..][..3].copy_from_slice(&[255; 3]);
            }
        }
        let (w, _, pixels) = ambient(width as u32, height as u32, &frame).unwrap();
        let row: Vec<u8> = (0..w as usize).map(|x| pixels[x * 4]).collect();
        assert!(row.windows(2).all(|pair| pair[0] <= pair[1]), "{row:?}");
        assert!(row[w as usize / 2 - 1] > 0 && row[w as usize / 2] < (255. * DIM) as u8);
    }

    #[test]
    fn rejects_empty_or_truncated_frames() {
        assert!(ambient(0, 10, &[]).is_none());
        assert!(ambient(10, 10, &[0; 10]).is_none());
        let tiny = ambient(4, 3, &[9; 48]).unwrap();
        assert_eq!((tiny.0, tiny.1), (4, 3));
    }
}
