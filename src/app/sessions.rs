use super::batch::open_batch;
use super::dialog::{open_dialog, open_pr_dialog};
use super::feed::observe_now;
use super::list::{RowKey, Tab, current_session, jump_to, selected_row};
use super::merge::{CannotFix, NoTarget, head_start};
use super::msg::{Cmd, Job, WriteJob};
use super::toast;
use super::{Focus, State};
use crate::domain::{BranchName, IssueNumber, PrNumber, PrState, PullRequest, SessionKey};
use crate::session::error::{AttachError, SessionError};
use crate::session::request::{Launch, StartFrom, StartRequest};
use crate::session::{AttachTo, StartOutcome};

enum StartAction {
    Batch,
    Goto(SessionKey),
    IssueDialog(IssueNumber),
    PrDialog { pr: PrNumber, branch: BranchName, from: StartFrom },
}

enum StartBlocked {
    NoRow,
    AlreadyStarting(SessionKey),
    NotOpen(PrNumber),
    CannotFix(CannotFix),
}

fn existing_pr_session(state: &State, pr: &PullRequest) -> Option<SessionKey> {
    let key = SessionKey::Pr(pr.number);
    let on_branch = state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)).map(|s| s.key);
    on_branch.or_else(|| state.sessions.contains_key(&key).then_some(key))
}

fn plan_start(state: &State) -> Result<StartAction, StartBlocked> {
    match selected_row(state) {
        Some(RowKey::Issue(n)) if state.tab == Tab::Issues => {
            if !state.marked.is_empty() {
                return Ok(StartAction::Batch);
            }
            let key = SessionKey::Issue(n);
            if state.sessions.contains_key(&key) {
                Ok(StartAction::Goto(key))
            } else if state.starting.contains(&key) {
                Err(StartBlocked::AlreadyStarting(key))
            } else if state.issues.items.iter().any(|i| i.number == n) {
                Ok(StartAction::IssueDialog(n))
            } else {
                Err(StartBlocked::NoRow)
            }
        }
        Some(RowKey::Pr(n)) if state.tab == Tab::Prs => {
            let Some(pr) = state.prs.items.iter().find(|p| p.number == n) else {
                return Err(StartBlocked::NoRow);
            };
            if matches!(pr.state, PrState::Merged | PrState::Closed) {
                return Err(StartBlocked::NotOpen(n));
            }
            let key = SessionKey::Pr(n);
            if let Some(existing) = existing_pr_session(state, pr) {
                Ok(StartAction::Goto(existing))
            } else if state.starting.contains(&key) {
                Err(StartBlocked::AlreadyStarting(key))
            } else {
                head_start(pr)
                    .map(|(branch, from)| StartAction::PrDialog { pr: n, branch, from })
                    .map_err(StartBlocked::CannotFix)
            }
        }
        _ => Err(StartBlocked::NoRow),
    }
}

pub(super) fn can_start(state: &State) -> bool {
    plan_start(state).is_ok()
}

/// `s` on an issue or a PR, Enter on an issue, or a double-click on an issue.
pub(super) fn start_key(state: &mut State) -> Vec<Cmd> {
    match plan_start(state) {
        Ok(StartAction::Batch) => open_batch(state),
        Ok(StartAction::Goto(key)) => goto_session(state, key),
        Ok(StartAction::IssueDialog(n)) => {
            if let Some(issue) = state.issues.items.iter().find(|i| i.number == n) {
                state.focus = Focus::Dialog(Box::new(open_dialog(issue)));
            }
        }
        Ok(StartAction::PrDialog { pr, branch, from }) => {
            if let Some(pr) = state.prs.items.iter().find(|p| p.number == pr) {
                state.focus = Focus::Dialog(Box::new(open_pr_dialog(pr, &branch, from)));
            }
        }
        Err(StartBlocked::NoRow) => {}
        Err(StartBlocked::AlreadyStarting(key)) => toast::info(state, format!("{key} is already starting")),
        Err(StartBlocked::NotOpen(n)) => toast::info(state, NoTarget::NotOpen(n).to_string()),
        Err(StartBlocked::CannotFix(why)) => toast::info(state, why.to_string()),
    }
    Vec::new()
}

