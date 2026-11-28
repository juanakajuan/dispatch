use super::*;
use crate::app::toast::Level;
use crate::domain::CommitOid;
use crate::session::{CleanReport, RemoveOutcome, RemoveRefusal, RemoveTarget};
use std::num::NonZeroU32;

fn removes(cmds: &[Cmd]) -> Vec<RemoveTarget> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Remove(t))) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn cleans(cmds: &[Cmd]) -> Vec<Vec<RemoveTarget>> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Clean(targets))) => Some(targets.clone()),
            _ => None,
        })
        .collect()
}

fn on_session(s: SessionState) -> State {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, s));
    press(&mut state, key('3'));
    state
}

fn level(state: &State) -> Option<Level> {
    state.toast.as_ref().map(Toast::level)
}

fn target(n: u32, sha: Option<&str>) -> RemoveTarget {
    RemoveTarget { key: issue_key(n), merged_head: sha.map(|s| CommitOid::new(s.to_string())) }
}

fn merged_pr(number: u32, head: &str, closes: &[u32], sha: &str) -> PullRequest {
    PullRequest { state: PrState::Merged, head_oid: CommitOid::new(sha.to_string()), ..pr(number, head, closes) }
}

#[test]
fn d_twice_removes_an_exited_session() {
    let mut state = on_session(SessionState::Exited);
    assert!(shows(&state, "D", "remove"));
    assert!(press(&mut state, key('D')).is_empty(), "the first press only arms");
    assert_eq!(toast(&state), "Press D again to remove #3's worktree and branch.");
    assert_eq!(removes(&press(&mut state, key('D'))), [target(3, None)]);
    assert_eq!(toast(&state), "Removing #3…");
    assert!(state.armed.is_none());
}

#[test]
fn another_key_between_the_presses_disarms() {
    let mut state = on_session(SessionState::Exited);
    press(&mut state, key('D'));
    press(&mut state, key('j'));
    assert!(press(&mut state, key('D')).is_empty(), "armed again, not removed");
    assert_eq!(toast(&state), "Press D again to remove #3's worktree and branch.");
}

#[test]
fn a_press_after_the_selection_moved_to_another_issue_only_arms() {
    let mut state = on_session(SessionState::Exited);
    state.sessions.insert(issue_key(2), session(2, SessionState::Exited));
    state.tabs.sessions.selected = Some(issue_key(3));
    press(&mut state, key('D'));
    assert_eq!(state.armed, Some(Armed::Remove(issue_key(3))));
    state.tabs.sessions.selected = Some(issue_key(2));
    assert!(press(&mut state, key('D')).is_empty());
    assert_eq!(state.armed, Some(Armed::Remove(issue_key(2))));
}

#[test]
fn a_live_session_without_a_merged_pr_is_refused() {
    for live in [SessionState::Running, SessionState::NeedsYou] {
        let mut state = on_session(live);
        assert!(!shows(&state, "D", "remove"));
        assert!(press(&mut state, key('D')).is_empty());
        assert_eq!(toast(&state), "#3 is still running. Press x to stop it first.");
        assert!(state.armed.is_none());
    }
}

#[test]
fn a_live_session_whose_pr_merged_can_be_removed_with_merged_head() {
    let mut state = on_session(SessionState::Running);
    state.prs.items[0].state = PrState::Merged;
    assert!(shows(&state, "D", "remove"));
    press(&mut state, key('D'));
    assert_eq!(removes(&press(&mut state, key('D'))), [target(3, Some("sha10"))]);
}

#[test]
fn no_session_selected_says_so() {
    let (mut state, _) = app();
    assert!(!shows(&state, "D", "remove"));
    assert!(press(&mut state, key('D')).is_empty());
    assert_eq!(toast(&state), "No session for this issue. Press s to start one.");
}

#[test]
fn a_removed_result_toasts_and_asks_for_an_observation() {
    let (mut state, _) = app();
    let cmds = press(&mut state, Msg::Removed { key: issue_key(3), result: Ok(RemoveOutcome::Removed) });
    assert_eq!(toast(&state), "Removed #3's worktree and branch.");
    assert_eq!(level(&state), Some(Level::Info));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::ObserveSessions))]));
}

