//! `git config branch.<b>.dispatch-*`: what dispatch keeps on a branch.
//! `dispatch-issue <n>` marks an issue session, `dispatch-pr <n>` a PR
//! session, and `dispatch-harness <label>` the harness the session runs.

use super::git::git;
use crate::config::Harness;
use crate::domain::{BranchName, IssueNumber, PrNumber, SessionKey};
use crate::proc::ProcError;
use std::collections::BTreeMap;
use std::path::Path;

/// branch -> session, from `dispatch-issue` and `dispatch-pr`. Only values that parsed.
pub(super) type Markers = BTreeMap<BranchName, SessionKey>;

/// branch -> harness, from `dispatch-harness`. Only values dispatch knows;
/// a branch without one runs Claude.
pub(super) type Harnesses = BTreeMap<BranchName, Harness>;

/// Everything dispatch keeps on a branch in `.git/config`.
#[derive(Debug, Default)]
pub(super) struct Marked {
    pub sessions: Markers,
    pub harnesses: Harnesses,
}

pub(super) fn list_markers(root: &Path) -> Result<Marked, ProcError> {
    match git(root, ["config", "--get-regexp", r"^branch\..*\.dispatch-(issue|pr|harness)$"]) {
        Ok(t) => Ok(parse_markers(&t)),
        Err(e) if e.exit_code() == Some(1) => Ok(Marked::default()),
        Err(e) => Err(e),
    }
}

/// Lines like `branch.fix/142-retry-queue.dispatch-issue 142`,
/// `branch.fix/dep-bump.dispatch-pr 7`, or
/// `branch.fix/142-retry-queue.dispatch-harness codex`. The branch is
/// everything between `branch.` and the marker suffix; the value is
/// everything after the first space. A branch with both session markers is an
/// issue session: the legacy marker wins, whatever the line order.
fn parse_markers(text: &str) -> Marked {
    let mut marked = Marked::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        let Some(rest) = key.strip_prefix("branch.") else { continue };
        let value = value.trim();
        if let Some(b) = rest.strip_suffix(".dispatch-harness") {
            if let (Ok(branch), Some(harness)) = (BranchName::parse(b), Harness::from_label(value)) {
                marked.harnesses.insert(branch, harness);
            }
            continue;
        }
        let Ok(n) = value.parse() else { continue };
        let (name, session) = match (rest.strip_suffix(".dispatch-issue"), rest.strip_suffix(".dispatch-pr")) {
            (Some(b), _) => (b, SessionKey::Issue(IssueNumber(n))),
            (_, Some(b)) => (b, SessionKey::Pr(PrNumber(n))),
            _ => continue,
        };
        let Ok(branch) = BranchName::parse(name) else { continue };
        if !matches!(marked.sessions.get(&branch), Some(SessionKey::Issue(_))) {
            marked.sessions.insert(branch, session);
        }
    }
    marked
}

pub(super) fn set_marker(root: &Path, branch: &BranchName, key: SessionKey) -> Result<(), ProcError> {
    let (suffix, n) = match key {
        SessionKey::Issue(n) => ("issue", n.0),
        SessionKey::Pr(n) => ("pr", n.0),
    };
    git(root, ["config".to_string(), format!("branch.{}.dispatch-{suffix}", branch.as_str()), n.to_string()])
        .map(|_| ())
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
        let m = parse_markers(text).sessions;
        assert_eq!(m.len(), 2);
        assert_eq!(m[&branch("fix/1-a.b-c")], SessionKey::Issue(IssueNumber(1)));
        assert_eq!(m[&branch("feat/3")], SessionKey::Issue(IssueNumber(3)));
        assert!(!m.contains_key(&branch("feat/2-x")), "non-numeric issue is dropped");
    }

    #[test]
    fn pr_markers_parse_and_an_issue_marker_wins_on_one_branch() {
        let text = "branch.chore/dep.dispatch-pr 7\n\
                    branch.both.dispatch-pr 9\n\
                    branch.both.dispatch-issue 3\n\
                    branch.both2.dispatch-issue 4\n\
                    branch.both2.dispatch-pr 8\n\
                    branch.bad.dispatch-pr x\n";
        let m = parse_markers(text).sessions;
        assert_eq!(m[&branch("chore/dep")], SessionKey::Pr(PrNumber(7)));
        assert_eq!(m[&branch("both")], SessionKey::Issue(IssueNumber(3)));
        assert_eq!(m[&branch("both2")], SessionKey::Issue(IssueNumber(4)));
        assert!(!m.contains_key(&branch("bad")));
    }

    #[test]
    fn harness_markers_parse_and_unknown_values_are_dropped() {
        let text = "branch.fix/1-a.dispatch-issue 1\n\
                    branch.fix/1-a.dispatch-harness codex\n\
                    branch.feat/2-x.dispatch-harness gemini\n\
                    branch.feat/3.dispatch-harness opencode\n\
                    branch.feat/4.dispatch-harness pi\n";
        let marked = parse_markers(text);
        assert_eq!(marked.sessions.len(), 1);
        assert_eq!(marked.harnesses.len(), 3);
        assert_eq!(marked.harnesses[&branch("fix/1-a")], Harness::Codex);
        assert_eq!(marked.harnesses[&branch("feat/3")], Harness::OpenCode);
        assert_eq!(marked.harnesses[&branch("feat/4")], Harness::Pi);
    }

    #[test]
    fn set_marker_writes_the_issue_or_pr_key() {
        fake::install(|_| Ok(String::new()));
        set_marker(Path::new("/"), &branch("fix/1-a"), SessionKey::Pr(PrNumber(7))).expect("ok");
        assert_eq!(fake::calls(), ["git config branch.fix/1-a.dispatch-pr 7"]);
    }

    #[test]
    fn set_harness_writes_the_label() {
        fake::install(|_| Ok(String::new()));
        set_harness(Path::new("/"), &branch("fix/1-a"), Harness::Antigravity).expect("ok");
        set_harness(Path::new("/"), &branch("fix/2-b"), Harness::Pi).expect("ok");
        assert_eq!(
            fake::calls(),
            ["git config branch.fix/1-a.dispatch-harness antigravity", "git config branch.fix/2-b.dispatch-harness pi"]
        );
    }

    #[test]
    fn marker_list_exit_one_is_empty() {
        fake::install(|_| Err(fake::fail("git", 1, "")));
        let marked = list_markers(Path::new("/")).expect("ok");
        assert!(marked.sessions.is_empty() && marked.harnesses.is_empty());
    }
}
