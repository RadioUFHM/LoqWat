//! Transcription benchmark: cold load vs. warm transcription, per model and file.
//!
//!   cargo run --release --example bench -- --models base.en,small.en [--runs 3]
//!       [--beam 5] [--prompt "FinCEN, OFAC"] [--cpu] [--vad] [--format] [--show-text] file1.wav ...
//!
//! `--vad` runs the Silero speech gate first; files without speech skip transcription.
//! `--format` applies LoqWat's text formatting (spoken punctuation, number style) to the
//! output and, like the app, appends the punctuation hint to the `--prompt` vocabulary; then if `<file>.expected.txt` exists the result is compared
//! to it exactly and a ✓/✗ line is printed.
//!
//! If `<file>.txt` exists next to a WAV, word error rate (WER) against it is reported.
//! Each model is benchmarked in a fresh child process so memory figures and
//! cold-load times are not contaminated by previously loaded models.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use loqwat_lib::{audio, engine, format, metrics, models, vad};

struct Args {
    models: Vec<String>,
    runs: usize,
    beam: Option<i32>,
    prompt: Option<String>,
    cpu: bool,
    vad: bool,
    format: bool,
    show_text: bool,
    child: bool,
    files: Vec<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        models: vec!["base.en".into()],
        runs: 3,
        beam: None,
        prompt: None,
        cpu: false,
        vad: false,
        format: false,
        show_text: false,
        child: false,
        files: vec![],
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--models" => {
                a.models = it
                    .next()
                    .context("--models value")?
                    .split(',')
                    .map(String::from)
                    .collect()
            }
            "--runs" => a.runs = it.next().context("--runs value")?.parse()?,
            "--beam" => a.beam = Some(it.next().context("--beam value")?.parse()?),
            "--prompt" => a.prompt = Some(it.next().context("--prompt value")?),
            "--cpu" => a.cpu = true,
            "--vad" => a.vad = true,
            "--format" => a.format = true,
            "--show-text" => a.show_text = true,
            "--child" => a.child = true,
            _ => a.files.push(PathBuf::from(arg)),
        }
    }
    if a.files.is_empty() {
        bail!("no WAV files given");
    }
    Ok(a)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    if args.child {
        return run_model(&args, &args.models[0]);
    }
    // Parent: re-run self once per model in a fresh process.
    let exe = std::env::current_exe()?;
    let passthrough: Vec<String> = std::env::args().skip(1).collect();
    for model in &args.models {
        let mut child_args: Vec<String> = Vec::new();
        let mut skip = false;
        for a in &passthrough {
            if skip {
                skip = false;
                continue;
            }
            if a == "--models" {
                skip = true;
                continue;
            }
            child_args.push(a.clone());
        }
        let status = Command::new(&exe)
            .arg("--child")
            .arg("--models")
            .arg(model)
            .args(&child_args)
            .status()?;
        if !status.success() {
            eprintln!("model {model}: benchmark failed ({status})");
        }
    }
    Ok(())
}

