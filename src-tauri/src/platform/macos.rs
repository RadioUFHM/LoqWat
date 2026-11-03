//! macOS: Accessibility (AXUIElement), NSPasteboard, CGEvent.
//!
//! Insertion strategy: save every pasteboard item/type, put the transcript on
//! the pasteboard (marked transient so clipboard managers that honour
//! `org.nspasteboard.TransientType` skip it), post ⌘V, confirm via the field's
//! `AXNumberOfCharacters` when available, then restore the saved contents —
//! but only if the pasteboard still holds our text (the user didn't copy
//! something new meanwhile). Only a character count is read; never the text.

use std::ptr::NonNull;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeString, NSPasteboardWriting, NSWorkspace,
};
use objc2_application_services::{AXError, AXIsProcessTrustedWithOptions, AXUIElement};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFEqual, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
};
use objc2_foundation::{NSArray, NSData, NSString};

use super::{InsertOutcome, Target, TargetCheck};

const AX_TIMEOUT_SECS: f32 = 0.5;
const KEY_V: u16 = 9;
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
/// How long to watch the field's character count for the paste to land.
const CONFIRM_WINDOW: Duration = Duration::from_millis(1500);
/// When the app can't report its length, how long to let it read the
/// pasteboard before restoring. Empirical, not a guarantee (see PROCESS.md).
const UNVERIFIED_SETTLE: Duration = Duration::from_millis(400);

/// AX elements are CF objects usable from any thread.
pub struct TargetInner {
    window: Option<CFRetained<AXUIElement>>,
    field: Option<CFRetained<AXUIElement>>,
}
unsafe impl Send for TargetInner {}
unsafe impl Sync for TargetInner {}

pub fn accessibility_trusted(prompt: bool) -> bool {
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::new(prompt);
    let dict = CFDictionary::from_slices(&[&*key], &[value]);
    unsafe { AXIsProcessTrustedWithOptions(Some(dict.as_opaque())) }
}

pub fn open_accessibility_settings() {
    // Fixed argument, no shell.
    let _ = std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

fn attr(el: &AXUIElement, name: &'static str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_static_str(name);
    let mut out: *const CFType = std::ptr::null();
    let err = unsafe { el.copy_attribute_value(&key, NonNull::from(&mut out)) };
    if err != AXError::Success {
        return None;
    }
    NonNull::new(out as *mut CFType).map(|p| unsafe { CFRetained::from_raw(p) })
}

fn attr_element(el: &AXUIElement, name: &'static str) -> Option<CFRetained<AXUIElement>> {
    attr(el, name)?.downcast::<AXUIElement>().ok()
}

fn attr_string(el: &AXUIElement, name: &'static str) -> Option<String> {
    attr(el, name)?
        .downcast::<CFString>()
        .ok()
        .map(|s| s.to_string())
}

fn attr_i64(el: &AXUIElement, name: &'static str) -> Option<i64> {
    attr(el, name)?.downcast::<CFNumber>().ok()?.as_i64()
}

fn same(a: &AXUIElement, b: &AXUIElement) -> bool {
    CFEqual(Some(a), Some(b))
}

struct Focus {
    pid: i32,
    window: Option<CFRetained<AXUIElement>>,
    field: Option<CFRetained<AXUIElement>>,
}

fn current_focus() -> Option<Focus> {
    let app_info = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = app_info.processIdentifier();
    let app = unsafe { AXUIElement::new_application(pid) };
    unsafe {
        app.set_messaging_timeout(AX_TIMEOUT_SECS);
    }
    let window = attr_element(&app, "AXFocusedWindow");
    let field = attr_element(&app, "AXFocusedUIElement");
    Some(Focus { pid, window, field })
}

fn is_secure(field: &AXUIElement) -> bool {
    attr_string(field, "AXSubrole").as_deref() == Some("AXSecureTextField")
        || attr_string(field, "AXRole").as_deref() == Some("AXSecureTextField")
}

pub fn capture_target() -> Option<Target> {
    let app_info = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let focus = current_focus()?;
    Some(Target {
        pid: focus.pid,
        app_name: app_info
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "the app".into()),
        bundle_id: app_info.bundleIdentifier().map(|b| b.to_string()),
        field_known: focus.field.is_some(),
        inner: TargetInner {
            window: focus.window,
            field: focus.field,
        },
    })
}

