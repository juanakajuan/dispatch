use crate::domain::IssueNumber;
use std::path::{Path, PathBuf};

/// Where session worktrees live, relative to the repo root. It is only a
/// folder name, not a Claude dependency: every harness runs here. Moving it
/// would orphan every existing session, because a session is found by its
/// worktree path.
pub(super) const WORKTREES_DIR: &str = ".claude/worktrees";

/// The `.git/info/exclude` line that hides `WORKTREES_DIR` from `git status`
/// in the main checkout.
pub(super) const WORKTREES_EXCLUDE: &str = ".claude/worktrees/";

/// `<repo root>/.claude/worktrees/<issue>`. Deterministic per issue: the
/// path itself is a second guard against two worktrees for one issue,
/// because `git worktree add` refuses an existing path.
pub(super) fn worktree_dir_for(repo_root: &Path, issue: IssueNumber) -> PathBuf {
    repo_root.join(WORKTREES_DIR).join(issue.0.to_string())
}
