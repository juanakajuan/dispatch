use crate::domain::SessionKey;
use std::path::{Path, PathBuf};

/// Where session worktrees live, relative to the repo root. The same place
/// Claude Code uses for its own worktrees.
pub(super) const WORKTREES_DIR: &str = ".claude/worktrees";

/// The `.git/info/exclude` line that hides `WORKTREES_DIR` from `git status`
/// in the main checkout.
pub(super) const WORKTREES_EXCLUDE: &str = ".claude/worktrees/";

/// `<repo root>/.claude/worktrees/<n>` for an issue, `.../pr-<n>` for a PR.
/// Deterministic per key: the path itself is a second guard against two
/// worktrees for one session, because `git worktree add` refuses an
/// existing path.
pub(super) fn worktree_dir_for(repo_root: &Path, key: SessionKey) -> PathBuf {
    repo_root.join(WORKTREES_DIR).join(key.dir_name())
}
