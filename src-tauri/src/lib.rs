//! LoqWat application entry point: menu bar app, global shortcut, indicator,
//! setup, preferences and model manager.

pub mod audio;
pub mod capture;
pub mod controller;
pub mod engine;
pub mod metrics;
pub mod models;
pub mod platform;
pub mod runtime;
pub mod settings;
pub mod shortcut;
pub mod vad;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use controller::{EngineStatus, Event, InsertResult, Phase, RecordMode, SessionId, Snapshot};
use platform::{Cue, InsertOutcome, Target};
use runtime::{Hooks, Runtime};
use settings::Settings;

const CANCEL_SHORTCUT: &str = "Escape";
const INDICATOR_SIZE: (f64, f64) = (220.0, 44.0);
/// Countdown before an explicit "insert into another field" from recovery.
const RECOVERY_INSERT_DELAY: Duration = Duration::from_secs(3);
/// Length of the microphone test in setup/preferences.
const MIC_TEST_SECS: u64 = 3;
/// Peak level above which the microphone test counts as "heard you".
const MIC_TEST_HEARD_PEAK: f32 = 0.02;

/// Everything the UI shows.
#[derive(Debug, Clone, Serialize)]
struct UiState {
    #[serde(flatten)]
    snapshot: Snapshot,
    settings: Settings,
    shortcut_error: Option<String>,
    /// Catalog id + label of the model the engine is configured with.
    model_id: Option<String>,
    model: Option<String>,
    vad_installed: bool,
    version: &'static str,
    accessibility: bool,
    has_last_text: bool,
    launch_at_login: bool,
}

#[derive(Default)]
struct ShortcutReg {
    /// The desired state we last acted on (`Some(None)` = deliberately unregistered).
    applied: Option<Option<String>>,
    /// Shortcut currently registered with the OS.
    registered: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DownloadState {
    id: String,
    done: u64,
    total: u64,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
}

struct Shell {
    settings_path: Option<PathBuf>,
    settings: Mutex<Settings>,
    shortcut: Mutex<ShortcutReg>,
    shortcut_error: Mutex<Option<String>>,
    /// The UI is recording a new shortcut; the current one is released meanwhile.
    capturing_shortcut: AtomicBool,
    /// Where the user was typing when the current session started.
    target: Mutex<Option<(SessionId, Target)>>,
    /// Most recent transcript, memory only, for "Copy Last Transcript".
    last_text: Mutex<Option<String>>,
    download: Mutex<Option<DownloadState>>,
    mic_test_running: AtomicBool,
    /// Previous phase, for sound cues.
    last_phase: Mutex<Phase>,
}

impl Shell {
    fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Change and persist settings. Saving failures are logged, not fatal.
    fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = self.settings.lock().unwrap();
        f(&mut s);
        if let Some(path) = &self.settings_path
            && let Err(e) = s.save_to(path)
        {
            log::error!("saving settings: {e:#}");
        }
        s.clone()
    }
}

struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    mode_toggle: CheckMenuItem<tauri::Wry>,
    mode_hold: CheckMenuItem<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
    copy_last: MenuItem<tauri::Wry>,
}

/// Catalog entry for a model file path, if it is one of ours.
fn model_for_path(path: Option<&PathBuf>) -> Option<&'static models::ModelInfo> {
    let file = path?.file_name()?.to_str()?;
    models::CATALOG.iter().find(|m| m.file == file)
}

fn ui_state(app: &AppHandle, snapshot: Snapshot) -> UiState {
    let shell = app.state::<Shell>();
    let config = app.try_state::<Runtime>().map(|rt| rt.config());
    let model = model_for_path(config.as_ref().and_then(|c| c.model_path.as_ref()));
    UiState {
        snapshot,
        settings: shell.settings(),
        shortcut_error: shell.shortcut_error.lock().unwrap().clone(),
        model_id: model.map(|m| m.id.to_string()),
        model: model.map(|m| m.label.to_string()),
        vad_installed: runtime::installed_model_path(models::VAD_MODEL.id).is_some(),
        version: env!("CARGO_PKG_VERSION"),
        accessibility: platform::accessibility_trusted(false),
        has_last_text: shell.last_text.lock().unwrap().is_some(),
        launch_at_login: app.autolaunch().is_enabled().unwrap_or(false),
    }
}

fn publish_state(app: &AppHandle) {
    if let Some(rt) = app.try_state::<Runtime>() {
        let _ = app.emit("state", ui_state(app, rt.snapshot()));
    }
}

