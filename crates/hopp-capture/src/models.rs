//! Transcription models: which ones Hopp offers, where they live, and downloading them.
//!
//! Each model is checked against a SHA-256 built into Hopp before it is used, so a
//! tampered or truncated download is never loaded.

use crate::store::Store;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

/// Downloads started from Settings: progress in percent, or the error that stopped one.
static DOWNLOADS: Mutex<Option<HashMap<&'static str, Result<u8, String>>>> = Mutex::new(None);

fn set_progress(name: &'static str, state: Option<Result<u8, String>>) {
    if let Ok(mut downloads) = DOWNLOADS.lock() {
        let downloads = downloads.get_or_insert_with(HashMap::new);
        match state {
            Some(state) => downloads.insert(name, state),
            None => downloads.remove(name),
        };
    }
}

/// `Some(Ok(percent))` while downloading, `Some(Err(…))` if the last attempt failed.
pub fn download_state(name: &str) -> Option<Result<u8, String>> {
    DOWNLOADS.lock().ok()?.as_ref()?.get(name).cloned()
}

pub fn any_downloading() -> bool {
    DOWNLOADS.lock().is_ok_and(|downloads| {
        downloads
            .as_ref()
            .is_some_and(|downloads| downloads.values().any(Result::is_ok))
    })
}

/// Download `model` on a thread of its own, reporting through [`download_state`].
pub fn download_in_background(model: &'static Model) {
    if matches!(download_state(model.name), Some(Ok(_))) {
        return;
    }
    set_progress(model.name, Some(Ok(0)));
    let spawned = std::thread::Builder::new()
        .name("hopp-model".into())
        .spawn(move || {
            let result = download(model, |done, total| {
                set_progress(model.name, Some(Ok((done * 100 / total.max(1)) as u8)));
            });
            match result {
                Ok(_) => set_progress(model.name, None),
                Err(error) => set_progress(model.name, Some(Err(format!("{error:#}")))),
            }
        });
    if spawned.is_err() {
        set_progress(model.name, Some(Err("Could not start the download".into())));
    }
}

pub struct Model {
    pub name: &'static str,
    pub(crate) file: &'static str,
    /// The Hugging Face repository and the commit the file is pinned to.
    repository: &'static str,
    revision: &'static str,
    pub bytes: u64,
    sha256: &'static str,
    /// Only transcribes English.
    english: bool,
    pub note: &'static str,
}

impl Model {
    pub fn english_only(&self) -> bool {
        self.english
    }

    /// The transcription helper's hard memory limit with this model. Measured peaks
    /// with whisper.cpp while transcribing full windows on the CPU: base.en 383 MB,
    /// large-v3-turbo 2.0 GB, large-v3 3.8 GB. On the GPU more, since its memory is
    /// system RAM charged to the helper: 576 MB, 2.4 GB and 4.6 GB. This leaves 12–30%
    /// above those.
    pub fn memory_limit(&self, gpu: bool) -> u64 {
        if gpu {
            self.bytes * 3 / 2 + (512 << 20)
        } else {
            self.bytes * 5 / 4 + (512 << 20)
        }
    }

    fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository, self.revision, self.file
        )
    }
}

pub const DEFAULT: &str = "parakeet-en";

const PARAKEET_V3_LANGUAGES: &str = "25 European languages";

