//! `gh` adapter. Wire structs are private; callers only see domain types.
//! `gh` infers the repo from the working directory, so no owner/name here.

mod wire;

use crate::domain::{GithubRef, Issue, MergeMethod, PrNumber, PullRequest};
use crate::proc::{self, ProcError};
use serde::de::DeserializeOwned;
use std::fmt;
use std::path::Path;
use wire::{WireRepoMerge, decode, issues_from_wire, pr_from_wire, prs_from_wire};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhError {
    Run(ProcError),
    /// `gh` printed text that `what` can not read.
    Decode {
        what: &'static str,
        detail: String,
    },
    BadTimestamp {
        item: GithubRef,
        field: &'static str,
        raw: String,
        detail: String,
    },
    UnknownPrState {
        pr: PrNumber,
        state: String,
    },
    /// `gh <kind> view --json url` printed nothing.
    EmptyUrl {
        kind: &'static str,
    },
}

impl fmt::Display for GhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GhError::Run(e) => write!(f, "{e}"),
            GhError::Decode { what, detail } => write!(f, "unexpected {what} output: {detail}"),
            GhError::BadTimestamp { item, field, raw, detail } => write!(f, "{item}: bad {field} {raw:?}: {detail}"),
            GhError::UnknownPrState { pr, state } => write!(f, "PR {pr} has unknown state {state:?}"),
            GhError::EmptyUrl { kind } => write!(f, "gh {kind} view printed no URL"),
        }
    }
}

fn gh<const N: usize>(repo_root: &Path, args: [&str; N]) -> Result<String, GhError> {
    proc::run("gh", args, repo_root).map_err(GhError::Run)
}

/// Runs `gh` and reads its JSON. `what` names the command in the error.
fn gh_json<T: DeserializeOwned, const N: usize>(
    repo_root: &Path,
    args: [&str; N],
    what: &'static str,
) -> Result<T, GhError> {
    decode(&gh(repo_root, args)?, what)
}

pub fn fetch_issues(repo_root: &Path) -> Result<Vec<Issue>, GhError> {
    let args = ["issue", "list", "--state", "open", "--limit", "200", "--json", ISSUE_FIELDS];
    gh_json(repo_root, args, "gh issue list").and_then(issues_from_wire)
}

/// The fields `Issue` needs, for `gh issue list`.
const ISSUE_FIELDS: &str = "number,title,labels,author,createdAt,updatedAt,comments";

/// The fields `PullRequest` needs, for both `gh pr list` and `gh pr view`.
const PR_FIELDS: &str = "number,title,state,isDraft,author,createdAt,updatedAt,headRefName,isCrossRepository,baseRefName,additions,deletions,headRefOid,statusCheckRollup,mergeable,mergeStateStatus,closingIssuesReferences";

pub fn fetch_prs(repo_root: &Path) -> Result<Vec<PullRequest>, GhError> {
    let args = ["pr", "list", "--state", "all", "--limit", "100", "--json", PR_FIELDS];
    gh_json(repo_root, args, "gh pr list").and_then(prs_from_wire)
}

/// One PR as GitHub sees it now, for a decision the cached list is too old for.
pub fn view_pr(repo_root: &Path, pr: PrNumber) -> Result<PullRequest, GhError> {
    gh_json(repo_root, ["pr", "view", &pr.0.to_string(), "--json", PR_FIELDS], "gh pr view").and_then(pr_from_wire)
}

/// The merge methods the repo allows, in the order `merge`, `squash`, `rebase`.
pub fn allowed_merge_methods(repo_root: &Path) -> Result<Vec<MergeMethod>, GhError> {
    let args = ["repo", "view", "--json", "mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed"];
    gh_json(repo_root, args, "gh repo view").map(|wire: WireRepoMerge| wire.methods())
}

/// `gh pr merge <n> --<method> --match-head-commit <sha>`. GitHub refuses the
/// merge if the head moved after dispatch checked it. No `--delete-branch`:
/// the session's worktree still has the branch checked out, and dispatch
/// never deletes branches.
pub fn merge_pr(repo_root: &Path, pr: &PullRequest, method: MergeMethod) -> Result<(), GhError> {
    let flag = format!("--{method}");
    let args = ["pr", "merge", &pr.number.0.to_string(), &flag, "--match-head-commit", pr.head_oid.as_str()];
    gh(repo_root, args).map(|_| ())
}

/// `gh pr ready <n>`: takes a draft out of draft.
pub fn mark_ready(repo_root: &Path, pr: PrNumber) -> Result<(), GhError> {
    gh(repo_root, ["pr", "ready", &pr.0.to_string()]).map(|_| ())
}

