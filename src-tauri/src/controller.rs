//! Dictation session state machine.
//!
//! Pure logic: consumes [`Event`]s, updates state, and returns [`Action`]s for
//! the runtime to perform. Holds no OS resources, so every transition is unit
//! tested. Each recording gets a session ID; results from any session other
//! than the active one are ignored, so a cancelled or superseded session can
//! never insert text.

use serde::{Deserialize, Serialize};

pub type SessionId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordMode {
    /// Press once to start, press again to stop.
    Toggle,
    /// Record while the shortcut is held down.
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Recording,
    Transcribing,
    Inserting,
    Recovery,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum EngineStatus {
    NotLoaded,
    Loading,
    Ready,
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Text(String),
    NoSpeech,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    ShortcutPressed,
    ShortcutReleased,
    /// Start/stop from a UI button or menu (toggle semantics in any mode).
    ToggleRequested,
    CancelPressed,
    /// Capture hit the duration cap; stop and process what was captured.
    CaptureLimitReached(SessionId),
    CaptureFailed(SessionId, String),
    TranscriptionFinished(SessionId, Outcome),
    /// `Err(text)` = insertion could not be confirmed; keep text for recovery.
    InsertFinished(SessionId, Result<(), String>),
    EngineStatusChanged(EngineStatus),
    SetMode(RecordMode),
    SetPaused(bool),
    /// User acknowledged an error or dismissed the recovery text.
    Dismiss,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    StartCapture(SessionId),
    /// Stop capture and send the audio for transcription.
    StopCapture(SessionId),
    /// Stop capture and throw the audio away.
    DiscardCapture(SessionId),
    CancelTranscription(SessionId),
    Insert(SessionId, String),
    /// Register/unregister the Escape shortcut (only while a session is active).
    SetCancelShortcut(bool),
    /// State changed; refresh indicator/tray/UI.
    Publish,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub phase: Phase,
    pub session: SessionId,
    pub mode: RecordMode,
    pub paused: bool,
    pub engine: EngineStatus,
    pub message: Option<String>,
    pub limit_reached: bool,
    /// Transcript held for recovery (memory only).
    pub recovery_text: Option<String>,
}

pub struct Controller {
    phase: Phase,
    session: SessionId,
    mode: RecordMode,
    paused: bool,
    key_down: bool,
    engine: EngineStatus,
    message: Option<String>,
    limit_reached: bool,
    recovery_text: Option<String>,
    cancel_registered: bool,
}

impl Controller {
    pub fn new(mode: RecordMode) -> Self {
        Self {
            phase: Phase::Idle,
            session: 0,
            mode,
            paused: false,
            key_down: false,
            engine: EngineStatus::NotLoaded,
            message: None,
            limit_reached: false,
            recovery_text: None,
            cancel_registered: false,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.phase,
            session: self.session,
            mode: self.mode,
            paused: self.paused,
            engine: self.engine.clone(),
            message: self.message.clone(),
            limit_reached: self.limit_reached,
            recovery_text: self.recovery_text.clone(),
        }
    }

    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();
        match event {
            Event::ShortcutPressed => {
                // Key auto-repeat delivers repeated presses without a release.
                if self.key_down {
                    return actions;
                }
                self.key_down = true;
                if self.paused {
                    return actions;
                }
                match self.phase {
                    Phase::Idle | Phase::Error | Phase::Recovery => self.start(&mut actions),
                    Phase::Recording => self.stop(&mut actions),
                    // No overlapping sessions and no silent queueing.
                    Phase::Transcribing | Phase::Inserting => {}
                }
            }
            Event::ShortcutReleased => {
                self.key_down = false;
                if self.mode == RecordMode::Hold && self.phase == Phase::Recording {
                    self.stop(&mut actions);
                }
            }
            Event::ToggleRequested => {
                if self.paused {
                    return actions;
                }
                match self.phase {
                    Phase::Idle | Phase::Error | Phase::Recovery => self.start(&mut actions),
                    Phase::Recording => self.stop(&mut actions),
                    Phase::Transcribing | Phase::Inserting => {}
                }
            }
            Event::CancelPressed => match self.phase {
                Phase::Recording => {
                    actions.push(Action::DiscardCapture(self.session));
                    self.go_idle(Some("Cancelled".into()), &mut actions);
                }
                Phase::Transcribing => {
                    actions.push(Action::CancelTranscription(self.session));
                    self.go_idle(Some("Cancelled".into()), &mut actions);
                }
                // Insertion may already be under way; cancelling could leave
                // the destination in an unknown state, so let it finish.
                _ => {}
            },
            Event::CaptureLimitReached(s) => {
                if s == self.session && self.phase == Phase::Recording {
                    self.limit_reached = true;
                    self.stop(&mut actions);
                }
            }
            Event::CaptureFailed(s, msg) => {
                if s == self.session && self.phase == Phase::Recording {
                    actions.push(Action::DiscardCapture(s));
                    self.go_error(msg, &mut actions);
                }
            }
            Event::TranscriptionFinished(s, outcome) => {
                if s != self.session || self.phase != Phase::Transcribing {
                    return actions; // stale or cancelled session
                }
                match outcome {
                    Outcome::Text(text) => {
                        self.phase = Phase::Inserting;
                        self.message = None;
                        self.update_cancel(false, &mut actions);
                        actions.push(Action::Insert(s, text));
                        actions.push(Action::Publish);
                    }
                    Outcome::NoSpeech => {
                        self.go_idle(Some("No speech detected".into()), &mut actions)
                    }
                    Outcome::Cancelled => self.go_idle(None, &mut actions),
                    Outcome::Failed(msg) => self.go_error(msg, &mut actions),
                }
            }
            Event::InsertFinished(s, result) => {
                if s != self.session || self.phase != Phase::Inserting {
                    return actions;
                }
                match result {
                    Ok(()) => self.go_idle(None, &mut actions),
                    Err(text) => {
                        self.phase = Phase::Recovery;
                        self.recovery_text = Some(text);
                        self.message = Some("Text was not inserted".into());
                        actions.push(Action::Publish);
                    }
                }
            }
            Event::EngineStatusChanged(status) => {
                self.engine = status;
                actions.push(Action::Publish);
            }
            Event::SetMode(mode) => {
                self.mode = mode;
                actions.push(Action::Publish);
            }
            Event::SetPaused(paused) => {
                self.paused = paused;
                if paused {
                    // Pausing during a recording cancels it rather than leaving
                    // the microphone open with no way to stop it.
                    if self.phase == Phase::Recording {
                        actions.push(Action::DiscardCapture(self.session));
                        self.go_idle(Some("Paused".into()), &mut actions);
                    }
                }
                actions.push(Action::Publish);
            }
            Event::Dismiss => {
                if matches!(self.phase, Phase::Error | Phase::Recovery) {
                    self.recovery_text = None;
                    self.go_idle(None, &mut actions);
                }
            }
        }
        actions
    }

    fn start(&mut self, actions: &mut Vec<Action>) {
        if let EngineStatus::Unavailable(reason) = &self.engine {
            let msg = format!("Transcription engine unavailable: {reason}");
            self.go_error(msg, actions);
            return;
        }
        self.session += 1;
        self.phase = Phase::Recording;
        self.message = None;
        self.limit_reached = false;
        actions.push(Action::StartCapture(self.session));
        self.update_cancel(true, actions);
        actions.push(Action::Publish);
    }

    fn stop(&mut self, actions: &mut Vec<Action>) {
        self.phase = Phase::Transcribing;
        actions.push(Action::StopCapture(self.session));
        actions.push(Action::Publish);
    }

    fn go_idle(&mut self, message: Option<String>, actions: &mut Vec<Action>) {
        self.phase = Phase::Idle;
        self.message = message;
        self.update_cancel(false, actions);
        actions.push(Action::Publish);
    }

    fn go_error(&mut self, message: String, actions: &mut Vec<Action>) {
        self.phase = Phase::Error;
        self.message = Some(message);
        self.update_cancel(false, actions);
        actions.push(Action::Publish);
    }

    fn update_cancel(&mut self, on: bool, actions: &mut Vec<Action>) {
        if self.cancel_registered != on {
            self.cancel_registered = on;
            actions.push(Action::SetCancelShortcut(on));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(mode: RecordMode) -> Controller {
        let mut c = Controller::new(mode);
        c.handle(Event::EngineStatusChanged(EngineStatus::Ready));
        c
    }

    fn tap(c: &mut Controller) -> Vec<Action> {
        let mut a = c.handle(Event::ShortcutPressed);
        a.extend(c.handle(Event::ShortcutReleased));
        a
    }

    #[test]
    fn toggle_full_cycle() {
        let mut c = ready(RecordMode::Toggle);
        let a = tap(&mut c);
        assert!(a.contains(&Action::StartCapture(1)));
        assert!(a.contains(&Action::SetCancelShortcut(true)));
        assert_eq!(c.phase(), Phase::Recording);

        let a = tap(&mut c);
        assert!(a.contains(&Action::StopCapture(1)));
        assert_eq!(c.phase(), Phase::Transcribing);

        let a = c.handle(Event::TranscriptionFinished(1, Outcome::Text("hi".into())));
        assert!(a.contains(&Action::Insert(1, "hi".into())));
        assert!(a.contains(&Action::SetCancelShortcut(false)));
        assert_eq!(c.phase(), Phase::Inserting);

        c.handle(Event::InsertFinished(1, Ok(())));
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn key_repeat_does_not_create_duplicate_sessions() {
        let mut c = ready(RecordMode::Toggle);
        c.handle(Event::ShortcutPressed);
        for _ in 0..20 {
            assert!(c.handle(Event::ShortcutPressed).is_empty());
        }
        assert_eq!(c.session(), 1);
        assert_eq!(c.phase(), Phase::Recording);
    }

    #[test]
    fn hold_mode_records_while_held() {
        let mut c = ready(RecordMode::Hold);
        c.handle(Event::ShortcutPressed);
        c.handle(Event::ShortcutPressed); // repeat
        assert_eq!(c.phase(), Phase::Recording);
        let a = c.handle(Event::ShortcutReleased);
        assert!(a.contains(&Action::StopCapture(1)));
        assert_eq!(c.phase(), Phase::Transcribing);
    }

    #[test]
    fn toggle_mode_ignores_release() {
        let mut c = ready(RecordMode::Toggle);
        c.handle(Event::ShortcutPressed);
        assert!(
            !c.handle(Event::ShortcutReleased)
                .iter()
                .any(|a| matches!(a, Action::StopCapture(_)))
        );
        assert_eq!(c.phase(), Phase::Recording);
    }

    #[test]
    fn ui_toggle_works_in_hold_mode() {
        let mut c = ready(RecordMode::Hold);
        c.handle(Event::ToggleRequested);
        assert_eq!(c.phase(), Phase::Recording);
        c.handle(Event::ToggleRequested);
        assert_eq!(c.phase(), Phase::Transcribing);
    }

    #[test]
    fn presses_during_transcription_are_ignored() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        tap(&mut c);
        assert_eq!(c.phase(), Phase::Transcribing);
        let a = tap(&mut c);
        assert!(!a.iter().any(|a| matches!(a, Action::StartCapture(_))));
        assert_eq!(c.session(), 1);
    }

    #[test]
    fn cancel_during_recording_discards() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        let a = c.handle(Event::CancelPressed);
        assert!(a.contains(&Action::DiscardCapture(1)));
        assert!(a.contains(&Action::SetCancelShortcut(false)));
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn cancelled_transcription_result_is_never_inserted() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        tap(&mut c);
        let a = c.handle(Event::CancelPressed);
        assert!(a.contains(&Action::CancelTranscription(1)));
        let a = c.handle(Event::TranscriptionFinished(
            1,
            Outcome::Text("late".into()),
        ));
        assert!(a.is_empty());
        assert_eq!(c.phase(), Phase::Idle);
    }

    #[test]
    fn stale_result_from_superseded_session_is_ignored() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        tap(&mut c);
        c.handle(Event::CancelPressed);
        tap(&mut c); // session 2 recording
        tap(&mut c); // session 2 transcribing
        assert!(
            c.handle(Event::TranscriptionFinished(1, Outcome::Text("old".into())))
                .is_empty()
        );
        let a = c.handle(Event::TranscriptionFinished(2, Outcome::Text("new".into())));
        assert!(a.contains(&Action::Insert(2, "new".into())));
    }

    #[test]
    fn escape_is_not_registered_while_idle() {
        let mut c = ready(RecordMode::Toggle);
        assert!(c.handle(Event::CancelPressed).is_empty());
        tap(&mut c);
        tap(&mut c);
        let a = c.handle(Event::TranscriptionFinished(1, Outcome::NoSpeech));
        assert!(a.contains(&Action::SetCancelShortcut(false)));
        assert_eq!(c.phase(), Phase::Idle);
        assert_eq!(c.snapshot().message.as_deref(), Some("No speech detected"));
    }

    #[test]
    fn cancel_during_insert_is_ignored() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        tap(&mut c);
        c.handle(Event::TranscriptionFinished(1, Outcome::Text("x".into())));
        assert!(c.handle(Event::CancelPressed).is_empty());
        assert_eq!(c.phase(), Phase::Inserting);
    }

    #[test]
    fn capture_limit_stops_and_transcribes() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        let a = c.handle(Event::CaptureLimitReached(1));
        assert!(a.contains(&Action::StopCapture(1)));
        assert!(c.snapshot().limit_reached);
        // Stale limit event from an old session does nothing.
        assert!(c.handle(Event::CaptureLimitReached(0)).is_empty());
    }

    #[test]
    fn capture_failure_goes_to_error_and_recovers() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        c.handle(Event::CaptureFailed(1, "device disconnected".into()));
        assert_eq!(c.phase(), Phase::Error);
        tap(&mut c);
        assert_eq!(c.phase(), Phase::Recording);
        assert_eq!(c.session(), 2);
    }

    #[test]
    fn failed_insert_keeps_text_for_recovery() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        tap(&mut c);
        c.handle(Event::TranscriptionFinished(
            1,
            Outcome::Text("keep me".into()),
        ));
        c.handle(Event::InsertFinished(1, Err("keep me".into())));
        assert_eq!(c.phase(), Phase::Recovery);
        assert_eq!(c.snapshot().recovery_text.as_deref(), Some("keep me"));
        c.handle(Event::Dismiss);
        assert_eq!(c.phase(), Phase::Idle);
        assert!(c.snapshot().recovery_text.is_none());
    }

    #[test]
    fn unavailable_engine_reports_error_instead_of_recording() {
        let mut c = Controller::new(RecordMode::Toggle);
        c.handle(Event::EngineStatusChanged(EngineStatus::Unavailable(
            "no model".into(),
        )));
        let a = tap(&mut c);
        assert!(!a.iter().any(|a| matches!(a, Action::StartCapture(_))));
        assert_eq!(c.phase(), Phase::Error);
    }

    #[test]
    fn recording_allowed_while_engine_loading() {
        let mut c = Controller::new(RecordMode::Toggle);
        c.handle(Event::EngineStatusChanged(EngineStatus::Loading));
        tap(&mut c);
        assert_eq!(c.phase(), Phase::Recording);
    }

    #[test]
    fn paused_ignores_shortcut_and_pausing_cancels_recording() {
        let mut c = ready(RecordMode::Toggle);
        tap(&mut c);
        let a = c.handle(Event::SetPaused(true));
        assert!(a.contains(&Action::DiscardCapture(1)));
        assert_eq!(c.phase(), Phase::Idle);
        assert!(
            !tap(&mut c)
                .iter()
                .any(|a| matches!(a, Action::StartCapture(_)))
        );
        c.handle(Event::SetPaused(false));
        tap(&mut c);
        assert_eq!(c.phase(), Phase::Recording);
    }
}
