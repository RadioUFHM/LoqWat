//! LoqWat application entry point: menu bar app, global shortcut, indicator.

pub mod audio;
pub mod capture;
pub mod controller;
pub mod engine;
pub mod metrics;
pub mod models;
pub mod runtime;
pub mod vad;

use std::sync::Mutex;

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use controller::{EngineStatus, Event, Phase, RecordMode, SessionId, Snapshot};
use runtime::{Hooks, Runtime};

/// Default dictation shortcut (configurable in preferences later).
const DICTATE_SHORTCUT: &str = "Alt+Space";
const CANCEL_SHORTCUT: &str = "Escape";
const INDICATOR_SIZE: (f64, f64) = (220.0, 44.0);

/// Everything the UI shows.
#[derive(Debug, Clone, Serialize)]
struct UiState {
    #[serde(flatten)]
    snapshot: Snapshot,
    shortcut: String,
    shortcut_error: Option<String>,
    model: Option<String>,
    version: &'static str,
}

#[derive(Default)]
struct Shell {
    shortcut_error: Mutex<Option<String>>,
    model: Option<String>,
}

struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    mode_toggle: CheckMenuItem<tauri::Wry>,
    mode_hold: CheckMenuItem<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
}

fn ui_state(app: &AppHandle, snapshot: Snapshot) -> UiState {
    let shell = app.state::<Shell>();
    UiState {
        snapshot,
        shortcut: DICTATE_SHORTCUT.into(),
        shortcut_error: shell.shortcut_error.lock().unwrap().clone(),
        model: shell.model.clone(),
        version: env!("CARGO_PKG_VERSION"),
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

    fn insert(&self, runtime: Runtime, session: SessionId, text: String) {
        // Milestone 2: no text insertion yet. Hand the transcript to the
        // recovery path (memory only) instead of claiming it was inserted.
        runtime.dispatch(Event::InsertFinished(session, Err(text)));
    }

    fn level(&self, level: f32) {
        let _ = self.app.emit_to("indicator", "level", level);
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
            dismiss
        ])
        .setup(|app| {
            // Menu bar utility: no Dock icon, and never becomes the active app
            // just because its indicator appears.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config = runtime::default_config();
            app.manage(Shell {
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
