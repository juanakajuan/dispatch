use crate::session::worktree::{WORKTREES_DIR, WORKTREES_EXCLUDE, worktree_dir_for};
use crate::testutil::{issue_key, pr_key};
use std::path::Path;

#[test]
fn worktree_dir_is_inside_the_repo() {
    assert_eq!(worktree_dir_for(Path::new("/a/orbit"), issue_key(142)), Path::new("/a/orbit/.claude/worktrees/142"));
    assert!(Path::new(WORKTREES_EXCLUDE).starts_with(WORKTREES_DIR));
}

#[test]
fn a_pr_session_has_its_own_dir_so_pr_7_and_issue_7_never_share_one() {
    let root = Path::new("/a/orbit");
    assert_eq!(worktree_dir_for(root, pr_key(7)), Path::new("/a/orbit/.claude/worktrees/pr-7"));
    assert_ne!(worktree_dir_for(root, pr_key(7)), worktree_dir_for(root, issue_key(7)));
}
