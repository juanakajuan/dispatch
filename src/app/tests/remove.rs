use super::*;
use crate::app::toast::Level;
use crate::session::{RemoveOutcome, RemoveRefusal};
use std::num::NonZeroU32;

fn removes(cmds: &[Cmd]) -> Vec<IssueNumber> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Remove(n))) => Some(*n),
            _ => None,
        })
        .collect()
}

fn on_session(s: SessionState) -> State {
    let (mut state, _) = app();
    state.sessions.insert(IssueNumber(3), session(3, s));
    press(&mut state, key('3'));
    state
}

fn level(state: &State) -> Option<Level> {
    state.toast.as_ref().map(Toast::level)
}

#[test]
fn d_twice_removes_an_exited_session() {
    let mut state = on_session(SessionState::Exited);
    assert!(shows(&state, "D", "remove"));
    assert!(press(&mut state, key('D')).is_empty(), "the first press only arms");
    assert_eq!(toast(&state), "Press D again to remove #3's worktree and branch");
    assert!(removes(&press(&mut state, key('D'))) == [IssueNumber(3)]);
    assert_eq!(toast(&state), "Removing #3");
    assert!(state.armed.is_none());
}

#[test]
fn another_key_between_the_presses_disarms() {
    let mut state = on_session(SessionState::Exited);
    press(&mut state, key('D'));
    press(&mut state, key('j'));
    assert!(press(&mut state, key('D')).is_empty(), "armed again, not removed");
    assert_eq!(toast(&state), "Press D again to remove #3's worktree and branch");
}

#[test]
fn a_press_after_the_selection_moved_to_another_issue_only_arms() {
    let mut state = on_session(SessionState::Exited);
    state.sessions.insert(IssueNumber(2), session(2, SessionState::Exited));
    state.tabs.sessions.selected = Some(IssueNumber(3));
    press(&mut state, key('D'));
    assert_eq!(state.armed, Some(Armed::Remove(IssueNumber(3))));
    state.tabs.sessions.selected = Some(IssueNumber(2));
    assert!(press(&mut state, key('D')).is_empty());
    assert_eq!(state.armed, Some(Armed::Remove(IssueNumber(2))));
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
fn a_live_session_whose_pr_merged_can_be_removed() {
    let mut state = on_session(SessionState::Running);
    state.prs.items[0].state = PrState::Merged;
    assert!(shows(&state, "D", "remove"));
    press(&mut state, key('D'));
    assert_eq!(removes(&press(&mut state, key('D'))), [IssueNumber(3)]);
}

#[test]
fn no_session_selected_says_so() {
    let (mut state, _) = app();
    assert!(!shows(&state, "D", "remove"));
    assert!(press(&mut state, key('D')).is_empty());
    assert_eq!(toast(&state), "No session here.");
}

#[test]
fn a_removed_result_toasts_and_asks_for_an_observation() {
    let (mut state, _) = app();
    let cmds = press(&mut state, Msg::Removed { issue: IssueNumber(3), result: Ok(RemoveOutcome::Removed) });
    assert_eq!(toast(&state), "Removed #3's worktree and branch");
    assert_eq!(level(&state), Some(Level::Info));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::ObserveSessions))]));
}

fn refused(why: RemoveRefusal) -> (String, Option<Level>) {
    let (mut state, _) = app();
    press(&mut state, Msg::Removed { issue: IssueNumber(3), result: Ok(RemoveOutcome::Refused(why)) });
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
    press(&mut state, Msg::Removed { issue: IssueNumber(3), result: Err(git_error("boom")) });
    assert_eq!(toast(&state), "Remove #3 failed: git failed with code 128: boom");
    assert_eq!(level(&state), Some(Level::Error));
}