struct AppHooks {
    app: AppHandle,
}

impl Hooks for AppHooks {
    fn publish(&self, snapshot: &Snapshot) {
        let shell = self.app.state::<Shell>();
        play_cues(&self.app, snapshot);
        if shell.settings.lock().unwrap().mode != snapshot.mode {
            shell.update_settings(|s| s.mode = snapshot.mode);
        }
        sync_shortcut(&self.app, snapshot.paused);
        let state = ui_state(&self.app, snapshot.clone());
        let _ = self.app.emit("state", &state);
        update_tray(&self.app, snapshot);
        update_indicator(&self.app, snapshot);
        if snapshot.phase == Phase::Recovery {
            show_main_window(&self.app);
        }
    }

    fn set_cancel_shortcut(&self, enabled: bool) {
        // Escape is only claimed while a session is active, so normal Escape
        // use in other apps is untouched when LoqWat is idle.
        let gs = self.app.global_shortcut();
        let result = if enabled {
            gs.on_shortcut(CANCEL_SHORTCUT, |app, _, ev| {
                if ev.state == ShortcutState::Pressed
                    && let Some(rt) = app.try_state::<Runtime>()
                {
                    rt.dispatch(Event::CancelPressed);
                }
            })
        } else {
            gs.unregister(CANCEL_SHORTCUT)
        };
        if let Err(e) = result {
            log::warn!("Escape shortcut (enabled={enabled}): {e}");
        }
    }

    fn session_started(&self, session: SessionId) {
        let app = self.app.clone();
        std::thread::spawn(move || {
            let target = platform::capture_target();
            if let Some(t) = &target {
                log::info!(
                    "session {session}: target {} (field known: {})",
                    t.bundle_id.as_deref().unwrap_or(&t.app_name),
                    t.field_known
                );
            }
            *app.state::<Shell>().target.lock().unwrap() = target.map(|t| (session, t));
        });
    }

    fn insert(&self, runtime: Runtime, session: SessionId, text: String) {
        let app = self.app.clone();
        std::thread::spawn(move || {
            let shell = app.state::<Shell>();
            *shell.last_text.lock().unwrap() = Some(text.clone());
            let target = shell
                .target
                .lock()
                .unwrap()
                .take()
                .filter(|(s, _)| *s == session)
                .map(|(_, t)| t);
            let result = deliver(target.as_ref(), text);
            log::info!(
                "session {session}: insert → {}",
                match &result {
                    InsertResult::Inserted => "confirmed".to_string(),
                    InsertResult::Unverified => "sent, unverified".to_string(),
                    InsertResult::NotInserted { reason, .. } => format!("not inserted ({reason})"),
                }
            );
            runtime.dispatch(Event::InsertFinished(session, result));
        });
    }

    fn level(&self, level: f32) {
        let _ = self.app.emit_to("indicator", "level", level);
    }
}

/// Optional sound cues on phase changes.
fn play_cues(app: &AppHandle, s: &Snapshot) {
    let shell = app.state::<Shell>();
    let prev = std::mem::replace(&mut *shell.last_phase.lock().unwrap(), s.phase);
    if prev == s.phase || !shell.settings.lock().unwrap().sound_cues {
        return;
    }
    let cue = match (prev, s.phase) {
        (_, Phase::Recording) => Some(Cue::Start),
        (Phase::Recording, Phase::Transcribing) => Some(Cue::Stop),
        (Phase::Recording | Phase::Transcribing, Phase::Idle)
            if s.message.as_deref() == Some("Cancelled") =>
        {
            Some(Cue::Cancel)
        }
        _ => None,
    };
    if let Some(cue) = cue {
        platform::play_cue(cue);
    }
}

/// Insert into the captured target after revalidating it. Any doubt about the
/// destination goes to the recovery window instead of guessing.
fn deliver(target: Option<&Target>, text: String) -> InsertResult {
    let not_inserted = |reason: String| InsertResult::NotInserted {
        text: text.clone(),
        reason,
    };
    if !platform::accessibility_trusted(false) {
        return not_inserted(platform::TargetCheck::NoPermission.reason());
    }
    let Some(target) = target else {
        return not_inserted(
            "LoqWat couldn't tell where you were typing when recording started.".into(),
        );
    };
    // ⌥ from the shortcut must be up, or ⌘V would become ⌥⌘V.
    if !platform::wait_for_modifiers_released(Duration::from_secs(2)) {
        return not_inserted("Modifier keys were still held down, so LoqWat didn't paste.".into());
    }
    let check = platform::check_target(Some(target));
    if !check.allows_insert() {
        return not_inserted(check.reason());
    }
    match platform::insert_text(&text) {
        InsertOutcome::Confirmed => InsertResult::Inserted,
        InsertOutcome::Unverified => InsertResult::Unverified,
        InsertOutcome::Failed(e) => not_inserted(format!("Paste failed: {e}.")),
    }
}

