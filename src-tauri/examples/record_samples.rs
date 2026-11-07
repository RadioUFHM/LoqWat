//! Record your own voice reading the passages in `scripts/voice-prompts.txt`.
//!
//!   cargo run --release --example record_samples -- [--device "Name"] [--only 03-numbers] [--redo]
//!
//! Run it in a normal terminal window (it needs the keyboard). For each passage it shows
//! the text, records until you press Enter, and saves `recordings/voice/<id>.wav` plus
//! reference files for the benchmark:
//!   <id>.txt           words to score against (word error rate)
//!   <id>.expected.txt  exact text LoqWat should produce (checked by `bench --format`)
//! `recordings/` is git-ignored; the audio never leaves this Mac.
//! Passages that already have a recording are skipped unless you pass `--redo`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use loqwat_lib::{audio, capture};

struct Passage {
    id: String,
    note: String,
    read: String,
    expect: String,
}

fn parse_prompts(text: &str) -> Result<Vec<Passage>> {
    let mut out: Vec<Passage> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(id) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push(Passage {
                id: id.trim().to_string(),
                note: String::new(),
                read: String::new(),
                expect: String::new(),
            });
            continue;
        }
        let Some(p) = out.last_mut() else {
            bail!("line {}: text before the first [id]", n + 1);
        };
        let (key, value) = line
            .split_once(':')
            .with_context(|| format!("line {}: expected `key: value`", n + 1))?;
        let value = value.trim().to_string();
        match key.trim() {
            "note" => p.note = value,
            "read" => p.read = value,
            "expect" => p.expect = value,
            other => bail!("line {}: unknown key `{other}`", n + 1),
        }
    }
    Ok(out)
}

/// Print `text` wrapped at about 88 columns with an indent.
fn print_wrapped(text: &str) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + word.len() + 1 > 88 {
            println!("    {line}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        println!("    {line}");
    }
}

/// One line from the keyboard; None on EOF (non-interactive stdin).
fn read_key(lines: &mpsc::Receiver<String>) -> Option<String> {
    lines.recv().ok().map(|l| l.trim().to_lowercase())
}

fn write_wav(path: &Path, raw: &audio::RawAudio) -> Result<()> {
    let spec = hound::WavSpec {
        channels: raw.channels,
        sample_rate: raw.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let tmp = path.with_extension("wav.part");
    let mut w = hound::WavWriter::create(&tmp, spec)?;
    for &s in &raw.samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn main() -> Result<()> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut device: Option<String> = None;
    let mut only: Option<String> = None;
    let mut redo = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--device" => device = Some(it.next().context("--device value")?),
            "--only" => only = Some(it.next().context("--only value")?),
            "--redo" => redo = true,
            other => bail!("unknown argument {other}"),
        }
    }

    let prompts = repo.join("scripts/voice-prompts.txt");
    let passages = parse_prompts(&std::fs::read_to_string(&prompts)?)?;
    let out_dir = repo.join("recordings/voice");
    std::fs::create_dir_all(&out_dir)?;

    // All keyboard input comes through one reader thread.
    let (tx, lines) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    println!("LoqWat voice samples → {}", out_dir.display());
    println!("Use the microphone you dictate with, in your usual spot.\n");

    let mut recorded = 0;
    for p in &passages {
        if only.as_deref().is_some_and(|o| o != p.id) {
            continue;
        }
        let wav = out_dir.join(format!("{}.wav", p.id));
        if wav.exists() && !redo {
            println!("[{}] already recorded, skipping (use --redo to record again)", p.id);
            continue;
        }
        loop {
            println!("──────────────────────────────────────────────────────────────");
            println!("[{}] {}", p.id, p.note);
            if !p.read.is_empty() {
                println!();
                print_wrapped(&p.read);
            }
            println!("\nEnter = start recording · s = skip · q = quit");
            match read_key(&lines).as_deref() {
                None => bail!("no keyboard input; run this in a normal terminal window"),
                Some("q") => return Ok(()),
                Some("s") => break,
                _ => {}
            }

            let rec = capture::Recording::start(device.as_deref(), capture::DEFAULT_MAX_SECS)?;
            println!("● Recording from \"{}\". Press Enter when you finish.", rec.device_name);
            let mut max_level = 0.0f32;
            loop {
                match lines.recv_timeout(Duration::from_millis(100)) {
                    Ok(_) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                let lvl = rec.level();
                max_level = max_level.max(lvl);
                print!(
                    "\r  {:>5.1}s |{:<40}|",
                    rec.elapsed_secs(),
                    "#".repeat((lvl * 40.0).min(40.0) as usize)
                );
                std::io::stdout().flush()?;
                if let Some(e) = rec.stream_error() {
                    println!("\nMicrophone error: {e}");
                    break;
                }
            }
            let raw = rec.finish();
            println!("\n  captured {:.1}s", raw.duration_secs());
            if max_level < 0.001 {
                println!(
                    "  The input was silent: check Terminal's microphone permission and the input device."
                );
            }
            println!("Enter = keep · r = record again");
            if read_key(&lines).as_deref() == Some("r") {
                continue;
            }
            write_wav(&wav, &raw)?;
            let reference = if p.expect.is_empty() { &p.read } else { &p.expect };
            let txt = wav.with_extension("txt");
            if reference.is_empty() {
                let _ = std::fs::remove_file(&txt);
            } else {
                std::fs::write(&txt, reference)?;
            }
            let expected = out_dir.join(format!("{}.expected.txt", p.id));
            if p.expect.is_empty() {
                let _ = std::fs::remove_file(&expected);
            } else {
                std::fs::write(&expected, &p.expect)?;
            }
            println!("  saved {}", wav.display());
            recorded += 1;
            break;
        }
    }
    println!(
        "\nDone: {recorded} recorded. Score them with:\n  cargo run --release --example bench -- --models small.en-q5_1 --vad --format --show-text \\\n    --prompt \"FCI Tradecraft, FinCEN, BSA, AML, KYC, OFAC.\" ../recordings/voice/*.wav"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_prompts_parse() {
        let text = include_str!("../../scripts/voice-prompts.txt");
        let p = super::parse_prompts(text).unwrap();
        assert!(p.len() >= 5);
        assert!(p.iter().all(|p| !p.id.is_empty() && !p.note.is_empty()));
    }

    /// A perfect transcription of a punctuation passage (the read words, with no
    /// number words) must format to exactly its `expect` line.
    #[test]
    fn expectations_match_the_formatter() {
        let text = include_str!("../../scripts/voice-prompts.txt");
        let vocab: Vec<String> = loqwat_lib::settings::DEFAULT_VOCABULARY
            .iter()
            .map(|s| s.to_string())
            .collect();
        for p in super::parse_prompts(text).unwrap() {
            if p.expect.is_empty() || p.id.contains("numbers") {
                continue;
            }
            assert_eq!(
                loqwat_lib::format::format_transcript(&p.read, &vocab),
                p.expect,
                "passage {}",
                p.id
            );
        }
    }
}