fn refused(why: RemoveRefusal) -> (String, Option<Level>) {
    let (mut state, _) = app();
    press(&mut state, Msg::Removed { key: issue_key(3), result: Ok(RemoveOutcome::Refused(why)) });
    (toast(&state), level(&state))
}

fn unpushed(n: u32) -> RemoveRefusal {
    RemoveRefusal::Unpushed(NonZeroU32::new(n).expect("non-zero"))
}

#[test]
fn refusals_name_the_problem_and_are_not_errors() {
    assert_eq!(
        refused(RemoveRefusal::Uncommitted),
        (
            "#3 has changes that are not committed. Commit or drop them, then press D again.".to_string(),
            Some(Level::Info)
        )
    );
    assert_eq!(
        refused(unpushed(1)),
        ("#3 has 1 commit that is not pushed. Push it, then press D again.".to_string(), Some(Level::Info))
    );
    assert_eq!(
        refused(unpushed(3)),
        ("#3 has 3 commits that are not pushed. Push them, then press D again.".to_string(), Some(Level::Info))
    );
}

#[test]
fn a_failed_remove_is_an_error_toast() {
    let (mut state, _) = app();
    press(&mut state, Msg::Removed { key: issue_key(3), result: Err(git_error("boom")) });
    assert_eq!(toast(&state), "Removing #3 failed: git failed with code 128: boom.");
    assert_eq!(level(&state), Some(Level::Error));
}

#[test]
fn clean_key_bar_shows_only_on_sessions_tab_when_a_merged_session_exists() {
    let mut state = on_session(SessionState::Exited);
    assert!(!shows(&state, "c", "clean merged"), "open PR does not show clean");

    state.prs.items[0].state = PrState::Merged;
    assert!(shows(&state, "c", "clean merged"), "shows on Sessions tab with merged session");

    press(&mut state, key('1'));
    assert!(!shows(&state, "c", "clean merged"), "hidden on Issues tab");
    press(&mut state, key('2'));
    assert!(!shows(&state, "c", "clean merged"), "hidden on PRs tab");
}

#[test]
fn c_with_no_merged_sessions_toasts_and_arms_nothing() {
    let mut state = on_session(SessionState::Exited);
    assert!(press(&mut state, key('c')).is_empty());
    assert_eq!(toast(&state), "No merged sessions to clean.");
    assert!(state.armed.is_none());
}

#[test]
fn c_arms_with_singular_and_plural_and_disarms_on_other_key() {
    let mut state = on_session(SessionState::Running);
    state.prs.items[0].state = PrState::Merged;
    assert!(press(&mut state, key('c')).is_empty());
    assert_eq!(toast(&state), "Press c again to remove 1 merged session.");
    assert_eq!(state.armed, Some(Armed::Clean(vec![issue_key(3)])));

    press(&mut state, key('j'));
    assert!(state.armed.is_none());

    state.sessions.insert(issue_key(2), session(2, SessionState::Exited));
    state.prs.items.push(merged_pr(12, "fix/2-x", &[2], "sha12"));
    assert!(press(&mut state, key('c')).is_empty());
    assert_eq!(toast(&state), "Press c again to remove 2 merged sessions.");
    assert_eq!(state.armed, Some(Armed::Clean(vec![issue_key(2), issue_key(3)])));
}

#[test]
fn c_rearms_if_merged_sessions_change_between_presses() {
    let mut state = on_session(SessionState::Running);
    state.prs.items[0].state = PrState::Merged;
    press(&mut state, key('c'));
    assert_eq!(state.armed, Some(Armed::Clean(vec![issue_key(3)])));

    state.sessions.insert(issue_key(2), session(2, SessionState::Exited));
    state.prs.items.push(merged_pr(12, "fix/2-x", &[2], "sha12"));

    assert!(press(&mut state, key('c')).is_empty(), "changed set re-arms instead of removing");
    assert_eq!(toast(&state), "Press c again to remove 2 merged sessions.");
    assert_eq!(state.armed, Some(Armed::Clean(vec![issue_key(2), issue_key(3)])));
}

