use super::State;
use super::feed::{observe_now, refresh_github};
use super::list::{RowKey, lead_pr, pr_for_session, selected_row};
use super::msg::{Cmd, Job, SendKind, WriteJob};
use super::remove::short_refusal;
use super::sessions::goto_session;
use super::toast;
use crate::config::RemoveOnMerge;
use crate::domain::{
    BranchName, BranchNameError, CommitOid, ForkHead, ForkPush, IssueNumber, MergeMethod, Mode, PrBranch, PrNumber,
    PrState, PullRequest, SessionKey,
};
use crate::merge::job::{DraftStep, MergeOutcome, MergeRun};
use crate::merge::plan::{self, MergePlan, MergeProblem};
use crate::merge::{Consent, MergeAsk, MergeError};
use crate::session::error::SessionError;
use crate::session::request::{Launch, StartFrom, StartRequest};
use crate::session::{RemoveOutcome, RemoveTarget, SessionState};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// The next `m` checks the PR again, then merges it or has it fixed.
    Merge(MergeAsk),
    /// The check found a problem and no running session is on the PR's
    /// branch. The next `m` starts `req`, a session that fixes it.
    Fix {
        pr: PrNumber,
        method: MergeMethod,
        req: StartRequest,
    },
    /// The next `D` removes the worktree and branch of this issue's session.
    Remove(SessionKey),
    Clean(Vec<SessionKey>),
}

/// Who fixes a PR that can not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fixer {
    /// A running session on the PR's branch. It gets a prompt.
    Agent(SessionKey),
    /// A new session for this issue or PR, on the PR's branch.
    NewSession(SessionKey),
}

/// Why `m` has no PR to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoTarget {
    NothingSelected,
    NotListed(PrNumber),
    NoPr(IssueNumber),
    NotOpen(PrNumber),
}

impl fmt::Display for NoTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoTarget::NothingSelected => f.write_str("Nothing is selected."),
            NoTarget::NotListed(n) => write!(f, "PR {n} is not in the list."),
            NoTarget::NoPr(n) => write!(f, "{n} has no PR yet."),
            NoTarget::NotOpen(n) => write!(f, "PR {n} is not open."),
        }
    }
}

/// Why no session can fix a PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CannotFix {
    /// The PR closes no issue, and its own session key is taken.
    PrBusy(PrNumber),
    EachIssueBusy(PrNumber),
    UnusableHead {
        pr: PrNumber,
        raw: String,
        why: BranchNameError,
    },
    ForkDeniesEdits(PrNumber),
}

impl fmt::Display for CannotFix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CannotFix::PrBusy(n) => write!(f, "PR {n} already has a session on another branch, or one is starting."),
            CannotFix::EachIssueBusy(n) => {
                write!(f, "Each issue that PR {n} closes already has a session on another branch.")
            }
            CannotFix::UnusableHead { pr, raw, why } => write!(f, "PR {pr} has the branch name {raw:?}: {why:?}"),
            CannotFix::ForkDeniesEdits(n) => write!(
                f,
                "PR {n} is from a fork that does not let maintainers push to it, so a session can not push a fix."
            ),
        }
    }
}

/// The PR `m` acts on: the selected PR, or the PR of the selected issue or
/// session. No session is needed.
pub(super) fn merge_target(state: &State) -> Result<&PullRequest, NoTarget> {
    let (pr, missing) = match selected_row(state).ok_or(NoTarget::NothingSelected)? {
        RowKey::Pr(n) => (state.prs.items.iter().find(|p| p.number == n), NoTarget::NotListed(n)),
        RowKey::Issue(n) => (
            state.sessions.get(&n.into()).and_then(|s| pr_for_session(state, s)).or_else(|| lead_pr(state, n)),
            NoTarget::NoPr(n),
        ),
        RowKey::Session(SessionKey::Issue(n)) => {
            (state.sessions.get(&n.into()).and_then(|s| pr_for_session(state, s)), NoTarget::NoPr(n))
        }
        RowKey::Session(SessionKey::Pr(n)) => (state.prs.items.iter().find(|p| p.number == n), NoTarget::NotListed(n)),
    };
    let pr = pr.ok_or(missing)?;
    match pr.state {
        PrState::Open | PrState::Draft => Ok(pr),
        PrState::Merged | PrState::Closed => Err(NoTarget::NotOpen(pr.number)),
    }
}

