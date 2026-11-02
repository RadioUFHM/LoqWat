//! LoqWat application entry point: menu bar app, global shortcut, indicator.

pub mod audio;
pub mod capture;
pub mod controller;
pub mod engine;
pub mod metrics;
pub mod models;
pub mod platform;
pub mod runtime;
pub mod vad;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use controller::{EngineStatus, Event, InsertResult, Phase, RecordMode, SessionId, Snapshot};
use platform::{InsertOutcome, Target};
use runtime::{Hooks, Runtime};

/// Default dictation shortcut (configurable in preferences later).
const DICTATE_SHORTCUT: &str = "Alt+Space";
const CANCEL_SHORTCUT: &str = "Escape";
const INDICATOR_SIZE: (f64, f64) = (220.0, 44.0);
/// Countdown before an explicit "insert into another field" from recovery.
const RECOVERY_INSERT_DELAY: Duration = Duration::from_secs(3);

/// Everything the UI shows.
#[derive(Debug, Clone, Serialize)]
struct UiState {
    #[serde(flatten)]
    snapshot: Snapshot,
    shortcut: String,
    shortcut_error: Option<String>,
    model: Option<String>,
    version: &'static str,
    accessibility: bool,
    has_last_text: bool,
}

#[derive(Default)]
struct Shell {
    shortcut_error: Mutex<Option<String>>,
    model: Option<String>,
    /// Where the user was typing when the current session started.
    target: Mutex<Option<(SessionId, Target)>>,
    /// Most recent transcript, memory only, for "Copy Last Transcript".
    last_text: Mutex<Option<String>>,
}

struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    mode_toggle: CheckMenuItem<tauri::Wry>,
    mode_hold: CheckMenuItem<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
    copy_last: MenuItem<tauri::Wry>,
}

fn ui_state(app: &AppHandle, snapshot: Snapshot) -> UiState {
    let shell = app.state::<Shell>();
    UiState {
        snapshot,
        shortcut: DICTATE_SHORTCUT.into(),
        shortcut_error: shell.shortcut_error.lock().unwrap().clone(),
        model: shell.model.clone(),
        version: env!("CARGO_PKG_VERSION"),
        accessibility: platform::accessibility_trusted(false),
        has_last_text: shell.last_text.lock().unwrap().is_some(),
    }
}

struct AppHooks {
    app: AppHandle,
}

impl Hooks for AppHooks {
    fn publish(&self, snapshot: &Snapshot) {
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
                if ev.state == ShortcutState::Pressed {
                    app.state::<Runtime>().dispatch(Event::CancelPressed);
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
            let rt = app.state::<Runtime>();
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
                "quit" => {
                    rt.shutdown();
                    app.exit(0);
                }
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

fn register_dictation_shortcut(app: &AppHandle) {
    let result = app
        .global_shortcut()
        .on_shortcut(DICTATE_SHORTCUT, |app, _, ev| {
            let rt = app.state::<Runtime>();
            match ev.state {
                ShortcutState::Pressed => rt.dispatch(Event::ShortcutPressed),
                ShortcutState::Released => rt.dispatch(Event::ShortcutReleased),
            }
        });
    if let Err(e) = result {
        let msg =
            format!("Could not register {DICTATE_SHORTCUT}: {e}. Another app may be using it.");
        log::error!("{msg}");
        *app.state::<Shell>().shortcut_error.lock().unwrap() = Some(msg);
    }
}

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("loqwat_lib=info"))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_state,
            toggle_dictation,
            set_mode,
            set_paused,
            dismiss,
            copy_recovery,
            insert_recovery,
            grant_accessibility
        ])
        .setup(|app| {
            // Menu bar utility: no Dock icon, and never becomes the active app
            // just because its indicator appears.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config = runtime::default_config();
            app.manage(Shell {
                target: Mutex::new(None),
                last_text: Mutex::new(None),
                shortcut_error: Mutex::new(None),
                model: config
                    .model_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned()),
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
            register_dictation_shortcut(&handle);
            update_tray(&handle, &rt.snapshot());
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
