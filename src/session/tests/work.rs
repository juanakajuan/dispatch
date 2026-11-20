use super::{br, workspace, wp};
use crate::proc::{ProcError, fake};
use crate::session::{Checkout, Push, Session, SessionState, Sessions, Work};
use crate::testutil::issue_key;
use crate::testutil::temp_dir;

fn sessions(branches: &[(u32, &str)]) -> Sessions {
    branches
        .iter()
        .map(|&(n, b)| {
            let s = Session {
                key: issue_key(n),
                branch: br(b),
                checkout: Checkout::SessionBranch,
                worktree: wp(&format!("/w/{n}")),
                harness: crate::config::Harness::Claude,
                state: SessionState::Running,
            };
            (s.key, s)
        })
        .collect()
}

/// Answers `for-each-ref` with `refs`, `status` with `status`, and `rev-list`
/// with `left_right`.
fn git(refs: &'static str, status: Result<&'static str, ()>, left_right: &'static str) {
    fake::install(move |call| -> Result<String, ProcError> {
        if call.starts_with("git for-each-ref") {
            Ok(refs.to_string())
        } else if call.starts_with("git --no-optional-locks status --porcelain") {
            status.map(str::to_string).map_err(|()| fake::fail("git", 128, "not a git repository"))
        } else if call.starts_with("git rev-list --left-right --count") {
            Ok(left_right.to_string())
        } else {
            Err(fake::fail("git", 1, "unexpected"))
        }
    });
}

#[test]
fn a_branch_that_origin_lacks_is_not_pushed() {
    let ws = workspace(&temp_dir("work-not-pushed"));
    git("refs/heads/fix/3-x aaa 2 0\n", Ok(" M src/lib.rs\n?? new.rs\n"), "");
    let got = ws.work(&sessions(&[(3, "fix/3-x")]));
    assert_eq!(got[&issue_key(3)], Work { commits: 2, uncommitted: 2, push: Push::NotPushed });
    assert_eq!(
        fake::calls(),
        [
            "git for-each-ref --format=%(refname) %(objectname) %(ahead-behind:refs/remotes/origin/main) \
             refs/heads/fix/3-x refs/remotes/origin/fix/3-x refs/remotes/dispatch-fix-3-x",
            "git --no-optional-locks status --porcelain",
        ],
        "one for-each-ref and one status; no rev-list without an origin branch"
    );
}

#[test]
fn origin_at_the_same_commit_is_pushed_with_no_rev_list() {
    let ws = workspace(&temp_dir("work-pushed"));
    git("refs/heads/fix/3-x aaa 1 0\nrefs/remotes/origin/fix/3-x aaa 1 0\n", Ok(""), "");
    let got = ws.work(&sessions(&[(3, "fix/3-x")]));
    assert_eq!(got[&issue_key(3)], Work { commits: 1, uncommitted: 0, push: Push::Pushed });
    assert!(!fake::calls().iter().any(|c| c.starts_with("git rev-list")), "{:?}", fake::calls());
}

#[test]
fn origin_at_another_commit_counts_both_sides() {
    let ws = workspace(&temp_dir("work-differs"));
    git("refs/heads/fix/3-x aaa 4 0\nrefs/remotes/origin/fix/3-x bbb 3 0\n", Ok(""), "2\t1\n");
    let got = ws.work(&sessions(&[(3, "fix/3-x")]));
    assert_eq!(got[&issue_key(3)].push, Push::Differs { to_push: 2, to_pull: 1 });
    assert!(
        fake::calls().contains(
            &"git rev-list --left-right --count refs/heads/fix/3-x...refs/remotes/origin/fix/3-x".to_string()
        ),
        "{:?}",
        fake::calls()
    );
}

#[test]
fn a_fork_session_is_pushed_when_its_fork_branch_is_at_its_commit() {
    let ws = workspace(&temp_dir("work-fork"));
    git("refs/heads/fork/pr-6 aaa 2 0\nrefs/remotes/dispatch-fork-pr-6/feat/x aaa 2 0\n", Ok(""), "");
    let got = ws.work(&sessions(&[(4, "fork/pr-6")]));
    assert_eq!(got[&issue_key(4)], Work { commits: 2, uncommitted: 0, push: Push::Pushed });
}

#[test]
fn a_session_whose_reads_fail_has_no_entry_and_no_sessions_run_nothing() {
    let ws = workspace(&temp_dir("work-fails"));
    git("refs/heads/fix/3-x aaa 1 0\n", Err(()), "");
    assert!(ws.work(&sessions(&[(3, "fix/3-x")])).is_empty(), "a worktree that git can not read");

    git("", Ok(""), "");
    assert!(ws.work(&sessions(&[(3, "fix/3-x")])).is_empty(), "a branch that for-each-ref did not list");

    git("", Ok(""), "");
    assert!(ws.work(&Sessions::new()).is_empty());
    assert!(fake::calls().is_empty(), "for-each-ref with no pattern would list every ref");
}
