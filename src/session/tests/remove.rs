use super::{GitWorld, workspace};
use crate::domain::IssueNumber;
use crate::proc::fake;
use crate::session::error::SessionError;
use crate::session::{RemoveOutcome, RemoveRefusal};
use crate::testutil::temp_dir;
use std::num::NonZeroU32;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tree {
    Clean,
    Dirty,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Window {
    Open,
    Gone,
}

/// A started session for issue 142 with a window, `dirty` or clean, with
/// `unpushed` commits on no remote.
fn install_world(base: &Path, tree: Tree, unpushed: u32, window: Window) {
    let base = base.to_path_buf();
    let mut git = GitWorld::started();
    let dir = base.join("orbit/.claude/worktrees/142");
    fake::install(move |call| {
        if let Some(answer) = git.answer(call, &base) {
            answer
        } else if call.starts_with("tmux list-windows") {
            Ok(if window == Window::Open { format!("@4\t0\t{}\t\n", dir.display()) } else { String::new() })
        } else if call.contains("status --porcelain") {
            Ok(if tree == Tree::Dirty { " M src/lib.rs\n".to_string() } else { String::new() })
        } else if call.starts_with("git rev-list") {
            Ok(format!("{unpushed}\n"))
        } else {
            Ok(String::new())
        }
    });
}

fn started(name: &str, tree: Tree, unpushed: u32, window: Window) -> (crate::session::Workspace, std::path::PathBuf) {
    let base = temp_dir(name);
    let ws = workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    install_world(&base, tree, unpushed, window);
    (ws, base)
}

fn changed(calls: &[String]) -> Vec<&str> {
    let prefixes = ["tmux kill-window", "git worktree remove", "git branch"];
    calls.iter().map(String::as_str).filter(|c| prefixes.iter().any(|p| c.starts_with(p))).collect()
}

#[test]
fn a_dirty_worktree_is_refused_and_nothing_is_removed() {
    let (ws, _) = started("remove-dirty", Tree::Dirty, 0, Window::Open);
    let got = ws.remove(IssueNumber(142)).expect("an answer");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Uncommitted));
    assert!(changed(&fake::calls()).is_empty(), "{:?}", fake::calls());
}

#[test]
fn unpushed_commits_are_refused_with_their_count() {
    let (ws, _) = started("remove-unpushed", Tree::Clean, 3, Window::Open);
    let got = ws.remove(IssueNumber(142)).expect("an answer");
    let three = NonZeroU32::new(3).expect("non-zero");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Unpushed(three)));
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c.starts_with("git rev-list --count fix/142-x --not --remotes")), "{calls:?}");
    assert!(changed(&calls).is_empty(), "{calls:?}");
}

#[test]
fn a_clean_pushed_session_closes_its_window_then_removes_the_worktree_then_the_branch() {
    let (ws, base) = started("remove-clean", Tree::Clean, 0, Window::Open);
    let got = ws.remove(IssueNumber(142)).expect("removed");
    assert_eq!(got, RemoveOutcome::Removed);
    let dir = base.join("orbit/.claude/worktrees/142");
    let calls = fake::calls();
    assert_eq!(
        changed(&calls),
        ["tmux kill-window -t @4", &format!("git worktree remove {}", dir.display()), "git branch -D fix/142-x"]
    );
}

#[test]
fn without_a_window_nothing_is_closed() {
    let (ws, _) = started("remove-no-window", Tree::Clean, 0, Window::Gone);
    assert_eq!(ws.remove(IssueNumber(142)).expect("removed"), RemoveOutcome::Removed);
    assert!(!fake::calls().iter().any(|c| c.starts_with("tmux kill-window")));
}

#[test]
fn an_issue_without_a_session_is_an_error() {
    let (ws, _) = started("remove-none", Tree::Clean, 0, Window::Gone);
    let err = ws.remove(IssueNumber(999)).expect_err("no session");
    assert_eq!(err, SessionError::NoSession(IssueNumber(999)));
    assert!(changed(&fake::calls()).is_empty());
}

#[test]
fn a_failed_worktree_remove_keeps_the_branch() {
    let (ws, base) = started("remove-fails", Tree::Clean, 0, Window::Gone);
    let mut git = GitWorld::started();
    fake::install(move |call| {
        if call.starts_with("git worktree remove") {
            Err(fake::fail("git", 128, "fatal: cannot remove"))
        } else if call.starts_with("git rev-list") {
            Ok("0\n".to_string())
        } else {
            git.answer(call, &base).unwrap_or_else(|| Ok(String::new()))
        }
    });
    let err = ws.remove(IssueNumber(142)).expect_err("git failed");
    assert!(err.to_string().contains("fatal: cannot remove"), "{err}");
    assert!(!fake::calls().iter().any(|c| c.starts_with("git branch")));
}

#[test]
fn a_commit_count_that_is_not_a_number_is_an_error_and_nothing_is_removed() {
    let (ws, base) = started("remove-bad-count", Tree::Clean, 0, Window::Open);
    let mut git = GitWorld::started();
    fake::install(move |call| {
        if call.starts_with("git rev-list") {
            Ok("warning: something\n".to_string())
        } else {
            git.answer(call, &base).unwrap_or_else(|| Ok(String::new()))
        }
    });
    let err = ws.remove(IssueNumber(142)).expect_err("not a count");
    assert_eq!(err, SessionError::NotACount("warning: something".to_string()));
    assert!(changed(&fake::calls()).is_empty());
}
