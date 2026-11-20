use super::{br, markers, win, wt};
use crate::session::join::join;
use crate::session::mux::Liveness::{Dead, Waiting, Working};
use crate::session::{Checkout, SessionState};
use crate::testutil::{issue_key, pr_key};
use std::path::Path;

fn root() -> &'static Path {
    Path::new("/a/orbit")
}

#[test]
fn join_live_window_is_running_and_waiting_flag_needs_you() {
    let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-1", Some("fix/1-x"))];
    let m = markers(&[("fix/1-x", 1)]);
    let s = join(root(), &worktrees, &m, &[win("@1", "/a/orbit-1", Working)]);
    assert_eq!(s.len(), 1);
    assert_eq!(s[&issue_key(1)].state(), SessionState::Running);
    let s = join(root(), &worktrees, &m, &[win("@1", "/a/orbit-1", Waiting)]);
    assert_eq!(s[&issue_key(1)].state(), SessionState::NeedsYou);
}

#[test]
fn join_dead_pane_and_missing_window_are_exited() {
    let worktrees = [wt("/a/orbit-1", Some("fix/1-x"))];
    let m = markers(&[("fix/1-x", 1)]);
    let dead = join(root(), &worktrees, &m, &[win("@1", "/a/orbit-1", Dead)]);
    assert_eq!(dead[&issue_key(1)].state(), SessionState::Exited);
    assert!(dead[&issue_key(1)].window.is_some());
    let none = join::<()>(root(), &worktrees, &m, &[]);
    assert_eq!(none[&issue_key(1)].state(), SessionState::Exited);
    assert!(none[&issue_key(1)].window.is_none());
}

#[test]
fn join_ignores_unmarked_detached_and_orphan_windows() {
    let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-2", None)];
    let s = join(root(), &worktrees, &markers(&[("other", 9)]), &[win("@5", "/a/gone", Working)]);
    assert!(s.is_empty());
}

#[test]
fn join_one_issue_marked_twice_keeps_first_path_and_prefers_live_window() {
    let worktrees = [wt("/a/z", Some("b2")), wt("/a/a", Some("b1"))];
    let m = markers(&[("b1", 7), ("b2", 7)]);
    let s = join(root(), &worktrees, &m, &[win("@1", "/a/a", Dead), win("@2", "/a/a", Working)]);
    assert_eq!(s.len(), 1);
    assert_eq!(s[&issue_key(7)].branch, br("b1"));
    assert_eq!(s[&issue_key(7)].window.as_ref().map(|(w, _)| w.as_str()), Some("@2"));
}

#[test]
fn join_marked_branch_checked_out_is_on_the_session_branch() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/58", Some("fix/58-x"))];
    let s = join::<()>(root(), &worktrees, &markers(&[("fix/58-x", 58)]), &[]);
    assert_eq!(s[&issue_key(58)].checkout, Checkout::SessionBranch);
}

#[test]
fn join_keeps_a_session_whose_worktree_changed_branch() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/58", Some("try-other"))];
    let m = markers(&[("fix/58-x", 58)]);
    let s = join(root(), &worktrees, &m, &[win("@3", "/a/orbit/.claude/worktrees/58", Working)]);
    let row = &s[&issue_key(58)];
    assert_eq!(row.branch, br("fix/58-x"), "PRs and remove still go by the marked branch");
    assert_eq!(row.checkout, Checkout::Other(br("try-other")));
    assert_eq!(row.state(), SessionState::Running);
}

#[test]
fn join_keeps_a_session_whose_worktree_is_detached() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/58", None)];
    let s = join::<()>(root(), &worktrees, &markers(&[("fix/58-x", 58)]), &[]);
    assert_eq!(s[&issue_key(58)].checkout, Checkout::Detached);
}

#[test]
fn join_ignores_a_worktree_dir_with_no_marker_for_its_issue() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/58", Some("try-other"))];
    let s = join::<()>(root(), &worktrees, &markers(&[("fix/9-x", 9)]), &[]);
    assert!(s.is_empty());
}

#[test]
fn join_a_match_by_branch_wins_over_a_match_by_path() {
    let worktrees = [wt("/a/elsewhere", Some("fix/58-x")), wt("/a/orbit/.claude/worktrees/58", Some("try-other"))];
    let s = join::<()>(root(), &worktrees, &markers(&[("fix/58-x", 58)]), &[]);
    assert_eq!(s.len(), 1);
    assert_eq!(s[&issue_key(58)].worktree, super::wp("/a/elsewhere"));
    assert_eq!(s[&issue_key(58)].checkout, Checkout::SessionBranch);
}

#[test]
fn join_one_worktree_is_at_most_one_session() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/58", Some("fix/9-x"))];
    let s = join::<()>(root(), &worktrees, &markers(&[("fix/58-x", 58), ("fix/9-x", 9)]), &[]);
    assert_eq!(s.keys().copied().collect::<Vec<_>>(), [issue_key(9)]);
}

const PR_DIR: &str = "/a/orbit/.claude/worktrees/pr-7";

fn pr_markers(rows: &[(&str, crate::domain::SessionKey)]) -> crate::session::markers::Markers {
    rows.iter().map(|(b, k)| (br(b), *k)).collect()
}

#[test]
fn join_a_dispatch_pr_marker_makes_a_pr_session() {
    let worktrees = [wt("/a/orbit", Some("main")), wt(PR_DIR, Some("chore/dep"))];
    let m = pr_markers(&[("chore/dep", pr_key(7))]);
    let s = join(root(), &worktrees, &m, &[win("@1", PR_DIR, Working)]);
    assert_eq!(s.keys().copied().collect::<Vec<_>>(), [pr_key(7)]);
    assert_eq!(s[&pr_key(7)].state(), SessionState::Running);
    assert_eq!(s[&pr_key(7)].branch, br("chore/dep"));
}

#[test]
fn join_pr_7_and_issue_7_are_two_sessions() {
    let worktrees = [wt("/a/orbit/.claude/worktrees/7", Some("fix/7-x")), wt(PR_DIR, Some("chore/dep"))];
    let m = pr_markers(&[("fix/7-x", issue_key(7)), ("chore/dep", pr_key(7))]);
    let s = join::<()>(root(), &worktrees, &m, &[]);
    assert_eq!(s.keys().copied().collect::<Vec<_>>(), [issue_key(7), pr_key(7)]);
    assert_eq!(s[&issue_key(7)].branch, br("fix/7-x"));
    assert_eq!(s[&pr_key(7)].branch, br("chore/dep"));
}

#[test]
fn join_a_pr_worktree_on_another_branch_stays_that_pr_session() {
    let worktrees = [wt(PR_DIR, Some("try-other"))];
    let s = join::<()>(root(), &worktrees, &pr_markers(&[("chore/dep", pr_key(7))]), &[]);
    assert_eq!(s[&pr_key(7)].checkout, Checkout::Other(br("try-other")));
    let dir_of_issue_7 = [wt("/a/orbit/.claude/worktrees/7", Some("try-other"))];
    let s = join::<()>(root(), &dir_of_issue_7, &pr_markers(&[("chore/dep", pr_key(7))]), &[]);
    assert!(s.is_empty(), "the issue 7 dir is not the PR 7 dir");
}
