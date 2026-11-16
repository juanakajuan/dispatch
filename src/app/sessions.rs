use super::dialog::open_dialog;
use super::feed::observe_now;
use super::list::{RowKey, Tab, current_session, selected_row};
use super::msg::{Cmd, Job};
use super::toast::say;
use super::{Focus, State};
use crate::domain::IssueNumber;
use crate::session::StartOutcome;
use crate::session::request::Launch;

/// `s` or Enter on an issue, or a double-click on one.
pub(super) fn start_key(state: &mut State) -> Vec<Cmd> {
    let Some(RowKey::Issue(n)) = selected_row(state).filter(|_| state.tab == Tab::Issues) else {
        return Vec::new();
    };
    start_issue(state, n);
    Vec::new()
}

pub(super) fn start_issue(state: &mut State, n: IssueNumber) {
    if state.sessions.contains_key(&n) {
        goto_session(state, n);
    } else if state.starting.contains(&n) {
        say(state, format!("{n} is already starting"));
    } else if let Some(issue) = state.issues.items.iter().find(|i| i.number == n) {
        state.focus = Focus::Dialog(Box::new(open_dialog(issue)));
    }
}

pub(super) fn goto_session(state: &mut State, n: IssueNumber) {
    state.tab = Tab::Sessions;
    state.selected.session = Some(n);
    state.filter.clear();
}

pub(super) fn attach_key(state: &mut State) -> Vec<Cmd> {
    let Some(session) = current_session(state) else {
        say(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    let issue = session.issue;
    if session.state.is_live() {
        vec![Cmd::Attach(issue)]
    } else {
        say(state, format!("Resuming {issue} with claude --continue"));
        vec![Cmd::Job(Job::Resume(issue, Launch::from_config(&state.config)))]
    }
}

pub(super) fn stop_key(state: &mut State) -> Vec<Cmd> {
    let live = current_session(state).filter(|s| s.state.is_live());
    if let Some(issue) = live.map(|s| s.issue) {
        say(state, format!("Stopping {issue}"));
        return vec![Cmd::Job(Job::Stop(issue))];
    }
    say(state, "No running session here.".to_string());
    Vec::new()
}

pub(super) fn started(state: &mut State, issue: IssueNumber, result: Result<StartOutcome, String>) -> Vec<Cmd> {
    state.starting.remove(&issue);
    match result {
        Ok(outcome) => {
            let text = match outcome.warnings.first() {
                Some(w) => format!("Started {issue} (warning: {w})"),
                None => format!("Started {issue}"),
            };
            state.sessions.insert(issue, outcome.session);
            state.due.github_soon = true;
            say(state, text);
        }
        Err(e) => say(state, format!("Start {issue} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn stopped(state: &mut State, issue: IssueNumber, result: Result<(), String>) -> Vec<Cmd> {
    state.due.github_soon = true;
    match result {
        Ok(()) => say(state, format!("Stopped {issue}")),
        Err(e) => say(state, format!("Stop {issue} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn resumed(state: &mut State, issue: IssueNumber, result: Result<(), String>) -> Vec<Cmd> {
    match result {
        Ok(()) => say(state, format!("Resumed {issue}. Press a to attach.")),
        Err(e) => say(state, format!("Resume {issue} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn attached(state: &mut State, result: Result<(), String>) -> Vec<Cmd> {
    if let Err(e) = result {
        say(state, format!("attach failed: {e}"));
    }
    observe_now(state)
}

pub(super) fn sent(state: &mut State, issue: IssueNumber, result: Result<(), String>) -> Vec<Cmd> {
    match result {
        Ok(()) => say(state, format!("Asked {issue}'s agent to fix and merge its PR")),
        Err(e) => say(state, format!("Merge request to {issue} failed: {e}")),
    }
    Vec::new()
}