/// GGUF files from `handy-computer` on Hugging Face, which publishes them for
/// transcribe.cpp. Hashes are their LFS object ids. Parakeet is the 8-bit build Hex
/// uses, so Hex's download serves both; Whisper is full precision (F16), as Hopp ran
/// it before (quantized Whisper cost accuracy, tasks/capture-bench).
pub const MODELS: [Model; 10] = [
    Model {
        name: "parakeet-en",
        file: "parakeet-unified-en-0.6b-Q8_0.gguf",
        repository: "handy-computer/parakeet-unified-en-0.6b-gguf",
        revision: "7e948f21b7bdbac698d3318db9d350f1096f3b6c",
        bytes: 731_357_568,
        sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
        english: true,
        note: "the default: English, the most accurate and about 27× faster than speech \
               on the GPU; the same model as Hex",
    },
    Model {
        name: "parakeet-v3",
        file: "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        repository: "handy-computer/parakeet-tdt-0.6b-v3-gguf",
        revision: "85ac09ea12fc4b1112fa76810059364bc6adc9de",
        bytes: 739_508_576,
        sha256: "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7",
        english: false,
        note: PARAKEET_V3_LANGUAGES,
    },
    Model {
        name: "tiny.en",
        file: "whisper-tiny.en-F16.gguf",
        repository: "handy-computer/whisper-tiny.en-gguf",
        revision: "9dd46e00ef0577e354e658c07b3ac5580e9cd211",
        bytes: 80_058_464,
        sha256: "7ae6eae54d625472e52b6af03b09dec586f0e948ffa5872815bb1988e973ad2b",
        english: true,
        note: "Whisper, fastest; rough on names",
    },
    Model {
        name: "base.en",
        file: "whisper-base.en-F16.gguf",
        repository: "handy-computer/whisper-base.en-gguf",
        revision: "0dd374130837de419035c77d76440aad61ca135c",
        bytes: 151_068_608,
        sha256: "e6c447b0cc761e8ac3fc2f5e0e6918f604e415a41cff97825f1503ccde376e1e",
        english: true,
        note: "Whisper, light; quick even on the CPU",
    },
    Model {
        name: "base",
        file: "whisper-base-F16.gguf",
        repository: "handy-computer/whisper-base-gguf",
        revision: "42fcd2077de47c36d779309e0e91b76ec620f56d",
        bytes: 151_145_760,
        sha256: "38ab6b0ed742e9eded4d5a2ba7fc34d44fc28cdb71d6360f6321d4b306ef8039",
        english: false,
        note: "Whisper, light, any language",
    },
    Model {
        name: "small.en",
        file: "whisper-small.en-F16.gguf",
        repository: "handy-computer/whisper-small.en-gguf",
        revision: "ab86448a84fcc15a96a9a2216d93a8dac8427da3",
        bytes: 492_810_784,
        sha256: "59eade60e072bd289c9ff39313ef14b565691bfe373275ea904ecea497e47c8a",
        english: true,
        note: "Whisper, more accurate than base",
    },
    Model {
        name: "small",
        file: "whisper-small-F16.gguf",
        repository: "handy-computer/whisper-small-gguf",
        revision: "4afdc7942ecf1997f359deae48a63506b062eac4",
        bytes: 492_888_480,
        sha256: "bef65e1ac9d012269453243243aac0d0f67792693ba6c99b584a124e0ee326fc",
        english: false,
        note: "Whisper, more accurate than base, any language",
    },
    Model {
        name: "medium.en",
        file: "whisper-medium.en-F16.gguf",
        repository: "handy-computer/whisper-medium.en-gguf",
        revision: "e6370f41cfa6d73e2a89969f4f5055f8c7e4dbb6",
        bytes: 1_541_853_248,
        sha256: "c8be41b68306ad35382af749d4681b33da403b7d82eb3b6d3225a01a02dc1196",
        english: true,
        note: "Whisper, accurate, but slower than speech on most machines",
    },
    Model {
        name: "large-v3-turbo",
        file: "whisper-large-v3-turbo-F16.gguf",
        repository: "handy-computer/whisper-large-v3-turbo-gguf",
        revision: "18ada752005f5fddef499256f9a458d0b37a203a",
        bytes: 1_625_935_488,
        sha256: "c8506ddfa3d4eb9279170d722a75422db00dbdbd7618531bbb81926e0598eb8f",
        english: false,
        note: "Whisper, nearly large-v3 at about twice its speed, any language; 2.4 GB",
    },
    Model {
        name: "large-v3",
        file: "whisper-large-v3-F16.gguf",
        repository: "handy-computer/whisper-large-v3-gguf",
        revision: "274ad1d1cf680b7ee53cc70f78461394cd3a344b",
        bytes: 3_107_236_640,
        sha256: "e633ab1d74b0e98f4f57daedaee34291297dbbf01389bda3d890766600c1c584",
        english: false,
        note: "Whisper, the most accurate for languages other than English; 4.6 GB",
    },
];

