//! Reading 16-bit PCM WAV files, for `hopp capture transcribe`.

use anyhow::{Context, Result, bail};

/// Sample rate and mono samples (channels averaged).
pub fn read(bytes: &[u8]) -> Result<(u32, Vec<i16>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("Not a WAV file");
    }
    let mut format = None;
    let mut position = 12;
    while position + 8 <= bytes.len() {
        let id = &bytes[position..position + 4];
        let size = u32::from_le_bytes(bytes[position + 4..position + 8].try_into()?) as usize;
        let body = bytes
            .get(position + 8..position + 8 + size)
            .context("WAV chunk runs past the end of the file")?;
        match id {
            b"fmt " if size >= 16 => {
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into()?);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if !matches!(tag, 1 | 0xfffe) || bits != 16 || channels == 0 {
                    bail!("Only 16-bit PCM WAV is supported");
                }
                format = Some((channels as usize, rate));
            }
            b"data" => {
                let (channels, rate) = format.context("WAV data before its format")?;
                let samples = body
                    .chunks_exact(2 * channels)
                    .map(|frame| {
                        let sum: i32 = frame
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|sample| i32::from(i16::from_le_bytes(*sample)))
                            .sum();
                        (sum / channels as i32) as i16
                    })
                    .collect();
                return Ok((rate, samples));
            }
            _ => {}
        }
        position += 8 + size + (size & 1);
    }
    bail!("WAV file has no audio")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect();
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        out.extend_from_slice(&(channels * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn reads_mono_and_mixes_stereo() -> Result<()> {
        assert_eq!(read(&wav(1, 16_000, &[1, 2, 3]))?, (16_000, vec![1, 2, 3]));
        assert_eq!(
            read(&wav(2, 48_000, &[10, 20, -4, -8]))?,
            (48_000, vec![15, -6])
        );
        assert!(read(b"RIFF....WAVE").is_err());
        assert!(read(b"not audio").is_err());
        Ok(())
    }
}
