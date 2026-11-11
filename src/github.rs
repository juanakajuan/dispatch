//! `gh` adapter. Wire structs are private; callers only see domain types.
//! `gh` infers the repo from the working directory, so no owner/name here.

use crate::domain::{Checks, GithubRef, Issue, IssueNumber, MergeMethod, Mergeability, PrNumber, PrState, PullRequest};
use crate::proc;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub struct FetchError(pub String);

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn fetch_issues(repo_root: &Path) -> Result<Vec<Issue>, FetchError> {
    let json = proc::run(
        "gh",
        ["issue", "list", "--state", "open", "--limit", "200", "--json", "number,title,labels,author,createdAt,updatedAt,comments,body,url"],
        repo_root,
    )
    .map_err(|e| FetchError(e.to_string()))?;
    parse_issues(&json)
}

/// The fields `PullRequest` needs, for both `gh pr list` and `gh pr view`.
const PR_FIELDS: &str = "number,title,state,isDraft,author,createdAt,updatedAt,headRefName,baseRefName,additions,deletions,headRefOid,statusCheckRollup,mergeable,mergeStateStatus,closingIssuesReferences,url";

pub fn fetch_prs(repo_root: &Path) -> Result<Vec<PullRequest>, FetchError> {
    let json = proc::run("gh", ["pr", "list", "--state", "all", "--limit", "100", "--json", PR_FIELDS], repo_root)
        .map_err(|e| FetchError(e.to_string()))?;
    parse_prs(&json)
}

/// One PR as GitHub sees it now, for a decision the cached list is too old for.
pub fn view_pr(repo_root: &Path, pr: PrNumber) -> Result<PullRequest, FetchError> {
    let json = proc::run("gh", ["pr", "view", &pr.0.to_string(), "--json", PR_FIELDS], repo_root)
        .map_err(|e| FetchError(e.to_string()))?;
    let wire: WirePr = serde_json::from_str(&json).map_err(|e| FetchError(format!("unexpected gh pr view output: {e}")))?;
    pr_from_wire(wire)
}

/// The merge methods the repo allows, in the order `merge`, `squash`, `rebase`.
pub fn allowed_merge_methods(repo_root: &Path) -> Result<Vec<MergeMethod>, FetchError> {
    let json = proc::run("gh", ["repo", "view", "--json", "mergeCommitAllowed,squashMergeAllowed,rebaseMergeAllowed"], repo_root)
        .map_err(|e| FetchError(e.to_string()))?;
    let wire: WireRepoMerge =
        serde_json::from_str(&json).map_err(|e| FetchError(format!("unexpected gh repo view output: {e}")))?;
    Ok([
        (wire.merge_commit_allowed, MergeMethod::Merge),
        (wire.squash_merge_allowed, MergeMethod::Squash),
        (wire.rebase_merge_allowed, MergeMethod::Rebase),
    ]
    .into_iter()
    .filter_map(|(allowed, method)| allowed.then_some(method))
    .collect())
}

/// `gh pr merge <n> --<method> --match-head-commit <sha>`. GitHub refuses the
/// merge if the head moved after dispatch checked it. No `--delete-branch`:
/// the session's worktree still has the branch checked out, and dispatch
/// never deletes branches.
pub fn merge_pr(repo_root: &Path, pr: &PullRequest, method: MergeMethod) -> Result<(), FetchError> {
    let flag = format!("--{method}");
    let args = ["pr", "merge", &pr.number.0.to_string(), &flag, "--match-head-commit", &pr.head_oid];
    proc::run("gh", args, repo_root).map(|_| ()).map_err(|e| FetchError(e.to_string()))
}

/// Opens the issue or PR page in the browser with `gh <kind> view <n> --web`,
/// so it follows whatever host `gh` is logged in to. `proc::run` captures the
/// output, so nothing `gh` prints reaches the TUI.
pub fn open_in_browser(repo_root: &Path, target: GithubRef) -> Result<(), FetchError> {
    let (kind, number) = match target {
        GithubRef::Issue(n) => ("issue", n.0),
        GithubRef::Pr(n) => ("pr", n.0),
    };
    proc::run("gh", [kind, "view", &number.to_string(), "--web"], repo_root).map(|_| ()).map_err(|e| FetchError(e.to_string()))
}

fn timestamp(raw: &str, field: &str, what: &str) -> Result<jiff::Timestamp, FetchError> {
    raw.parse().map_err(|e| FetchError(format!("{what}: bad {field} {raw:?}: {e}")))
}

fn login(author: Option<WireAuthor>) -> String {
    author.map_or_else(|| "ghost".to_string(), |a| a.login)
}