pub fn find(name: &str) -> Result<&'static Model> {
    MODELS
        .iter()
        .find(|model| model.name == name)
        .with_context(|| {
            let names: Vec<_> = MODELS.iter().map(|model| model.name).collect();
            format!("No model '{name}'; choose one of {}", names.join(", "))
        })
}

pub fn dir() -> Result<PathBuf> {
    Ok(crate::paths::data_dir()?.join("models"))
}

/// Where Hopp downloads `model` to.
fn own_path(model: &Model) -> Result<PathBuf> {
    Ok(dir()?.join(model.file))
}

/// Hex's models folder, next to Hopp's data folder: `~/.local/share/voice-control/models`.
fn hex_dir() -> Option<PathBuf> {
    Some(
        crate::paths::data_dir()
            .ok()?
            .parent()?
            .join("voice-control/models"),
    )
}

/// What Hex writes next to a model once it has checked its hash.
#[derive(serde::Deserialize)]
struct HexReceipt {
    bytes: u64,
    sha256: String,
    modified_ns: u128,
}

/// The receipt matches the file, untouched since Hex checked it, and the hash is the
/// one Hopp pins.
fn hex_verified(model: &Model, path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_nanos());
    let receipt = path.with_extension("gguf.verified.json");
    std::fs::read(receipt)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HexReceipt>(&bytes).ok())
        .is_some_and(|receipt| {
            receipt.bytes == model.bytes
                && metadata.len() == model.bytes
                && receipt.sha256 == model.sha256
                && Some(receipt.modified_ns) == modified_ns
        })
}

/// Hex's copy of `model`, if Hex has downloaded and checked the same file.
fn hex_copy(model: &Model) -> Option<PathBuf> {
    let path = hex_dir()?.join(model.file);
    hex_verified(model, &path).then_some(path)
}

/// Whether `model` is Hex's copy rather than Hopp's own (Hopp never deletes it).
pub fn shared(model: &Model) -> bool {
    !own_installed(model) && hex_copy(model).is_some()
}

/// The file to load: Hopp's own download, or else Hex's copy.
pub fn path(model: &Model) -> Result<PathBuf> {
    if !own_installed(model)
        && let Some(path) = hex_copy(model)
    {
        return Ok(path);
    }
    own_path(model)
}

/// Downloaded and the right size. The hash was checked when it was downloaded.
fn own_installed(model: &Model) -> bool {
    own_path(model)
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|metadata| metadata.len() == model.bytes)
}

/// Ready to use: downloaded by Hopp, or checked by Hex.
pub fn installed(model: &Model) -> bool {
    own_installed(model) || hex_copy(model).is_some()
}

/// Delete a downloaded model, unless new captures will use it or a running one does: the
/// helper lets go of a model after a quiet spell and the capture would download it all over
/// again to carry on. `store` is `None` if nothing has been captured yet. Hex's copy is
/// Hex's to delete.
pub fn delete(store: Option<&Store>, model: &Model) -> Result<()> {
    if shared(model) {
        bail!("{} is Hex's copy; remove it from Hex", model.name);
    }
    if crate::config::load().model == model.name {
        bail!("{} is in use; choose another first", model.name);
    }
    if let Some(store) = store {
        crate::liveness::close_orphans(store)?;
        let running = store.live_calls()?;
        if running
            .iter()
            .any(|call| call.model.as_deref() == Some(model.name))
        {
            bail!("A capture is still using {}", model.name);
        }
    }
    match std::fs::remove_file(own_path(model)?) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Deleting {}", model.name))
        }
        _ => {
            set_progress(model.name, None);
            Ok(())
        }
    }
}