/// A running session on the PR's branch fixes it. Else a new session does:
/// for the key of an exited session on that branch, or else for the first
/// issue the PR closes that has no session yet, or else for the PR itself
/// when it closes no issue.
pub(super) fn fixer(state: &State, pr: &PullRequest) -> Result<Fixer, CannotFix> {
    if let Some(s) = state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)) {
        return Ok(if s.state.is_live() { Fixer::Agent(s.key) } else { Fixer::NewSession(s.key) });
    }
    head_start(pr)?;
    let n = pr.number;
    let free = |key: &SessionKey| !state.sessions.contains_key(key) && !state.starting.contains(key);
    if pr.closes.is_empty() {
        let key = SessionKey::Pr(n);
        return if free(&key) { Ok(Fixer::NewSession(key)) } else { Err(CannotFix::PrBusy(n)) };
    }
    pr.closes.iter().map(|&i| SessionKey::Issue(i)).find(free).map(Fixer::NewSession).ok_or(CannotFix::EachIssueBusy(n))
}

/// `m`: the first press arms, the next press on the same PR acts. Merging
/// is hard to undo and a new session is a big step, so one stray key never
/// does either. The first press says what the PR list expects; the job
/// checks GitHub again before it acts.
pub(super) fn merge_key(state: &mut State, armed: Option<&Armed>) -> Vec<Cmd> {
    let pr = match merge_target(state) {
        Ok(pr) => pr,
        Err(why) => {
            toast::info(state, why.to_string());
            return Vec::new();
        }
    };
    let n = pr.number;
    if let Some(Armed::Fix { pr, method, req }) = armed
        && *pr == n
    {
        state.handed_off.insert(n, *method);
        return start_fix(state, req.clone());
    }
    let (consent, expect) = match (plan::merge_plan(pr), fixer(state, pr)) {
        (MergePlan::Handoff(problem), fix) => {
            let sentence = plan::problem_sentence(n, pr.base.display(), problem);
            match fix {
                Ok(Fixer::Agent(i)) => {
                    (Consent::MergeOnly, format!("{sentence} Press m again to ask {i}'s agent to fix and merge it."))
                }
                Ok(Fixer::NewSession(i)) => (
                    Consent::StartFixSession,
                    format!("{sentence} Press m again to start a session for {i} that fixes it."),
                ),
                Err(why) => {
                    toast::info(state, format!("{sentence} {why}"));
                    return Vec::new();
                }
            }
        }
        // A cached refusal still arms, because the list can be stale and the
        // job asks GitHub again.
        (MergePlan::Direct | MergePlan::Refuse(_), _) => (Consent::MergeOnly, format!("Press m again to merge PR {n}")),
        (MergePlan::MarkReady, _) => {
            (Consent::MarkReadyThenMerge, format!("PR {n} is a draft. Press m again to mark it ready and merge it."))
        }
    };
    let ask = MergeAsk { pr: n, consent };
    if armed != Some(&Armed::Merge(ask)) {
        state.armed = Some(Armed::Merge(ask));
        toast::info(state, expect);
        return Vec::new();
    }
    toast::info(state, format!("Checking PR {n}"));
    vec![Cmd::Job(Job::Write(WriteJob::Merge(ask)))]
}

pub(super) fn start_fix(state: &mut State, req: StartRequest) -> Vec<Cmd> {
    let n = req.key;
    state.starting.insert(n);
    goto_session(state, n);
    vec![Cmd::Job(Job::Write(WriteJob::Start(req, Launch::from_config(&state.config))))]
}

