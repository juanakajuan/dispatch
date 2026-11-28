use super::{GitWorld, workspace};
use crate::domain::CommitOid;
use crate::proc::fake;
use crate::session::error::SessionError;
use crate::session::{CleanReport, RemoveOutcome, RemoveRefusal, RemoveTarget};
use crate::testutil::issue_key;
use crate::testutil::temp_dir;
use std::cell::Cell;
use std::fmt::Write;
use std::num::NonZeroU32;
use std::path::Path;

const HEAD_SHA: &str = "07e2ba7143f4de2812171b22d9670532a4857cff";

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

fn target(issue: u32) -> RemoveTarget {
    RemoveTarget { key: issue_key(issue), merged_head: None }
}

fn merged_target(issue: u32, sha: &str) -> RemoveTarget {
    RemoveTarget { key: issue_key(issue), merged_head: Some(CommitOid::new(sha.to_string())) }
}

/// A started session for issue 142 with a window, `dirty` or clean, with
/// `unpushed` commits on no remote. Two `!` shells are open in its worktree,
/// and one in another.
fn install_world(base: &Path, tree: Tree, unpushed: u32, window: Window) {
    let base = base.to_path_buf();
    let mut git = GitWorld::started();
    let mut open = window == Window::Open;
    let dir = base.join("orbit/.claude/worktrees/142");
    fake::install(move |call| {
        if let Some(answer) = git.answer(call, &base) {
            answer
        } else if call.starts_with("tmux list-windows") && call.contains("@dispatch-shell") {
            Ok(format!("@4\t\n@7\t{0}\n@8\t{0}\n@9\t{0}-other\n", dir.display()))
        } else if call.starts_with("tmux list-windows") {
            Ok(if open { format!("@4\t0\t{}\t\n", dir.display()) } else { String::new() })
        } else if call.starts_with("tmux kill-window") {
            open = false;
            Ok(String::new())
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
    let prefixes = ["tmux kill-window", "git worktree remove", "git branch", "git worktree prune"];
    calls.iter().map(String::as_str).filter(|c| prefixes.iter().any(|p| c.starts_with(p))).collect()
}

#[test]
fn a_dirty_worktree_is_refused_and_nothing_is_removed() {
    let (ws, _) = started("remove-dirty", Tree::Dirty, 0, Window::Open);
    let got = ws.remove(&target(142)).expect("an answer");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Uncommitted));
    assert!(changed(&fake::calls()).is_empty(), "{:?}", fake::calls());
}

#[test]
fn unpushed_commits_are_refused_with_their_count() {
    let (ws, _) = started("remove-unpushed", Tree::Clean, 3, Window::Open);
    let got = ws.remove(&target(142)).expect("an answer");
    let three = NonZeroU32::new(3).expect("non-zero");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Unpushed(three)));
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c.starts_with("git rev-list --count fix/142-x HEAD --not --remotes")), "{calls:?}");
    assert!(changed(&calls).is_empty(), "{calls:?}");
}

#[test]
fn a_clean_pushed_session_closes_its_window_then_removes_the_worktree_then_the_branch_then_prunes() {
    let (ws, base) = started("remove-clean", Tree::Clean, 0, Window::Open);
    let got = ws.remove(&target(142)).expect("removed");
    assert_eq!(got, RemoveOutcome::Removed);
    let dir = base.join("orbit/.claude/worktrees/142");
    let calls = fake::calls();
    assert_eq!(
        changed(&calls),
        [
            "tmux kill-window -t @4",
            "tmux kill-window -t @7",
            "tmux kill-window -t @8",
            &format!("git worktree remove {}", dir.display()),
            "git branch -D fix/142-x",
            "git worktree prune",
        ]
    );
}

#[test]
fn without_a_window_only_the_shells_are_closed() {
    let (ws, _) = started("remove-no-window", Tree::Clean, 0, Window::Gone);
    assert_eq!(ws.remove(&target(142)).expect("removed"), RemoveOutcome::Removed);
    let calls = fake::calls();
    let kills: Vec<&String> = calls.iter().filter(|c| c.starts_with("tmux kill-window")).collect();
    assert_eq!(kills, ["tmux kill-window -t @7", "tmux kill-window -t @8"], "the shells, not the other worktree's");
}

#[test]
fn an_issue_without_a_session_is_an_error() {
    let (ws, _) = started("remove-none", Tree::Clean, 0, Window::Gone);
    let err = ws.remove(&target(999)).expect_err("no session");
    assert_eq!(err, SessionError::NoSession(issue_key(999)));
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
    let err = ws.remove(&target(142)).expect_err("git failed");
    assert!(err.to_string().contains("fatal: cannot remove"), "{err}");
    assert!(!fake::calls().iter().any(|c| c.starts_with("git branch") || c == "git worktree prune"));
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
    let err = ws.remove(&target(142)).expect_err("not a count");
    assert_eq!(err, SessionError::NotACount("warning: something".to_string()));
    assert!(changed(&fake::calls()).is_empty());
}

