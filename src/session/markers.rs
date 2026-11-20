//! `git config branch.<b>.dispatch-*`: what dispatch keeps on a branch.

use super::git::git;
use crate::config::Harness;
use crate::domain::{BranchName, IssueNumber};
use crate::proc::ProcError;
use std::collections::BTreeMap;
use std::path::Path;

/// branch -> issue, from `dispatch-issue`. Only values that parsed.
pub(super) type Markers = BTreeMap<BranchName, IssueNumber>;

/// branch -> harness, from `dispatch-harness`. Only values dispatch knows;
/// a branch without one runs Claude.
pub(super) type Harnesses = BTreeMap<BranchName, Harness>;

/// Everything dispatch keeps on a branch in `.git/config`.
#[derive(Debug, Default)]
pub(super) struct Marked {
    pub issues: Markers,
    pub harnesses: Harnesses,
}

pub(super) fn list_markers(root: &Path) -> Result<Marked, ProcError> {
    match git(root, ["config", "--get-regexp", r"^branch\..*\.dispatch-(issue|harness)$"]) {
        Ok(t) => Ok(parse_markers(&t)),
        Err(e) if e.exit_code() == Some(1) => Ok(Marked::default()),
        Err(e) => Err(e),
    }
}

/// Lines like `branch.fix/142-retry-queue.dispatch-issue 142` and
/// `branch.fix/142-retry-queue.dispatch-harness codex`.
/// The branch is everything between `branch.` and the last `.`; the value is
/// everything after the first space.
fn parse_markers(text: &str) -> Marked {
    let mut marked = Marked::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        let Some(rest) = key.strip_prefix("branch.") else { continue };
        let value = value.trim();
        if let Some(b) = rest.strip_suffix(".dispatch-issue")
            && let (Ok(branch), Ok(issue)) = (BranchName::parse(b), value.parse())
        {
            marked.issues.insert(branch, IssueNumber(issue));
        } else if let Some(b) = rest.strip_suffix(".dispatch-harness")
            && let (Ok(branch), Some(harness)) = (BranchName::parse(b), Harness::from_label(value))
        {
            marked.harnesses.insert(branch, harness);
        }
    }
    marked
}

pub(super) fn set_marker(root: &Path, branch: &BranchName, issue: IssueNumber) -> Result<(), ProcError> {
    let key = format!("branch.{}.dispatch-issue", branch.as_str());
    git(root, ["config".to_string(), key, issue.0.to_string()]).map(|_| ())
}

pub(super) fn set_harness(root: &Path, branch: &BranchName, harness: Harness) -> Result<(), ProcError> {
    let key = format!("branch.{}.dispatch-harness", branch.as_str());
    git(root, ["config".to_string(), key, harness.label().to_string()]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;

    fn branch(s: &str) -> BranchName {
        BranchName::parse(s).expect("valid branch")
    }

    #[test]
    fn markers_from_real_regexp_output() {
        let text = "branch.fix/1-a.b-c.dispatch-issue 1\n\
                    branch.feat/2-x.dispatch-issue abc\n\
                    branch.feat/3.dispatch-issue 3\n";
        let m = parse_markers(text).issues;
        assert_eq!(m.len(), 2);
        assert_eq!(m[&branch("fix/1-a.b-c")], IssueNumber(1));
        assert_eq!(m[&branch("feat/3")], IssueNumber(3));
        assert!(!m.contains_key(&branch("feat/2-x")), "non-numeric issue is dropped");
    }

    #[test]
    fn harness_markers_parse_and_unknown_values_are_dropped() {
        let text = "branch.fix/1-a.dispatch-issue 1\n\
                    branch.fix/1-a.dispatch-harness codex\n\
                    branch.feat/2-x.dispatch-harness gemini\n\
                    branch.feat/3.dispatch-harness opencode\n";
        let marked = parse_markers(text);
        assert_eq!(marked.issues.len(), 1);
        assert_eq!(marked.harnesses.len(), 2);
        assert_eq!(marked.harnesses[&branch("fix/1-a")], Harness::Codex);
        assert_eq!(marked.harnesses[&branch("feat/3")], Harness::OpenCode);
    }

    #[test]
    fn set_harness_writes_the_label() {
        fake::install(|_| Ok(String::new()));
        set_harness(Path::new("/"), &branch("fix/1-a"), Harness::Antigravity).expect("ok");
        assert_eq!(fake::calls(), ["git config branch.fix/1-a.dispatch-harness antigravity"]);
    }

    #[test]
    fn marker_list_exit_one_is_empty() {
        fake::install(|_| Err(fake::fail("git", 1, "")));
        let marked = list_markers(Path::new("/")).expect("ok");
        assert!(marked.issues.is_empty() && marked.harnesses.is_empty());
    }
}
