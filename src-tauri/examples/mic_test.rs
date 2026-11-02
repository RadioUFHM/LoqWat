//! Record from the microphone, then transcribe with a warmed engine.
//!   cargo run --release --example mic_test -- [--model small.en-q5_1] [--secs 8] [--device "Name"] [--list]
//! Press Enter to stop early. Text is printed to this terminal only; nothing is saved.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use loqwat_lib::{audio, capture, engine, metrics, models, vad};

fn main() -> Result<()> {
    let mut model_id = "small.en-q5_1".to_string();
    let mut secs = 8u32;
    let mut device: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" => model_id = it.next().context("--model value")?,
            "--secs" => secs = it.next().context("--secs value")?.parse()?,
            "--device" => device = Some(it.next().context("--device value")?),
            "--list" => {
                for d in capture::list_input_devices()? {
                    println!("{} {}", if d.is_default { "*" } else { " " }, d.name);
                }
                return Ok(());
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }

    let dir = models::models_dir()?;
    let info = models::find(&model_id).context("unknown model")?;
    let path = info.path_in(&dir);
    let mut gate = vad::SpeechDetector::load(&models::VAD_MODEL.path_in(&dir)).ok();
    let opts = engine::TranscribeOptions {
        initial_prompt: Some("FCI Tradecraft, FinCEN, BSA, AML, KYC, OFAC.".into()),
        ..Default::default()
    };
    println!("Loading {model_id}…");
    let mut eng = engine::TranscriptionEngine::load(&path, true)?;
    // Warm-up pass so the measured transcription reflects steady-state speed.
    let warm = eng.transcribe(&vec![0.0; 16_000], &Default::default(), None)?;
    println!(
        "Loaded in {:.0} ms, warm-up {:.0} ms.",
        eng.load_time().as_secs_f64() * 1000.0,
        warm.elapsed.as_secs_f64() * 1000.0
    );

    let rec = capture::Recording::start(device.as_deref(), capture::DEFAULT_MAX_SECS)?;
    println!(
        "Recording from \"{}\" ({} ch @ {} Hz) for up to {secs}s — speak now, Enter to stop.",
        rec.device_name,
        rec.channels(),
        rec.sample_rate()
    );

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = tx.send(());
    });
    let start = Instant::now();
    let mut max_level = 0.0f32;
    while start.elapsed() < Duration::from_secs(secs as u64) && rx.try_recv().is_err() {
        std::thread::sleep(Duration::from_millis(100));
        let lvl = rec.level();
        max_level = max_level.max(lvl);
        let bars = (lvl * 40.0).min(40.0) as usize;
        print!(
            "\r  {:>4.1}s |{:<40}|",
            rec.elapsed_secs(),
            "#".repeat(bars)
        );
        std::io::stdout().flush()?;
        if let Some(e) = rec.stream_error() {
            println!("\nStream error: {e}");
            break;
        }
    }
    let raw = rec.finish();
    println!(
        "\nCaptured {:.1}s, peak level {:.3}, RMS {:.4}.",
        raw.duration_secs(),
        max_level,
        audio::rms(&raw.samples)
    );
    if max_level < 0.001 {
        println!(
            "Input is silent — check microphone permission (System Settings → Privacy & Security → Microphone) and input volume."
        );
    }

    let input = raw.to_whisper_input()?;
    if let Some(g) = gate.as_mut() {
        let check = g.check(&input)?;
        println!(
            "VAD: {} speech segment(s), {:.1}s of speech.",
            check.segments, check.speech_secs
        );
        if !check.has_speech() {
            println!("  <no speech detected — nothing would be inserted>");
            return Ok(());
        }
    } else {
        println!("VAD model not installed — speech gate skipped.");
    }
    let t = eng.transcribe(&input, &opts, None)?;
    println!(
        "Transcribed in {:.0} ms (no-speech prob {:.2}), peak RSS {}:",
        t.elapsed.as_secs_f64() * 1000.0,
        t.max_no_speech_prob,
        metrics::format_mb(metrics::peak_rss_bytes())
    );
    println!(
        "  {}",
        if t.text.is_empty() {
            "<no speech detected>"
        } else {
            &t.text
        }
    );
    Ok(())
}
