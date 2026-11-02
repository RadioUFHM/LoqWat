//! Speech gate using Silero VAD (via whisper.cpp).
//!
//! Used only to decide whether a recording contains speech at all, so silent or
//! noise-only recordings don't produce invented text (whisper tends to emit
//! "you" / "Thank you." on silence). Audio is never trimmed by the gate, so
//! quiet speech and final syllables reach the transcriber intact.

use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

#[derive(Debug, Clone, Copy)]
pub struct SpeechCheck {
    pub segments: usize,
    pub speech_secs: f32,
}

impl SpeechCheck {
    pub fn has_speech(&self) -> bool {
        self.segments > 0
    }
}

pub struct SpeechDetector {
    ctx: WhisperVadContext,
}

impl SpeechDetector {
    pub fn load(model_path: &Path) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        let path = model_path
            .to_str()
            .context("VAD model path is not valid UTF-8")?;
        let mut params = WhisperVadContextParams::default();
        params.set_n_threads(2);
        // The VAD model is tiny; CPU avoids contending with the GPU transcriber.
        params.set_use_gpu(false);
        let ctx = WhisperVadContext::new(path, params)
            .with_context(|| format!("loading VAD model {}", model_path.display()))?;
        Ok(Self { ctx })
    }

    /// Check mono 16 kHz audio for speech.
    pub fn check(&mut self, samples: &[f32]) -> Result<SpeechCheck> {
        if samples.is_empty() {
            return Ok(SpeechCheck {
                segments: 0,
                speech_secs: 0.0,
            });
        }
        let mut params = WhisperVadParams::new();
        // Conservative: the default 0.5 threshold, short minimum speech so a
        // single quick word ("yes") still counts.
        params.set_threshold(0.5);
        params.set_min_speech_duration(150);
        let segs = self
            .ctx
            .segments_from_samples(params, samples)
            .context("running VAD")?;
        let mut speech_secs = 0.0;
        let n = segs.num_segments().max(0) as usize;
        for i in 0..n as i32 {
            if let Some(seg) = segs.get_segment(i) {
                // Timestamps are in centiseconds.
                speech_secs += (seg.end - seg.start).max(0.0) / 100.0;
            }
        }
        Ok(SpeechCheck {
            segments: n,
            speech_secs,
        })
    }
}