/// Registers a start and returns its job. The issue is no longer marked.
pub(super) fn begin_start(state: &mut State, req: StartRequest) -> Cmd {
    state.starting.insert(req.key);
    if let SessionKey::Issue(n) = req.key {
        state.marked.remove(&n);
    }
    Cmd::Job(Job::Write(WriteJob::Start(req, Launch::from_config(&state.config))))
}

pub(super) fn goto_session(state: &mut State, key: SessionKey) {
    jump_to(state, RowKey::Session(key));
}

pub(super) fn attach_key(state: &mut State) -> Vec<Cmd> {
    let Some(session) = current_session(state) else {
        toast::info(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    let (key, harness) = (session.key, session.harness);
    if session.state.is_live() {
        vec![Cmd::Attach { key, to: AttachTo::Claude }]
    } else if harness.has_transcript() && !state.transcripts.contains_key(&key) {
        start_again(state, key);
        Vec::new()
    } else {
        let launch = Launch::for_session(&state.config, harness);
        toast::info(state, format!("Resuming {key} with {}", launch.resume_words()));
        vec![Cmd::Job(Job::Write(WriteJob::Resume(key, launch)))]
    }
}

/// `claude --continue` exits at once in a worktree with no conversation,
/// as after a start whose Claude never ran. Only Claude has a transcript to
/// tell, so another harness always resumes.
fn start_again(state: &mut State, key: SessionKey) {
    let dialog = match key {
        SessionKey::Issue(n) => state.issues.items.iter().find(|i| i.number == n).map(open_dialog),
        SessionKey::Pr(n) => state
            .prs
            .items
            .iter()
            .find(|p| p.number == n)
            .and_then(|pr| head_start(pr).ok().map(|(branch, from)| open_pr_dialog(pr, &branch, from))),
    };
    if let Some(dialog) = dialog {
        state.focus = Focus::Dialog(Box::new(dialog));
        toast::info(state, format!("{key} has no conversation to continue. Start it again."));
    } else {
        let what = match key {
            SessionKey::Issue(_) => "issue",
            SessionKey::Pr(_) => "PR",
        };
        toast::error(state, format!("{key} has no conversation to continue, and the {what} is not listed."));
    }
}

/// `!`: a plain shell in the session's worktree, for a live session or an
/// exited one.
pub(super) fn shell_key(state: &mut State) -> Vec<Cmd> {
    let Some(key) = current_session(state).map(|s| s.key) else {
        toast::info(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    vec![Cmd::Attach { key, to: AttachTo::Shell }]
}

pub(super) fn stop_key(state: &mut State) -> Vec<Cmd> {
    let live = current_session(state).filter(|s| s.state.is_live());
    if let Some(key) = live.map(|s| s.key) {
        toast::info(state, format!("Stopping {key}"));
        return vec![Cmd::Job(Job::Write(WriteJob::Stop(key)))];
    }
    toast::info(state, "No running session here.".to_string());
    Vec::new()
}

pub(super) fn started(state: &mut State, key: SessionKey, result: Result<StartOutcome, SessionError>) -> Vec<Cmd> {
    state.starting.remove(&key);
    match result {
        Ok(outcome) => {
            let text = match outcome.warnings.first() {
                Some(w) => format!("Started {key} (warning: {w})"),
                None => format!("Started {key}"),
            };
            state.sessions.insert(key, outcome.session);
            state.due.github_soon = true;
            toast::info(state, text);
        }
        Err(e) => toast::error(state, format!("Start {key} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn stopped(state: &mut State, key: SessionKey, result: Result<(), SessionError>) -> Vec<Cmd> {
    state.due.github_soon = true;
    match result {
        Ok(()) => toast::info(state, format!("Stopped {key}")),
        Err(e) => toast::error(state, format!("Stop {key} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn resumed(state: &mut State, key: SessionKey, result: Result<(), SessionError>) -> Vec<Cmd> {
    match result {
        Ok(()) => toast::info(state, format!("Resumed {key}. Press a to attach.")),
        Err(e) => toast::error(state, format!("Resume {key} failed: {e}")),
    }
    observe_now(state)
}

pub(super) fn attached(state: &mut State, key: SessionKey, to: AttachTo, result: Result<(), AttachError>) -> Vec<Cmd> {
    if let Err(e) = result {
        match to {
            AttachTo::Claude => toast::error(state, format!("attach failed: {e}")),
            AttachTo::Shell => toast::error(state, format!("Opening a shell in {key} failed: {e}")),
        }
    }
    observe_now(state)
}
