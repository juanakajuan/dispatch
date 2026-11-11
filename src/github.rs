//! `gh` adapter. Wire structs are private; callers only see domain types.
//! `gh` infers the repo from the working directory, so no owner/name here.

use crate::domain::{Checks, Issue, IssueNumber, PrNumber, PrState, PullRequest};
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
        ["issue", "list", "--state", "open", "--limit", "200", "--json", "number,title,labels,author,createdAt,comments,body,url"],
        repo_root,
    )
    .map_err(|e| FetchError(e.to_string()))?;
    parse_issues(&json)
}

pub fn fetch_prs(repo_root: &Path) -> Result<Vec<PullRequest>, FetchError> {
    let json = proc::run(
        "gh",
        [
            "pr",
            "list",
            "--state",
            "all",
            "--limit",
            "100",
            "--json",
            "number,title,state,isDraft,author,createdAt,headRefName,baseRefName,additions,deletions,statusCheckRollup,closingIssuesReferences,url",
        ],
        repo_root,
    )
    .map_err(|e| FetchError(e.to_string()))?;
    parse_prs(&json)
}

fn timestamp(raw: &str, what: &str) -> Result<jiff::Timestamp, FetchError> {
    raw.parse().map_err(|e| FetchError(format!("{what}: bad createdAt {raw:?}: {e}")))
}

fn login(author: Option<WireAuthor>) -> String {
    author.map_or_else(|| "ghost".to_string(), |a| a.login)
}

/// Pure. Tested against fixture text captured from real `gh` output.
fn parse_issues(json: &str) -> Result<Vec<Issue>, FetchError> {
    let wire: Vec<WireIssue> = serde_json::from_str(json).map_err(|e| FetchError(format!("unexpected gh issue list output: {e}")))?;
    wire.into_iter()
        .map(|w| {
            Ok(Issue {
                number: IssueNumber(w.number),
                created_at: timestamp(&w.created_at, &format!("issue #{}", w.number))?,
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
    wire.into_iter()
        .map(|w| {
            let state = match (w.state.as_str(), w.is_draft) {
                ("OPEN", true) => PrState::Draft,
                ("OPEN", false) => PrState::Open,
                ("MERGED", _) => PrState::Merged,
                ("CLOSED", _) => PrState::Closed,
                (other, _) => return Err(FetchError(format!("PR #{} has unknown state {other:?}", w.number))),
            };
            Ok(PullRequest {
                number: PrNumber(w.number),
                created_at: timestamp(&w.created_at, &format!("PR #{}", w.number))?,
                title: w.title,
                state,
                author: login(w.author),
                head: w.head_ref_name,
                base: w.base_ref_name,
                additions: w.additions,
                deletions: w.deletions,
                checks: rollup(&w.status_check_rollup),
                closes: w.closing_issues_references.iter().map(|r| IssueNumber(r.number)).collect(),
                url: w.url,
            })
        })
        .collect()
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
    head_ref_name: String,
    base_ref_name: String,
    additions: u32,
    deletions: u32,
    status_check_rollup: Vec<WireCheck>,
    closing_issues_references: Vec<WireIssueRef>,
    url: String,
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
        assert_eq!((open.state, open.checks), (PrState::Open, Checks::Pass));
        assert_eq!(open.closes, [IssueNumber(711)]);
        assert_eq!(open.head, "feature/711-hardware-metadata");
        assert_eq!(open.base, "master");
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
            "createdAt":"2026-01-02T03:04:05Z","headRefName":"h","baseRefName":"b","additions":1,"deletions":2,
            "statusCheckRollup":[{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},
                                 {"__typename":"StatusContext","state":"SUCCESS"},
                                 {"__typename":"Future","x":1}],
            "closingIssuesReferences":[],"url":"u"}]"#;
        assert_eq!(parse_prs(json).expect("parses")[0].checks, Checks::Pending);
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
        assert_eq!(first.author, "A4-Tacks");
        assert!(first.url.starts_with("https://github.com/"));
    }

    #[test]
    fn bad_json_and_bad_dates_are_errors() {
        assert!(parse_issues("[{\"number\":1}]").is_err());
        assert!(parse_issues("nope").is_err());
        let bad_date = ISSUES.replace("\"createdAt\": \"2", "\"createdAt\": \"x2");
        assert!(parse_issues(&bad_date).is_err());
    }
}
