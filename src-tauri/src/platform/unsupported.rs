//! Placeholder until the Windows integration (Milestone 7). Everything falls
//! back to the recovery window, so text is never lost.

use std::time::Duration;

use super::{InsertOutcome, Target, TargetCheck};

pub struct TargetInner;

pub fn accessibility_trusted(_prompt: bool) -> bool {
    false
}

pub fn open_accessibility_settings() {}

pub fn capture_target() -> Option<Target> {
    None
}

pub fn check_target(_target: Option<&Target>) -> TargetCheck {
    TargetCheck::NoPermission
}

pub fn wait_for_modifiers_released(_timeout: Duration) -> bool {
    true
}

pub fn insert_text(_text: &str) -> InsertOutcome {
    InsertOutcome::Failed("text insertion is not implemented on this platform yet".into())
}

pub fn copy_to_clipboard(_text: &str) -> bool {
    false
}
