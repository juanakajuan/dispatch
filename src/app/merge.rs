use super::State;
use super::feed::refresh_github;
use super::list::{RowKey, lead_pr, pr_for_session, selected_row};
use super::msg::{Cmd, Job, WriteJob};
use super::sessions::goto_session;
use super::toast;
use crate::domain::{IssueNumber, MergeMethod, Mode, PrBranch, PrNumber, PrState, PullRequest, Unusable};
use crate::merge::job::{DraftStep, MergeOutcome, MergeRun};
use crate::merge::plan::{self, MergePlan, MergeProblem};
use crate::merge::{Consent, MergeAsk, MergeError};
use crate::session::request::{Launch, StartFrom, StartRequest};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// The next `m` checks the PR again, then merges it or has it fixed.
    Merge(MergeAsk),
    /// The check found a problem and no running session is on the PR's
    /// branch. The next `m` starts `req`, a session that fixes it.
    Fix { pr: PrNumber, req: StartRequest },
}

/// Who fixes a PR that can not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fixer {
    /// A running session on the PR's branch. It gets a prompt.
    Agent(IssueNumber),
    /// A new session for this issue, on the PR's branch.
    NewSession(IssueNumber),
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
    ClosesNoIssue(PrNumber),
    EachIssueBusy(PrNumber),
    UnusableHead { pr: PrNumber, raw: String, why: Unusable },
}

impl fmt::Display for CannotFix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CannotFix::ClosesNoIssue(n) => {
                write!(f, "PR {n} closes no issue, so dispatch can not start a session for it.")
            }
            CannotFix::EachIssueBusy(n) => {
                write!(f, "Each issue that PR {n} closes already has a session on another branch.")
            }
            CannotFix::UnusableHead { pr, raw, why } => write!(f, "PR {pr} has the branch name {raw:?}: {why}"),
        }
    }
}

/// The PR `m` acts on: the selected PR, or the PR of the selected issue or
/// session. No session is needed.
pub(super) fn merge_target(state: &State) -> Result<&PullRequest, NoTarget> {
    let (pr, missing) = match selected_row(state).ok_or(NoTarget::NothingSelected)? {
        RowKey::Pr(n) => (state.prs.items.iter().find(|p| p.number == n), NoTarget::NotListed(n)),
        RowKey::Issue(n) => (
            state.sessions.get(&n).and_then(|s| pr_for_session(state, s)).or_else(|| lead_pr(state, n)),
            NoTarget::NoPr(n),
        ),
        RowKey::Session(n) => (state.sessions.get(&n).and_then(|s| pr_for_session(state, s)), NoTarget::NoPr(n)),
    };
    let pr = pr.ok_or(missing)?;
    match pr.state {
        PrState::Open | PrState::Draft => Ok(pr),
        PrState::Merged | PrState::Closed => Err(NoTarget::NotOpen(pr.number)),
    }
}

/// A running session on the PR's branch fixes it. Else a new session does:
/// for the issue of an exited session on that branch, or else for the first
/// issue the PR closes that has no session yet.
pub(super) fn fixer(state: &State, pr: &PullRequest) -> Result<Fixer, CannotFix> {
    if let Some(s) = state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)) {
        return Ok(if s.state.is_live() { Fixer::Agent(s.issue) } else { Fixer::NewSession(s.issue) });
    }
    let n = pr.number;
    if pr.closes.is_empty() {
        return Err(CannotFix::ClosesNoIssue(n));
    }
    pr.closes
        .iter()
        .find(|i| !state.sessions.contains_key(i) && !state.starting.contains(i))
        .map(|&i| Fixer::NewSession(i))
        .ok_or(CannotFix::EachIssueBusy(n))
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
    if let Some(Armed::Fix { pr, req }) = armed
        && *pr == n
    {
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
    let n = req.issue;
    state.starting.insert(n);
    goto_session(state, n);
    vec![Cmd::Job(Job::Write(WriteJob::Start(req, Launch::from_config(&state.config))))]
}

pub(super) fn fix_request(
    issue: IssueNumber,
    pr: &PullRequest,
    problem: MergeProblem,
    method: MergeMethod,
) -> Result<StartRequest, CannotFix> {
    let branch = match &pr.head {
        PrBranch::Valid(name) => name.clone(),
        PrBranch::Unusable { raw, why } => {
            return Err(CannotFix::UnusableHead { pr: pr.number, raw: raw.clone(), why: why.clone() });
        }
    };
    Ok(StartRequest {
        issue,
        branch,
        mode: Mode::Implement,
        prompt: plan::fix_session_prompt(pr.number, pr.base.display(), problem, method),
        from: StartFrom::Origin,
    })
}

pub(super) fn merged(state: &mut State, ask: MergeAsk, run: MergeRun) -> Vec<Cmd> {
    let ended_ready = run.draft == DraftStep::MarkedReady
        && !matches!(run.result, Ok(MergeOutcome::Merged(_) | MergeOutcome::Queued));
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

pub(super) fn merge_result(state: &mut State, ask: MergeAsk, result: Result<MergeOutcome, MergeError>) -> Vec<Cmd> {
    match result {
        Ok(MergeOutcome::Merged(method)) => {
            let mut cmds = Vec::new();
            match live_session_on_pr(state, ask.pr) {
                Some(issue) => {
                    toast::info(state, format!("Merged PR {} ({method}) and stopped {issue}'s session", ask.pr));
                    cmds.push(Cmd::Job(Job::Write(WriteJob::Stop(issue))));
                }
                None => toast::info(state, format!("Merged PR {} ({method})", ask.pr)),
            }
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

/// The issue of the session with a live window on the PR's head branch. A
/// merged PR needs no agent, so `merged` stops it like `x` does.
pub(super) fn live_session_on_pr(state: &State, pr: PrNumber) -> Option<IssueNumber> {
    let head = state.prs.items.iter().find(|p| p.number == pr)?.head.branch()?;
    state.sessions.values().find(|s| &s.branch == head && s.state.is_live()).map(|s| s.issue)
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
    let issue = match fixer(state, pr) {
        Ok(Fixer::Agent(i)) => {
            toast::info(state, format!("{sentence} Asking {i}'s agent to fix it."));
            return vec![Cmd::Job(Job::Write(WriteJob::Send {
                issue: i,
                text: plan::handoff_prompt(pr.number, pr.base.display(), problem, method),
            }))];
        }
        Ok(Fixer::NewSession(i)) => i,
        Err(why) => {
            toast::info(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    let req = match fix_request(issue, pr, problem, method) {
        Ok(req) => req,
        Err(why) => {
            toast::info(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    if ask.consent == Consent::StartFixSession {
        toast::info(state, format!("{sentence} Starting a session for {issue} to fix it."));
        return start_fix(state, req);
    }
    state.armed = Some(Armed::Fix { pr: pr.number, req });
    toast::info(state, format!("{sentence} Press m again to start a session for {issue} that fixes it."));
    Vec::new()
}