pub fn check_target(target: Option<&Target>) -> TargetCheck {
    if !accessibility_trusted(false) {
        return TargetCheck::NoPermission;
    }
    let Some(now) = current_focus() else {
        return TargetCheck::NoFocusedApp;
    };
    if now.pid == std::process::id() as i32 {
        return TargetCheck::OwnApp;
    }
    if let Some(field) = now.field.as_ref()
        && is_secure(field)
    {
        return TargetCheck::SecureField;
    }
    let Some(t) = target else {
        return if now.field.is_some() {
            TargetCheck::Same
        } else {
            TargetCheck::SameAppFieldUnknown
        };
    };
    if t.pid == std::process::id() as i32 {
        return TargetCheck::OwnApp;
    }
    if now.pid != t.pid {
        let name = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .and_then(|a| a.localizedName())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "another app".into());
        return TargetCheck::AppChanged { now: name };
    }
    match (&t.inner.window, &now.window) {
        (Some(a), Some(b)) if !same(a, b) => return TargetCheck::WindowChanged,
        _ => {}
    }
    match (&t.inner.field, &now.field) {
        (Some(a), Some(b)) if same(a, b) => TargetCheck::Same,
        (Some(_), _) => TargetCheck::FieldChanged,
        (None, _) => TargetCheck::SameAppFieldUnknown,
    }
}

const MODIFIERS: CGEventFlags = CGEventFlags(
    CGEventFlags::MaskCommand.0
        | CGEventFlags::MaskAlternate.0
        | CGEventFlags::MaskControl.0
        | CGEventFlags::MaskShift.0,
);

pub fn wait_for_modifiers_released(timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        let flags = CGEventSource::flags_state(CGEventSourceStateID::HIDSystemState);
        if flags.0 & MODIFIERS.0 == 0 {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Every item and type on the pasteboard, so it can be put back faithfully.
type SavedPasteboard = Vec<Vec<(String, Vec<u8>)>>;

fn save_pasteboard(pb: &NSPasteboard) -> SavedPasteboard {
    let Some(items) = pb.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            item.types()
                .iter()
                .filter_map(|t| item.dataForType(&t).map(|d| (t.to_string(), d.to_vec())))
                .collect()
        })
        .collect()
}

fn write_items(pb: &NSPasteboard, items: Vec<Retained<NSPasteboardItem>>) -> bool {
    let objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = items
        .into_iter()
        .map(ProtocolObject::from_retained)
        .collect();
    pb.writeObjects(&NSArray::from_retained_slice(&objs))
}

fn restore_pasteboard(pb: &NSPasteboard, saved: SavedPasteboard) {
    pb.clearContents();
    if saved.is_empty() {
        return;
    }
    let items = saved
        .into_iter()
        .map(|types| {
            let item = NSPasteboardItem::new();
            for (t, data) in types {
                item.setData_forType(&NSData::from_vec(data), &NSString::from_str(&t));
            }
            item
        })
        .collect();
    write_items(pb, items);
}

fn write_text(pb: &NSPasteboard, text: &str, transient: bool) -> bool {
    pb.clearContents();
    let item = NSPasteboardItem::new();
    item.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    if transient {
        item.setData_forType(&NSData::new(), &NSString::from_str(TRANSIENT_TYPE));
    }
    write_items(pb, vec![item])
}

fn post_paste() -> bool {
    let (Some(down), Some(up)) = (
        CGEvent::new_keyboard_event(None, KEY_V, true),
        CGEvent::new_keyboard_event(None, KEY_V, false),
    ) else {
        return false;
    };
    CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
    CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    true
}

fn focused_char_count() -> Option<i64> {
    let focus = current_focus()?;
    attr_i64(focus.field.as_ref()?, "AXNumberOfCharacters")
}