/// Opens the issue or PR page in the browser with `gh <kind> view <n> --web`,
/// so it follows whatever host `gh` is logged in to. `proc::run` captures the
/// output, so nothing `gh` prints reaches the TUI.
pub fn open_in_browser(repo_root: &Path, target: GithubRef) -> Result<(), GhError> {
    let (kind, number) = view_target(target);
    gh(repo_root, [kind, "view", &number, "--web"]).map(|_| ())
}

/// The page `open_in_browser` opens, from the same `gh` host.
pub fn page_url(repo_root: &Path, target: GithubRef) -> Result<String, GhError> {
    let (kind, number) = view_target(target);
    let url = gh(repo_root, [kind, "view", &number, "--json", "url", "--jq", ".url"])?;
    match url.trim() {
        "" => Err(GhError::EmptyUrl { kind }),
        url => Ok(url.to_string()),
    }
}

fn view_target(target: GithubRef) -> (&'static str, String) {
    match target {
        GithubRef::Issue(n) => ("issue", n.0.to_string()),
        GithubRef::Pr(n) => ("pr", n.0.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CommitOid, IssueNumber, Mergeability};

    #[test]
    fn view_pr_reads_one_pr_with_the_list_fields() {
        proc::fake::install(|_| {
            Ok(r#"{"number":12,"title":"t","state":"OPEN","isDraft":false,"author":null,
                "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-03T03:04:05Z","headRefName":"h","isCrossRepository":false,"headRefOid":"abc","baseRefName":"main","additions":1,"deletions":2,
                "statusCheckRollup":[],"mergeable":"CONFLICTING","mergeStateStatus":"DIRTY",
                "closingIssuesReferences":[{"number":3}]}"#
                .to_string())
        });
        let pr = view_pr(Path::new("/r"), PrNumber(12)).expect("parses");
        assert_eq!(
            (pr.number, pr.mergeability, pr.closes.as_slice()),
            (PrNumber(12), Mergeability::Conflict, [IssueNumber(3)].as_slice())
        );
        assert_eq!(proc::fake::calls(), [format!("gh pr view 12 --json {PR_FIELDS}")]);
    }

    #[test]
    fn allowed_merge_methods_keep_gh_order() {
        proc::fake::install(|_| {
            Ok(r#"{"mergeCommitAllowed":false,"rebaseMergeAllowed":true,"squashMergeAllowed":true}"#.to_string())
        });
        assert_eq!(allowed_merge_methods(Path::new("/r")).expect("parses"), [MergeMethod::Squash, MergeMethod::Rebase]);
    }

    #[test]
    fn merge_pr_pins_the_head_and_passes_the_method_but_no_branch_delete() {
        proc::fake::install(|_| Ok(String::new()));
        let pr = PullRequest { head_oid: CommitOid::new("abc123".to_string()), ..crate::testutil::pr(12, "h", &[]) };
        merge_pr(Path::new("/r"), &pr, MergeMethod::Rebase).expect("merges");
        assert_eq!(proc::fake::calls(), ["gh pr merge 12 --rebase --match-head-commit abc123"]);
    }

    #[test]
    fn mark_ready_runs_gh_pr_ready() {
        proc::fake::install(|_| Ok(String::new()));
        mark_ready(Path::new("/r"), PrNumber(12)).expect("marks ready");
        assert_eq!(proc::fake::calls(), ["gh pr ready 12"]);
    }

    #[test]
    fn open_in_browser_runs_gh_view_web_for_the_right_kind() {
        proc::fake::install(|_| Ok(String::new()));
        let root = Path::new("/r");
        open_in_browser(root, GithubRef::Issue(IssueNumber(3))).expect("opens");
        open_in_browser(root, GithubRef::Pr(PrNumber(10))).expect("opens");
        assert_eq!(proc::fake::calls(), ["gh issue view 3 --web", "gh pr view 10 --web"]);
    }

    #[test]
    fn page_url_asks_gh_for_the_url_of_the_right_kind() {
        proc::fake::install(|_| Ok("https://github.com/o/r/pull/10\n".to_string()));
        let url = page_url(Path::new("/r"), GithubRef::Pr(PrNumber(10))).expect("url");
        assert_eq!(url, "https://github.com/o/r/pull/10");
        assert_eq!(proc::fake::calls(), ["gh pr view 10 --json url --jq .url"]);
    }

    #[test]
    fn page_url_with_empty_output_is_an_error() {
        proc::fake::install(|_| Ok("\n".to_string()));
        let err = page_url(Path::new("/r"), GithubRef::Issue(IssueNumber(3))).expect_err("no url");
        assert_eq!(err.to_string(), "gh issue view printed no URL");
    }

    #[test]
    fn open_in_browser_failure_carries_the_gh_error() {
        proc::fake::install(|_| Err(proc::fake::fail("gh", 1, "no browser")));
        let err = open_in_browser(Path::new("/r"), GithubRef::Issue(IssueNumber(3))).expect_err("fails");
        assert_eq!(err.to_string(), "gh failed with code 1: no browser");
    }
}
