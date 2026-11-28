//! Domain types. Everything here is already validated. Wire, git, and tmux
//! text never reaches this module; `github`, `git`, `tmux`, and `config`
//! parse into these types at the boundary.

use std::path::PathBuf;

mod branch_name;
mod session_key;

pub use branch_name::{BranchName, BranchNameError, OriginRef};
pub use session_key::SessionKey;

/// GitHub issue or PR number. Issues and PRs share one number space on
/// GitHub, but keeping two types stops "PR #151" being used as an issue key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueNumber(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrNumber(pub u32);

impl std::fmt::Display for IssueNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

impl std::fmt::Display for PrNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

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
            GithubRef::Issue(n) => write!(f, "issue {n}"),
            GithubRef::Pr(n) => write!(f, "PR {n}"),
        }
    }
}

/// A PR's head or base branch as GitHub names it. A head that is a bad
/// branch name stays `Unusable`: the PR still lists and can merge, but no
/// session can start on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrBranch {
    Valid(BranchName),
    /// The head of a PR from a fork. `origin/<branch>` is some other branch,
    /// or none, so a session uses the local branch `local`.
    Fork(ForkHead),
    Unusable {
        raw: String,
        why: BranchNameError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkHead {
    /// The branch in the fork.
    pub branch: BranchName,
    pub local: BranchName,
    pub push: ForkPush,
}

/// Whether a maintainer of the base repo can push to the fork's branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkPush {
    Allowed(RepoSlug),
    /// The PR does not allow edits from maintainers, or the fork is gone.
    Denied,
}

impl PrBranch {
    pub fn parse(raw: String) -> PrBranch {
        match BranchName::parse(&raw) {
            Ok(name) => PrBranch::Valid(name),
            Err(why) => PrBranch::Unusable { raw, why },
        }
    }

    /// The head of PR `pr` from a fork. A bad branch name is never used,
    /// in a fork too.
    pub fn fork(pr: PrNumber, raw: String, push: ForkPush) -> PrBranch {
        match BranchName::parse(&raw) {
            Ok(branch) => PrBranch::Fork(ForkHead { branch, local: BranchName::fork_pr(pr), push }),
            Err(why) => PrBranch::Unusable { raw, why },
        }
    }

    /// The local branch a session on this head uses.
    pub fn branch(&self) -> Option<&BranchName> {
        match self {
            PrBranch::Valid(name) => Some(name),
            PrBranch::Fork(fork) => Some(&fork.local),
            PrBranch::Unusable { .. } => None,
        }
    }

    /// The name as GitHub shows it.
    pub fn display(&self) -> &str {
        match self {
            PrBranch::Valid(name) => name.as_str(),
            PrBranch::Fork(fork) => fork.branch.as_str(),
            PrBranch::Unusable { raw, .. } => raw,
        }
    }
}

/// `owner/name` of a GitHub repo: owner of ASCII letters, digits, and `-`;
/// name of ASCII letters, digits, `.`, `_`, and `-`, and not `.` or `..`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSlug {
    owner: String,
    name: String,
}

impl RepoSlug {
    pub fn parse(raw: &str) -> Option<RepoSlug> {
        let (owner, name) = raw.split_once('/')?;
        let owner_ok = !owner.is_empty() && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let name_ok =
            !matches!(name, "" | "." | "..") && name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
        (owner_ok && name_ok).then(|| RepoSlug { owner: owner.to_string(), name: name.to_string() })
    }
}

impl std::fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// The web page of an issue or PR, as `gh` reports it. Only `https://` and
/// printable ASCII with no space, so the URL can go inside a terminal escape
/// sequence (OSC 8) and never end it early.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageUrl(String);

impl PageUrl {
    pub fn parse(raw: &str) -> Option<PageUrl> {
        let ok =
            raw.len() > "https://".len() && raw.starts_with("https://") && raw.bytes().all(|b| b.is_ascii_graphic());
        ok.then(|| PageUrl(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A commit SHA as GitHub reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitOid(String);

impl CommitOid {
    pub fn new(sha: String) -> CommitOid {
        CommitOid(sha)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    PlanFirst,
    #[default]
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

    /// For a window's tag, which outlives a dir that the user deleted by
    /// hand. A gone dir keeps its canonical parent, so a start still finds
    /// the window and closes it. `None` when the parent is gone too.
    pub fn from_tag(raw: &str) -> Option<WorktreePath> {
        let path = std::path::Path::new(raw);
        WorktreePath::from_observed(raw).or_else(|| {
            let parent = WorktreePath::from_observed(path.parent()?.to_str()?)?;
            Some(WorktreePath(parent.0.join(path.file_name()?)))
        })
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
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub author: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
    pub comment_count: usize,
    pub url: Option<PageUrl>,
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
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub author: String,
    pub created_at: jiff::Timestamp,
    pub updated_at: jiff::Timestamp,
    pub head: PrBranch,
    /// A direct merge pins it, so a push after the check can not merge
    /// unchecked code.
    pub head_oid: CommitOid,
    pub base: PrBranch,
    pub additions: u32,
    pub deletions: u32,
    pub checks: Checks,
    pub mergeability: Mergeability,
    pub closes: Vec<IssueNumber>,
    pub url: Option<PageUrl>,
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
    fn pr_branch_keeps_a_bad_name_for_display() {
        let good = PrBranch::parse("fix/3-x".to_string());
        assert_eq!((good.branch().map(BranchName::as_str), good.display()), (Some("fix/3-x"), "fix/3-x"));
        let bad = PrBranch::parse("a b".to_string());
        assert_eq!((bad.branch(), bad.display()), (None, "a b"));
    }

    #[test]
    fn a_fork_head_gets_a_local_branch_and_a_bad_fork_name_stays_unusable() {
        let fork = PrBranch::fork(PrNumber(7), "main".to_string(), ForkPush::Denied);
        assert_eq!((fork.branch().map(BranchName::as_str), fork.display()), (Some("fork/pr-7"), "main"));
        let bad = PrBranch::fork(PrNumber(7), "a..b".to_string(), ForkPush::Denied);
        assert_eq!((bad.branch(), bad.display()), (None, "a..b"));
    }

    #[test]
    fn repo_slug_takes_only_github_owner_and_name_characters() {
        let slug = RepoSlug::parse("max-beizer/cli.rs_2").expect("slug");
        assert_eq!(slug.to_string(), "max-beizer/cli.rs_2");
        for bad in ["cli", "/cli", "o/", "o/..", "o/a/b", "o_x/cli", "o/a b", "o/-x;rm"] {
            assert_eq!(RepoSlug::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn numbers_display_with_a_hash() {
        assert_eq!((IssueNumber(7).to_string(), PrNumber(12).to_string()), ("#7".to_string(), "#12".to_string()));
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