pub fn insert_text(text: &str) -> InsertOutcome {
    let pb = NSPasteboard::generalPasteboard();
    let saved = save_pasteboard(&pb);
    let before = focused_char_count();

    if !write_text(&pb, text, true) {
        restore_pasteboard(&pb, saved);
        return InsertOutcome::Failed("could not place text on the clipboard".into());
    }
    let ours = pb.changeCount();

    if !post_paste() {
        restore_pasteboard(&pb, saved);
        return InsertOutcome::Failed("could not send the paste keystroke".into());
    }

    let outcome = match before {
        Some(before) => {
            let start = Instant::now();
            let mut confirmed = false;
            while start.elapsed() < CONFIRM_WINDOW {
                std::thread::sleep(Duration::from_millis(25));
                if focused_char_count().is_some_and(|now| now != before) {
                    confirmed = true;
                    break;
                }
            }
            if confirmed {
                // Small margin for apps that finish handling the paste event
                // after updating their text.
                std::thread::sleep(Duration::from_millis(60));
                InsertOutcome::Confirmed
            } else {
                InsertOutcome::Unverified
            }
        }
        None => {
            std::thread::sleep(UNVERIFIED_SETTLE);
            InsertOutcome::Unverified
        }
    };

    // Restore only if the pasteboard still holds our text; if the user copied
    // something meanwhile, leave their new content alone.
    if pb.changeCount() == ours {
        restore_pasteboard(&pb, saved);
    } else {
        log::info!("clipboard changed during insertion; not restoring");
    }
    outcome
}

/// Play a short built-in system sound. Returns false if it couldn't play.
pub fn play_cue(cue: super::Cue) -> bool {
    let name = match cue {
        super::Cue::Start => "Tink",
        super::Cue::Stop => "Pop",
        super::Cue::Cancel => "Bottle",
    };
    objc2_app_kit::NSSound::soundNamed(&NSString::from_str(name)).is_some_and(|s| s.play())
}

pub fn copy_to_clipboard(text: &str) -> bool {
    write_text(&NSPasteboard::generalPasteboard(), text, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses a private, uniquely named pasteboard — never the user's clipboard.
    fn scratch() -> Retained<NSPasteboard> {
        NSPasteboard::pasteboardWithUniqueName()
    }

    fn item(types: &[(&str, &[u8])]) -> Retained<NSPasteboardItem> {
        let item = NSPasteboardItem::new();
        for (t, d) in types {
            item.setData_forType(&NSData::with_bytes(d), &NSString::from_str(t));
        }
        item
    }

    #[test]
    fn restore_is_faithful_for_multiple_items_and_types() {
        let pb = scratch();
        pb.clearContents();
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 0, 1, 2, 3, 255];
        write_items(
            &pb,
            vec![
                item(&[
                    ("public.utf8-plain-text", "héllo — 🎙".as_bytes()),
                    ("public.rtf", b"{\\rtf1 hello}"),
                ]),
                item(&[
                    ("public.png", png),
                    ("com.example.custom", b"\x00\x01binary"),
                ]),
            ],
        );
        let original = save_pasteboard(&pb);
        assert_eq!(original.len(), 2);

        assert!(write_text(&pb, "dictated text", true));
        let during = save_pasteboard(&pb);
        assert_eq!(during.len(), 1);
        assert!(during[0].iter().any(|(t, _)| t == TRANSIENT_TYPE));
        assert!(
            during[0]
                .iter()
                .any(|(t, d)| t == "public.utf8-plain-text" && d == b"dictated text")
        );

        restore_pasteboard(&pb, original.clone());
        let after = save_pasteboard(&pb);
        let sorted = |mut v: SavedPasteboard| {
            for i in &mut v {
                i.sort();
            }
            v
        };
        assert_eq!(sorted(after), sorted(original));
    }

    #[test]
    fn restore_of_empty_clipboard_leaves_it_empty() {
        let pb = scratch();
        pb.clearContents();
        let original = save_pasteboard(&pb);
        write_text(&pb, "x", true);
        restore_pasteboard(&pb, original);
        assert!(save_pasteboard(&pb).is_empty());
    }

    #[test]
    fn change_count_detects_user_copy() {
        let pb = scratch();
        write_text(&pb, "ours", true);
        let ours = pb.changeCount();
        write_text(&pb, "user copied this", false);
        assert_ne!(pb.changeCount(), ours, "a later copy must be detectable");
    }
}
