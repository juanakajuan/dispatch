//! Domain types. Everything here is already validated. Wire, git, and tmux
//! text never reaches this module; `github`, `git`, `tmux`, and `config`
//! parse into these types at the boundary.

use std::path::PathBuf;

/// GitHub issue or PR number. Issues and PRs share one number space on
/// GitHub, but keeping two types stops "PR #151" being used as an issue key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueNumber(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrNumber(pub u32);

/// A branch name that passes the subset of `git check-ref-format --branch`
/// rules that matter here: non-empty, no space, no `..`, no `~^:?*[\`,
/// no leading `-` or `/`, no trailing `/` or `.lock`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    pub fn parse(raw: &str) -> Result<BranchName, BranchNameError> {
        // TODO: trim; reject per the rules above; keep the string as is.
        todo!()
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BranchNameError {
    Empty,
    BadChar(char),
    BadSequence(&'static str),
}

/// A skill name as typed after `/`, for example `pstack:tdd`.
/// Invariant: matches `[A-Za-z0-9_-]+(:[A-Za-z0-9_-]+)?`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillName(String);

impl SkillName {
    pub fn parse(raw: &str) -> Result<SkillName, String> {
        todo!()
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `None` in the UI is "automatic": no `/<skill>` prefix.
pub type SkillChoice = Option<SkillName>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    PlanFirst,
    Implement,
}

/// Absolute, canonical path of a git worktree. This is the join key between
/// git (worktree list) and tmux (`@dispatch-worktree` window option).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorktreePath(PathBuf);

impl WorktreePath {
    /// Canonicalizes. Both sides of the join go through this, so a trailing
    /// slash or a symlinked parent can not split one session in two.
    pub fn from_observed(raw: &str) -> Option<WorktreePath> {
        todo!()
    }
    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub number: IssueNumber,
    pub title: String,
    pub labels: Vec<String>,
    pub author: String,
    pub created_at: jiff::Timestamp,
    pub comment_count: usize,
    pub body: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// Rolled up from all checks: any failure wins, then any pending, then pass.
/// `None` when the PR has no checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    None,
    Pending,
    Pass,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    pub number: PrNumber,
    pub title: String,
    pub state: PrState,
    pub author: String,
    pub created_at: jiff::Timestamp,
    pub head: String,
    pub base: String,
    pub additions: u32,
    pub deletions: u32,
    pub checks: Checks,
    pub closes: Vec<IssueNumber>,
    pub url: String,
}
