//! Integration test against a real installed model (skipped if none installed).
//! Regression: an unset cancel flag must not abort inference (whisper error -6).

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use loqwat_lib::{engine, models};

fn installed_model() -> Option<std::path::PathBuf> {
    let dir = models::models_dir().ok()?;
    ["base.en", "small.en-q5_1", "tiny.en"]
        .iter()
        .filter_map(|id| models::find(id))
        .map(|m| m.path_in(&dir))
        .find(|p| p.exists())
}

/// Two seconds of a 220 Hz tone at 16 kHz.
fn tone() -> Vec<f32> {
    (0..32_000)
        .map(|i| (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin() * 0.3)
        .collect()
}

#[test]
fn cancel_flag_behaviour() {
    let Some(path) = installed_model() else {
        eprintln!("no model installed; skipping");
        return;
    };
    let mut eng = engine::TranscriptionEngine::load(&path, true).unwrap();
    let opts = engine::TranscribeOptions::default();

    let unset = Arc::new(AtomicBool::new(false));
    let t = eng
        .transcribe(&tone(), &opts, Some(unset))
        .expect("unset flag must not abort");
    assert!(!t.cancelled);

    let set = Arc::new(AtomicBool::new(true));
    let t = eng.transcribe(&tone(), &opts, Some(set)).unwrap();
    assert!(t.cancelled);

    // Engine stays usable after a cancelled run.
    let t = eng.transcribe(&tone(), &opts, None).unwrap();
    assert!(!t.cancelled);
}
