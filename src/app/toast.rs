use super::State;
use std::time::Instant;

/// A message for the key bar. `update` has no clock for key presses, so the
/// next tick stamps `shown_at`, and a later tick removes the toast.
pub struct Toast {
    pub text: String,
    pub shown_at: Option<Instant>,
}

pub(super) fn say(state: &mut State, text: String) {
    state.toast = Some(Toast { text, shown_at: None });
}
