use super::keys::{Key, key_of};
use super::list::current_session;
use super::msg::Cmd;
use super::toast;
use super::{Focus, State};
use crossterm::event::{KeyCode, KeyEvent};

/// `i`: show all of what the selected session's Claude last said. The row
/// shows only its first line.
pub(super) fn summary_key(state: &mut State) -> Vec<Cmd> {
    let Some(issue) = current_session(state).map(|s| s.issue) else {
        toast::info(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    if state.transcripts.get(&issue).and_then(|t| t.said.as_ref()).is_none() {
        toast::info(state, format!("Claude has not said anything in {issue} yet."));
        return Vec::new();
    }
    state.focus = Focus::Summary { issue, scroll: 0 };
    Vec::new()
}

pub(super) fn summary_overlay_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Summary { scroll, .. } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Char('j') | Key::Code(KeyCode::Down)) => {
            *scroll = scroll.saturating_add(1).min(state.hits.max_scroll);
        }
        Some(Key::Char('k') | Key::Code(KeyCode::Up)) => *scroll = scroll.saturating_sub(1),
        Some(Key::Code(KeyCode::Esc) | Key::Char('i')) => state.focus = Focus::List,
        _ => {}
    }
    Vec::new()
}