fn phase_label(s: &Snapshot) -> String {
    if s.paused {
        return "Paused".into();
    }
    match s.phase {
        Phase::Idle => match &s.engine {
            EngineStatus::Ready => "Ready".into(),
            EngineStatus::Loading | EngineStatus::NotLoaded => "Loading model…".into(),
            EngineStatus::Unavailable(_) => "Model unavailable".into(),
        },
        Phase::Recording => "Listening…".into(),
        Phase::Transcribing => "Transcribing…".into(),
        Phase::Inserting => "Inserting…".into(),
        Phase::Recovery => "Text ready (not inserted)".into(),
        Phase::Error => "Error".into(),
    }
}

fn update_tray(app: &AppHandle, s: &Snapshot) {
    let Some(items) = app.try_state::<TrayItems>() else {
        return;
    };
    let _ = items
        .status
        .set_text(format!("LoqWat — {}", phase_label(s)));
    let _ = items.toggle.set_text(if s.phase == Phase::Recording {
        "Stop Dictation"
    } else {
        "Start Dictation"
    });
    let _ = items
        .toggle
        .set_enabled(!s.paused && !matches!(s.phase, Phase::Transcribing | Phase::Inserting));
    let _ = items.mode_toggle.set_checked(s.mode == RecordMode::Toggle);
    let _ = items.mode_hold.set_checked(s.mode == RecordMode::Hold);
    let _ = items.pause.set_checked(s.paused);
    let has_last = app.state::<Shell>().last_text.lock().unwrap().is_some();
    let _ = items.copy_last.set_enabled(has_last);
    if let Some(tray) = app.tray_by_id("main") {
        let title = match s.phase {
            Phase::Recording => Some("● REC"),
            Phase::Transcribing | Phase::Inserting => Some("…"),
            Phase::Error => Some("!"),
            _ => None,
        };
        let _ = tray.set_title(title);
        let _ = tray.set_tooltip(Some(format!("LoqWat — {}", phase_label(s))));
    }
}

fn update_indicator(app: &AppHandle, s: &Snapshot) {
    let Some(win) = app.get_webview_window("indicator") else {
        return;
    };
    let _ = app.emit_to("indicator", "indicator", phase_label(s));
    let visible = matches!(
        s.phase,
        Phase::Recording | Phase::Transcribing | Phase::Inserting
    );
    // The indicator is non-focusable, so showing it must not take keyboard
    // focus from the app the user is dictating into.
    let _ = if visible { win.show() } else { win.hide() };
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_indicator(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, "indicator", WebviewUrl::App("indicator.html".into()))
        .title("LoqWat")
        .inner_size(INDICATOR_SIZE.0, INDICATOR_SIZE.1)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .shadow(true)
        .visible(false)
        .build()?;
    if let Some(m) = win.primary_monitor()? {
        let scale = m.scale_factor();
        let size = m.size().to_logical::<f64>(scale);
        let pos = m.position().to_logical::<f64>(scale);
        let x = pos.x + (size.width - INDICATOR_SIZE.0) / 2.0;
        let y = pos.y + 36.0; // just below the menu bar
        win.set_position(tauri::LogicalPosition::new(x, y))?;
    }
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "LoqWat — Starting…", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start Dictation", true, None::<&str>)?;
    let mode_toggle = CheckMenuItem::with_id(
        app,
        "mode_toggle",
        "Press to start / press to stop",
        true,
        true,
        None::<&str>,
    )?;
    let mode_hold =
        CheckMenuItem::with_id(app, "mode_hold", "Hold to talk", true, false, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause Shortcut", true, false, None::<&str>)?;
    let copy_last = MenuItem::with_id(
        app,
        "copy_last",
        "Copy Last Transcript",
        false,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, "open", "Open LoqWat…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit LoqWat", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &PredefinedMenuItem::separator(app)?,
            &mode_toggle,
            &mode_hold,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &copy_last,
            &open,
            &quit,
        ],
    )?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("LoqWat")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let Some(rt) = app.try_state::<Runtime>() else {
                return;
            };
            match event.id.as_ref() {
                "toggle" => rt.dispatch(Event::ToggleRequested),
                "mode_toggle" => rt.dispatch(Event::SetMode(RecordMode::Toggle)),
                "mode_hold" => rt.dispatch(Event::SetMode(RecordMode::Hold)),
                "pause" => {
                    let paused = rt.snapshot().paused;
                    rt.dispatch(Event::SetPaused(!paused));
                }
                "copy_last" => {
                    if let Some(t) = app.state::<Shell>().last_text.lock().unwrap().as_deref() {
                        platform::copy_to_clipboard(t);
                    }
                }
                "open" => show_main_window(app),
                "quit" => quit_app(app),
                _ => {}
            }
        })
        .build(app)?;
    app.manage(TrayItems {
        status,
        toggle,
        mode_toggle,
        mode_hold,
        pause,
        copy_last,
    });
    Ok(())
}

