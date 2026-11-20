//! Session markers in git config: `branch.<b>.dispatch-issue <n>` for an
//! issue session, `branch.<b>.dispatch-pr <n>` for a PR session.

use crate::domain::{BranchName, IssueNumber, PrNumber, SessionKey};
use crate::proc::{self, ProcError};
use std::collections::BTreeMap;
use std::path::Path;

/// branch -> session, from `dispatch-issue` and `dispatch-pr`. Only values that parsed.
pub(super) type Markers = BTreeMap<BranchName, SessionKey>;

pub(super) fn list_markers(root: &Path) -> Result<Markers, ProcError> {
    match proc::run("git", ["config", "--get-regexp", r"^branch\..*\.dispatch-(issue|pr)$"], root) {
        Ok(t) => Ok(parse_markers(&t)),
        Err(e) if e.exit_code() == Some(1) => Ok(Markers::new()),
        Err(e) => Err(e),
    }
}

/// Lines like `branch.fix/142-retry-queue.dispatch-issue 142` or
/// `branch.fix/dep-bump.dispatch-pr 7`. The branch is everything between
/// `branch.` and the marker suffix; the value is everything after the first
/// space. A branch with both markers is an issue session: the legacy marker
/// wins, whatever the line order.
fn parse_markers(text: &str) -> Markers {
    let mut markers = Markers::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        let Some(rest) = key.strip_prefix("branch.") else { continue };
        let Ok(n) = value.trim().parse() else { continue };
        let (name, session) = match (rest.strip_suffix(".dispatch-issue"), rest.strip_suffix(".dispatch-pr")) {
            (Some(b), _) => (b, SessionKey::Issue(IssueNumber(n))),
            (_, Some(b)) => (b, SessionKey::Pr(PrNumber(n))),
            _ => continue,
        };
        let Ok(branch) = BranchName::parse(name) else { continue };
        if !matches!(markers.get(&branch), Some(SessionKey::Issue(_))) {
            markers.insert(branch, session);
        }
    }
    markers
}

pub(super) fn set_marker(root: &Path, branch: &BranchName, key: SessionKey) -> Result<(), ProcError> {
    let (suffix, n) = match key {
        SessionKey::Issue(n) => ("issue", n.0),
        SessionKey::Pr(n) => ("pr", n.0),
    };
    proc::run(
        "git",
        ["config".to_string(), format!("branch.{}.dispatch-{suffix}", branch.as_str()), n.to_string()],
        root,
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;

    fn branch(s: &str) -> BranchName {
        BranchName::parse(s).expect("branch")
    }

    #[test]
    fn markers_from_real_regexp_output() {
        let text = "branch.fix/1-a.b-c.dispatch-issue 1\n\
                    branch.feat/2-x.dispatch-issue abc\n\
                    branch.feat/3.dispatch-issue 3\n";
        let m = parse_markers(text);
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
        let m = parse_markers(text);
        assert_eq!(m[&branch("chore/dep")], SessionKey::Pr(PrNumber(7)));
        assert_eq!(m[&branch("both")], SessionKey::Issue(IssueNumber(3)));
        assert_eq!(m[&branch("both2")], SessionKey::Issue(IssueNumber(4)));
        assert!(!m.contains_key(&branch("bad")));
    }

    #[test]
    fn marker_list_exit_one_is_empty() {
        fake::install(|_| Err(fake::fail("git", 1, "")));
        assert!(list_markers(Path::new("/")).expect("ok").is_empty());
    }
}
