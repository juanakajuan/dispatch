//! Every toast follows one style, and `info`, `error`, and `append` check it
//! in debug builds:
//!
//! - A toast is whole sentences. Each starts with a capital letter or a
//!   reference such as `#16`, and ends with a full stop:
//!   `#16 has no changes.`
//! - A toast for work that has started and not ended ends with `…`:
//!   `Starting #16…`.
//! - A failure is `<Gerund> <object> failed: <error>.`, made by `failed`:
//!   `Starting #16 failed: tmux exited.`

use super::State;
use super::msg::TOAST_FOR;
use std::fmt::Display;
use std::time::Instant;

/// A message for the key bar. `update` has no clock for key presses, so the
/// next tick stamps `shown_at`, and a later tick removes an `Info` toast. An
/// `Error` toast stays until the next key, so the user can read its cause.
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
        debug_assert!(follows_style(&self.text), "toast breaks the style: {:?}", self.text);
    }
}

/// `<what> failed: <error>.` An error that ends in a full stop keeps one.
pub(super) fn failed(what: impl Display, error: impl Display) -> String {
    let error = error.to_string();
    format!("{what} failed: {}.", error.strip_suffix('.').unwrap_or(&error))
}

/// The checks of the style in the module docs that need no grammar.
pub(super) fn follows_style(text: &str) -> bool {
    let starts_well = text.chars().next().is_some_and(|c| !c.is_lowercase() && !c.is_whitespace());
    let ends_well = (text.ends_with('.') && !text.ends_with("..")) || text.ends_with('…');
    starts_well && ends_well
}

pub(super) fn info(state: &mut State, text: String) {
    debug_assert!(follows_style(&text), "toast breaks the style: {text:?}");
    state.toast = Some(Toast { level: Level::Info, text, shown_at: None });
}

pub(super) fn error(state: &mut State, text: String) {
    debug_assert!(follows_style(&text), "toast breaks the style: {text:?}");
    state.toast = Some(Toast { level: Level::Error, text, shown_at: None });
}

/// Stamps a new toast, and removes an `Info` toast shown for `TOAST_FOR`.
pub(super) fn expire(state: &mut State, now: Instant) {
    let Some(toast) = &mut state.toast else { return };
    match (toast.level, toast.shown_at) {
        (_, None) => toast.shown_at = Some(now),
        (Level::Info, Some(at)) if now.duration_since(at) >= TOAST_FOR => state.toast = None,
        (_, Some(_)) => {}
    }
}

/// A key press removes an `Error` toast that is on screen.
pub(super) fn dismiss_error(state: &mut State) {
    if state.toast.as_ref().is_some_and(|t| t.level == Level::Error) {
        state.toast = None;
    }
}
