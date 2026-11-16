use super::{br, markers, win, wt};
use crate::domain::IssueNumber;
use crate::session::{SessionState, join};

#[test]
fn join_live_window_is_running_and_waiting_flag_needs_you() {
    let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-1", Some("fix/1-x"))];
    let m = markers(&[("fix/1-x", 1)]);
    let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, false)]);
    assert_eq!(s.len(), 1);
    assert_eq!(s[&IssueNumber(1)].state(), SessionState::Running);
    let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, true)]);
    assert_eq!(s[&IssueNumber(1)].state(), SessionState::NeedsYou);
}

#[test]
fn join_dead_pane_and_missing_window_are_exited() {
    let worktrees = [wt("/a/orbit-1", Some("fix/1-x"))];
    let m = markers(&[("fix/1-x", 1)]);
    let dead = join(&worktrees, &m, &[win("@1", "/a/orbit-1", true, true)]);
    assert_eq!(dead[&IssueNumber(1)].state(), SessionState::Exited);
    assert!(dead[&IssueNumber(1)].window.is_some());
    let none = join(&worktrees, &m, &[]);
    assert_eq!(none[&IssueNumber(1)].state(), SessionState::Exited);
    assert!(none[&IssueNumber(1)].window.is_none());
}

#[test]
fn join_ignores_unmarked_detached_and_orphan_windows() {
    let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-2", None)];
    let s = join(&worktrees, &markers(&[("other", 9)]), &[win("@5", "/a/gone", false, false)]);
    assert!(s.is_empty());
}

#[test]
fn join_one_issue_marked_twice_keeps_first_path_and_prefers_live_window() {
    let worktrees = [wt("/a/z", Some("b2")), wt("/a/a", Some("b1"))];
    let m = markers(&[("b1", 7), ("b2", 7)]);
    let s = join(&worktrees, &m, &[win("@1", "/a/a", true, false), win("@2", "/a/a", false, false)]);
    assert_eq!(s.len(), 1);
    assert_eq!(s[&IssueNumber(7)].branch, br("b1"));
    assert_eq!(s[&IssueNumber(7)].window.as_ref().map(|w| w.id.as_str()), Some("@2"));
}