#[test]
fn c_twice_emits_clean_job_in_issue_order_with_merged_heads() {
    let mut state = on_session(SessionState::Running);
    state.prs.items[0].state = PrState::Merged;
    state.sessions.insert(issue_key(1), session(1, SessionState::Exited));
    state.sessions.insert(issue_key(2), session(2, SessionState::NeedsYou));
    state.prs.items.push(merged_pr(12, "fix/2-x", &[2], "sha12"));
    state.prs.items.push(pr(13, "fix/1-x", &[1]));

    assert!(press(&mut state, key('c')).is_empty());
    assert_eq!(toast(&state), "Press c again to remove 2 merged sessions.");
    let cmds = press(&mut state, key('c'));
    assert_eq!(cleans(&cmds), [vec![target(2, Some("sha12")), target(3, Some("sha10"))]]);
    assert_eq!(toast(&state), "Removing 2 merged sessions…");
    assert!(state.armed.is_none());
}

#[test]
fn cleaned_reports_counts_and_levels_and_observes_sessions() {
    let check = |report: CleanReport, want_text: &str, want_level: Level| {
        let (mut state, _) = app();
        let cmds = press(&mut state, Msg::Cleaned(report));
        assert_eq!(toast(&state), want_text);
        assert_eq!(level(&state), Some(want_level));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::ObserveSessions))]));
    };

    check(CleanReport::default(), "No merged sessions to clean.", Level::Info);
    check(
        CleanReport { removed: vec![issue_key(3)], ..CleanReport::default() },
        "Removed 1 merged session.",
        Level::Info,
    );
    check(
        CleanReport { removed: vec![issue_key(2), issue_key(3)], ..CleanReport::default() },
        "Removed 2 merged sessions.",
        Level::Info,
    );
    check(
        CleanReport {
            removed: vec![issue_key(2)],
            skipped: vec![(issue_key(3), RemoveRefusal::Uncommitted), (issue_key(4), unpushed(1))],
            failed: Vec::new(),
        },
        "Removed 1 merged session. Skipped 2: #3 (uncommitted changes), #4 (1 unpushed commit).",
        Level::Info,
    );
    check(
        CleanReport { removed: Vec::new(), skipped: vec![(issue_key(3), unpushed(3))], failed: Vec::new() },
        "Skipped 1 merged session: #3 (3 unpushed commits).",
        Level::Info,
    );
    check(
        CleanReport { removed: Vec::new(), skipped: Vec::new(), failed: vec![(issue_key(3), git_error("boom"))] },
        "Removing 1 merged session failed: #3 (git failed with code 128: boom).",
        Level::Error,
    );
    check(
        CleanReport {
            removed: vec![issue_key(1)],
            skipped: vec![(issue_key(2), RemoveRefusal::Uncommitted)],
            failed: vec![(issue_key(3), git_error("boom"))],
        },
        "Removed 1 merged session. Skipped 1: #2 (uncommitted changes). Removing 1 failed: #3 (git failed with code 128: boom).",
        Level::Error,
    );
}

