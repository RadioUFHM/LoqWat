//! Audio format conversion: everything the engine sees is mono f32 at 16 kHz.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

/// Sample rate whisper.cpp expects.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Raw audio as delivered by a device or file: interleaved f32 in [-1, 1].
#[derive(Debug, Clone)]
pub struct RawAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

impl RawAudio {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn duration_secs(&self) -> f32 {
        self.frames() as f32 / self.sample_rate as f32
    }

    /// Convert to mono 16 kHz for the engine.
    pub fn to_whisper_input(&self) -> Result<Vec<f32>> {
        let mono = downmix_to_mono(&self.samples, self.channels);
        resample_to_whisper(&mono, self.sample_rate)
    }
}

/// Average interleaved channels into one.
pub fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Resample mono audio to 16 kHz using an FFT resampler (fixed ratio, anti-aliased).
pub fn resample_to_whisper(mono: &[f32], input_rate: u32) -> Result<Vec<f32>> {
    if input_rate == WHISPER_SAMPLE_RATE || mono.is_empty() {
        return Ok(mono.to_vec());
    }
    if input_rate == 0 {
        bail!("input sample rate is zero");
    }
    let mut resampler = Fft::<f32>::new(
        input_rate as usize,
        WHISPER_SAMPLE_RATE as usize,
        1024,
        1,
        FixedSync::Input,
    )
    .context("creating resampler")?;
    let input = InterleavedSlice::new(mono, 1, mono.len()).context("wrapping input buffer")?;
    let needed = resampler.process_all_needed_output_len(mono.len());
    let mut output = vec![0.0f32; needed];
    let mut output_buf =
        rubato::audioadapter_buffers::direct::InterleavedSlice::new_mut(&mut output, 1, needed)
            .context("wrapping output buffer")?;
    let (_, produced) = resampler
        .process_all_into_buffer(&input, &mut output_buf, mono.len(), None)
        .context("resampling")?;
    output.truncate(produced);
    Ok(output)
}

/// Root-mean-square level of a buffer (0.0 for empty input).
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Peak absolute sample value.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Read a WAV file of any common PCM/float format.
pub fn read_wav(path: &Path) -> Result<RawAudio> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .context("reading float samples")?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .context("reading integer samples")?
        }
    };
    Ok(RawAudio {
        samples,
        channels: spec.channels,
        sample_rate: spec.sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn downmix_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn downmix_mono_is_identity() {
        let mono = [0.1, 0.2, 0.3];
        assert_eq!(downmix_to_mono(&mono, 1), mono.to_vec());
    }

    #[test]
    fn resample_16k_is_identity() {
        let s = sine(440.0, 16_000, 0.1);
        assert_eq!(resample_to_whisper(&s, 16_000).unwrap(), s);
    }

    #[test]
    fn resample_preserves_duration_and_level() {
        for rate in [44_100u32, 48_000, 22_050, 8_000] {
            let s = sine(440.0, rate, 1.0);
            let out = resample_to_whisper(&s, rate).unwrap();
            let expected = 16_000usize;
            let diff = (out.len() as i64 - expected as i64).abs();
            assert!(diff <= 2, "rate {rate}: got {} samples", out.len());
            // Interior RMS of a 0.5-amplitude sine is ~0.354.
            let level = rms(&out[1000..out.len() - 1000]);
            assert!((level - 0.354).abs() < 0.02, "rate {rate}: rms {level}");
        }
    }

    #[test]
    fn rms_and_peak() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5]) - 0.5).abs() < 1e-6);
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
    }
}
