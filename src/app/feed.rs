use super::State;
use super::msg::{Cmd, GITHUB_COOLDOWN, Job, Observation, ReadJob, SESSIONS_EVERY};
use super::toast;
use crate::github::GhError;
use crate::session::SessionState;
use crate::session::error::SessionError;
use std::time::Instant;

/// Remote data plus its last fetch. `items` keeps the last good data when a
/// refresh fails, so a network error does not blank the list.
pub struct Feed<T> {
    pub items: Vec<T>,
    pub last: LastFetch,
    pub poll: Poll,
}

impl<T> Feed<T> {
    pub fn new() -> Self {
        Feed { items: Vec::new(), last: LastFetch::Never, poll: Poll::default() }
    }

    /// Records a fetch result and ends the poll that asked for it.
    pub(super) fn landed(&mut self, result: Result<Vec<T>, GhError>, at: Instant) {
        self.poll.done();
        match result {
            Ok(items) => {
                self.items = items;
                self.last = LastFetch::Fresh { at };
            }
            Err(error) => self.last = LastFetch::Failed { error, at },
        }
    }
}

/// `at` is when the response arrived, not when the request was sent.
/// `Never` is what the view shows as loading.
pub enum LastFetch {
    Never,
    Fresh { at: Instant },
    Failed { error: GhError, at: Instant },
}

/// At most one job of a kind in flight, so a slow `gh` never stacks up a
/// second call. `request` is the only way to send a poll job.
#[derive(Default)]
pub struct Poll {
    in_flight: bool,
}

impl Poll {
    /// The job to send, or `None` while one is already in flight.
    pub fn request(&mut self, job: ReadJob) -> Option<Cmd> {
        if self.in_flight {
            return None;
        }
        self.in_flight = true;
        Some(Cmd::Job(Job::Read(job)))
    }

    pub fn done(&mut self) {
        self.in_flight = false;
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }
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
    state.observe.request(ReadJob::ObserveSessions).into_iter().collect()
}

pub(super) fn tick(state: &mut State, now: Instant) -> Vec<Cmd> {
    toast::expire(state, now);
    let mut cmds = Vec::new();
    let idle = !state.issues.poll.in_flight() && !state.prs.poll.in_flight();
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
    cmds.extend(state.issues.poll.request(ReadJob::FetchIssues));
    cmds.extend(state.prs.poll.request(ReadJob::FetchPrs));
}

pub(super) fn observed(state: &mut State, result: Result<Observation, SessionError>) -> Vec<Cmd> {
    state.observe.done();
    match result {
        Ok(Observation { sessions, transcripts }) => {
            if state.sessions.iter().any(|(n, s)| went_idle(s.state, sessions.get(n).map(|s| s.state))) {
                state.due.github_soon = true;
            }
            state.sessions = sessions;
            state.transcripts = transcripts;
        }
        Err(e) => toast::error(state, format!("sessions: {e}")),
    }
    Vec::new()
}
