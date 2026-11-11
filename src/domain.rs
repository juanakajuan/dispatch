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
        if let Some(c) = name
            .chars()
            .find(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(*c))
        {
            return Err(BranchNameError::BadChar(c));
        }
        let bad_sequences = [
            (name.starts_with('-'), "a leading -"),
            (name.starts_with('/'), "a leading /"),
            (name.ends_with('/'), "a trailing /"),
            (name.ends_with('.'), "a trailing ."),
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

/// A skill name as typed after `/`, for example `pstack:tdd`.
/// Invariant: matches `[A-Za-z0-9_-]+(:[A-Za-z0-9_-]+)?`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillName(String);

impl SkillName {
    pub fn parse(raw: &str) -> Result<SkillName, String> {
        let valid_part = |part: &str| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };
        let mut parts = raw.split(':');
        let ok = match (parts.next(), parts.next(), parts.next()) {
            (Some(a), None, _) => valid_part(a),
            (Some(a), Some(b), None) => valid_part(a) && valid_part(b),
            _ => false,
        };
        if ok {
            Ok(SkillName(raw.to_string()))
        } else {
            Err(format!("invalid skill name {raw:?}, expected name or plugin:name"))
        }
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

/// Where the border between the list and the detail pane sits: the list
/// pane's share of the body width, in thousandths. Thousandths, not percent,
/// so a drag puts the border on the exact column under the pointer.
///
/// The stored value is the user's intent, not a width. `list_width` clamps it
/// to the terminal on read, so a terminal that shrinks and grows back gets
/// the same split, and nothing has to react to a terminal resize.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split(u16); // invariant: 0..=1000

impl Split {
    pub const DEFAULT: Split = Split(420);
    /// What `<` and `>` move: 5% of the body.
    const STEP: u16 = 50;
    /// Narrowest list pane: marker, number, PR mark, and the start of a title.
    pub const MIN_LIST: u16 = 24;
    /// Narrowest detail pane: border, padding, and a readable value column.
    pub const MIN_DETAIL: u16 = 30;
    pub const PERCENT: std::ops::RangeInclusive<u64> = 10..=90;

    /// The config boundary. `None` outside `PERCENT`.
    pub fn from_percent(p: u64) -> Option<Split> {
        Self::PERCENT.contains(&p).then(|| Split(u16::try_from(p * 10).unwrap_or(1000)))
    }

    /// The list pane's width in a body `body` columns wide; the detail pane
    /// gets the rest. The one place a split becomes columns. Each pane keeps
    /// its minimum while the body has room for both; a narrower body shares
    /// out its columns in proportion to the minimums, so neither pane vanishes.
    pub fn list_width(self, body: u16) -> u16 {
        let wide = u32::from(body);
        if body < Self::MIN_LIST + Self::MIN_DETAIL {
            let share = wide * u32::from(Self::MIN_LIST) / u32::from(Self::MIN_LIST + Self::MIN_DETAIL);
            return u16::try_from(share).unwrap_or(0).max(body.min(1)).min(body.saturating_sub(1));
        }
        let want = (wide * u32::from(self.0) + 500) / 1000;
        u16::try_from(want).unwrap_or(u16::MAX).clamp(Self::MIN_LIST, body - Self::MIN_DETAIL)
    }

    /// The split that draws the border `list` columns into a body `body`
    /// wide, with `list` held to the bounds `list_width` keeps. Inverse of
    /// `list_width` for bodies up to 1000 columns: it picks the smallest share
    /// that rounds to `list`.
    pub fn at_column(body: u16, list: u16) -> Split {
        if body == 0 {
            return Split::DEFAULT;
        }
        let list = if body < Self::MIN_LIST + Self::MIN_DETAIL {
            list.min(body)
        } else {
            list.clamp(Self::MIN_LIST, body - Self::MIN_DETAIL)
        };
        let need = (u32::from(list) * 1000).saturating_sub(500);
        let share = need.div_ceil(u32::from(body)).min(1000);
        Split(u16::try_from(share).unwrap_or(1000))
    }

    /// One step toward the detail pane (`>`). Steps from the border as drawn,
    /// so a press at a minimum banks no overshoot that the next press must undo.
    pub fn wider(self, body: u16) -> Split {
        self.stepped(body, |share| share.saturating_add(Self::STEP).min(1000))
    }

    /// One step toward the list pane (`<`).
    pub fn narrower(self, body: u16) -> Split {
        self.stepped(body, |share| share.saturating_sub(Self::STEP))
    }

    fn stepped(self, body: u16, step: impl Fn(u16) -> u16) -> Split {
        if body < Self::MIN_LIST + Self::MIN_DETAIL {
            return self; // no room to move the border
        }
        let shown = Split::at_column(body, self.list_width(body));
        Split::at_column(body, Split(step(shown.0)).list_width(body))
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
    fn skill_name_shapes() {
        for ok in ["pstack:tdd", "tdd", "a_b-c:d_e"] {
            assert!(SkillName::parse(ok).is_ok(), "{ok}");
        }
        for bad in ["", ":", "a:", ":a", "a:b:c", "a b", "/tdd", "a/b"] {
            assert!(SkillName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn worktree_path_needs_an_existing_dir() {
        assert!(WorktreePath::from_observed("/definitely/not/here").is_none());
        let a = WorktreePath::from_observed("/tmp");
        let b = WorktreePath::from_observed("/tmp/");
        assert!(a.is_some());
        assert_eq!(a, b);
    }

    #[test]
    fn split_widths_keep_both_panes_on_every_terminal() {
        for body in 0..=400u16 {
            for share in (0..=1000).step_by(5) {
                let list = Split(share).list_width(body);
                assert!(list <= body);
                if body >= 2 {
                    assert!(list >= 1 && body - list >= 1, "body {body} share {share}: list {list}");
                }
                if body >= Split::MIN_LIST + Split::MIN_DETAIL {
                    assert!(list >= Split::MIN_LIST && body - list >= Split::MIN_DETAIL);
                }
            }
        }
        assert_eq!(Split::DEFAULT.list_width(100), 42);
    }

    #[test]
    fn a_drag_puts_the_border_on_the_column_under_the_pointer() {
        for body in Split::MIN_LIST + Split::MIN_DETAIL..=1000 {
            for list in Split::MIN_LIST..=body - Split::MIN_DETAIL {
                assert_eq!(Split::at_column(body, list).list_width(body), list, "body {body}");
            }
            assert_eq!(Split::at_column(body, 0).list_width(body), Split::MIN_LIST);
            assert_eq!(Split::at_column(body, body).list_width(body), body - Split::MIN_DETAIL);
        }
    }

    #[test]
    fn steps_move_five_percent_and_stop_at_the_minimums() {
        let body = 100;
        assert_eq!(Split::DEFAULT.wider(body).list_width(body), 47);
        assert_eq!(Split::DEFAULT.narrower(body).list_width(body), 37);
        assert_eq!(Split::DEFAULT.wider(body).narrower(body).list_width(body), 42);
        let mut s = Split::DEFAULT;
        for _ in 0..30 {
            s = s.narrower(body);
        }
        assert_eq!(s.list_width(body), Split::MIN_LIST);
        assert_eq!(s.wider(body).list_width(body), 29, "no banked overshoot at the minimum");
        for _ in 0..30 {
            s = s.wider(body);
        }
        assert_eq!(s.list_width(body), body - Split::MIN_DETAIL);
        assert_eq!(Split::DEFAULT.wider(40), Split::DEFAULT, "no room to move on a tiny body");
    }

    #[test]
    fn the_split_survives_a_shrink_and_grow() {
        let s = Split::from_percent(80).expect("in range");
        assert_eq!(s.list_width(60), 30);
        assert_eq!(s.list_width(200), 160);
        assert_eq!(Split::from_percent(9), None);
        assert_eq!(Split::from_percent(91), None);
    }
}