/// Pure. Tested against fixture text captured from real `gh` output.
fn parse_issues(json: &str) -> Result<Vec<Issue>, FetchError> {
    let wire: Vec<WireIssue> = serde_json::from_str(json).map_err(|e| FetchError(format!("unexpected gh issue list output: {e}")))?;
    wire.into_iter()
        .map(|w| {
            let what = format!("issue #{}", w.number);
            Ok(Issue {
                number: IssueNumber(w.number),
                created_at: timestamp(&w.created_at, "createdAt", &what)?,
                updated_at: timestamp(&w.updated_at, "updatedAt", &what)?,
                title: w.title,
                labels: w.labels.into_iter().map(|l| l.name).collect(),
                author: login(w.author),
                comment_count: w.comments.len(),
                body: w.body,
                url: w.url,
            })
        })
        .collect()
}

/// `state` OPEN with `isDraft` is a draft. Any state string that is not one
/// of the four is an error naming the PR, never a silent default.
fn parse_prs(json: &str) -> Result<Vec<PullRequest>, FetchError> {
    let wire: Vec<WirePr> = serde_json::from_str(json).map_err(|e| FetchError(format!("unexpected gh pr list output: {e}")))?;
    wire.into_iter().map(pr_from_wire).collect()
}

fn pr_from_wire(w: WirePr) -> Result<PullRequest, FetchError> {
    let state = match (w.state.as_str(), w.is_draft) {
        ("OPEN", true) => PrState::Draft,
        ("OPEN", false) => PrState::Open,
        ("MERGED", _) => PrState::Merged,
        ("CLOSED", _) => PrState::Closed,
        (other, _) => return Err(FetchError(format!("PR #{} has unknown state {other:?}", w.number))),
    };
    let what = format!("PR #{}", w.number);
    Ok(PullRequest {
        number: PrNumber(w.number),
        created_at: timestamp(&w.created_at, "createdAt", &what)?,
        updated_at: timestamp(&w.updated_at, "updatedAt", &what)?,
        title: w.title,
        state,
        author: login(w.author),
        head: w.head_ref_name,
        head_oid: w.head_ref_oid,
        base: w.base_ref_name,
        additions: w.additions,
        deletions: w.deletions,
        checks: rollup(&w.status_check_rollup),
        mergeability: mergeability(&w.mergeable, &w.merge_state_status),
        closes: w.closing_issues_references.iter().map(|r| IssueNumber(r.number)).collect(),
        url: w.url,
    })
}

/// `mergeable` is MERGEABLE, CONFLICTING, or UNKNOWN. `mergeStateStatus` is
/// CLEAN, HAS_HOOKS, DIRTY, BEHIND, BLOCKED, UNSTABLE, DRAFT, or UNKNOWN. A
/// value GitHub adds later reads as `Unknown`, so dispatch waits rather than
/// merges.
fn mergeability(mergeable: &str, status: &str) -> Mergeability {
    match (mergeable, status) {
        ("CONFLICTING", _) | (_, "DIRTY") => Mergeability::Conflict,
        (_, "CLEAN" | "HAS_HOOKS") => Mergeability::Clean,
        (_, "BEHIND") => Mergeability::Behind,
        (_, "BLOCKED") => Mergeability::Blocked,
        (_, "UNSTABLE") => Mergeability::Unstable,
        _ => Mergeability::Unknown,
    }
}

