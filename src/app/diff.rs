use super::keys::{Key, key_of};
use super::list::current_session;
use super::msg::{Cmd, Job, ReadJob};
use super::toast;
use super::{DiffBody, Focus, State};
use crate::domain::IssueNumber;
use crate::session::Diff;
use crate::session::error::SessionError;
use crossterm::event::{KeyCode, KeyEvent};

/// `d`: show the diff of the selected session's worktree. The read runs on a
/// worker, and the overlay says so until it lands.
pub(super) fn diff_key(state: &mut State) -> Vec<Cmd> {
    let Some(issue) = current_session(state).map(|s| s.issue) else {
        toast::info(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    state.focus = Focus::Diff { issue, scroll: 0, body: DiffBody::Reading };
    vec![Cmd::Job(Job::Read(ReadJob::Diff(issue)))]
}

pub(super) fn diff_overlay_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Diff { scroll, .. } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Char('j') | Key::Code(KeyCode::Down)) => {
            *scroll = scroll.saturating_add(1).min(state.hits.max_scroll);
        }
        Some(Key::Char('k') | Key::Code(KeyCode::Up)) => *scroll = scroll.saturating_sub(1),
        Some(Key::Code(KeyCode::Esc) | Key::Char('d')) => state.focus = Focus::List,
        _ => {}
    }
    Vec::new()
}

/// A result for an overlay that closed, or that shows another issue, is dropped.
pub(super) fn read_diff(state: &mut State, issue: IssueNumber, result: Result<Diff, SessionError>) -> Vec<Cmd> {
    let waiting = matches!(state.focus, Focus::Diff { issue: i, body: DiffBody::Reading, .. } if i == issue);
    if !waiting {
        return Vec::new();
    }
    match result {
        Ok(diff) if diff.is_empty() => {
            state.focus = Focus::List;
            toast::info(state, format!("{issue} has no changes."));
        }
        Ok(diff) => state.focus = Focus::Diff { issue, scroll: 0, body: DiffBody::Read(diff) },
        Err(e) => {
            state.focus = Focus::List;
            toast::error(state, format!("Reading the diff of {issue} failed: {e}"));
        }
    }
    Vec::new()
}