pub(super) fn fix_request(
    key: SessionKey,
    pr: &PullRequest,
    problem: MergeProblem,
    method: MergeMethod,
) -> Result<StartRequest, CannotFix> {
    let (branch, from) = head_start(pr)?;
    let (n, base) = (pr.number, pr.base.display());
    let prompt = match &from {
        StartFrom::Fork { repo, .. } => plan::fork_fix_session_prompt(n, repo, base, problem, method),
        StartFrom::Base | StartFrom::Origin => plan::fix_session_prompt(n, base, problem, method),
    };
    Ok(StartRequest { key, branch, mode: Mode::Implement, prompt, from })
}

/// The branch a session on the PR's head uses, and where that branch starts.
pub(super) fn head_start(pr: &PullRequest) -> Result<(BranchName, StartFrom), CannotFix> {
    match &pr.head {
        PrBranch::Valid(name) => Ok((name.clone(), StartFrom::Origin)),
        PrBranch::Fork(ForkHead { branch, local, push: ForkPush::Allowed(repo) }) => {
            Ok((local.clone(), StartFrom::Fork { repo: repo.clone(), branch: branch.clone() }))
        }
        PrBranch::Fork(ForkHead { push: ForkPush::Denied, .. }) => Err(CannotFix::ForkDeniesEdits(pr.number)),
        PrBranch::Unusable { raw, why } => {
            Err(CannotFix::UnusableHead { pr: pr.number, raw: raw.clone(), why: why.clone() })
        }
    }
}

pub(super) fn merged(state: &mut State, ask: MergeAsk, run: MergeRun) -> Vec<Cmd> {
    let ended_ready = run.draft == DraftStep::MarkedReady
        && !matches!(run.result, Ok(MergeOutcome::Merged { .. } | MergeOutcome::Queued));
    let refused = matches!(run.result, Ok(MergeOutcome::Refused(_)));
    let mut cmds = merge_result(state, ask, run.result);
    if ended_ready {
        // dispatch does not undo `gh pr ready`, so the toast must say it ran.
        if let Some(toast) = &mut state.toast {
            toast.append(&format!("PR {} is now ready for review.", ask.pr));
        }
    }
    if ended_ready || refused {
        refresh_github(state, &mut cmds);
    }
    cmds
}

fn closed_phrase(key: SessionKey, session: SessionState) -> String {
    match session {
        SessionState::Running | SessionState::NeedsYou => format!("stopped {key}'s session"),
        SessionState::Exited => format!("closed {key}'s window"),
    }
}

pub(super) fn merge_result(state: &mut State, ask: MergeAsk, result: Result<MergeOutcome, MergeError>) -> Vec<Cmd> {
    match result {
        Ok(MergeOutcome::Merged { method, head_oid }) => {
            state.handed_off.remove(&ask.pr);
            let mut cmds = after_merge(state, ask.pr, method, head_oid);
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Queued) => {
            toast::info(state, format!("PR {} is in the merge queue", ask.pr));
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Handoff { problem, method, pr }) => handoff(state, ask, &pr, problem, method),
        Ok(MergeOutcome::Refused(why)) => {
            toast::info(state, why.to_string());
            Vec::new()
        }
        Err(e) => {
            toast::error(state, format!("Merge PR {} failed: {e}", ask.pr));
            Vec::new()
        }
    }
}

/// Stops or removes the session on `pr`'s branch, as `remove_on_merge` says.
fn after_merge(state: &mut State, pr: PrNumber, method: MergeMethod, head_oid: CommitOid) -> Vec<Cmd> {
    match session_on_pr(state, pr) {
        Some((key, session)) if state.config.remove_on_merge == RemoveOnMerge::On => {
            toast::info(state, format!("Merged PR {pr} ({method}), removing {key}"));
            let target = RemoveTarget { key, merged_head: Some(head_oid) };
            vec![Cmd::Job(Job::Write(WriteJob::RemoveAfterMerge { pr, method, session, target }))]
        }
        Some((key, session)) => {
            toast::info(state, format!("Merged PR {pr} ({method}) and {}", closed_phrase(key, session)));
            vec![Cmd::Job(Job::Write(WriteJob::Stop(key)))]
        }
        None => {
            toast::info(state, format!("Merged PR {pr} ({method})"));
            Vec::new()
        }
    }
}

