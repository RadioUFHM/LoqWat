//! OS integration behind a narrow interface: accessibility permission, the
//! dictation target (app/window/field), clipboard, and text insertion.
//! Everything else in LoqWat is platform-independent.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(target_os = "macos"))]
mod unsupported;
#[cfg(not(target_os = "macos"))]
use unsupported as imp;

use std::time::Duration;

/// Where the user was typing when recording started.
pub struct Target {
    pub pid: i32,
    pub app_name: String,
    pub bundle_id: Option<String>,
    /// Field role/subrole were readable (some apps expose no accessibility tree).
    pub field_known: bool,
    pub(crate) inner: imp::TargetInner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetCheck {
    /// Same app, window and field.
    Same,
    /// Same app and window, but the app doesn't expose its focused field, so
    /// the field can't be confirmed.
    SameAppFieldUnknown,
    NoPermission,
    NoFocusedApp,
    AppChanged {
        now: String,
    },
    WindowChanged,
    FieldChanged,
    SecureField,
    OwnApp,
}

impl TargetCheck {
    pub fn allows_insert(&self) -> bool {
        matches!(self, TargetCheck::Same | TargetCheck::SameAppFieldUnknown)
    }

    /// User-facing explanation when insertion is refused.
    pub fn reason(&self) -> String {
        match self {
            TargetCheck::Same | TargetCheck::SameAppFieldUnknown => String::new(),
            TargetCheck::NoPermission => "LoqWat needs Accessibility permission to insert text. Open System Settings → Privacy & Security → Accessibility and turn on LoqWat.".into(),
            TargetCheck::NoFocusedApp => "No app was ready for typing.".into(),
            TargetCheck::AppChanged { now } => format!("You switched to {now} while LoqWat was transcribing, so the text was not inserted."),
            TargetCheck::WindowChanged => "You switched windows while LoqWat was transcribing, so the text was not inserted.".into(),
            TargetCheck::FieldChanged => "You moved to a different field while LoqWat was transcribing, so the text was not inserted.".into(),
            TargetCheck::SecureField => "The destination is a password field. LoqWat never types into password fields.".into(),
            TargetCheck::OwnApp => "No text field was selected outside LoqWat.".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertOutcome {
    /// The field's character count changed after pasting.
    Confirmed,
    /// Paste was sent but the app doesn't report its text length, so the
    /// insertion can't be confirmed.
    Unverified,
    /// Nothing was pasted.
    Failed(String),
}

/// Is LoqWat allowed to use Accessibility? `prompt` shows the macOS dialog.
pub fn accessibility_trusted(prompt: bool) -> bool {
    imp::accessibility_trusted(prompt)
}

/// Open the OS settings page where the user grants Accessibility access.
pub fn open_accessibility_settings() {
    imp::open_accessibility_settings()
}

/// Snapshot the currently focused app/window/field.
pub fn capture_target() -> Option<Target> {
    imp::capture_target()
}

/// Compare the current focus with a captured target. With `None`, only
/// checks that the current focus is a safe, non-LoqWat destination.
pub fn check_target(target: Option<&Target>) -> TargetCheck {
    imp::check_target(target)
}

/// Wait until no modifier keys are held (so ⌥ from the shortcut can't turn
/// ⌘V into something else). Returns false on timeout.
pub fn wait_for_modifiers_released(timeout: Duration) -> bool {
    imp::wait_for_modifiers_released(timeout)
}

/// Paste `text` into the focused field, then restore the user's clipboard if
/// nothing else has changed it. Never sends Return. Never retries.
pub fn insert_text(text: &str) -> InsertOutcome {
    imp::insert_text(text)
}

/// Put text on the clipboard at the user's explicit request.
pub fn copy_to_clipboard(text: &str) -> bool {
    imp::copy_to_clipboard(text)
}

/// Optional audible cues for recording start/stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Start,
    Stop,
    Cancel,
}

/// Play a short sound cue (best effort).
pub fn play_cue(cue: Cue) -> bool {
    imp::play_cue(cue)
}

/// Show a folder in the system file manager. Arguments go straight to the
/// process (no shell), so paths are never interpreted.
pub fn reveal_folder(path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(path).spawn().is_ok()
}
