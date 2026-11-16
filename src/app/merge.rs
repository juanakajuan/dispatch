use super::State;
use super::feed::refresh_github;
use super::list::{RowKey, lead_pr, pr_for_session, selected_row};
use super::msg::{Cmd, Job};
use super::sessions::goto_session;
use super::toast::say;
use crate::domain::{IssueNumber, MergeMethod, Mode, PrBranch, PrNumber, PrState, PullRequest};
use crate::merge::MergeError;
use crate::merge::job::{MergeOutcome, MergeRun};
use crate::merge::plan::{self, MergePlan, MergeProblem};
use crate::session::request::{Launch, StartFrom, StartRequest};
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// The next `m` checks the PR again, then merges it or has it fixed.
    Merge(MergeAsk),
    /// The check found a problem and no running session is on the PR's
    /// branch. The next `m` starts `req`, a session that fixes it.
    Fix { pr: PrNumber, req: StartRequest },
}

/// Merge `pr`. If it has a problem, a running session on its branch fixes it.
/// Without one, a new session fixes it only when `new_session_ok`, because
/// the first `m` told the user so. Otherwise one more `m` must agree. A draft
/// is marked ready only when `ready_ok`, for the same reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeAsk {
    pub pr: PrNumber,
    pub new_session_ok: bool,
    pub ready_ok: bool,
}

/// Who fixes a PR that can not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fixer {
    /// A running session on the PR's branch. It gets a prompt.
    Agent(IssueNumber),
    /// A new session for this issue, on the PR's branch.
    NewSession(IssueNumber),
}

/// The PR `m` acts on: the selected PR, or the PR of the selected issue or
/// session. No session is needed.
pub(super) fn merge_target(state: &State) -> Result<&PullRequest, String> {
    let (pr, missing) = match selected_row(state).ok_or("Nothing is selected.")? {
        RowKey::Pr(n) => (state.prs.items.iter().find(|p| p.number == n), format!("PR {n} is not in the list.")),
        RowKey::Issue(n) => (
            state.sessions.get(&n).and_then(|s| pr_for_session(state, s)).or_else(|| lead_pr(state, n)),
            format!("{n} has no PR yet."),
        ),
        RowKey::Session(n) => {
            (state.sessions.get(&n).and_then(|s| pr_for_session(state, s)), format!("{n} has no PR yet."))
        }
    };
    let pr = pr.ok_or(missing)?;
    match pr.state {
        PrState::Open | PrState::Draft => Ok(pr),
        PrState::Merged | PrState::Closed => Err(format!("PR {} is not open.", pr.number)),
    }
}

/// A running session on the PR's branch fixes it. Else a new session does:
/// for the issue of an exited session on that branch, or else for the first
/// issue the PR closes that has no session yet.
pub(super) fn fixer(state: &State, pr: &PullRequest) -> Result<Fixer, String> {
    if let Some(s) = state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)) {
        return Ok(if s.state.is_live() { Fixer::Agent(s.issue) } else { Fixer::NewSession(s.issue) });
    }
    let n = pr.number;
    if pr.closes.is_empty() {
        return Err(format!("PR {n} closes no issue, so dispatch can not start a session for it."));
    }
    pr.closes
        .iter()
        .find(|i| !state.sessions.contains_key(i) && !state.starting.contains(i))
        .map(|&i| Fixer::NewSession(i))
        .ok_or_else(|| format!("Each issue that PR {n} closes already has a session on another branch."))
}

