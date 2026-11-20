//! A PR that `m` hands to an agent gets merged by the agent, not by
//! dispatch. The PR refresh that first shows it merged runs the same
//! stop or remove as a direct merge.

use super::*;
use crate::config::RemoveOnMerge;
use crate::domain::CommitOid;
use crate::merge::job::DraftStep;
use crate::merge::{Consent, MergeAsk};
use crate::session::RemoveTarget;

const ASK: MergeAsk = MergeAsk { pr: PrNumber(10), consent: Consent::MergeOnly };

fn conflict(ask: MergeAsk) -> Msg {
    let mut fresh = pr(10, "fix/3-x", &[3]);
    fresh.mergeability = Mergeability::Conflict;
    Msg::Merged {
        ask,
        run: MergeRun {
            draft: DraftStep::WasReady,
            result: Ok(MergeOutcome::Handoff {
                problem: MergeProblem::Conflict,
                method: MergeMethod::Rebase,
                pr: Box::new(fresh),
            }),
        },
    }
}

/// The PR list with #10 merged at `sha`.
fn merged_list(state: &State, sha: &str) -> Msg {
    let mut prs = state.prs.items.clone();
    prs[0].state = PrState::Merged;
    prs[0].head_oid = CommitOid::new(sha.to_string());
    Msg::Prs { result: Ok(prs), at: Instant::now() }
}

fn after_merge(cmds: &[Cmd]) -> Vec<&WriteJob> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(w @ (WriteJob::Stop(_) | WriteJob::RemoveAfterMerge { .. }))) => Some(w),
            _ => None,
        })
        .collect()
}

fn agent_on_conflict(remove: RemoveOnMerge) -> State {
    let (mut state, _) = app();
    state.config.remove_on_merge = remove;
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, conflict(ASK));
    state
}

#[test]
fn a_pr_the_agent_merged_removes_its_session_once() {
    let mut state = agent_on_conflict(RemoveOnMerge::On);
    let msg = merged_list(&state, "agent-sha");
    let cmds = press(&mut state, msg);
    assert!(matches!(
        after_merge(&cmds)[..],
        [WriteJob::RemoveAfterMerge {
            pr: PrNumber(10),
            method: MergeMethod::Rebase,
            session: SessionState::Running,
            target: RemoveTarget { key, merged_head: Some(oid) },
        }] if *key == issue_key(3) && oid.as_str() == "agent-sha"
    ));
    assert_eq!(toast(&state), "Merged PR #10 (rebase), removing #3");
    let msg = merged_list(&state, "agent-sha");
    assert!(after_merge(&press(&mut state, msg)).is_empty(), "the next refresh does not remove again");
}

#[test]
fn a_pr_the_agent_merged_stops_its_session_when_remove_on_merge_is_off() {
    let mut state = agent_on_conflict(RemoveOnMerge::Off);
    let msg = merged_list(&state, "agent-sha");
    let cmds = press(&mut state, msg);
    assert!(matches!(after_merge(&cmds)[..], [WriteJob::Stop(k)] if *k == issue_key(3)));
    assert_eq!(toast(&state), "Merged PR #10 (rebase) and stopped #3's session");
}

#[test]
fn a_fix_session_that_merged_its_pr_is_removed() {
    let (mut state, _) = app();
    state.config.remove_on_merge = RemoveOnMerge::On;
    press(&mut state, conflict(ASK));
    press(&mut state, key('m'));
    state.starting.remove(&issue_key(3));
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let msg = merged_list(&state, "fix-sha");
    let cmds = press(&mut state, msg);
    assert!(matches!(after_merge(&cmds)[..], [WriteJob::RemoveAfterMerge { .. }]));
}

#[test]
fn a_pr_merged_without_a_handoff_leaves_its_session() {
    let (mut state, _) = app();
    state.config.remove_on_merge = RemoveOnMerge::On;
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let msg = merged_list(&state, "web-sha");
    assert!(after_merge(&press(&mut state, msg)).is_empty());
}