/// After a PR refresh: each handed-off PR that is now merged gets the stop
/// or remove of a direct merge, once. A closed one is dropped.
pub(super) fn handed_off_merged(state: &mut State) -> Vec<Cmd> {
    let done: Vec<(PrNumber, PrState, CommitOid)> = state
        .prs
        .items
        .iter()
        .filter(|p| state.handed_off.contains_key(&p.number) && matches!(p.state, PrState::Merged | PrState::Closed))
        .map(|p| (p.number, p.state, p.head_oid.clone()))
        .collect();
    let mut cmds = Vec::new();
    for (pr, pr_state, head_oid) in done {
        if let Some(method) = state.handed_off.remove(&pr)
            && pr_state == PrState::Merged
        {
            cmds.extend(after_merge(state, pr, method, head_oid));
        }
    }
    cmds
}

pub(super) fn removed_after_merge(
    state: &mut State,
    pr: PrNumber,
    method: MergeMethod,
    key: SessionKey,
    session: SessionState,
    result: Result<RemoveOutcome, SessionError>,
) -> Vec<Cmd> {
    match result {
        Ok(RemoveOutcome::Removed) => {
            toast::info(state, format!("Merged PR {pr} ({method}) and removed {key}'s worktree and branch"));
        }
        Ok(RemoveOutcome::Refused(why)) => {
            toast::info(
                state,
                format!(
                    "Merged PR {pr} ({method}) and {}. Kept worktree ({}).",
                    closed_phrase(key, session),
                    short_refusal(why)
                ),
            );
        }
        Err(e) => {
            toast::error(state, format!("Merged PR {pr} ({method}), but removing {key} failed: {e}"));
        }
    }
    observe_now(state)
}

/// The session on the PR's head branch, live or exited. A merged PR needs no
/// agent, so the merge stops it like `x` does. An exited session still gets
/// the stop, because its terminal or space can outlive Claude.
fn session_on_pr(state: &State, pr: PrNumber) -> Option<(SessionKey, SessionState)> {
    let head = state.prs.items.iter().find(|p| p.number == pr)?.head.branch()?;
    state.sessions.values().find(|s| &s.branch == head).map(|s| (s.key, s.state))
}

/// The check found `problem` on `pr`, the PR as GitHub showed it now.
pub(super) fn handoff(
    state: &mut State,
    ask: MergeAsk,
    pr: &PullRequest,
    problem: MergeProblem,
    method: MergeMethod,
) -> Vec<Cmd> {
    let sentence = plan::problem_sentence(pr.number, pr.base.display(), problem);
    let key = match fixer(state, pr) {
        Ok(Fixer::Agent(i)) => {
            toast::info(state, format!("{sentence} Asking {i}'s agent to fix it."));
            state.handed_off.insert(pr.number, method);
            return vec![Cmd::Job(Job::Write(WriteJob::Send {
                key: i,
                kind: SendKind::MergeFix,
                text: plan::handoff_prompt(pr.number, pr.base.display(), problem, method),
            }))];
        }
        Ok(Fixer::NewSession(i)) => i,
        Err(why) => {
            toast::info(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    let req = match fix_request(key, pr, problem, method) {
        Ok(req) => req,
        Err(why) => {
            toast::info(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    if ask.consent == Consent::StartFixSession {
        toast::info(state, format!("{sentence} Starting a session for {key} to fix it."));
        state.handed_off.insert(pr.number, method);
        return start_fix(state, req);
    }
    state.armed = Some(Armed::Fix { pr: pr.number, method, req });
    toast::info(state, format!("{sentence} Press m again to start a session for {key} that fixes it."));
    Vec::new()
}