#[test]
fn remove_on_merge_emits_remove_after_merge_with_fresh_head_oid_for_session_on_pr() {
    use crate::config::RemoveOnMerge;
    use crate::merge::job::DraftStep;
    use crate::merge::{Consent, MergeAsk};

    let ask = MergeAsk { pr: PrNumber(10), consent: Consent::MergeOnly };
    let msg = || Msg::Merged {
        ask,
        run: MergeRun {
            draft: DraftStep::WasReady,
            result: Ok(MergeOutcome::Merged {
                method: MergeMethod::Squash,
                head_oid: CommitOid::new("fresh-sha".to_string()),
            }),
        },
    };
    for session_state in [SessionState::Running, SessionState::NeedsYou, SessionState::Exited] {
        let (mut state, _) = app();
        state.config.remove_on_merge = RemoveOnMerge::On;
        state.sessions.insert(issue_key(3), session(3, session_state));
        let cmds = press(&mut state, msg());
        assert_eq!(toast(&state), "Merged PR #10 (squash). Removing #3…");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Cmd::Job(Job::Write(WriteJob::RemoveAfterMerge {
                    pr: PrNumber(10),
                    method: MergeMethod::Squash,
                    session: s,
                    target: RemoveTarget { key: SessionKey::Issue(IssueNumber(3)), merged_head: Some(oid) },
                })) if *s == session_state && oid.as_str() == "fresh-sha"
            )),
            "missing RemoveAfterMerge for {session_state:?}"
        );
    }
}

#[test]
fn remove_on_merge_does_not_stop_or_remove_when_queued_or_no_session() {
    use crate::config::RemoveOnMerge;
    use crate::merge::job::DraftStep;
    use crate::merge::{Consent, MergeAsk};

    let ask = MergeAsk { pr: PrNumber(10), consent: Consent::MergeOnly };
    let has_stop_or_remove = |cmds: &[Cmd]| {
        cmds.iter().any(|c| {
            matches!(
                c,
                Cmd::Job(Job::Write(WriteJob::Stop(_) | WriteJob::Remove(_) | WriteJob::RemoveAfterMerge { .. }))
            )
        })
    };

    let (mut state, _) = app();
    state.config.remove_on_merge = RemoveOnMerge::On;
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let queued = press(
        &mut state,
        Msg::Merged { ask, run: MergeRun { draft: DraftStep::WasReady, result: Ok(MergeOutcome::Queued) } },
    );
    assert!(!has_stop_or_remove(&queued));

    let (mut state, _) = app();
    state.config.remove_on_merge = RemoveOnMerge::On;
    let no_session = press(
        &mut state,
        Msg::Merged {
            ask,
            run: MergeRun {
                draft: DraftStep::WasReady,
                result: Ok(MergeOutcome::Merged {
                    method: MergeMethod::Squash,
                    head_oid: CommitOid::new("fresh-sha".to_string()),
                }),
            },
        },
    );
    assert!(!has_stop_or_remove(&no_session));
}

#[test]
fn removed_after_merge_reports_outcomes_and_observes_sessions() {
    let check =
        |session: SessionState, result: Result<RemoveOutcome, SessionError>, want_text: &str, want_level: Level| {
            let (mut state, _) = app();
            let cmds = press(
                &mut state,
                Msg::RemovedAfterMerge {
                    pr: PrNumber(10),
                    method: MergeMethod::Squash,
                    key: issue_key(3),
                    session,
                    result,
                },
            );
            assert_eq!(toast(&state), want_text);
            assert_eq!(level(&state), Some(want_level));
            assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::ObserveSessions))]));
        };

    check(
        SessionState::Running,
        Ok(RemoveOutcome::Removed),
        "Merged PR #10 (squash) and removed #3's worktree and branch.",
        Level::Info,
    );
    check(
        SessionState::Running,
        Ok(RemoveOutcome::Refused(RemoveRefusal::Uncommitted)),
        "Merged PR #10 (squash) and stopped #3's session. Kept worktree (uncommitted changes).",
        Level::Info,
    );
    check(
        SessionState::Exited,
        Ok(RemoveOutcome::Refused(RemoveRefusal::Uncommitted)),
        "Merged PR #10 (squash) and closed #3's window. Kept worktree (uncommitted changes).",
        Level::Info,
    );
    check(
        SessionState::NeedsYou,
        Ok(RemoveOutcome::Refused(unpushed(1))),
        "Merged PR #10 (squash) and stopped #3's session. Kept worktree (1 unpushed commit).",
        Level::Info,
    );
    check(
        SessionState::Running,
        Err(git_error("boom")),
        "Merged PR #10 (squash), but removing #3 failed: git failed with code 128: boom.",
        Level::Error,
    );
}
