use super::{IssueNumber, PrNumber};

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
            (name.starts_with('-'), "can not start with \"-\""),
            (name.starts_with('/'), "can not start with \"/\""),
            (name.ends_with('/'), "can not end with \"/\""),
            (name.ends_with('.'), "can not end with \".\""),
            #[expect(
                clippy::case_sensitive_file_extension_comparisons,
                reason = "git ref names are case-sensitive and `.lock` is no file extension"
            )]
            (name.ends_with(".lock"), "can not end with \".lock\""),
            (name.contains(".."), "can not have \"..\""),
            (name.contains("//"), "can not have \"//\""),
            (name.contains("@{"), "can not have \"@{\""),
            (name == "@", "can not be \"@\""),
            (name.split('/').any(|part| part.starts_with('.')), "can not have a part that starts with \".\""),
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

    /// `fork/pr-<n>`, valid by construction. The local branch of a session
    /// on the head of a PR from a fork.
    pub fn fork_pr(pr: PrNumber) -> BranchName {
        BranchName(format!("fork/pr-{}", pr.0))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A branch on the `origin` remote. Displays as `origin/<branch>`, the form
/// that `git symbolic-ref --short` prints and `git worktree add` takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginRef(BranchName);

impl OriginRef {
    pub fn of(branch: BranchName) -> OriginRef {
        OriginRef(branch)
    }

    /// `origin/<branch>`, with a valid branch.
    pub fn parse(raw: &str) -> Option<OriginRef> {
        let branch = raw.trim().strip_prefix("origin/")?;
        BranchName::parse(branch).ok().map(OriginRef)
    }

    pub fn branch(&self) -> &BranchName {
        &self.0
    }
}

impl std::fmt::Display for OriginRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "origin/{}", self.0.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchNameError {
    Empty,
    BadChar(char),
    /// The rule the name breaks, as a predicate: `can not have ".."`.
    BadSequence(&'static str),
}

/// A sentence for the user, such as `The branch name can not have a space.`
impl std::fmt::Display for BranchNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BranchNameError::Empty => write!(f, "The branch name is empty."),
            BranchNameError::BadChar(' ') => write!(f, "The branch name can not have a space."),
            BranchNameError::BadChar('\t') => write!(f, "The branch name can not have a tab."),
            BranchNameError::BadChar(c) if c.is_whitespace() || c.is_control() => {
                write!(f, "The branch name can not have the character {}.", c.escape_unicode())
            }
            BranchNameError::BadChar(c) => write!(f, "The branch name can not have \"{c}\"."),
            BranchNameError::BadSequence(rule) => write!(f, "The branch name {rule}."),
        }
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
    fn each_bad_branch_name_has_a_sentence() {
        let cases = [
            ("", "The branch name is empty."),
            ("bad name", "The branch name can not have a space."),
            ("a\tb", "The branch name can not have a tab."),
            ("a\u{7}b", "The branch name can not have the character \\u{7}."),
            ("a~b", "The branch name can not have \"~\"."),
            ("a\\b", "The branch name can not have \"\\\"."),
            ("-x", "The branch name can not start with \"-\"."),
            ("/x", "The branch name can not start with \"/\"."),
            ("x/", "The branch name can not end with \"/\"."),
            ("x.", "The branch name can not end with \".\"."),
            ("x.lock", "The branch name can not end with \".lock\"."),
            ("..x", "The branch name can not have \"..\"."),
            ("a//b", "The branch name can not have \"//\"."),
            ("a@{b", "The branch name can not have \"@{\"."),
            ("@", "The branch name can not be \"@\"."),
            ("a/.b", "The branch name can not have a part that starts with \".\"."),
        ];
        for (raw, want) in cases {
            assert_eq!(BranchName::parse(raw).map_err(|e| e.to_string()), Err(want.to_string()), "{raw:?}");
        }
    }

    #[test]
    fn origin_ref_needs_the_origin_prefix_and_a_valid_branch() {
        let main = OriginRef::parse("origin/main\n").expect("origin/main");
        assert_eq!(main.branch().as_str(), "main");
        assert_eq!(main.to_string(), "origin/main");
        assert_eq!(OriginRef::parse("origin/release/1.0").map(|r| r.to_string()), Some("origin/release/1.0".into()));
        for bad in ["main", "upstream/main", "origin/", "origin/a..b"] {
            assert_eq!(OriginRef::parse(bad), None, "{bad}");
        }
    }
}
