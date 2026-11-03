//! Validation for user-chosen dictation shortcuts.

use std::str::FromStr;

use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

/// Check a shortcut before trying to register it. Rejects shortcuts that
/// would interfere with normal typing or with Escape-to-cancel.
pub fn validate(accelerator: &str) -> Result<Shortcut, String> {
    let sc = Shortcut::from_str(accelerator)
        .map_err(|e| format!("“{accelerator}” isn't a valid shortcut ({e})."))?;
    if sc.key == Code::Escape {
        return Err("Escape is reserved for cancelling dictation.".into());
    }
    let is_function_key = matches!(
        sc.key,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
    );
    let typing_mods = sc.mods - Modifiers::SHIFT;
    if !is_function_key && typing_mods.is_empty() {
        return Err(
            "Use at least one of ⌥ Option, ⌃ Control or ⌘ Command (or a function key), \
             so the shortcut doesn't get in the way of typing."
                .into(),
        );
    }
    if matches!(
        sc.key,
        Code::ShiftLeft
            | Code::ShiftRight
            | Code::AltLeft
            | Code::AltRight
            | Code::ControlLeft
            | Code::ControlRight
            | Code::MetaLeft
            | Code::MetaRight
    ) {
        return Err("Add a regular key to the modifier keys.".into());
    }
    let reserved = [
        (Modifiers::SUPER, Code::KeyV),
        (Modifiers::SUPER, Code::KeyC),
        (Modifiers::SUPER, Code::KeyX),
        (Modifiers::SUPER, Code::KeyZ),
        (Modifiers::SUPER, Code::KeyQ),
        (Modifiers::SUPER, Code::KeyW),
        (Modifiers::SUPER, Code::Tab),
        (Modifiers::SUPER, Code::Space),
    ];
    if reserved.contains(&(sc.mods, sc.key)) {
        return Err("That shortcut is used by the system or by LoqWat's paste.".into());
    }
    Ok(sc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_reasonable_shortcuts() {
        for s in ["Alt+Space", "Control+Shift+KeyD", "Super+Alt+Digit1", "F5", "Shift+F13"] {
            assert!(validate(s).is_ok(), "{s}");
        }
    }

    #[test]
    fn rejects_problem_shortcuts() {
        for s in [
            "Space",
            "KeyA",
            "Shift+KeyA",
            "Alt+Escape",
            "Escape",
            "Super+KeyV",
            "Super+Space",
            "not a key",
            "",
        ] {
            assert!(validate(s).is_err(), "{s}");
        }
    }
}
