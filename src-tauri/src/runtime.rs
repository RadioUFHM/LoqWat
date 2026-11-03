//! Wires the [`Controller`] to worker threads.
//!
//! - Audio thread: owns the microphone stream (cpal streams are not `Send` on
//!   every platform), reports levels, the duration cap, and device errors.
//! - Engine thread: owns the whisper model and VAD; loads once, then serves
//!   transcription jobs. Inference never runs on the UI thread.
//!
//! OS-facing effects (indicator, tray, shortcuts, insertion) go through
//! [`Hooks`], implemented by the app shell.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::audio::RawAudio;
use crate::capture::{self, Recording};
use crate::controller::{
    Action, Controller, EngineStatus, Event, Outcome, RecordMode, SessionId, Snapshot,
};
use crate::engine::{TranscribeOptions, TranscriptionEngine};
use crate::vad::SpeechDetector;

pub trait Hooks: Send + Sync + 'static {
    fn publish(&self, snapshot: &Snapshot);
    /// Recording started: capture where the user is typing.
    fn session_started(&self, session: SessionId);
    fn set_cancel_shortcut(&self, enabled: bool);
    /// Deliver text. Must eventually dispatch `Event::InsertFinished`.
    fn insert(&self, runtime: Runtime, session: SessionId, text: String);
    fn level(&self, level: f32);
}

#[derive(Debug, Clone)]
pub struct Config {
    pub model_path: Option<PathBuf>,
    pub vad_path: Option<PathBuf>,
    pub vocabulary: Vec<String>,
    pub input_device: Option<String>,
    pub max_secs: u32,
    pub mode: RecordMode,
}

enum AudioCmd {
    Start(SessionId),
    Stop(SessionId, Arc<AtomicBool>),
    Discard(SessionId),
    Shutdown,
}

enum EngineCmd {
    Load,
    Transcribe(SessionId, RawAudio, Arc<AtomicBool>),
    Shutdown,
}

struct Inner {
    controller: Mutex<Controller>,
    audio_tx: Sender<AudioCmd>,
    engine_tx: Sender<EngineCmd>,
    /// Cancel flag for the session currently being transcribed.
    cancel: Mutex<Option<(SessionId, Arc<AtomicBool>)>>,
    /// Current configuration. The audio thread reads it at each recording
    /// start; the engine thread at each load and job.
    config: Mutex<Config>,
    hooks: Box<dyn Hooks>,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

impl Runtime {
    pub fn start(config: Config, hooks: impl Hooks) -> Self {
        let (audio_tx, audio_rx) = mpsc::channel();
        let (engine_tx, engine_rx) = mpsc::channel();
        let rt = Runtime {
            inner: Arc::new(Inner {
                controller: Mutex::new(Controller::new(config.mode)),
                audio_tx,
                engine_tx,
                cancel: Mutex::new(None),
                config: Mutex::new(config),
                hooks: Box::new(hooks),
            }),
        };
        {
            let rt = rt.clone();
            std::thread::Builder::new()
                .name("loqwat-audio".into())
                .spawn(move || audio_thread(rt, audio_rx))
                .expect("spawn audio thread");
        }
        {
            let rt = rt.clone();
            std::thread::Builder::new()
                .name("loqwat-engine".into())
                .spawn(move || engine_thread(rt, engine_rx))
                .expect("spawn engine thread");
        }
        let _ = rt.inner.engine_tx.send(EngineCmd::Load);
        rt
    }

    pub fn config(&self) -> Config {
        self.inner.config.lock().unwrap().clone()
    }

