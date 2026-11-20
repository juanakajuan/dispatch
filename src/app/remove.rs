use super::State;
use super::feed::observe_now;
use super::list::{current_session, pr_for_session};
use super::merge::Armed;
use super::msg::{Cmd, Job, WriteJob};
use super::toast;
use crate::domain::{CommitOid, IssueNumber, PrState};
use crate::session::error::SessionError;
use crate::session::{CleanReport, RemoveOutcome, RemoveRefusal, RemoveTarget, Session, SessionState};

/// Safe to remove: Claude is not working in it, or its PR is merged, so the
/// work is done and the job closes the window.
fn is_removable(state: &State, session: &Session) -> bool {
    session.state == SessionState::Exited || merged_pr(state, session).is_some()
}

/// The selected session can be removed. Drives the key bar entry.
pub(super) fn removable(state: &State) -> bool {
    current_session(state).is_some_and(|s| is_removable(state, s))
}

fn merged_pr<'a>(state: &'a State, session: &Session) -> Option<&'a crate::domain::PullRequest> {
    pr_for_session(state, session).filter(|pr| pr.state == PrState::Merged)
}

fn merged_head(state: &State, session: &Session) -> Option<CommitOid> {
    merged_pr(state, session).map(|pr| pr.head_oid.clone())
}

fn remove_target(state: &State, session: &Session) -> RemoveTarget {
    RemoveTarget { issue: session.issue, merged_head: merged_head(state, session) }
}

fn merged_targets(state: &State) -> Vec<RemoveTarget> {
    state
        .sessions
        .values()
        .filter_map(|s| merged_head(state, s).map(|head| RemoveTarget { issue: s.issue, merged_head: Some(head) }))
        .collect()
}

pub(super) fn has_merged_sessions(state: &State) -> bool {
    state.sessions.values().any(|s| merged_pr(state, s).is_some())
}

fn merged_sessions_count(n: usize) -> String {
    if n == 1 { "1 merged session".to_string() } else { format!("{n} merged sessions") }
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
    let target = remove_target(state, session);
    toast::info(state, format!("Removing {issue}"));
    vec![Cmd::Job(Job::Write(WriteJob::Remove(target)))]
}

pub(super) fn clean_key(state: &mut State, armed: Option<&Armed>) -> Vec<Cmd> {
    let targets = merged_targets(state);
    if targets.is_empty() {
        toast::info(state, "No merged sessions to clean.".to_string());
        return Vec::new();
    }
    let issues: Vec<IssueNumber> = targets.iter().map(|t| t.issue).collect();
    let label = merged_sessions_count(issues.len());
    if !matches!(armed, Some(Armed::Clean(a)) if *a == issues) {
        state.armed = Some(Armed::Clean(issues));
        toast::info(state, format!("Press c again to remove {label}"));
        return Vec::new();
    }
    toast::info(state, format!("Removing {label}"));
    vec![Cmd::Job(Job::Write(WriteJob::Clean(targets)))]
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

pub(super) fn short_refusal(why: RemoveRefusal) -> String {
    match why {
        RemoveRefusal::Uncommitted => "uncommitted changes".to_string(),
        RemoveRefusal::Unpushed(n) if n.get() == 1 => "1 unpushed commit".to_string(),
        RemoveRefusal::Unpushed(n) => format!("{n} unpushed commits"),
    }
}

fn format_clean_report(report: &CleanReport) -> String {
    let skipped_details =
        report.skipped.iter().map(|(i, why)| format!("{i} ({})", short_refusal(*why))).collect::<Vec<_>>().join(", ");
    let failed_details = report.failed.iter().map(|(i, e)| format!("{i} ({e})")).collect::<Vec<_>>().join(", ");
    match (report.removed.len(), report.skipped.len(), report.failed.len()) {
        (0, 0, 0) => "No merged sessions to clean.".to_string(),
        (r, 0, 0) => format!("Removed {}", merged_sessions_count(r)),
        (0, s, 0) => format!("Skipped {}: {skipped_details}.", merged_sessions_count(s)),
        (0, 0, f) => format!("Failed to remove {}: {failed_details}.", merged_sessions_count(f)),
        (r, s, f) => {
            let mut parts = Vec::new();
            if r > 0 {
                parts.push(format!("Removed {}", merged_sessions_count(r)));
            }
            if s > 0 {
                parts.push(format!("Skipped {s}: {skipped_details}"));
            }
            if f > 0 {
                parts.push(format!("Failed {f}: {failed_details}"));
            }
            format!("{}.", parts.join(". "))
        }
    }
}

pub(super) fn cleaned(state: &mut State, report: &CleanReport) -> Vec<Cmd> {
    let msg = format_clean_report(report);
    if report.failed.is_empty() {
        toast::info(state, msg);
    } else {
        toast::error(state, msg);
    }
    observe_now(state)
}
