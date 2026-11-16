use crate::domain::IssueNumber;
use crate::session::worktree::{WORKTREES_DIR, WORKTREES_EXCLUDE, worktree_dir_for};
use std::path::Path;

#[test]
fn worktree_dir_is_inside_the_repo() {
    assert_eq!(worktree_dir_for(Path::new("/a/orbit"), IssueNumber(142)), Path::new("/a/orbit/.claude/worktrees/142"));
    assert!(Path::new(WORKTREES_EXCLUDE).starts_with(WORKTREES_DIR));
}
