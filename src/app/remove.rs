use super::State;
use super::feed::observe_now;
use super::list::{current_session, pr_for_session};
use super::merge::Armed;
use super::msg::{Cmd, Job, WriteJob};
use super::toast;
use crate::domain::{IssueNumber, PrState};
use crate::session::error::SessionError;
use crate::session::{RemoveOutcome, RemoveRefusal, Session, SessionState};

/// Safe to remove: Claude is not working in it, or its PR is merged, so the
/// work is done and the job closes the window.
fn is_removable(state: &State, session: &Session) -> bool {
    session.state == SessionState::Exited
        || pr_for_session(state, session).is_some_and(|pr| pr.state == PrState::Merged)
}

/// The selected session can be removed. Drives the key bar entry.
pub(super) fn removable(state: &State) -> bool {
    current_session(state).is_some_and(|s| is_removable(state, s))
}

/// `D`: the first press arms, the next press on the same issue removes. The
/// job checks the worktree again before it removes anything.
pub(super) fn remove_key(state: &mut State, armed: Option<&Armed>) -> Vec<Cmd> {
    let Some(session) = current_session(state) else {
        toast::info(state, "No session here.".to_string());
        return Vec::new();
    };
    let issue = session.issue;
    if !is_removable(state, session) {
        toast::info(state, format!("{issue} is still running. Press x to stop it first."));
        return Vec::new();
    }
    if armed != Some(&Armed::Remove(issue)) {
        state.armed = Some(Armed::Remove(issue));
        toast::info(state, format!("Press D again to remove {issue}'s worktree and branch"));
        return Vec::new();
    }
    toast::info(state, format!("Removing {issue}"));
    vec![Cmd::Job(Job::Write(WriteJob::Remove(issue)))]
}

fn refusal_sentence(issue: IssueNumber, why: RemoveRefusal) -> String {
    match why {
        RemoveRefusal::Uncommitted => {
            format!("{issue} has changes that are not committed. Commit or drop them, then press D again.")
        }
        RemoveRefusal::Unpushed(n) if n.get() == 1 => {
            format!("{issue} has 1 commit that is not pushed. Push it, then press D again.")
        }
        RemoveRefusal::Unpushed(n) => {
            format!("{issue} has {n} commits that are not pushed. Push them, then press D again.")
        }
    }
}

pub(super) fn removed(state: &mut State, issue: IssueNumber, result: Result<RemoveOutcome, SessionError>) -> Vec<Cmd> {
    match result {
        Ok(RemoveOutcome::Removed) => toast::info(state, format!("Removed {issue}'s worktree and branch")),
        Ok(RemoveOutcome::Refused(why)) => toast::info(state, refusal_sentence(issue, why)),
        Err(e) => toast::error(state, format!("Remove {issue} failed: {e}")),
    }
    observe_now(state)
}
