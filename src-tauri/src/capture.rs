//! Microphone capture via cpal. Audio stays in memory, bounded by `max_secs`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use crate::audio::RawAudio;

/// Initial recording cap (spec: five minutes).
pub const DEFAULT_MAX_SECS: u32 = 300;

#[derive(Debug, Clone)]
pub struct InputDeviceInfo {
    pub name: String,
    pub is_default: bool,
}

pub fn list_input_devices() -> Result<Vec<InputDeviceInfo>> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| device_name(&d));
    let devices = host.input_devices().context("enumerating input devices")?;
    Ok(devices
        .filter_map(|d| device_name(&d))
        .map(|name| InputDeviceInfo {
            is_default: Some(&name) == default_name.as_ref(),
            name,
        })
        .collect())
}

fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|desc| desc.to_string())
}

struct Shared {
    samples: Mutex<Vec<f32>>,
    max_samples: usize,
    limit_reached: AtomicBool,
    /// Last callback's peak level, as f32 bits, for a live meter.
    level_bits: AtomicU32,
    stream_error: Mutex<Option<String>>,
}

/// An active recording. Dropping it stops capture and releases the microphone.
pub struct Recording {
    stream: Option<cpal::Stream>,
    shared: Arc<Shared>,
    channels: u16,
    sample_rate: u32,
    pub device_name: String,
}

impl Recording {
    /// Start recording from the named device, or the system default.
    pub fn start(device: Option<&str>, max_secs: u32) -> Result<Self> {
        let host = cpal::default_host();
        let device = match device {
            Some(name) => host
                .input_devices()
                .context("enumerating input devices")?
                .find(|d| device_name(d).as_deref() == Some(name))
                .ok_or_else(|| anyhow!("input device \"{name}\" not found"))?,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow!("no microphone / input device available"))?,
        };
        let name = device_name(&device).unwrap_or_else(|| "unknown".into());
        let config = device
            .default_input_config()
            .context("reading input device configuration")?;
        let channels = config.channels();
        let sample_rate = config.sample_rate();
        let shared = Arc::new(Shared {
            samples: Mutex::new(Vec::with_capacity(
                sample_rate as usize * channels as usize * 30,
            )),
            max_samples: sample_rate as usize * channels as usize * max_secs as usize,
            limit_reached: AtomicBool::new(false),
            level_bits: AtomicU32::new(0),
            stream_error: Mutex::new(None),
        });

        let stream = match config.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config.into(), &shared),
            SampleFormat::I16 => build::<i16>(&device, &config.into(), &shared),
            SampleFormat::I32 => build::<i32>(&device, &config.into(), &shared),
            SampleFormat::U16 => build::<u16>(&device, &config.into(), &shared),
            SampleFormat::I8 => build::<i8>(&device, &config.into(), &shared),
            SampleFormat::U8 => build::<u8>(&device, &config.into(), &shared),
            other => bail!("unsupported microphone sample format {other}"),
        }?;
        stream.play().context("starting microphone stream")?;

        Ok(Self {
            stream: Some(stream),
            shared,
            channels,
            sample_rate,
            device_name: name,
        })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Peak level (0..1) of the most recent audio callback.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level_bits.load(Ordering::Relaxed))
    }

    pub fn limit_reached(&self) -> bool {
        self.shared.limit_reached.load(Ordering::Relaxed)
    }

    pub fn elapsed_secs(&self) -> f32 {
        let n = self.shared.samples.lock().map_or(0, |s| s.len());
        n as f32 / (self.sample_rate as f32 * self.channels as f32)
    }

    /// Error reported by the audio backend (e.g. device disconnected), if any.
    pub fn stream_error(&self) -> Option<String> {
        self.shared.stream_error.lock().ok().and_then(|e| e.clone())
    }

    /// Stop capture, release the microphone, and return the captured audio.
    pub fn finish(mut self) -> RawAudio {
        self.stream.take(); // dropping the stream stops it and releases the device
        let samples = std::mem::take(&mut *self.shared.samples.lock().unwrap());
        RawAudio {
            samples,
            channels: self.channels,
            sample_rate: self.sample_rate,
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let data_shared = shared.clone();
    let err_shared = shared.clone();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut peak = 0.0f32;
                let Ok(mut buf) = data_shared.samples.lock() else {
                    return;
                };
                let room = data_shared.max_samples.saturating_sub(buf.len());
                if room < data.len() {
                    data_shared.limit_reached.store(true, Ordering::Relaxed);
                }
                for &s in data.iter().take(room) {
                    let v: f32 = s.to_sample();
                    peak = peak.max(v.abs());
                    buf.push(v);
                }
                data_shared
                    .level_bits
                    .store(peak.to_bits(), Ordering::Relaxed);
            },
            move |err| {
                if let Ok(mut e) = err_shared.stream_error.lock() {
                    *e = Some(err.to_string());
                }
            },
            None,
        )
        .context("opening microphone stream (is microphone access allowed?)")
}
