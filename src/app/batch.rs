//! Starting several sessions at once: Space marks issues, then `s` opens one
//! dialog for all marked issues. Each uses its default branch and prompt.

use super::keys::{Key, key_of};
use super::list::{RowKey, Tab, selected_row};
use super::mouse::HitTarget;
use super::msg::Cmd;
use super::sessions::{begin_start, goto_session};
use super::toast;
use super::{Focus, State};
use crate::domain::{Issue, IssueNumber, Mode};
use crate::session::request::{StartFrom, StartRequest, start_defaults};
use crossterm::event::{KeyCode, KeyEvent};

pub struct BatchStart {
    /// The issues to start with their titles, in issue-number order.
    pub issues: Vec<(IssueNumber, String)>,
    pub mode: Mode,
}

const NONE_STARTABLE: &str = "No marked issue can start.";

fn has_session(state: &State, n: IssueNumber) -> bool {
    state.sessions.contains_key(&n) || state.starting.contains(&n)
}

/// Space on an issue row.
pub(super) fn mark_key(state: &mut State) {
    let Some(RowKey::Issue(n)) = selected_row(state).filter(|_| state.tab == Tab::Issues) else { return };
    if has_session(state, n) {
        toast::info(state, format!("{n} already has a session"));
    } else if !state.marked.remove(&n) {
        state.marked.insert(n);
    }
}

/// `s` with marks. Marks that can no longer start are dropped.
pub(super) fn open_batch(state: &mut State) {
    let issues: Vec<&Issue> = state
        .marked
        .iter()
        .filter(|n| !has_session(state, **n))
        .filter_map(|n| state.issues.items.iter().find(|i| i.number == *n))
        .collect();
    let Some(first) = issues.first() else {
        state.marked.clear();
        toast::info(state, NONE_STARTABLE.to_string());
        return;
    };
    let batch = BatchStart {
        mode: start_defaults(first).mode,
        issues: issues.iter().map(|i| (i.number, i.title.clone())).collect(),
    };
    state.focus = Focus::Batch(Box::new(batch));
}

pub(super) fn batch_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Batch(batch) = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Code(KeyCode::Esc)) => state.focus = Focus::List,
        Some(Key::Ctrl('s') | Key::Code(KeyCode::Enter)) => return confirm_batch(state),
        Some(Key::Code(KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab) | Key::Char(' ')) => {
            batch.mode = other(batch.mode);
        }
        _ => {}
    }
    Vec::new()
}

fn other(mode: Mode) -> Mode {
    match mode {
        Mode::PlanFirst => Mode::Implement,
        Mode::Implement => Mode::PlanFirst,
    }
}

fn confirm_batch(state: &mut State) -> Vec<Cmd> {
    let Focus::Batch(batch) = &state.focus else { return Vec::new() };
    let (numbers, mode): (Vec<IssueNumber>, Mode) = (batch.issues.iter().map(|(n, _)| *n).collect(), batch.mode);
    let mut started = Vec::new();
    let mut cmds = Vec::new();
    for n in numbers {
        let issue = state.issues.items.iter().find(|i| i.number == n);
        let Some(issue) = issue.filter(|_| !has_session(state, n)) else { continue };
        let defaults = start_defaults(issue);
        let req =
            StartRequest { issue: n, branch: defaults.branch, mode, prompt: defaults.prompt, from: StartFrom::Base };
        cmds.push(begin_start(state, req));
        started.push(n);
    }
    state.marked.clear();
    state.focus = Focus::List;
    let Some(first) = started.first() else {
        toast::info(state, NONE_STARTABLE.to_string());
        return cmds;
    };
    goto_session(state, *first);
    let list = started.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    toast::info(state, format!("Starting {list}"));
    cmds
}

/// A click inside the batch dialog: only the mode options take clicks.
pub(super) fn click_batch(batch: &mut BatchStart, target: HitTarget) {
    if let HitTarget::Mode(mode) = target {
        batch.mode = mode;
    }
}
