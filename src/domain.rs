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

/// Something with a page on GitHub. `gh` resolves the host, so this holds
/// the number only, never a URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubRef {
    Issue(IssueNumber),
    Pr(PrNumber),
}

impl std::fmt::Display for GithubRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GithubRef::Issue(n) => write!(f, "issue #{}", n.0),
            GithubRef::Pr(n) => write!(f, "PR #{}", n.0),
        }
    }
}

/// A branch name that passes the subset of `git check-ref-format --branch`
/// rules that matter here: non-empty, no whitespace or control character, no
/// `..`, `//`, or `@{`, none of `~^:?*[\`, no leading `-` or `/`, no trailing
/// `/`, `.`, or `.lock`, and no path part that starts with `.`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BranchName(String);

impl BranchName {
    pub fn parse(raw: &str) -> Result<BranchName, BranchNameError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(BranchNameError::Empty);
        }
        if let Some(c) = name.chars().find(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(*c)) {
            return Err(BranchNameError::BadChar(c));
        }
        let bad_sequences = [
            (name.starts_with('-'), "a leading -"),
            (name.starts_with('/'), "a leading /"),
            (name.ends_with('/'), "a trailing /"),
            (name.ends_with('.'), "a trailing ."),
            #[expect(
                clippy::case_sensitive_file_extension_comparisons,
                reason = "git ref names are case-sensitive and `.lock` is no file extension"
            )]
            (name.ends_with(".lock"), "a trailing .lock"),
            (name.contains(".."), ".."),
            (name.contains("//"), "//"),
            (name.contains("@{"), "@{"),
            (name == "@", "a bare @"),
            (name.split('/').any(|part| part.starts_with('.')), "a part that starts with ."),
        ];
        match bad_sequences.iter().find(|(bad, _)| *bad) {
            Some((_, what)) => Err(BranchNameError::BadSequence(what)),
            None => Ok(BranchName(name.to_string())),
        }
    }

    /// `issue/<n>`, valid by construction. The fallback when a generated
    /// name fails `parse`.
    pub fn numbered(issue: IssueNumber) -> BranchName {
        BranchName(format!("issue/{}", issue.0))
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
        std::fs::canonicalize(raw).ok().map(WorktreePath)
    }

    /// For a dir that a start plan will create. The caller builds `path` from
    /// an already canonical parent, so it is canonical without touching the
    /// filesystem and the planner stays pure.
    pub fn planned(path: PathBuf) -> WorktreePath {
        WorktreePath(path)
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
    pub updated_at: jiff::Timestamp,
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
    pub updated_at: jiff::Timestamp,
    pub head: String,
    /// The head commit SHA. A direct merge pins it, so a push after the
    /// check can not merge unchecked code.
    pub head_oid: String,
    pub base: String,
    pub additions: u32,
    pub deletions: u32,
    pub checks: Checks,
    pub mergeability: Mergeability,
    pub closes: Vec<IssueNumber>,
    pub url: String,
}

/// Whether GitHub can merge an open PR, from its `mergeable` and
/// `mergeStateStatus`. `Unknown` while GitHub still computes it, and for a PR
/// that is not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mergeability {
    Clean,
    Conflict,
    /// The repo requires an up-to-date branch and this one is behind its base.
    Behind,
    /// A branch rule, such as a required review, blocks the merge.
    Blocked,
    /// Mergeable, but a check that is not required fails.
    Unstable,
    Unknown,
}

/// The `gh pr merge` flag, without its dashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn parse(raw: &str) -> Option<MergeMethod> {
        match raw.trim() {
            "merge" => Some(MergeMethod::Merge),
            "squash" => Some(MergeMethod::Squash),
            "rebase" => Some(MergeMethod::Rebase),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

impl std::fmt::Display for MergeMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_accepts_typical_names() {
        for ok in ["fix/142-retry-queue", "feat/7", "a.b/c-d", " main "] {
            assert!(BranchName::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(BranchName::parse(" main ").map(|b| b.as_str().to_string()), Ok("main".to_string()));
    }

    #[test]
    fn branch_rejects_bad_names() {
        assert_eq!(BranchName::parse("  "), Err(BranchNameError::Empty));
        assert_eq!(BranchName::parse("a b"), Err(BranchNameError::BadChar(' ')));
        assert_eq!(BranchName::parse("a~b"), Err(BranchNameError::BadChar('~')));
        assert_eq!(BranchName::parse("a\\b"), Err(BranchNameError::BadChar('\\')));
        for bad in ["-x", "/x", "x/", "x.", "x.lock", "a..b", "a//b", "a@{b", "@", "a/.b"] {
            assert!(matches!(BranchName::parse(bad), Err(BranchNameError::BadSequence(_))), "{bad}");
        }
    }

    #[test]
    fn merge_method_round_trips_and_rejects_other_text() {
        for m in [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase] {
            assert_eq!(MergeMethod::parse(m.as_str()), Some(m));
        }
        assert_eq!(MergeMethod::parse(" squash\n"), Some(MergeMethod::Squash));
        assert_eq!(MergeMethod::parse("Squash"), None);
        assert_eq!(MergeMethod::parse("fast-forward"), None);
    }

    #[test]
    fn worktree_path_needs_an_existing_dir() {
        assert!(WorktreePath::from_observed("/definitely/not/here").is_none());
        let a = WorktreePath::from_observed("/tmp");
        let b = WorktreePath::from_observed("/tmp/");
        assert!(a.is_some());
        assert_eq!(a, b);
    }
}
