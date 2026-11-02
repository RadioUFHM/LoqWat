//! LoqWat application entry point.

/// Basic engine/build info for the status UI.
#[tauri::command]
fn app_info() -> serde_json::Value {
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "whisper": whisper_rs::get_whisper_version(),
        "arch": std::env::consts::ARCH,
        "os": std::env::consts::OS,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running LoqWat");
}