    /// Change the configuration. Microphone and vocabulary changes apply to
    /// the next recording; a model change needs [`Runtime::reload_engine`].
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.inner.config.lock().unwrap());
    }

    /// Unload the current model and load the configured one. Queued after any
    /// transcription already in progress.
    pub fn reload_engine(&self) {
        let _ = self.inner.engine_tx.send(EngineCmd::Load);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.controller.lock().unwrap().snapshot()
    }

    /// Feed an event to the state machine and perform the resulting actions.
    pub fn dispatch(&self, event: Event) {
        let actions = self.inner.controller.lock().unwrap().handle(event);
        for action in actions {
            self.perform(action);
        }
    }

    fn perform(&self, action: Action) {
        match action {
            Action::StartCapture(s) => {
                self.inner.hooks.session_started(s);
                let _ = self.inner.audio_tx.send(AudioCmd::Start(s));
            }
            Action::StopCapture(s) => {
                let flag = Arc::new(AtomicBool::new(false));
                *self.inner.cancel.lock().unwrap() = Some((s, flag.clone()));
                let _ = self.inner.audio_tx.send(AudioCmd::Stop(s, flag));
            }
            Action::DiscardCapture(s) => {
                let _ = self.inner.audio_tx.send(AudioCmd::Discard(s));
            }
            Action::CancelTranscription(s) => {
                if let Some((active, flag)) = self.inner.cancel.lock().unwrap().as_ref()
                    && *active == s
                {
                    flag.store(true, Ordering::Relaxed);
                }
            }
            Action::Insert(s, text) => self.inner.hooks.insert(self.clone(), s, text),
            Action::SetCancelShortcut(on) => self.inner.hooks.set_cancel_shortcut(on),
            // Always publish the latest state so concurrent dispatches can't
            // leave an older snapshot on screen.
            Action::Publish => self.inner.hooks.publish(&self.snapshot()),
        }
    }

    pub fn shutdown(&self) {
        let _ = self.inner.audio_tx.send(AudioCmd::Shutdown);
        let _ = self.inner.engine_tx.send(EngineCmd::Shutdown);
    }
}

fn audio_thread(rt: Runtime, rx: Receiver<AudioCmd>) {
    let mut active: Option<(SessionId, Recording)> = None;
    let mut limit_sent = false;
    let mut error_sent = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(AudioCmd::Start(s)) => {
                active = None; // release any previous stream first
                limit_sent = false;
                error_sent = false;
                let (device, max_secs) = {
                    let c = rt.inner.config.lock().unwrap();
                    (c.input_device.clone(), c.max_secs)
                };
                match Recording::start(device.as_deref(), max_secs) {
                    Ok(rec) => {
                        log::info!(
                            "session {s}: recording from {} ({} ch @ {} Hz)",
                            rec.device_name,
                            rec.channels(),
                            rec.sample_rate()
                        );
                        active = Some((s, rec));
                    }
                    Err(e) => rt.dispatch(Event::CaptureFailed(s, format!("{e:#}"))),
                }
            }
            Ok(AudioCmd::Stop(s, cancel)) => match active.take() {
                Some((id, rec)) if id == s => {
                    let audio = rec.finish();
                    log::info!("session {s}: captured {:.1}s", audio.duration_secs());
                    let _ = rt
                        .inner
                        .engine_tx
                        .send(EngineCmd::Transcribe(s, audio, cancel));
                }
                other => {
                    active = other;
                    rt.dispatch(Event::TranscriptionFinished(
                        s,
                        Outcome::Failed("no audio was captured".into()),
                    ));
                }
            },
            Ok(AudioCmd::Discard(s)) => {
                if active.as_ref().is_some_and(|(id, _)| *id == s) {
                    active = None; // drop releases the microphone; audio discarded
                    log::info!("session {s}: capture discarded");
                }
            }
            Ok(AudioCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some((s, rec)) = active.as_ref() {
            rt.inner.hooks.level(rec.level());
            if rec.limit_reached() && !limit_sent {
                limit_sent = true;
                rt.dispatch(Event::CaptureLimitReached(*s));
            }
            if let Some(err) = rec.stream_error()
                && !error_sent
            {
                error_sent = true;
                rt.dispatch(Event::CaptureFailed(*s, format!("Microphone error: {err}")));
            }
        }
    }
}

struct LoadedEngine {
    engine: TranscriptionEngine,
    vad: Option<SpeechDetector>,
}

fn engine_thread(rt: Runtime, rx: Receiver<EngineCmd>) {
    let mut loaded: Option<LoadedEngine> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            EngineCmd::Load => {
                // Free the old model before loading the next one.
                loaded = None;
                rt.dispatch(Event::EngineStatusChanged(EngineStatus::Loading));
                let cfg = rt.config();
                match load_engine(&cfg) {
                    Ok(l) => {
                        loaded = Some(l);
                        rt.dispatch(Event::EngineStatusChanged(EngineStatus::Ready));
                    }
                    Err(e) => {
                        log::error!("engine load failed: {e:#}");
                        rt.dispatch(Event::EngineStatusChanged(EngineStatus::Unavailable(
                            format!("{e:#}"),
                        )));
                    }
                }
            }
            EngineCmd::Transcribe(s, audio, cancel) => {
                let outcome = match loaded.as_mut() {
                    None => Outcome::Failed("transcription engine is not available".into()),
                    Some(l) => {
                        let vocabulary = rt.config().vocabulary;
                        let opts = TranscribeOptions {
                            initial_prompt: (!vocabulary.is_empty())
                                .then(|| vocabulary.join(", ") + "."),
                            ..Default::default()
                        };
                        run_job(l, s, &audio, &opts, cancel)
                    }
                };
                rt.dispatch(Event::TranscriptionFinished(s, outcome));
            }
            EngineCmd::Shutdown => return,
        }
    }
}

