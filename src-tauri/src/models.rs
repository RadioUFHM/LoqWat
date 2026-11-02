//! Model catalog, storage location, and verified atomic downloads.
//!
//! Sources: Hugging Face repos `ggerganov/whisper.cpp` (speech models) and
//! `ggml-org/whisper-vad` (Silero VAD), each pinned to a specific git revision;
//! every file is checked against its published SHA-256.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

pub const APP_ID: &str = "com.fcitradecraft.loqwat";
pub const MODEL_REPO: &str = "ggerganov/whisper.cpp";
/// Pinned repository revision (commit of 2024-10-29).
pub const MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
pub const VAD_REPO: &str = "ggml-org/whisper-vad";
/// Pinned repository revision (commit of 2025-11-17).
pub const VAD_REVISION: &str = "9ffd54a1e1ee413ddf265af9913beaf518d1639b";

#[derive(Debug, Clone, Copy)]
pub struct ModelInfo {
    pub repo: &'static str,
    pub revision: &'static str,
    pub id: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    pub label: &'static str,
}

pub const CATALOG: &[ModelInfo] = &[
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "tiny.en",
        file: "ggml-tiny.en.bin",
        size: 77_704_715,
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        label: "Tiny (English) — fastest, least accurate",
    },
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "base.en",
        file: "ggml-base.en.bin",
        size: 147_964_211,
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        label: "Base (English) — fast",
    },
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "small.en",
        file: "ggml-small.en.bin",
        size: 487_614_201,
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        label: "Small (English) — balanced",
    },
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "small.en-q5_1",
        file: "ggml-small.en-q5_1.bin",
        size: 190_098_681,
        sha256: "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30",
        label: "Small (English, quantized) — balanced, smaller",
    },
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "medium.en-q5_0",
        file: "ggml-medium.en-q5_0.bin",
        size: 539_225_533,
        sha256: "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0",
        label: "Medium (English, quantized) — more accurate, slower",
    },
    ModelInfo {
        repo: MODEL_REPO,
        revision: MODEL_REVISION,
        id: "large-v3-turbo-q5_0",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
        label: "Large v3 Turbo (quantized) — most accurate",
    },
];

/// Silero voice-activity-detection model used to reject silence/noise.
pub const VAD_MODEL: ModelInfo = ModelInfo {
    repo: VAD_REPO,
    revision: VAD_REVISION,
    id: "silero-v6.2.0",
    file: "ggml-silero-v6.2.0.bin",
    size: 885_098,
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    label: "Silero VAD v6.2.0 — speech detection",
};

pub fn find(id: &str) -> Option<&'static ModelInfo> {
    CATALOG
        .iter()
        .chain(std::iter::once(&VAD_MODEL))
        .find(|m| m.id == id)
}

impl ModelInfo {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repo, self.revision, self.file
        )
    }

    pub fn path_in(&self, dir: &Path) -> PathBuf {
        dir.join(self.file)
    }
}

/// `<OS app-data dir>/com.fcitradecraft.loqwat/models`
/// (macOS: ~/Library/Application Support/..., Windows: %APPDATA%\...).
pub fn models_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("no application-data directory on this system")?;
    Ok(base.join(APP_ID).join("models"))
}

/// Hash a file and compare size + SHA-256 against the catalog entry.
pub fn verify(info: &ModelInfo, path: &Path) -> Result<()> {
    let meta = fs::metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if meta.len() != info.size {
        bail!(
            "{}: size {} does not match expected {}",
            info.file,
            meta.len(),
            info.size
        );
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    check_digest(info, &hex(&hasher.finalize()))
}

fn check_digest(info: &ModelInfo, actual: &str) -> Result<()> {
    if actual != info.sha256 {
        bail!(
            "{}: SHA-256 mismatch (expected {}, got {actual})",
            info.file,
            info.sha256
        );
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download a model into `dir`, verifying size and SHA-256 before it becomes
/// visible under its final name. Data is written to `<file>.part` and renamed
/// only after verification, so an interrupted download is never mistaken for
/// a valid model. `progress` receives (bytes_done, bytes_total).
pub fn download(
    info: &ModelInfo,
    dir: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let final_path = info.path_in(dir);
    let part_path = dir.join(format!("{}.part", info.file));

    let result = (|| -> Result<()> {
        let response = ureq::get(&info.url())
            .call()
            .with_context(|| format!("requesting {}", info.url()))?;
        let mut reader = response.into_body().into_reader();
        let mut out = File::create(&part_path)
            .with_context(|| format!("creating {}", part_path.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("download cancelled");
            }
            let n = reader.read(&mut buf).context("reading download stream")?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > info.size {
                bail!("{}: server sent more data than expected", info.file);
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).context("writing model file")?;
            progress(done, info.size);
        }
        if done != info.size {
            bail!(
                "{}: incomplete download ({done} of {} bytes)",
                info.file,
                info.size
            );
        }
        check_digest(info, &hex(&hasher.finalize()))?;
        out.sync_all().context("flushing model file")?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            fs::rename(&part_path, &final_path).context("finalizing model file")?;
            Ok(final_path)
        }
        Err(e) => {
            let _ = fs::remove_file(&part_path);
            Err(e)
        }
    }
}

/// Remove leftover `.part` files from interrupted downloads.
pub fn clean_partial_downloads(dir: &Path) -> Result<usize> {
    let mut removed = 0;
    if !dir.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "part") {
            fs::remove_file(&path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("loqwat-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    const HELLO: ModelInfo = ModelInfo {
        repo: "test/test",
        revision: "0",
        id: "test",
        file: "test.bin",
        size: 5,
        // sha256("hello")
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        label: "test",
    };

    #[test]
    fn verify_accepts_matching_file() {
        let dir = temp_dir("ok");
        let p = dir.join("test.bin");
        fs::write(&p, b"hello").unwrap();
        verify(&HELLO, &p).unwrap();
    }

    #[test]
    fn verify_rejects_corrupt_file() {
        let dir = temp_dir("corrupt");
        let p = dir.join("test.bin");
        fs::write(&p, b"hellO").unwrap();
        let err = verify(&HELLO, &p).unwrap_err().to_string();
        assert!(err.contains("SHA-256 mismatch"), "{err}");
    }

    #[test]
    fn verify_rejects_truncated_file() {
        let dir = temp_dir("trunc");
        let p = dir.join("test.bin");
        fs::write(&p, b"hell").unwrap();
        let err = verify(&HELLO, &p).unwrap_err().to_string();
        assert!(err.contains("size"), "{err}");
    }

    #[test]
    fn clean_partial_removes_only_part_files() {
        let dir = temp_dir("partial");
        fs::write(dir.join("a.bin.part"), b"x").unwrap();
        fs::write(dir.join("a.bin"), b"x").unwrap();
        assert_eq!(clean_partial_downloads(&dir).unwrap(), 1);
        assert!(dir.join("a.bin").exists());
        assert!(!dir.join("a.bin.part").exists());
    }

    #[test]
    fn catalog_urls_are_pinned() {
        for m in CATALOG.iter().chain(std::iter::once(&VAD_MODEL)) {
            assert!(m.url().contains(m.revision));
            assert_eq!(m.revision.len(), 40);
            assert_eq!(m.sha256.len(), 64);
        }
    }
}
