//! Persistent whisper.cpp transcription engine.
//!
//! The model is loaded once and a single decoding state is reused, so warm
//! transcriptions don't pay model-load or GPU-buffer allocation costs.
//! Callers must keep transcription off the UI thread.

use std::ffi::c_void;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

use crate::audio::WHISPER_SAMPLE_RATE;

#[derive(Debug, Clone)]
pub struct TranscribeOptions {
    /// None = greedy decoding; Some(n) = beam search with width n.
    pub beam_size: Option<i32>,
    /// Vocabulary/context hint passed as the initial prompt.
    pub initial_prompt: Option<String>,
    pub threads: i32,
}

impl Default for TranscribeOptions {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        Self {
            beam_size: None,
            initial_prompt: None,
            threads: cores.clamp(1, 8) as i32,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub segments: usize,
    /// Highest per-segment no-speech probability (1.0 when there are no segments).
    pub max_no_speech_prob: f32,
    pub elapsed: Duration,
    pub cancelled: bool,
}

pub struct TranscriptionEngine {
    // Field order matters: state must drop before the context it was created from.
    state: WhisperState,
    _ctx: WhisperContext,
    model_label: String,
    load_time: Duration,
}

impl TranscriptionEngine {
    /// Load a ggml model file. Uses the GPU (Metal) where compiled in.
    pub fn load(model_path: &Path, use_gpu: bool) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        let start = Instant::now();
        let path = model_path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let params = WhisperContextParameters {
            use_gpu,
            flash_attn: use_gpu,
            ..Default::default()
        };
        let ctx = WhisperContext::new_with_params(path, params)
            .with_context(|| format!("loading model {}", model_path.display()))?;
        let state = ctx.create_state().context("creating whisper state")?;
        Ok(Self {
            state,
            _ctx: ctx,
            model_label: model_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            load_time: start.elapsed(),
        })
    }

    pub fn model_label(&self) -> &str {
        &self.model_label
    }

    pub fn load_time(&self) -> Duration {
        self.load_time
    }

    /// Transcribe mono 16 kHz audio. `cancel` aborts decoding when set.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        opts: &TranscribeOptions,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<Transcript> {
        let start = Instant::now();
        // whisper.cpp needs at least ~1 s; pad short clips with trailing silence.
        let min_len = WHISPER_SAMPLE_RATE as usize + WHISPER_SAMPLE_RATE as usize / 10;
        let padded;
        let input = if samples.len() < min_len {
            padded = {
                let mut v = samples.to_vec();
                v.resize(min_len, 0.0);
                v
            };
            &padded[..]
        } else {
            samples
        };

        let strategy = match opts.beam_size {
            Some(n) => SamplingStrategy::BeamSearch {
                beam_size: n,
                patience: -1.0,
            },
            None => SamplingStrategy::Greedy { best_of: 1 },
        };
        let mut params = FullParams::new(strategy);
        params.set_language(Some("en"));
        params.set_translate(false);
        params.set_n_threads(opts.threads);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(prompt) = opts.initial_prompt.as_deref().filter(|p| !p.is_empty()) {
            params.set_initial_prompt(prompt);
        }
        // whisper-rs 0.16's `set_abort_callback_safe` boxes the closure as
        // `Box<Box<dyn FnMut>>` but its trampoline casts the pointer to the
        // concrete closure type, so it reads garbage and aborts encoding
        // (whisper error -6). Use the raw API with our own trampoline instead.
        if let Some(flag) = cancel.as_ref() {
            // SAFETY: the pointer targets the AtomicBool inside `cancel`, which
            // this function keeps alive until `full` has returned.
            unsafe {
                params.set_abort_callback(Some(abort_trampoline));
                params.set_abort_callback_user_data(Arc::as_ptr(flag) as *mut c_void);
            }
        }

        let result = self.state.full(params, input);
        let cancelled = cancel.is_some_and(|f| f.load(Ordering::Relaxed));
        if cancelled {
            return Ok(Transcript {
                text: String::new(),
                segments: 0,
                max_no_speech_prob: 1.0,
                elapsed: start.elapsed(),
                cancelled: true,
            });
        }
        result.context("whisper inference failed")?;

        let mut text = String::new();
        let mut segments = 0;
        let mut max_no_speech = if self.state.full_n_segments() == 0 {
            1.0
        } else {
            0.0f32
        };
        for seg in self.state.as_iter() {
            segments += 1;
            max_no_speech = max_no_speech.max(seg.no_speech_probability());
            text.push_str(&seg.to_str_lossy().context("reading segment text")?);
        }
        Ok(Transcript {
            text: clean_transcript(&text),
            segments,
            max_no_speech_prob: max_no_speech,
            elapsed: start.elapsed(),
            cancelled: false,
        })
    }
}

/// whisper.cpp abort callback: returns true when the cancel flag is set.
unsafe extern "C" fn abort_trampoline(user_data: *mut c_void) -> bool {
    // SAFETY: `user_data` is the AtomicBool pointer installed in `transcribe`.
    unsafe { &*(user_data as *const AtomicBool) }.load(Ordering::Relaxed)
}

/// Remove non-speech annotations whisper emits for silence/noise, e.g.
/// "[BLANK_AUDIO]", "(wind blowing)", "[Music]", and normalize whitespace.
/// Spoken words are never altered.
pub fn clean_transcript(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut depth_sq = 0u32;
    let mut depth_paren = 0u32;
    for c in raw.chars() {
        match c {
            '[' => depth_sq += 1,
            ']' if depth_sq > 0 => depth_sq -= 1,
            '(' => depth_paren += 1,
            ')' if depth_paren > 0 => depth_paren -= 1,
            _ if depth_sq == 0 && depth_paren == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::clean_transcript;

    #[test]
    fn strips_annotations() {
        assert_eq!(clean_transcript(" [BLANK_AUDIO]"), "");
        assert_eq!(
            clean_transcript("(wind blowing) Hello  there."),
            "Hello there."
        );
        assert_eq!(clean_transcript(" Hello [Music] world"), "Hello world");
    }

    #[test]
    fn keeps_plain_text() {
        assert_eq!(
            clean_transcript(" FinCEN filed 3 SARs on 4/12."),
            "FinCEN filed 3 SARs on 4/12."
        );
    }
}