/// Any failure -> Fail, else any not completed -> Pending, else Pass.
/// Empty -> Checks::None.
fn rollup(items: &[WireCheck]) -> Checks {
    let outcomes: Vec<Checks> = items
        .iter()
        .filter_map(|c| match c {
            WireCheck::CheckRun { status, conclusion } => Some(if status != "COMPLETED" {
                Checks::Pending
            } else if matches!(conclusion.as_str(), "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED") {
                Checks::Fail
            } else {
                Checks::Pass
            }),
            WireCheck::StatusContext { state } => Some(match state.as_str() {
                "FAILURE" | "ERROR" => Checks::Fail,
                "SUCCESS" => Checks::Pass,
                _ => Checks::Pending,
            }),
            WireCheck::Unknown => None,
        })
        .collect();
    if outcomes.is_empty() {
        Checks::None
    } else if outcomes.contains(&Checks::Fail) {
        Checks::Fail
    } else if outcomes.contains(&Checks::Pending) {
        Checks::Pending
    } else {
        Checks::Pass
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireIssue {
    number: u32,
    title: String,
    labels: Vec<WireLabel>,
    #[serde(default)]
    author: Option<WireAuthor>,
    created_at: String,
    updated_at: String,
    comments: Vec<serde::de::IgnoredAny>,
    body: String,
    url: String,
}

#[derive(serde::Deserialize)]
struct WireLabel {
    name: String,
}

#[derive(serde::Deserialize)]
struct WireAuthor {
    login: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePr {
    number: u32,
    title: String,
    state: String,
    is_draft: bool,
    #[serde(default)]
    author: Option<WireAuthor>,
    created_at: String,
    updated_at: String,
    head_ref_name: String,
    head_ref_oid: String,
    base_ref_name: String,
    additions: u32,
    deletions: u32,
    status_check_rollup: Vec<WireCheck>,
    mergeable: String,
    merge_state_status: String,
    closing_issues_references: Vec<WireIssueRef>,
    url: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRepoMerge {
    merge_commit_allowed: bool,
    squash_merge_allowed: bool,
    rebase_merge_allowed: bool,
}

#[derive(serde::Deserialize)]
struct WireIssueRef {
    number: u32,
}

/// Real output mixes two shapes, told apart by `__typename`:
/// CheckRun {status: "COMPLETED"|"IN_PROGRESS"|.., conclusion: "SUCCESS"|"FAILURE"|""}
/// StatusContext {state: "SUCCESS"|"PENDING"|"FAILURE"|"ERROR"}.
#[derive(serde::Deserialize)]
#[serde(tag = "__typename")]
enum WireCheck {
    CheckRun {
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: String,
    },
    StatusContext {
        #[serde(default)]
        state: String,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRS: &str = include_str!("../fixtures/prs.json");
    const ISSUES: &str = include_str!("../fixtures/issues.json");

    fn pr(n: u32) -> PullRequest {
        parse_prs(PRS).expect("fixture parses").into_iter().find(|p| p.number == PrNumber(n)).expect("pr in fixture")
    }

    fn check(status: &str, conclusion: &str) -> WireCheck {
        WireCheck::CheckRun { status: status.to_string(), conclusion: conclusion.to_string() }
    }

    fn status(state: &str) -> WireCheck {
        WireCheck::StatusContext { state: state.to_string() }
    }

    #[test]
    fn real_prs_map_state_checks_and_closes() {
        let open = pr(1028);
        assert_eq!((open.state, open.checks, open.mergeability), (PrState::Open, Checks::Pass, Mergeability::Clean));
        assert_eq!(open.closes, [IssueNumber(711)]);
        assert_eq!(open.head, "feature/711-hardware-metadata");
        assert_eq!(open.base, "master");
        assert_eq!(pr(1002).updated_at.to_string(), "2026-10-12T08:33:48Z");
        assert_eq!(pr(1002).state, PrState::Merged);
        let draft = pr(14927);
        assert_eq!((draft.state, draft.checks), (PrState::Draft, Checks::Fail));
        let none = pr(14035);
        assert_eq!((none.state, none.checks), (PrState::Draft, Checks::None));
    }

    #[test]
    fn rollup_failure_beats_pending_beats_pass() {
        assert_eq!(rollup(&[]), Checks::None);
        assert_eq!(rollup(&[check("COMPLETED", "SUCCESS"), check("COMPLETED", "SKIPPED")]), Checks::Pass);
        assert_eq!(rollup(&[check("IN_PROGRESS", ""), check("COMPLETED", "SUCCESS")]), Checks::Pending);
        assert_eq!(rollup(&[check("IN_PROGRESS", ""), check("COMPLETED", "FAILURE")]), Checks::Fail);
        assert_eq!(rollup(&[status("PENDING"), status("SUCCESS")]), Checks::Pending);
        assert_eq!(rollup(&[status("ERROR"), check("COMPLETED", "SUCCESS")]), Checks::Fail);
        assert_eq!(rollup(&[WireCheck::Unknown]), Checks::None);
    }

    #[test]
    fn mixed_check_shapes_parse_by_typename() {
        let json = r#"[{"number":1,"title":"t","state":"OPEN","isDraft":false,"author":{"login":"a"},
            "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-03T03:04:05Z","headRefName":"h","headRefOid":"abc","baseRefName":"b","additions":1,"deletions":2,
            "statusCheckRollup":[{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},
                                 {"__typename":"StatusContext","state":"SUCCESS"},
                                 {"__typename":"Future","x":1}],
            "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","closingIssuesReferences":[],"url":"u"}]"#;
        assert_eq!(parse_prs(json).expect("parses")[0].checks, Checks::Pending);
    }

    #[test]
    fn mergeability_reads_both_fields_and_waits_on_anything_new() {
        assert_eq!(mergeability("MERGEABLE", "CLEAN"), Mergeability::Clean);
        assert_eq!(mergeability("MERGEABLE", "HAS_HOOKS"), Mergeability::Clean);
        assert_eq!(mergeability("CONFLICTING", "BLOCKED"), Mergeability::Conflict, "a conflict wins");
        assert_eq!(mergeability("UNKNOWN", "DIRTY"), Mergeability::Conflict);
        assert_eq!(mergeability("MERGEABLE", "BEHIND"), Mergeability::Behind);
        assert_eq!(mergeability("MERGEABLE", "BLOCKED"), Mergeability::Blocked);
        assert_eq!(mergeability("MERGEABLE", "UNSTABLE"), Mergeability::Unstable);
        assert_eq!(mergeability("UNKNOWN", "UNKNOWN"), Mergeability::Unknown);
        assert_eq!(mergeability("MERGEABLE", "DRAFT"), Mergeability::Unknown);
        assert_eq!(mergeability("MERGEABLE", "SOMETHING_NEW"), Mergeability::Unknown);
    }

    #[test]
    fn view_pr_reads_one_pr_with_the_list_fields() {
        proc::fake::install(|_| {
            Ok(r#"{"number":12,"title":"t","state":"OPEN","isDraft":false,"author":null,
                "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-03T03:04:05Z","headRefName":"h","headRefOid":"abc","baseRefName":"main","additions":1,"deletions":2,
                "statusCheckRollup":[],"mergeable":"CONFLICTING","mergeStateStatus":"DIRTY",
                "closingIssuesReferences":[{"number":3}],"url":"u"}"#
                .to_string())
        });
        let pr = view_pr(Path::new("/r"), PrNumber(12)).expect("parses");
        assert_eq!((pr.number, pr.mergeability, pr.closes.as_slice()), (PrNumber(12), Mergeability::Conflict, [IssueNumber(3)].as_slice()));
        assert_eq!(proc::fake::calls(), [format!("gh pr view 12 --json {PR_FIELDS}")]);
    }

    #[test]
    fn allowed_merge_methods_keep_gh_order() {
        proc::fake::install(|_| Ok(r#"{"mergeCommitAllowed":false,"rebaseMergeAllowed":true,"squashMergeAllowed":true}"#.to_string()));
        assert_eq!(allowed_merge_methods(Path::new("/r")).expect("parses"), [MergeMethod::Squash, MergeMethod::Rebase]);
    }

    #[test]
    fn merge_pr_pins_the_head_and_passes_the_method_but_no_branch_delete() {
        proc::fake::install(|_| Ok(String::new()));
        let pr = PullRequest { head_oid: "abc123".to_string(), ..crate::testutil::pr(12, "h", &[]) };
        merge_pr(Path::new("/r"), &pr, MergeMethod::Rebase).expect("merges");
        assert_eq!(proc::fake::calls(), ["gh pr merge 12 --rebase --match-head-commit abc123"]);
    }

    #[test]
    fn unknown_pr_state_is_an_error_naming_the_pr() {
        let bad = PRS.replacen("\"state\": \"MERGED\"", "\"state\": \"WEIRD\"", 1);
        let err = parse_prs(&bad).expect_err("must fail");
        assert!(err.0.contains("#1002") && err.0.contains("WEIRD"), "{err}");
    }

    #[test]
    fn real_issues_map_labels_and_comment_count() {
        let issues = parse_issues(ISSUES).expect("fixture parses");
        let first = &issues[0];
        assert_eq!(first.number, IssueNumber(14386));
        assert_eq!(first.comment_count, 9);
        assert_eq!(first.labels, ["bug", "priority-2", "gh-issue"]);
        assert_eq!(issues[1].comment_count, 1);
        assert!(issues[1].labels.is_empty());
        assert_eq!(first.created_at.to_string(), "2026-09-08T10:31:38Z");
        assert_eq!(first.updated_at.to_string(), "2026-10-04T12:33:40Z");
        assert_eq!(first.author, "A4-Tacks");
        assert!(first.url.starts_with("https://github.com/"));
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
    fn open_in_browser_failure_carries_the_gh_error() {
        proc::fake::install(|_| Err(proc::fake::fail("gh", 1, "no browser")));
        let err = open_in_browser(Path::new("/r"), GithubRef::Issue(IssueNumber(3))).expect_err("fails");
        assert!(err.0.contains("no browser"), "{err}");
    }

    #[test]
    fn bad_json_and_bad_dates_are_errors() {
        assert!(parse_issues("[{\"number\":1}]").is_err());
        assert!(parse_issues("nope").is_err());
        let bad_date = ISSUES.replace("\"createdAt\": \"2", "\"createdAt\": \"x2");
        assert!(parse_issues(&bad_date).is_err());
        let bad_update = PRS.replacen("\"updatedAt\": \"2", "\"updatedAt\": \"x2", 1);
        let err = parse_prs(&bad_update).expect_err("must fail");
        assert!(err.0.contains("#1028") && err.0.contains("updatedAt"), "{err}");
    }
}
