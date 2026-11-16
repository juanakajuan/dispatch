use super::State;
use super::msg::TOAST_FOR;
use std::time::Instant;

/// A message for the key bar. `update` has no clock for key presses, so the
/// next tick stamps `shown_at`, and a later tick removes the toast.
pub struct Toast {
    level: Level,
    text: String,
    shown_at: Option<Instant>,
}

/// `Error` is a job that failed. Everything else, warnings and refusals
/// included, is `Info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

impl Toast {
    pub fn level(&self) -> Level {
        self.level
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Adds a sentence and keeps the level.
    pub(super) fn append(&mut self, sentence: &str) {
        self.text.push(' ');
        self.text.push_str(sentence);
    }
}

pub(super) fn info(state: &mut State, text: String) {
    state.toast = Some(Toast { level: Level::Info, text, shown_at: None });
}

pub(super) fn error(state: &mut State, text: String) {
    state.toast = Some(Toast { level: Level::Error, text, shown_at: None });
}

/// Stamps a new toast, and removes one shown for `TOAST_FOR`.
pub(super) fn expire(state: &mut State, now: Instant) {
    let Some(toast) = &mut state.toast else { return };
    match toast.shown_at {
        None => toast.shown_at = Some(now),
        Some(at) if now.duration_since(at) >= TOAST_FOR => state.toast = None,
        Some(_) => {}
    }
}