fn quit_app(app: &AppHandle) {
    if let Some(rt) = app.try_state::<Runtime>() {
        rt.shutdown();
    }
    app.exit(0);
}

// ---------------------------------------------------------------- shortcut

fn register_dictation(app: &AppHandle, accelerator: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(accelerator, |app, _, ev| {
            let Some(rt) = app.try_state::<Runtime>() else {
                return;
            };
            match ev.state {
                ShortcutState::Pressed => rt.dispatch(Event::ShortcutPressed),
                ShortcutState::Released => rt.dispatch(Event::ShortcutReleased),
            }
        })
        .map_err(|e| e.to_string())
}

fn unregister_dictation(app: &AppHandle, accelerator: &str) {
    if let Err(e) = app.global_shortcut().unregister(accelerator) {
        log::warn!("unregistering {accelerator}: {e}");
    }
}

fn conflict_message(accelerator: &str, err: &str) -> String {
    format!(
        "Couldn't use {accelerator} as the shortcut ({err}). Another app may be using it; \
         choose a different shortcut in Preferences."
    )
}

/// Make the OS registration match what we want: the configured shortcut,
/// or nothing while paused or while the UI is recording a new shortcut.
/// Pausing really releases the key combination for other apps.
fn sync_shortcut(app: &AppHandle, paused: bool) {
    let shell = app.state::<Shell>();
    let want = if paused || shell.capturing_shortcut.load(Ordering::SeqCst) {
        None
    } else {
        Some(shell.settings.lock().unwrap().shortcut.clone())
    };
    let mut reg = shell.shortcut.lock().unwrap();
    if reg.applied.as_ref() == Some(&want) {
        return;
    }
    reg.applied = Some(want.clone());
    if reg.registered.as_ref() == want.as_ref() {
        return;
    }
    if let Some(old) = reg.registered.take() {
        unregister_dictation(app, &old);
    }
    if let Some(new) = want {
        match register_dictation(app, &new) {
            Ok(()) => {
                log::info!("shortcut registered: {new}");
                reg.registered = Some(new);
                *shell.shortcut_error.lock().unwrap() = None;
            }
            Err(e) => {
                let msg = conflict_message(&new, &e);
                log::error!("{msg}");
                *shell.shortcut_error.lock().unwrap() = Some(msg);
            }
        }
    } else {
        log::info!("shortcut released");
    }
}