fn load_engine(cfg: &Config) -> anyhow::Result<LoadedEngine> {
    let model = cfg
        .model_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no speech model installed"))?;
    let mut engine = TranscriptionEngine::load(model, true)?;
    // Warm-up pass so the first real dictation runs at steady-state speed.
    let warm = engine.transcribe(&[0.0; 16_000], &TranscribeOptions::default(), None)?;
    log::info!(
        "engine ready: {} load {:.0} ms, warm-up {:.0} ms",
        engine.model_label(),
        engine.load_time().as_secs_f64() * 1000.0,
        warm.elapsed.as_secs_f64() * 1000.0
    );
    let vad = match cfg.vad_path.as_ref() {
        Some(p) => match SpeechDetector::load(p) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("VAD unavailable, speech gate disabled: {e:#}");
                None
            }
        },
        None => None,
    };
    Ok(LoadedEngine { engine, vad })
}

fn run_job(
    l: &mut LoadedEngine,
    s: SessionId,
    audio: &RawAudio,
    opts: &TranscribeOptions,
    cancel: Arc<AtomicBool>,
) -> Outcome {
    let start = Instant::now();
    if cancel.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    let input = match audio.to_whisper_input() {
        Ok(i) => i,
        Err(e) => return Outcome::Failed(format!("audio conversion failed: {e:#}")),
    };
    if let Some(vad) = l.vad.as_mut() {
        match vad.check(&input) {
            Ok(check) if !check.has_speech() => {
                log::info!("session {s}: no speech (VAD)");
                return Outcome::NoSpeech;
            }
            Ok(_) => {}
            // Fail open: a VAD error shouldn't lose the user's dictation.
            Err(e) => log::warn!("session {s}: VAD failed, continuing: {e:#}"),
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    match l.engine.transcribe(&input, opts, Some(cancel)) {
        Ok(t) if t.cancelled => Outcome::Cancelled,
        Ok(t) => {
            // Timing and sizes only; never the dictated content.
            log::info!(
                "session {s}: {:.1}s audio → {} chars in {:.0} ms (total {:.0} ms)",
                audio.duration_secs(),
                t.text.chars().count(),
                t.elapsed.as_secs_f64() * 1000.0,
                start.elapsed().as_secs_f64() * 1000.0
            );
            if t.text.is_empty() {
                Outcome::NoSpeech
            } else {
                Outcome::Text(t.text)
            }
        }
        Err(e) => Outcome::Failed(format!("transcription failed: {e:#}")),
    }
}

/// Model ids tried, in order, when the chosen model isn't installed.
pub const MODEL_FALLBACK_ORDER: &[&str] = &[
    "small.en-q5_1",
    "small.en",
    "base.en",
    "large-v3-turbo-q5_0",
    "medium.en-q5_0",
    "tiny.en",
];

/// Path of an installed catalog model, if present.
pub fn installed_model_path(id: &str) -> Option<PathBuf> {
    let dir = crate::models::models_dir().ok()?;
    let p = crate::models::find(id)?.path_in(&dir);
    p.is_file().then_some(p)
}

/// Resolve the model to use: the chosen one if installed, else the first
/// installed model in [`MODEL_FALLBACK_ORDER`]. Returns (id, path).
pub fn resolve_model(chosen: Option<&str>) -> Option<(String, PathBuf)> {
    chosen
        .into_iter()
        .chain(MODEL_FALLBACK_ORDER.iter().copied())
        .find_map(|id| installed_model_path(id).map(|p| (id.to_string(), p)))
}

/// Build the runtime configuration from saved settings.
pub fn config_from(settings: &crate::settings::Settings) -> Config {
    Config {
        model_path: resolve_model(settings.model_id.as_deref()).map(|(_, p)| p),
        vad_path: installed_model_path(crate::models::VAD_MODEL.id),
        vocabulary: settings.vocabulary.clone(),
        input_device: settings.input_device.clone(),
        max_secs: capture::DEFAULT_MAX_SECS,
        mode: settings.mode,
    }
}
