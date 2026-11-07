//! Versioned user preferences, stored as JSON in the app-data directory.
//!
//! Writes are atomic (temp file + rename). A file that can't be parsed is
//! moved aside to `settings.json.bad` and defaults are used, so a corrupt
//! file never stops the app from starting. No dictated content is stored here.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::controller::RecordMode;

pub const SETTINGS_VERSION: u32 = 1;
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";
pub const DEFAULT_MODEL: &str = "small.en-q5_1";
pub const DEFAULT_VOCABULARY: &[&str] = &["FCI Tradecraft", "FinCEN", "BSA", "AML", "KYC", "OFAC", "lookback"];

/// Whisper's prompt window is ~224 tokens; keep hints well inside it.
pub const MAX_VOCAB_TERMS: usize = 40;
pub const MAX_VOCAB_TERM_CHARS: usize = 40;
pub const MAX_VOCAB_TOTAL_CHARS: usize = 400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// First-run setup finished (or explicitly skipped).
    pub setup_complete: bool,
    /// Global shortcut, in Tauri accelerator syntax (e.g. "Alt+Space").
    pub shortcut: String,
    pub mode: RecordMode,
    /// Input device name; `None` follows the system default.
    pub input_device: Option<String>,
    /// Catalog id of the chosen speech model.
    pub model_id: Option<String>,
    pub vocabulary: Vec<String>,
    pub sound_cues: bool,
    pub launch_at_login: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            setup_complete: false,
            shortcut: DEFAULT_SHORTCUT.into(),
            mode: RecordMode::Toggle,
            input_device: None,
            model_id: None,
            vocabulary: DEFAULT_VOCABULARY.iter().map(|s| s.to_string()).collect(),
            sound_cues: false,
            launch_at_login: false,
        }
    }
}

/// `<OS app-data dir>/com.fcitradecraft.loqwat/settings.json`
pub fn settings_path() -> Result<PathBuf> {
    let base = dirs::data_dir().context("no application-data directory on this system")?;
    Ok(base.join(crate::models::APP_ID).join("settings.json"))
}

impl Settings {
    /// Load settings; missing file → defaults, unreadable file → moved aside + defaults.
    pub fn load_from(path: &Path) -> Settings {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::warn!("settings unreadable ({e}); using defaults");
                return Settings::default();
            }
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                if s.version > SETTINGS_VERSION {
                    log::warn!(
                        "settings file is from a newer LoqWat (v{}); reading what we understand",
                        s.version
                    );
                }
                s.version = SETTINGS_VERSION;
                s.vocabulary = normalize_vocabulary(&s.vocabulary);
                s
            }
            Err(e) => {
                let bad = path.with_extension("json.bad");
                log::warn!("settings file invalid ({e}); moved to {}", bad.display());
                let _ = fs::rename(path, bad);
                Settings::default()
            }
        }
    }

    /// Atomic write: temp file in the same directory, fsync, rename.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let dir = path.parent().context("settings path has no directory")?;
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self)?;
        {
            use std::io::Write;
            let mut f = fs::File::create(&tmp)
                .with_context(|| format!("creating {}", tmp.display()))?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path).context("saving settings")?;
        Ok(())
    }

    /// Whisper initial prompt built from the vocabulary hints.
    pub fn prompt(&self) -> Option<String> {
        (!self.vocabulary.is_empty()).then(|| self.vocabulary.join(", ") + ".")
    }
}

/// Trim, drop blanks and duplicates (case-insensitive), cap term length,
/// count and total size. Order is kept.
pub fn normalize_vocabulary<S: AsRef<str>>(terms: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut total = 0;
    for t in terms {
        let t: String = t
            .as_ref()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_VOCAB_TERM_CHARS)
            .collect();
        let t = t.trim().to_string();
        if t.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            continue;
        }
        if out.len() >= MAX_VOCAB_TERMS || total + t.len() > MAX_VOCAB_TOTAL_CHARS {
            break;
        }
        total += t.len();
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("loqwat-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let p = temp_path("missing");
        assert_eq!(Settings::load_from(&p), Settings::default());
    }

    #[test]
    fn round_trip() {
        let p = temp_path("roundtrip");
        let s = Settings {
            setup_complete: true,
            shortcut: "Control+Shift+KeyD".into(),
            mode: RecordMode::Hold,
            input_device: Some("Yeti Stereo Microphone".into()),
            model_id: Some("base.en".into()),
            vocabulary: vec!["Acme".into()],
            sound_cues: true,
            launch_at_login: true,
            ..Settings::default()
        };
        s.save_to(&p).unwrap();
        assert_eq!(Settings::load_from(&p), s);
        assert!(!p.with_extension("json.tmp").exists());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let p = temp_path("partial");
        fs::write(&p, r#"{"version":1,"mode":"hold"}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.mode, RecordMode::Hold);
        assert_eq!(s.shortcut, DEFAULT_SHORTCUT);
        assert_eq!(s.vocabulary.len(), DEFAULT_VOCABULARY.len());
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let p = temp_path("corrupt");
        fs::write(&p, "{not json").unwrap();
        assert_eq!(Settings::load_from(&p), Settings::default());
        assert!(!p.exists());
        assert!(p.with_extension("json.bad").exists());
    }

    #[test]
    fn unknown_fields_from_newer_version_are_ignored() {
        let p = temp_path("newer");
        fs::write(&p, r#"{"version":9,"shortcut":"F5","future_thing":true}"#).unwrap();
        let s = Settings::load_from(&p);
        assert_eq!(s.shortcut, "F5");
        assert_eq!(s.version, SETTINGS_VERSION);
    }

    #[test]
    fn vocabulary_is_normalized() {
        let v = normalize_vocabulary(&["  FinCEN ", "", "fincen", "OFAC\tlist", "a\u{7}b"]);
        assert_eq!(v, vec!["FinCEN", "OFAC list", "ab"]);
    }

    #[test]
    fn vocabulary_is_capped() {
        let many: Vec<String> = (0..100).map(|i| format!("term{i}")).collect();
        assert_eq!(normalize_vocabulary(&many).len(), MAX_VOCAB_TERMS);
        let long = vec!["x".repeat(100)];
        assert_eq!(normalize_vocabulary(&long)[0].len(), MAX_VOCAB_TERM_CHARS);
        let big: Vec<String> = (0..40).map(|i| format!("{i:0>30}")).collect();
        let total: usize = normalize_vocabulary(&big).iter().map(|t| t.len()).sum();
        assert!(total <= MAX_VOCAB_TOTAL_CHARS);
    }

    #[test]
    fn prompt_joins_terms() {
        let s = Settings {
            vocabulary: vec!["A".into(), "B".into()],
            ..Settings::default()
        };
        assert_eq!(s.prompt().as_deref(), Some("A, B."));
        let empty = Settings {
            vocabulary: vec![],
            ..Settings::default()
        };
        assert_eq!(empty.prompt(), None);
    }
}