fn current_paused(app: &AppHandle) -> bool {
    app.try_state::<Runtime>()
        .is_some_and(|rt| rt.snapshot().paused)
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_state(app: AppHandle, rt: tauri::State<Runtime>) -> UiState {
    ui_state(&app, rt.snapshot())
}

#[tauri::command]
fn toggle_dictation(rt: tauri::State<Runtime>) {
    rt.dispatch(Event::ToggleRequested);
}

#[tauri::command]
fn set_mode(rt: tauri::State<Runtime>, mode: RecordMode) {
    rt.dispatch(Event::SetMode(mode));
}

#[tauri::command]
fn set_paused(rt: tauri::State<Runtime>, paused: bool) {
    rt.dispatch(Event::SetPaused(paused));
}

#[tauri::command]
fn dismiss(rt: tauri::State<Runtime>) {
    rt.dispatch(Event::Dismiss);
}

/// Recovery: copy the held transcript at the user's request, then clear it.
#[tauri::command]
fn copy_recovery(rt: tauri::State<Runtime>) -> bool {
    let Some(text) = rt.snapshot().recovery_text else {
        return false;
    };
    let ok = platform::copy_to_clipboard(&text);
    if ok {
        rt.dispatch(Event::Dismiss);
    }
    ok
}

/// Recovery: hide LoqWat, give the user a few seconds to click into the
/// destination, then insert there (still refusing password fields).
#[tauri::command]
fn insert_recovery(app: AppHandle, rt: tauri::State<Runtime>) {
    let Some(text) = rt.snapshot().recovery_text else {
        return;
    };
    let rt = rt.inner().clone();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    std::thread::spawn(move || {
        std::thread::sleep(RECOVERY_INSERT_DELAY);
        let failure = if !platform::wait_for_modifiers_released(Duration::from_secs(2)) {
            Some("Modifier keys were held down, so LoqWat didn't paste.".to_string())
        } else {
            let check = platform::check_target(None);
            if check.allows_insert() {
                match platform::insert_text(&text) {
                    InsertOutcome::Confirmed | InsertOutcome::Unverified => None,
                    InsertOutcome::Failed(e) => Some(format!("Paste failed: {e}.")),
                }
            } else {
                Some(check.reason())
            }
        };
        match failure {
            None => rt.dispatch(Event::Dismiss),
            Some(reason) => {
                rt.dispatch(Event::RecoveryInsertFailed(reason));
                show_main_window(&app);
            }
        }
    });
}

#[tauri::command]
fn grant_accessibility() -> bool {
    let trusted = platform::accessibility_trusted(true);
    if !trusted {
        platform::open_accessibility_settings();
    }
    trusted
}

/// While the UI records a new shortcut, release the current one so pressing
/// it doesn't start dictation.
#[tauri::command]
fn capture_shortcut(app: AppHandle, active: bool) {
    app.state::<Shell>()
        .capturing_shortcut
        .store(active, Ordering::SeqCst);
    sync_shortcut(&app, current_paused(&app));
    publish_state(&app);
}

/// Validate, test-register and save a new dictation shortcut. On failure the
/// previous shortcut stays in place.
#[tauri::command]
fn set_shortcut(app: AppHandle, shortcut: String) -> Result<String, String> {
    let shell = app.state::<Shell>();
    shell.capturing_shortcut.store(false, Ordering::SeqCst);
    if let Err(e) = shortcut::validate(&shortcut) {
        sync_shortcut(&app, current_paused(&app));
        return Err(e);
    }
    let old = shell.settings.lock().unwrap().shortcut.clone();
    {
        let mut reg = shell.shortcut.lock().unwrap();
        if let Some(r) = reg.registered.take() {
            unregister_dictation(&app, &r);
        }
        reg.applied = None;
        if let Err(e) = register_dictation(&app, &shortcut) {
            drop(reg);
            sync_shortcut(&app, current_paused(&app)); // restore the old one
            publish_state(&app);
            return Err(conflict_message(&shortcut, &e));
        }
        // Registered as a test; sync below keeps it or releases it if paused.
        reg.registered = Some(shortcut.clone());
    }
    shell.update_settings(|s| s.shortcut = shortcut.clone());
    *shell.shortcut_error.lock().unwrap() = None;
    sync_shortcut(&app, current_paused(&app));
    log::info!("shortcut changed from {old} to {shortcut}");
    publish_state(&app);
    Ok(shortcut)
}

#[derive(Serialize)]
struct Microphone {
    name: String,
    is_default: bool,
}

#[tauri::command]
fn list_microphones() -> Result<Vec<Microphone>, String> {
    capture::list_input_devices()
        .map(|v| {
            v.into_iter()
                .map(|d| Microphone {
                    name: d.name,
                    is_default: d.is_default,
                })
                .collect()
        })
        .map_err(|e| format!("{e:#}"))
}

/// `None` follows the system default input.
#[tauri::command]
fn set_microphone(app: AppHandle, rt: tauri::State<Runtime>, name: Option<String>) {
    let name = name.filter(|n| !n.is_empty());
    rt.update_config(|c| c.input_device = name.clone());
    app.state::<Shell>()
        .update_settings(|s| s.input_device = name);
    publish_state(&app);
}

#[derive(Serialize)]
struct MicTest {
    device: String,
    peak: f32,
    heard: bool,
}

/// Record a few seconds (memory only, then discarded) and report the level.
#[tauri::command]
async fn test_microphone(app: AppHandle) -> Result<MicTest, String> {
    let rt = app.state::<Runtime>().inner().clone();
    if rt.snapshot().phase != Phase::Idle {
        return Err("Finish the current dictation first.".into());
    }
    let shell = app.state::<Shell>();
    if shell.mic_test_running.swap(true, Ordering::SeqCst) {
        return Err("A microphone test is already running.".into());
    }
    let device = rt.config().input_device;
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<MicTest, String> {
        let rec = capture::Recording::start(device.as_deref(), 10).map_err(|e| format!("{e:#}"))?;
        let mut peak = 0f32;
        let until = Instant::now() + Duration::from_secs(MIC_TEST_SECS);
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
            let level = rec.level();
            peak = peak.max(level);
            let _ = app2.emit("mic-level", level);
            if let Some(e) = rec.stream_error() {
                return Err(format!("Microphone error: {e}"));
            }
        }
        let device = rec.device_name.clone();
        drop(rec.finish()); // audio discarded
        let _ = app2.emit("mic-level", 0.0f32);
        Ok(MicTest {
            device,
            peak,
            heard: peak >= MIC_TEST_HEARD_PEAK,
        })
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    shell.mic_test_running.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
fn set_vocabulary(app: AppHandle, rt: tauri::State<Runtime>, terms: Vec<String>) -> Vec<String> {
    let terms = settings::normalize_vocabulary(&terms);
    rt.update_config(|c| c.vocabulary = terms.clone());
    app.state::<Shell>()
        .update_settings(|s| s.vocabulary = terms.clone());
    publish_state(&app);
    terms
}

#[tauri::command]
fn reset_vocabulary(app: AppHandle, rt: tauri::State<Runtime>) -> Vec<String> {
    let defaults: Vec<String> = settings::DEFAULT_VOCABULARY
        .iter()
        .map(|s| s.to_string())
        .collect();
    set_vocabulary(app, rt, defaults)
}

#[tauri::command]
fn set_sound_cues(app: AppHandle, enabled: bool) {
    app.state::<Shell>()
        .update_settings(|s| s.sound_cues = enabled);
    if enabled {
        platform::play_cue(Cue::Start);
    }
    publish_state(&app);
}

#[tauri::command]
fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let al = app.autolaunch();
    let r = if enabled { al.enable() } else { al.disable() };
    r.map_err(|e| format!("Couldn't change launch at login: {e}"))?;
    let actual = al.is_enabled().unwrap_or(enabled);
    app.state::<Shell>()
        .update_settings(|s| s.launch_at_login = actual);
    publish_state(&app);
    Ok(actual)
}

#[tauri::command]
fn finish_setup(app: AppHandle) {
    app.state::<Shell>()
        .update_settings(|s| s.setup_complete = true);
    publish_state(&app);
}

#[tauri::command]
fn quit(app: AppHandle) {
    quit_app(&app);
}

// ---------------------------------------------------------------- models

#[derive(Serialize)]
struct ModelRow {
    id: &'static str,
    label: &'static str,
    size: u64,
    installed: bool,
    active: bool,
    recommended: bool,
}

#[derive(Serialize)]
struct ModelsView {
    dir: String,
    used_bytes: u64,
    models: Vec<ModelRow>,
    vad: ModelRow,
    download: Option<DownloadState>,
    source: String,
}

#[tauri::command]
fn list_models(app: AppHandle, rt: tauri::State<Runtime>) -> Result<ModelsView, String> {
    let dir = models::models_dir().map_err(|e| format!("{e:#}"))?;
    let active_path = rt.config().model_path;
    let row = |m: &'static models::ModelInfo| {
        let p = m.path_in(&dir);
        ModelRow {
            id: m.id,
            label: m.label,
            size: m.size,
            installed: p.is_file(),
            active: active_path.as_ref() == Some(&p),
            recommended: m.id == settings::DEFAULT_MODEL,
        }
    };
    let models: Vec<ModelRow> = models::CATALOG.iter().map(row).collect();
    let vad = row(&models::VAD_MODEL);
    let used_bytes = models
        .iter()
        .chain(std::iter::once(&vad))
        .filter(|r| r.installed)
        .filter_map(|r| models::find(r.id))
        .filter_map(|m| std::fs::metadata(m.path_in(&dir)).ok())
        .map(|md| md.len())
        .sum();
    Ok(ModelsView {
        dir: dir.display().to_string(),
        used_bytes,
        models,
        vad,
        download: app.state::<Shell>().download.lock().unwrap().clone(),
        source: format!(
            "huggingface.co/{} @ {}",
            models::MODEL_REPO,
            &models::MODEL_REVISION[..10]
        ),
    })
}

