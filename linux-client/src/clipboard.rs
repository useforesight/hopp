//! Clipboard text uses Hopp's existing packet format, with bounded transfers.
use crate::protocol::ClipboardPacket;
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const CHUNK_BYTES: usize = 3072;
pub const MAX_BYTES: usize = 256 * CHUNK_BYTES;

#[derive(Default)]
pub struct Assembler {
    packets: BTreeMap<u64, Vec<u8>>,
    total: u64,
    started: Option<Instant>,
    bytes: usize,
}

impl Assembler {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn push(&mut self, packet: ClipboardPacket) -> Result<Option<Vec<u8>>> {
        if packet.total_packets == 0
            || packet.total_packets > 256
            || packet.packet_id >= packet.total_packets
            || packet.data.len() > 64 * 1024
        {
            self.clear();
            bail!("Invalid clipboard packet");
        }
        if self
            .started
            .is_some_and(|at| at.elapsed() > Duration::from_secs(10))
        {
            self.clear();
        }
        if packet.packet_id == 0 {
            self.clear();
            self.total = packet.total_packets;
            self.started = Some(Instant::now());
        }
        if self.started.is_none() || self.total != packet.total_packets {
            self.clear();
            bail!("Incomplete clipboard transfer; try again");
        }
        if let Some(previous) = self.packets.get(&packet.packet_id) {
            if *previous != packet.data {
                self.clear();
                bail!("Conflicting clipboard packet");
            }
            return Ok(None);
        }
        self.bytes += packet.data.len();
        if self.bytes > MAX_BYTES {
            self.clear();
            bail!("Clipboard text is too large");
        }
        self.packets.insert(packet.packet_id, packet.data);
        if self.packets.len() != self.total as usize {
            return Ok(None);
        }
        let bytes = std::mem::take(&mut self.packets)
            .into_values()
            .flatten()
            .collect::<Vec<_>>();
        self.clear();
        std::str::from_utf8(&bytes).context("Clipboard text was not UTF-8")?;
        Ok(Some(bytes))
    }
}

pub fn packets(bytes: &[u8]) -> Result<Vec<ClipboardPacket>> {
    if bytes.len() > MAX_BYTES {
        bail!("Clipboard text is too large");
    }
    std::str::from_utf8(bytes).context("Clipboard text was not UTF-8")?;
    let total = bytes.len().div_ceil(CHUNK_BYTES).max(1) as u64;
    if bytes.is_empty() {
        return Ok(vec![ClipboardPacket {
            packet_id: 0,
            total_packets: 1,
            data: vec![],
        }]);
    }
    Ok(bytes
        .chunks(CHUNK_BYTES)
        .enumerate()
        .map(|(id, data)| ClipboardPacket {
            packet_id: id as u64,
            total_packets: total,
            data: data.to_vec(),
        })
        .collect())
}

/// Time limits prevent an unresponsive clipboard owner from hanging the call.
pub fn read() -> Result<Vec<u8>> {
    let mut child = Command::new("wl-paste")
        .args(["--no-newline", "--type", "text/plain"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Reading clipboard text")?;
    let stdout = child
        .stdout
        .take()
        .context("Clipboard reader unavailable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        stdout
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            bail!("Clipboard owner did not respond");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Clipboard reader stopped"))??;
    if !status.success() {
        bail!("Clipboard does not contain text");
    }
    if bytes.len() > MAX_BYTES {
        bail!("Clipboard text is too large");
    }
    std::str::from_utf8(&bytes).context("Clipboard text was not UTF-8")?;
    Ok(bytes)
}

pub fn write(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_BYTES {
        bail!("Clipboard text is too large");
    }
    std::str::from_utf8(bytes).context("Clipboard text was not UTF-8")?;
    let mut child = Command::new("wl-copy")
        .args(["--type", "text/plain;charset=utf-8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdin = child.stdin.take().context("Clipboard writer unavailable")?;
    let bytes = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            bail!("Clipboard writer did not respond");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("Clipboard writer stopped"))??;
    if !status.success() {
        bail!("Clipboard write failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfer_handles_multibyte_boundaries_empty_and_data_channel_budget() {
        for text in [String::new(), "é".repeat(4000)] {
            let mut assembly = Assembler::default();
            let mut result = None;
            for packet in packets(text.as_bytes()).unwrap() {
                let event = crate::protocol::InputEvent::PasteFromClipboard(
                    crate::protocol::ClipboardPaste {
                        data: Some(packet.clone()),
                    },
                );
                assert!(serde_json::to_vec(&event).unwrap().len() < 15 * 1024);
                result = assembly.push(packet).unwrap();
            }
            assert_eq!(result.unwrap(), text.as_bytes());
        }
    }
    #[test]
    fn incomplete_conflicting_and_invalid_transfers_are_rejected() {
        let mut assembly = Assembler::default();
        assert!(
            assembly
                .push(ClipboardPacket {
                    packet_id: 1,
                    total_packets: 2,
                    data: vec![1]
                })
                .is_err()
        );
        assert!(
            assembly
                .push(ClipboardPacket {
                    packet_id: 0,
                    total_packets: 2,
                    data: vec![1]
                })
                .unwrap()
                .is_none()
        );
        assert!(
            assembly
                .push(ClipboardPacket {
                    packet_id: 1,
                    total_packets: 2,
                    data: vec![255]
                })
                .is_err()
        );
        assert!(packets(&vec![b'a'; MAX_BYTES + 1]).is_err());
        assert!(
            assembly
                .push(ClipboardPacket {
                    packet_id: 0,
                    total_packets: 257,
                    data: vec![]
                })
                .is_err()
        );
    }
}
