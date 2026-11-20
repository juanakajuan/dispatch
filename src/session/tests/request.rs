use crate::domain::Mode;
use crate::session::request::{branch_for, prompt_for, prompt_for_pr, start_defaults};
use crate::testutil::{issue, pr};

#[test]
fn branch_prefix_follows_labels() {
    let b = |labels: &[&str]| branch_for(&issue(142, "Retry queue drops jobs when Redis reconnects", labels));
    assert_eq!(b(&["bug", "p1"]).as_str(), "fix/142-retry-queue-drops-jobs");
    assert_eq!(b(&["documentation"]).as_str(), "docs/142-retry-queue-drops-jobs");
    assert_eq!(b(&["chore"]).as_str(), "chore/142-retry-queue-drops-jobs");
    assert_eq!(b(&["feature"]).as_str(), "feat/142-retry-queue-drops-jobs");
    assert_eq!(b(&[]).as_str(), "feat/142-retry-queue-drops-jobs");
}

#[test]
fn branch_slug_is_ascii_and_may_be_empty() {
    assert_eq!(branch_for(&issue(7, "Größe `--json` ändern!", &[])).as_str(), "feat/7-gr-e-json-ndern");
    assert_eq!(branch_for(&issue(9, "日本語 ?!", &["bug"])).as_str(), "fix/9");
}

#[test]
fn prompt_asks_for_a_ready_pr_not_a_draft() {
    assert_eq!(
        prompt_for(&issue(142, "Retry queue drops jobs", &[])),
        "Implement GitHub issue #142: Retry queue drops jobs\n\n\
             Read the issue and its comments with `gh issue view 142 --comments`.\n\
             Add or update tests. Open a ready-for-review PR (not a draft) that closes #142 when the tests pass."
    );
}

#[test]
fn defaults_combine_the_rules() {
    let d = start_defaults(&issue(3, "Fix it", &["bug"]));
    assert_eq!(d.branch.as_str(), "fix/3-fix-it");
    assert_eq!(d.mode, Mode::Implement);
    assert!(d.prompt.starts_with("Implement GitHub issue #3: Fix it\n\n"));
    assert!(d.prompt.contains("`gh issue view 3 --comments`"));
}

#[test]
fn a_pr_prompt_names_the_pr_and_the_gh_commands_and_no_branch() {
    let pr = pr(7, "dep-bump", &[]);
    assert_eq!(
        prompt_for_pr(&pr),
        "Work on GitHub PR #7: PR 7\n\n\
         Read the PR and its comments with `gh pr view 7 --comments`, and its diff with `gh pr diff 7`. \
         This worktree is on the PR's branch. Review the change, then fix what needs fixing, add or update tests, \
         and push to the PR's branch."
    );
    assert!(!prompt_for_pr(&pr).contains("dep-bump"));
}