fn run_model(args: &Args, model_id: &str) -> Result<()> {
    let dir = models::models_dir()?;
    let info = models::find(model_id).with_context(|| format!("unknown model {model_id}"))?;
    let path = info.path_in(&dir);
    if !path.exists() {
        bail!(
            "{model_id} not installed; run: cargo run --release --example fetch_model -- {model_id}"
        );
    }
    let backend = if args.cpu { "CPU" } else { "Metal/GPU" };
    println!(
        "\n=== {model_id} ({backend}, {}) ===",
        args.beam
            .map_or("greedy".to_string(), |b| format!("beam {b}"))
    );

    let rss_before = metrics::peak_rss_bytes();
    let mut eng = engine::TranscriptionEngine::load(&path, !args.cpu)?;
    println!(
        "cold load: {:.0} ms   peak RSS before/after load: {} / {}",
        eng.load_time().as_secs_f64() * 1000.0,
        metrics::format_mb(rss_before),
        metrics::format_mb(metrics::peak_rss_bytes())
    );

    // Same vocabulary the prompt carries, as the app passes its vocabulary.
    let vocabulary: Vec<String> = args
        .prompt
        .as_deref()
        .unwrap_or("")
        .trim_end_matches('.')
        .split(", ")
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    let opts = engine::TranscribeOptions {
        beam_size: args.beam,
        initial_prompt: if args.format {
            Some(format::whisper_prompt(&vocabulary))
        } else {
            args.prompt.clone()
        },
        ..Default::default()
    };

    let mut gate = if args.vad {
        let p = models::VAD_MODEL.path_in(&dir);
        Some(
            vad::SpeechDetector::load(&p)
                .context("VAD model missing; run fetch_model silero-v6.2.0")?,
        )
    } else {
        None
    };

    println!(
        "{:<28} {:>6} {:>9} {:>9} {:>6} {:>6} {:>6}",
        "file", "audio", "1st ms", "warm ms", "RTF", "WER", "nospch"
    );
    for file in &args.files {
        let raw = audio::read_wav(file)?;
        let conv_start = Instant::now();
        let input = raw.to_whisper_input()?;
        let conv_ms = conv_start.elapsed().as_secs_f64() * 1000.0;

        if let Some(g) = gate.as_mut() {
            let t = Instant::now();
            let check = g.check(&input)?;
            let vad_ms = t.elapsed().as_secs_f64() * 1000.0;
            if !check.has_speech() {
                println!(
                    "{:<28} {:>5.1}s  no speech detected by VAD ({vad_ms:.0} ms) — transcription skipped",
                    file.file_name().unwrap().to_string_lossy(),
                    raw.duration_secs()
                );
                continue;
            }
            if args.show_text {
                println!(
                    "    vad: {} segment(s), {:.1}s speech, {vad_ms:.0} ms",
                    check.segments, check.speech_secs
                );
            }
        }

        let first = eng.transcribe(&input, &opts, None)?;
        let mut warm = Vec::new();
        let mut last = first.clone();
        for _ in 0..args.runs {
            let t = eng.transcribe(&input, &opts, None)?;
            warm.push(t.elapsed.as_secs_f64() * 1000.0);
            last = t;
        }
        if args.format {
            last.text = format::format_transcript(&last.text, &vocabulary);
        }
        warm.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let warm_med = warm.get(warm.len() / 2).copied().unwrap_or(f64::NAN);
        let dur = raw.duration_secs() as f64;
        let wer = reference_for(file).map(|r| word_error_rate(&r, &last.text));
        println!(
            "{:<28} {:>5.1}s {:>9.0} {:>9.0} {:>6.3} {:>6} {:>6.2}",
            file.file_name().unwrap().to_string_lossy(),
            dur,
            first.elapsed.as_secs_f64() * 1000.0,
            warm_med,
            warm_med / 1000.0 / dur,
            wer.map_or("-".into(), |w| format!("{:.1}%", w * 100.0)),
            last.max_no_speech_prob
        );
        if args.show_text {
            println!(
                "    text: {:?}  (convert {:.1} ms, {}ch@{}Hz)",
                last.text, conv_ms, raw.channels, raw.sample_rate
            );
        }
        let expected = std::fs::read_to_string(file.with_extension("expected.txt")).ok();
        if let (true, Some(exp)) = (args.format, expected) {
            let exp = exp.trim();
            if last.text == exp {
                println!("    exact: ✓");
            } else {
                println!("    exact: ✗\n      want: {exp:?}\n      got:  {:?}", last.text);
            }
        }
    }
    println!(
        "peak RSS after transcription: {}",
        metrics::format_mb(metrics::peak_rss_bytes())
    );
    Ok(())
}

fn reference_for(wav: &Path) -> Option<String> {
    std::fs::read_to_string(wav.with_extension("txt")).ok()
}

fn normalize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '\'' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Word-level Levenshtein distance / reference length.
fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let r = normalize(reference);
    let h = normalize(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            let sub = prev[j] + usize::from(rw != hw);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len() as f64
}