#[test]
fn a_session_on_another_branch_is_listed_and_removes_its_marked_branch() {
    let (ws, base) = started("remove-other-branch", Tree::Clean, 0, Window::Open);
    let dir = base.join("orbit/.claude/worktrees/142");
    let mut git = GitWorld::started();
    fake::install(move |call| {
        if call.starts_with("git worktree list") {
            Ok(format!("worktree {}\nHEAD x\nbranch refs/heads/try-other\n\n", dir.display()))
        } else if call.starts_with("git rev-list") {
            Ok("0\n".to_string())
        } else {
            git.answer(call, &base).unwrap_or_else(|| Ok(String::new()))
        }
    });
    let listed = ws.observe().expect("observed");
    assert_eq!(listed[&issue_key(142)].checked_out(), "try-other");
    assert_eq!(ws.remove(&target(142)).expect("removed"), RemoveOutcome::Removed);
    assert!(fake::calls().iter().any(|c| c == "git branch -D fix/142-x"), "{:?}", fake::calls());
}

#[test]
fn squash_merged_session_at_merged_head_is_removed_and_pruned() {
    let (ws, base) = started("remove-squash-ok", Tree::Clean, 0, Window::Open);
    let got = ws.remove(&merged_target(142, HEAD_SHA)).expect("removed");
    assert_eq!(got, RemoveOutcome::Removed);
    let dir = base.join("orbit/.claude/worktrees/142");
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c == &format!("git rev-parse --verify --quiet {HEAD_SHA}^{{commit}}")), "{calls:?}");
    assert!(
        calls.iter().any(|c| c == &format!("git rev-list --count fix/142-x HEAD --not --remotes {HEAD_SHA}")),
        "{calls:?}"
    );
    assert_eq!(
        changed(&calls),
        [
            "tmux kill-window -t @4",
            "tmux kill-window -t @7",
            "tmux kill-window -t @8",
            &format!("git worktree remove {}", dir.display()),
            "git branch -D fix/142-x",
            "git worktree prune",
        ]
    );
}

#[test]
fn squash_merged_session_with_commits_beyond_merged_head_is_refused() {
    let (ws, _) = started("remove-squash-extra", Tree::Clean, 2, Window::Open);
    let got = ws.remove(&merged_target(142, HEAD_SHA)).expect("an answer");
    let two = NonZeroU32::new(2).expect("non-zero");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Unpushed(two)));
    let calls = fake::calls();
    assert!(
        calls.iter().any(|c| c == &format!("git rev-list --count fix/142-x HEAD --not --remotes {HEAD_SHA}")),
        "{calls:?}"
    );
    assert!(changed(&calls).is_empty(), "{calls:?}");
}

#[test]
fn squash_merged_session_with_uncommitted_changes_is_refused_without_pruning() {
    let (ws, _) = started("remove-squash-dirty", Tree::Dirty, 0, Window::Open);
    let got = ws.remove(&merged_target(142, HEAD_SHA)).expect("an answer");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Uncommitted));
    let calls = fake::calls();
    assert!(!calls.iter().any(|c| c.starts_with("git rev-parse") || c.starts_with("git rev-list")), "{calls:?}");
    assert!(changed(&calls).is_empty(), "{calls:?}");
}

#[test]
fn squash_merged_session_on_other_branch_or_detached_ignores_merged_head() {
    for (name, wt_line) in
        [("remove-squash-other", "branch refs/heads/try-other"), ("remove-squash-detached", "detached")]
    {
        let (ws, base) = started(name, Tree::Clean, 1, Window::Open);
        let dir = base.join("orbit/.claude/worktrees/142");
        let mut git = GitWorld::started();
        let wt_line = wt_line.to_string();
        fake::install(move |call| {
            if call.starts_with("git worktree list") {
                Ok(format!("worktree {}\nHEAD x\n{wt_line}\n\n", dir.display()))
            } else if call.starts_with("git rev-list") {
                Ok("1\n".to_string())
            } else {
                git.answer(call, &base).unwrap_or_else(|| Ok(String::new()))
            }
        });
        let got = ws.remove(&merged_target(142, HEAD_SHA)).expect("refused");
        let one = NonZeroU32::new(1).expect("non-zero");
        assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Unpushed(one)));
        let calls = fake::calls();
        assert!(!calls.iter().any(|c| c.starts_with("git rev-parse")), "{calls:?}");
        assert!(calls.iter().any(|c| c == "git rev-list --count fix/142-x HEAD --not --remotes"), "{calls:?}");
        assert!(changed(&calls).is_empty(), "{calls:?}");
    }
}

