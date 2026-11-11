//! git adapter. Private to `session`. Parsers are pure and tested on text
//! captured from git 2.56.

use crate::domain::{BranchName, IssueNumber, SkillChoice, WorktreePath};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) struct Worktree {
    pub path: WorktreePath,
    /// `None` for a detached HEAD or a bare entry.
    pub branch: Option<BranchName>,
}

pub(super) struct Marker {
    pub issue: IssueNumber,
    pub skill: SkillChoice,
}

/// branch -> marker. Only branches whose `dispatch-issue` parsed.
pub(super) type Markers = BTreeMap<BranchName, Marker>;

pub(super) fn repo_root(cwd: &Path) -> Result<PathBuf, String> {
    // TODO: git rev-parse --show-toplevel
    todo!()
}

pub(super) fn origin_head(root: &Path) -> Result<String, String> {
    // TODO: git symbolic-ref --short refs/remotes/origin/HEAD -> "origin/main".
    //       Missing ref: tell the user to run `git remote set-head origin --auto`.
    todo!()
}

pub(super) fn list_worktrees(root: &Path) -> Result<Vec<Worktree>, String> {
    // TODO: git worktree list --porcelain, then parse_worktrees.
    todo!()
}

/// Porcelain blocks separated by blank lines:
/// `worktree <path>` / `HEAD <sha>` / `branch refs/heads/<name>` | `detached` | `bare`.
fn parse_worktrees(text: &str) -> Vec<Worktree> {
    todo!()
}

pub(super) fn list_markers(root: &Path) -> Result<Markers, String> {
    // TODO: git config --get-regexp '^branch\..*\.dispatch-(issue|skill)$'
    //       Exit code 1 means no match -> empty map. Then parse_markers.
    todo!()
}

/// Lines like `branch.fix/142-retry-queue.dispatch-issue 142`.
/// The branch is everything between `branch.` and the last `.`; the value is
/// everything after the first space. Skill without issue -> dropped.
fn parse_markers(text: &str) -> Markers {
    todo!()
}

pub(super) fn branch_exists(root: &Path, branch: &BranchName) -> Result<bool, String> {
    // TODO: git show-ref --verify --quiet refs/heads/<b>; exit 1 -> false.
    todo!()
}

pub(super) fn fetch(root: &Path, base: &str) -> Result<(), String> {
    // TODO: split "origin/main" -> git fetch origin main
    todo!()
}

pub(super) fn add_worktree(root: &Path, dir: &Path, branch: &BranchName, create_from: Option<&str>) -> Result<(), String> {
    // TODO: create_from Some(base) -> git worktree add <dir> -b <branch> <base>
    //       None                   -> git worktree add <dir> <branch>
    todo!()
}

pub(super) fn set_marker(root: &Path, branch: &BranchName, issue: IssueNumber, skill: &SkillChoice) -> Result<(), String> {
    // TODO: git config branch.<b>.dispatch-issue <n>
    //       skill Some -> git config branch.<b>.dispatch-skill <s>
    //       skill None -> git config --unset branch.<b>.dispatch-skill (exit 5 = absent, ok)
    todo!()
}

#[cfg(test)]
mod tests {
    // TODO: parse_worktrees on main + one linked + one detached block.
    // TODO: parse_markers with a slash branch, a non-numeric issue (dropped),
    //       skill-only branch (dropped), and a branch name that contains dots.
}