#[derive(Clone, Serialize)]
struct DownloadEvent {
    id: String,
    done: u64,
    total: u64,
    /// None while running; "ok" | "failed" | "cancelled" at the end.
    finished: Option<&'static str>,
    error: Option<String>,
}

/// Start a verified download (plus the speech detector if it's missing).
#[tauri::command]
fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    let info = models::find(&id).ok_or("Unknown model.")?;
    let shell = app.state::<Shell>();
    let mut slot = shell.download.lock().unwrap();
    if slot.is_some() {
        return Err("Another download is in progress.".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *slot = Some(DownloadState {
        id: id.clone(),
        done: 0,
        total: info.size,
        cancel: cancel.clone(),
    });
    drop(slot);
    let app2 = app.clone();
    std::thread::Builder::new()
        .name("loqwat-download".into())
        .spawn(move || download_worker(app2, info, cancel))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn download_worker(app: AppHandle, info: &'static models::ModelInfo, cancel: Arc<AtomicBool>) {
    let shell = app.state::<Shell>();
    let emit = |done: u64, finished: Option<&'static str>, error: Option<String>| {
        let _ = app.emit(
            "download",
            DownloadEvent {
                id: info.id.to_string(),
                done,
                total: info.size,
                finished,
                error,
            },
        );
    };
    let mut vad_new = false;
    let started = Instant::now();
    let result = (|| -> anyhow::Result<()> {
        let dir = models::models_dir()?;
        let vad = &models::VAD_MODEL;
        if info.id != vad.id && !vad.path_in(&dir).is_file() {
            models::download(vad, &dir, &cancel, |_, _| {})?;
            vad_new = true;
        }
        let mut last = Instant::now();
        models::download(info, &dir, &cancel, |done, total| {
            if let Some(d) = shell.download.lock().unwrap().as_mut() {
                d.done = done;
            }
            if last.elapsed() >= Duration::from_millis(200) || done == total {
                last = Instant::now();
                emit(done, None, None);
            }
        })?;
        if info.id == vad.id {
            vad_new = true;
        }
        Ok(())
    })();
    *shell.download.lock().unwrap() = None;
    let ok = match result {
        Ok(()) => {
            log::info!(
                "downloaded and verified {} in {:.1} s",
                info.id,
                started.elapsed().as_secs_f64()
            );
            emit(info.size, Some("ok"), None);
            true
        }
        Err(_) if cancel.load(Ordering::Relaxed) => {
            log::info!("download of {} cancelled", info.id);
            emit(0, Some("cancelled"), None);
            false
        }
        Err(e) => {
            log::error!("download of {} failed: {e:#}", info.id);
            emit(0, Some("failed"), Some(format!("{e:#}")));
            false
        }
    };
    let rt = app.state::<Runtime>();
    let mut reload = false;
    if vad_new {
        rt.update_config(|c| {
            c.vad_path = runtime::installed_model_path(models::VAD_MODEL.id);
        });
        reload = rt.config().model_path.is_some();
    }
    // First speech model: start using it right away.
    if ok && info.id != models::VAD_MODEL.id && rt.config().model_path.is_none() {
        let path = runtime::installed_model_path(info.id);
        rt.update_config(|c| c.model_path = path);
        shell.update_settings(|s| s.model_id = Some(info.id.to_string()));
        reload = true;
    }
    if reload {
        rt.reload_engine();
    }
    publish_state(&app);
}

#[tauri::command]
fn cancel_download(app: AppHandle) {
    if let Some(d) = app.state::<Shell>().download.lock().unwrap().as_ref() {
        d.cancel.store(true, Ordering::Relaxed);
    }
}

/// Switch the engine to another installed model.
#[tauri::command]
fn use_model(app: AppHandle, rt: tauri::State<Runtime>, id: String) -> Result<(), String> {
    let path = runtime::installed_model_path(&id).ok_or("That model isn't installed.")?;
    if !matches!(rt.snapshot().phase, Phase::Idle | Phase::Error) {
        return Err("Finish the current dictation first.".into());
    }
    rt.update_config(|c| c.model_path = Some(path));
    app.state::<Shell>()
        .update_settings(|s| s.model_id = Some(id.clone()));
    log::info!("switching model to {id}");
    rt.reload_engine();
    publish_state(&app);
    Ok(())
}

#[tauri::command]
fn remove_model(app: AppHandle, rt: tauri::State<Runtime>, id: String) -> Result<(), String> {
    let info = models::find(&id).ok_or("Unknown model.")?;
    let dir = models::models_dir().map_err(|e| format!("{e:#}"))?;
    if rt.config().model_path.as_ref() == Some(&info.path_in(&dir)) {
        return Err("This model is in use. Switch to another model before removing it.".into());
    }
    if id == models::VAD_MODEL.id {
        return Err("The speech detector is needed to avoid inventing text in silence.".into());
    }
    if app
        .state::<Shell>()
        .download
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|d| d.id == id)
    {
        return Err("Cancel the download first.".into());
    }
    models::remove(info, &dir).map_err(|e| format!("{e:#}"))?;
    log::info!("removed model {id}");
    publish_state(&app);
    Ok(())
}