/// `m`: the first press arms, the next press on the same PR acts. Merging
/// is hard to undo and a new session is a big step, so one stray key never
/// does either. The first press says what the PR list expects; the job
/// checks GitHub again before it acts.
pub(super) fn merge_key(state: &mut State, armed: Option<&Armed>) -> Vec<Cmd> {
    let pr = match merge_target(state) {
        Ok(pr) => pr,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    let n = pr.number;
    if let Some(Armed::Fix { pr, req }) = armed
        && *pr == n
    {
        return start_fix(state, req.clone());
    }
    let (new_session_ok, ready_ok, expect) = match (plan::merge_plan(pr), fixer(state, pr)) {
        (MergePlan::Handoff(problem), fix) => {
            let sentence = plan::problem_sentence(n, pr.base.display(), problem);
            match fix {
                Ok(Fixer::Agent(i)) => {
                    (false, false, format!("{sentence} Press m again to ask {i}'s agent to fix and merge it."))
                }
                Ok(Fixer::NewSession(i)) => {
                    (true, false, format!("{sentence} Press m again to start a session for {i} that fixes it."))
                }
                Err(why) => {
                    say(state, format!("{sentence} {why}"));
                    return Vec::new();
                }
            }
        }
        // A cached refusal still arms, because the list can be stale and the
        // job asks GitHub again.
        (MergePlan::Direct | MergePlan::Refuse(_), _) => (false, false, format!("Press m again to merge PR {n}")),
        (MergePlan::MarkReady, _) => {
            (false, true, format!("PR {n} is a draft. Press m again to mark it ready and merge it."))
        }
    };
    let ask = MergeAsk { pr: n, new_session_ok, ready_ok };
    if armed != Some(&Armed::Merge(ask)) {
        state.armed = Some(Armed::Merge(ask));
        say(state, expect);
        return Vec::new();
    }
    say(state, format!("Checking PR {n}"));
    vec![Cmd::Job(Job::Merge(ask))]
}

pub(super) fn start_fix(state: &mut State, req: StartRequest) -> Vec<Cmd> {
    let n = req.issue;
    state.starting.insert(n);
    goto_session(state, n);
    vec![Cmd::Job(Job::Start(req, Launch::from_config(&state.config)))]
}

pub(super) fn fix_request(
    issue: IssueNumber,
    pr: &PullRequest,
    problem: MergeProblem,
    method: MergeMethod,
) -> Result<StartRequest, String> {
    let branch = match &pr.head {
        PrBranch::Valid(name) => name.clone(),
        PrBranch::Unusable { raw, why } => {
            return Err(format!("PR {} has the branch name {raw:?}: {why}", pr.number));
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
    let ended_ready = run.marked_ready && !matches!(run.result, Ok(MergeOutcome::Merged(_) | MergeOutcome::Queued));
    let refused = matches!(run.result, Ok(MergeOutcome::Refused(_)));
    let mut cmds = merge_result(state, ask, run.result);
    if ended_ready {
        // dispatch does not undo `gh pr ready`, so the toast must say it ran.
        if let Some(toast) = &mut state.toast {
            let _ = write!(toast.text, " PR {} is now ready for review.", ask.pr);
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
                    say(state, format!("Merged PR {} ({method}) and stopped {issue}'s session", ask.pr));
                    cmds.push(Cmd::Job(Job::Stop(issue)));
                }
                None => say(state, format!("Merged PR {} ({method})", ask.pr)),
            }
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Queued) => {
            say(state, format!("PR {} is in the merge queue", ask.pr));
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Handoff { problem, method, pr }) => handoff(state, ask, &pr, problem, method),
        Ok(MergeOutcome::Refused(why)) => {
            say(state, why.to_string());
            Vec::new()
        }
        Err(e) => {
            say(state, format!("Merge PR {} failed: {e}", ask.pr));
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
            say(state, format!("{sentence} Asking {i}'s agent to fix it."));
            return vec![Cmd::Job(Job::Send {
                issue: i,
                text: plan::handoff_prompt(pr.number, pr.base.display(), problem, method),
            })];
        }
        Ok(Fixer::NewSession(i)) => i,
        Err(why) => {
            say(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    let req = match fix_request(issue, pr, problem, method) {
        Ok(req) => req,
        Err(why) => {
            say(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    if ask.new_session_ok {
        say(state, format!("{sentence} Starting a session for {issue} to fix it."));
        return start_fix(state, req);
    }
    state.armed = Some(Armed::Fix { pr: pr.number, req });
    say(state, format!("{sentence} Press m again to start a session for {issue} that fixes it."));
    Vec::new()
}