#[test]
fn missing_or_non_hex_merged_head_is_not_passed_to_rev_list() {
    let (ws, base) = started("remove-missing-oid", Tree::Clean, 1, Window::Gone);
    let mut git = GitWorld::started();
    fake::install(move |call| {
        if call.starts_with("git rev-parse") {
            Err(fake::fail("git", 1, ""))
        } else if call.starts_with("git rev-list") {
            Ok("1\n".to_string())
        } else {
            git.answer(call, &base).unwrap_or_else(|| Ok(String::new()))
        }
    });
    let one = NonZeroU32::new(1).expect("non-zero");
    assert_eq!(
        ws.remove(&merged_target(142, HEAD_SHA)).expect("refused"),
        RemoveOutcome::Refused(RemoveRefusal::Unpushed(one))
    );
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c.starts_with("git rev-parse")), "{calls:?}");
    assert!(calls.iter().any(|c| c == "git rev-list --count fix/142-x HEAD --not --remotes"), "{calls:?}");

    for bad in ["--all", "", "HEAD"] {
        let (ws, _) = started("remove-bad-oid", Tree::Clean, 1, Window::Gone);
        assert_eq!(
            ws.remove(&merged_target(142, bad)).expect("refused"),
            RemoveOutcome::Refused(RemoveRefusal::Unpushed(one))
        );
        let calls = fake::calls();
        assert!(!calls.iter().any(|c| c.starts_with("git rev-parse")), "{bad}: {calls:?}");
        assert!(calls.iter().any(|c| c == "git rev-list --count fix/142-x HEAD --not --remotes"), "{bad}: {calls:?}");
    }
}

#[test]
fn clean_removes_clean_sessions_skips_refusals_and_errors_without_aborting_and_prunes_once() {
    let base = temp_dir("clean-multi");
    let ws = workspace(&base);
    for n in [142, 143, 144] {
        std::fs::create_dir_all(base.join(format!("orbit/.claude/worktrees/{n}"))).expect("mkdir");
    }
    let status_calls = Cell::new(0u32);
    let b = base.clone();
    fake::install(move |call| {
        if call.starts_with("git worktree list") {
            let mut out = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", b.join("orbit").display());
            for n in [142, 143, 144] {
                let d = b.join(format!("orbit/.claude/worktrees/{n}"));
                let _ = write!(out, "worktree {}\nHEAD x\nbranch refs/heads/fix/{n}-x\n\n", d.display());
            }
            Ok(out)
        } else if call.starts_with("git config --get-regexp") {
            Ok("branch.fix/142-x.dispatch-issue 142\n\
                branch.fix/143-x.dispatch-issue 143\n\
                branch.fix/144-x.dispatch-issue 144\n"
                .to_string())
        } else if call.starts_with("tmux list-windows") {
            Ok(String::new())
        } else if call.contains("status --porcelain") {
            let n = status_calls.get();
            status_calls.set(n + 1);
            Ok(if n == 1 { " M src/lib.rs\n".to_string() } else { String::new() })
        } else if call.starts_with("git rev-list") {
            Ok(if call.contains("fix/144-x") { "2\n".to_string() } else { "0\n".to_string() })
        } else {
            Ok(String::new())
        }
    });

    let report = ws.clean(&[
        merged_target(142, HEAD_SHA),
        merged_target(143, HEAD_SHA),
        merged_target(144, HEAD_SHA),
        merged_target(999, HEAD_SHA),
    ]);
    let two = NonZeroU32::new(2).expect("non-zero");
    assert_eq!(
        report,
        CleanReport {
            removed: vec![issue_key(142)],
            skipped: vec![(issue_key(143), RemoveRefusal::Uncommitted), (issue_key(144), RemoveRefusal::Unpushed(two)),],
            failed: vec![(issue_key(999), SessionError::NoSession(issue_key(999)))],
        }
    );
    let calls = fake::calls();
    let dir142 = base.join("orbit/.claude/worktrees/142");
    assert_eq!(
        changed(&calls),
        [&format!("git worktree remove {}", dir142.display()), "git branch -D fix/142-x", "git worktree prune",]
    );
    assert_eq!(calls.iter().filter(|c| c.as_str() == "git worktree prune").count(), 1);
}

#[test]
fn stop_and_remove_stops_window_and_removes_clean_squash_merged_session() {
    let (ws, base) = started("stop-remove-clean", Tree::Clean, 0, Window::Open);
    let got = ws.stop_and_remove(&merged_target(142, HEAD_SHA)).expect("removed");
    assert_eq!(got, RemoveOutcome::Removed);
    let dir = base.join("orbit/.claude/worktrees/142");
    assert_eq!(
        changed(&fake::calls()),
        [
            "tmux kill-window -t @4",
            "tmux kill-window -t @7",
            "tmux kill-window -t @8",
            &format!("git worktree remove {}", dir.display()),
            "git branch -D fix/142-x",
            "git worktree prune",
        ]
    );
}

#[test]
fn stop_and_remove_stops_window_even_when_worktree_is_dirty() {
    let (ws, _) = started("stop-remove-dirty", Tree::Dirty, 0, Window::Open);
    let got = ws.stop_and_remove(&merged_target(142, HEAD_SHA)).expect("an answer");
    assert_eq!(got, RemoveOutcome::Refused(RemoveRefusal::Uncommitted));
    assert_eq!(changed(&fake::calls()), ["tmux kill-window -t @4"]);
}