/// Download `model` unless it is already here. `progress` gets (done, total) bytes.
pub fn download(model: &Model, mut progress: impl FnMut(u64, u64)) -> Result<PathBuf> {
    if installed(model) {
        return path(model);
    }
    let destination = own_path(model)?;
    let directory = dir()?;
    crate::paths::private_dir(&directory)?;
    remove_abandoned_downloads(&directory, Duration::from_secs(3600));
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // A blocking client's timeout is per read, so this only ends a download that stalls.
        .timeout(Duration::from_secs(60))
        .build()?;
    let mut response = client
        .get(model.url())
        .send()
        .and_then(|response| response.error_for_status())
        .with_context(|| format!("Downloading {}", model.name))?;
    let mut file = tempfile::NamedTempFile::new_in(&directory)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut done = 0u64;
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])?;
        done += read as u64;
        if done > model.bytes {
            bail!("{} is larger than expected", model.name);
        }
        progress(done, model.bytes);
    }
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if done != model.bytes || hex != model.sha256 {
        bail!("{} did not match its checksum; not using it", model.name);
    }
    file.as_file().sync_all()?;
    file.persist(&destination)?;
    Ok(destination)
}

/// A download keeps writing to its temporary file, and leaves it behind if Hopp quits part
/// way through. Those can be gigabytes, so whatever has sat untouched this long goes.
fn remove_abandoned_downloads(directory: &std::path::Path, untouched: Duration) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let abandoned = entry.file_name().to_string_lossy().starts_with(".tmp")
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age >= untouched));
        if abandoned {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// whisper.cpp models from before Hopp moved to transcribe.cpp, which it no longer
/// loads: `ggml-*.bin` in Hopp's models folder.
pub fn legacy_files() -> Vec<PathBuf> {
    let Ok(directory) = dir() else {
        return Vec::new();
    };
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ggml-") && name.ends_with(".bin"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_download_nobody_is_writing_is_abandoned() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
        let file = |name: &str, modified| -> Result<PathBuf> {
            let path = directory.path().join(name);
            let file = std::fs::File::create(&path)?;
            file.set_modified(modified)?;
            Ok(path)
        };
        let abandoned = file(".tmpABC", hour_ago)?;
        let running = file(".tmpDEF", std::time::SystemTime::now())?;
        let model = file("parakeet.gguf", hour_ago)?;
        remove_abandoned_downloads(directory.path(), Duration::from_secs(1800));
        assert!(!abandoned.exists());
        assert!(running.exists());
        assert!(model.exists());
        Ok(())
    }

    #[test]
    fn the_default_exists_and_names_are_unique() -> Result<()> {
        let default = find(DEFAULT)?;
        assert!(default.name.starts_with("parakeet"));
        assert!(default.english_only());
        assert!(!find("parakeet-v3")?.english_only());
        assert!(
            !find("large-v3")?.english_only(),
            "large-v3 handles any language"
        );
        assert!(find("base.en")?.english_only());
        assert!(!find("small")?.english_only());
        assert!(find("huge").is_err());
        for (index, model) in MODELS.iter().enumerate() {
            assert_eq!(model.sha256.len(), 64);
            assert_eq!(model.revision.len(), 40);
            assert!(model.file.ends_with(".gguf"));
            assert!(
                MODELS[index + 1..]
                    .iter()
                    .all(|other| other.name != model.name && other.file != model.file)
            );
        }
        Ok(())
    }

    #[test]
    fn hex_copies_count_only_with_a_matching_receipt() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let model = Model {
            name: "test",
            file: "test.gguf",
            repository: "",
            revision: "",
            bytes: 5,
            // sha256("hello")
            sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            english: true,
            note: "",
        };
        let path = directory.path().join(model.file);
        std::fs::write(&path, b"hello")?;
        assert!(!hex_verified(&model, &path), "no receipt");
        let modified_ns = std::fs::metadata(&path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let receipt = |sha256: &str, modified_ns: u128| {
            serde_json::json!({ "bytes": 5, "sha256": sha256, "modified_ns": modified_ns })
                .to_string()
        };
        let receipt_path = path.with_extension("gguf.verified.json");
        std::fs::write(&receipt_path, receipt(model.sha256, modified_ns))?;
        assert!(hex_verified(&model, &path));
        std::fs::write(&receipt_path, receipt(&"0".repeat(64), modified_ns))?;
        assert!(!hex_verified(&model, &path), "a different file");
        std::fs::write(&receipt_path, receipt(model.sha256, modified_ns + 1))?;
        assert!(!hex_verified(&model, &path), "changed since Hex checked it");
        Ok(())
    }
}