/// Import a model file the user picks. Only exact catalog matches are accepted.
#[tauri::command]
async fn import_model(app: AppHandle) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Import a LoqWat speech model")
        .add_filter("Whisper model (ggml .bin)", &["bin"])
        .blocking_pick_file();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let source = picked.into_path().map_err(|e| e.to_string())?;
    let dir = models::models_dir().map_err(|e| format!("{e:#}"))?;
    let info = tauri::async_runtime::spawn_blocking(move || models::import(&source, &dir))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Not imported: {e:#}."))?;
    log::info!("imported model {}", info.id);
    let rt = app.state::<Runtime>();
    if info.id == models::VAD_MODEL.id {
        rt.update_config(|c| c.vad_path = runtime::installed_model_path(info.id));
        if rt.config().model_path.is_some() {
            rt.reload_engine();
        }
    } else if rt.config().model_path.is_none() {
        rt.update_config(|c| c.model_path = runtime::installed_model_path(info.id));
        app.state::<Shell>()
            .update_settings(|s| s.model_id = Some(info.id.to_string()));
        rt.reload_engine();
    }
    publish_state(&app);
    Ok(Some(info.label.to_string()))
}

#[tauri::command]
fn reveal_models_folder() -> Result<(), String> {
    let dir = models::models_dir().map_err(|e| format!("{e:#}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if platform::reveal_folder(&dir) {
        Ok(())
    } else {
        Err("Couldn't open the folder.".into())
    }
}

// ---------------------------------------------------------------- startup

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("loqwat_lib=info"))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_state,
            toggle_dictation,
            set_mode,
            set_paused,
            dismiss,
            copy_recovery,
            insert_recovery,
            grant_accessibility,
            capture_shortcut,
            set_shortcut,
            list_microphones,
            set_microphone,
            test_microphone,
            set_vocabulary,
            reset_vocabulary,
            set_sound_cues,
            set_launch_at_login,
            finish_setup,
            quit,
            list_models,
            download_model,
            cancel_download,
            use_model,
            remove_model,
            import_model,
            reveal_models_folder
        ])
        .setup(|app| {
            // Menu bar utility: no Dock icon, and never becomes the active app
            // just because its indicator appears.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if let Ok(dir) = models::models_dir() {
                match models::clean_partial_downloads(&dir) {
                    Ok(n) if n > 0 => log::info!("removed {n} interrupted download(s)"),
                    Ok(_) => {}
                    Err(e) => log::warn!("cleaning interrupted downloads: {e:#}"),
                }
            }
            let settings_path = settings::settings_path().ok();
            let settings = settings_path
                .as_deref()
                .map(Settings::load_from)
                .unwrap_or_default();
            let config = runtime::config_from(&settings);
            let show_setup = !settings.setup_complete || config.model_path.is_none();
            app.manage(Shell {
                settings_path,
                settings: Mutex::new(settings),
                shortcut: Mutex::new(ShortcutReg::default()),
                shortcut_error: Mutex::new(None),
                capturing_shortcut: AtomicBool::new(false),
                target: Mutex::new(None),
                last_text: Mutex::new(None),
                download: Mutex::new(None),
                mic_test_running: AtomicBool::new(false),
                last_phase: Mutex::new(Phase::Idle),
            });
            let handle = app.handle().clone();
            build_indicator(&handle)?;
            build_tray(&handle)?;
            let rt = Runtime::start(
                config,
                AppHooks {
                    app: handle.clone(),
                },
            );
            app.manage(rt.clone());
            sync_shortcut(&handle, false);
            update_tray(&handle, &rt.snapshot());
            if show_setup {
                show_main_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window hides it; the app keeps running in the menu bar.
            if window.label() == "main"
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building LoqWat")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(rt) = app.try_state::<Runtime>()
            {
                rt.shutdown();
            }
        });
}
