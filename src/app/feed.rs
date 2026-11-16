use super::State;
use super::msg::{Cmd, GITHUB_COOLDOWN, Job, SESSIONS_EVERY, TOAST_FOR};
use super::toast::say;
use crate::github::GhError;
use crate::session::error::SessionError;
use crate::session::{SessionState, Sessions};
use std::time::Instant;

/// Remote data plus its load status. `items` keeps the last good data when a
/// refresh fails, so a network error does not blank the list.
pub struct Feed<T> {
    pub items: Vec<T>,
    pub status: FeedStatus,
}

/// `at` is when the response arrived, not when the request was sent.
pub enum FeedStatus {
    Loading,
    Fresh { at: Instant },
    Failed { error: GhError, at: Instant },
}

/// One flag per job kind. The tick only schedules a job whose flag is clear,
/// so a slow `gh` never stacks up a second call.
#[derive(Default)]
pub struct Inflight {
    pub issues: bool,
    pub prs: bool,
    pub sessions: bool,
}

pub struct Due {
    pub github: Instant,
    /// Set by an event that likely changed GitHub data. A tick fetches once
    /// no fetch is in flight and `GITHUB_COOLDOWN` has passed since
    /// `github_sent`, so a burst of events makes one fetch.
    pub github_soon: bool,
    /// When the last tick sent a GitHub fetch.
    pub github_sent: Instant,
    pub sessions: Instant,
}

/// True when a session's Claude stopped working: it waits for you, or it
/// exited. That is usually when it has just opened or pushed a PR.
pub(super) fn went_idle(before: SessionState, after: Option<SessionState>) -> bool {
    matches!(
        (before, after),
        (SessionState::Running, Some(SessionState::NeedsYou | SessionState::Exited) | None)
            | (SessionState::NeedsYou, Some(SessionState::Exited) | None)
    )
}

pub(super) fn observe_now(state: &mut State) -> Vec<Cmd> {
    if state.inflight.sessions {
        return Vec::new();
    }
    state.inflight.sessions = true;
    vec![Cmd::Job(Job::ObserveSessions)]
}

pub(super) fn apply_feed<T>(feed: &mut Feed<T>, result: Result<Vec<T>, GhError>, at: Instant) {
    match result {
        Ok(items) => {
            feed.items = items;
            feed.status = FeedStatus::Fresh { at };
        }
        Err(error) => feed.status = FeedStatus::Failed { error, at },
    }
}

pub(super) fn tick(state: &mut State, now: Instant) -> Vec<Cmd> {
    if let Some(toast) = &mut state.toast {
        match toast.shown_at {
            None => toast.shown_at = Some(now),
            Some(at) if now.duration_since(at) >= TOAST_FOR => state.toast = None,
            Some(_) => {}
        }
    }
    let mut cmds = Vec::new();
    let idle = !state.inflight.issues && !state.inflight.prs;
    let soon = state.due.github_soon && idle && now >= state.due.github_sent + GITHUB_COOLDOWN;
    if soon || now >= state.due.github {
        // A fetch already in flight may predate the event, so `soon` waits for it.
        if idle {
            state.due.github_soon = false;
        }
        state.due.github = now + state.config.github_refresh;
        state.due.github_sent = now;
        refresh_github(state, &mut cmds);
    }
    if now >= state.due.sessions {
        state.due.sessions = now + SESSIONS_EVERY;
        cmds.extend(observe_now(state));
    }
    cmds
}

pub(super) fn refresh_github(state: &mut State, cmds: &mut Vec<Cmd>) {
    if !state.inflight.issues {
        state.inflight.issues = true;
        cmds.push(Cmd::Job(Job::FetchIssues));
    }
    if !state.inflight.prs {
        state.inflight.prs = true;
        cmds.push(Cmd::Job(Job::FetchPrs));
    }
}

pub(super) fn observed(state: &mut State, result: Result<Sessions, SessionError>) -> Vec<Cmd> {
    state.inflight.sessions = false;
    match result {
        Ok(sessions) => {
            if state.sessions.iter().any(|(n, s)| went_idle(s.state, sessions.get(n).map(|s| s.state))) {
                state.due.github_soon = true;
            }
            state.sessions = sessions;
        }
        Err(e) => say(state, format!("sessions: {e}")),
    }
    Vec::new()
}
