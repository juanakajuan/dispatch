//! `gh` JSON as serde reads it, and the parse into domain types. A PR from a
//! fork gets an unusable head here, so no session starts on it.

use super::GhError;
use crate::domain::{
    Checks, CommitOid, GithubRef, Issue, IssueNumber, Mergeability, PrBranch, PrNumber, PrState, PullRequest, Unusable,
};
use serde::de::DeserializeOwned;

pub(super) fn decode<T: DeserializeOwned>(json: &str, what: &'static str) -> Result<T, GhError> {
    serde_json::from_str(json).map_err(|e| GhError::Decode { what, detail: e.to_string() })
}

fn timestamp(raw: &str, field: &'static str, item: GithubRef) -> Result<jiff::Timestamp, GhError> {
    raw.parse().map_err(|e: jiff::Error| GhError::BadTimestamp {
        item,
        field,
        raw: raw.to_string(),
        detail: e.to_string(),
    })
}

fn login(author: Option<WireAuthor>) -> String {
    author.map_or_else(|| "ghost".to_string(), |a| a.login)
}

/// Pure. Tested against fixture text captured from real `gh` output.
pub(super) fn issues_from_wire(wire: Vec<WireIssue>) -> Result<Vec<Issue>, GhError> {
    wire.into_iter()
        .map(|w| {
            let item = GithubRef::Issue(IssueNumber(w.number));
            Ok(Issue {
                number: IssueNumber(w.number),
                created_at: timestamp(&w.created_at, "createdAt", item)?,
                updated_at: timestamp(&w.updated_at, "updatedAt", item)?,
                title: w.title,
                labels: w.labels.into_iter().map(|l| l.name).collect(),
                author: login(w.author),
                comment_count: w.comments.len(),
            })
        })
        .collect()
}

/// `state` OPEN with `isDraft` is a draft. Any state string that is not one
/// of the four is an error naming the PR, never a silent default.
pub(super) fn prs_from_wire(wire: Vec<WirePr>) -> Result<Vec<PullRequest>, GhError> {
    wire.into_iter().map(pr_from_wire).collect()
}

pub(super) fn pr_from_wire(w: WirePr) -> Result<PullRequest, GhError> {
    let number = PrNumber(w.number);
    let state = match (w.state.as_str(), w.is_draft) {
        ("OPEN", true) => PrState::Draft,
        ("OPEN", false) => PrState::Open,
        ("MERGED", _) => PrState::Merged,
        ("CLOSED", _) => PrState::Closed,
        (_, _) => return Err(GhError::UnknownPrState { pr: number, state: w.state }),
    };
    let item = GithubRef::Pr(number);
    Ok(PullRequest {
        number,
        created_at: timestamp(&w.created_at, "createdAt", item)?,
        updated_at: timestamp(&w.updated_at, "updatedAt", item)?,
        title: w.title,
        state,
        author: login(w.author),
        head: if w.is_cross_repository {
            PrBranch::Unusable { raw: w.head_ref_name, why: Unusable::InFork }
        } else {
            PrBranch::parse(w.head_ref_name)
        },
        head_oid: CommitOid::new(w.head_ref_oid),
        base: PrBranch::parse(w.base_ref_name),
        additions: w.additions,
        deletions: w.deletions,
        checks: rollup(&w.status_check_rollup),
        mergeability: mergeability(&w.mergeable, &w.merge_state_status),
        closes: w.closing_issues_references.iter().map(|r| IssueNumber(r.number)).collect(),
    })
}

/// `mergeable` is MERGEABLE, CONFLICTING, or UNKNOWN. `mergeStateStatus` is
/// CLEAN, `HAS_HOOKS`, DIRTY, BEHIND, BLOCKED, UNSTABLE, DRAFT, or UNKNOWN. A
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
/// Empty -> `Checks::None`.
fn rollup(items: &[WireCheck]) -> Checks {
    let outcomes: Vec<Checks> = items
        .iter()
        .filter_map(|c| match c {
            WireCheck::CheckRun { status, conclusion } => Some(if status != "COMPLETED" {
                Checks::Pending
            } else if matches!(
                conclusion.as_str(),
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED"
            ) {
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
pub(super) struct WireIssue {
    number: u32,
    title: String,
    labels: Vec<WireLabel>,
    #[serde(default)]
    author: Option<WireAuthor>,
    created_at: String,
    updated_at: String,
    comments: Vec<serde::de::IgnoredAny>,
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
pub(super) struct WirePr {
    number: u32,
    title: String,
    state: String,
    is_draft: bool,
    #[serde(default)]
    author: Option<WireAuthor>,
    created_at: String,
    updated_at: String,
    head_ref_name: String,
    is_cross_repository: bool,
    head_ref_oid: String,
    base_ref_name: String,
    additions: u32,
    deletions: u32,
    status_check_rollup: Vec<WireCheck>,
    mergeable: String,
    merge_state_status: String,
    closing_issues_references: Vec<WireIssueRef>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireRepoMerge {
    pub(super) merge_commit_allowed: bool,
    pub(super) squash_merge_allowed: bool,
    pub(super) rebase_merge_allowed: bool,
}

#[derive(serde::Deserialize)]
struct WireIssueRef {
    number: u32,
}

/// Real output mixes two shapes, told apart by `__typename`:
/// `CheckRun` {status: "`COMPLETED"|"IN_PROGRESS`"|.., conclusion: "SUCCESS"|"FAILURE"|""}
/// `StatusContext` {state: "SUCCESS"|"PENDING"|"FAILURE"|"ERROR"}.
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
    use crate::github::{ISSUE_FIELDS, PR_FIELDS};

    const PRS: &str = include_str!("../../fixtures/prs.json");
    const ISSUES: &str = include_str!("../../fixtures/issues.json");

    fn parse_issues(json: &str) -> Result<Vec<Issue>, GhError> {
        decode(json, "gh issue list").and_then(issues_from_wire)
    }

    fn parse_prs(json: &str) -> Result<Vec<PullRequest>, GhError> {
        decode(json, "gh pr list").and_then(prs_from_wire)
    }

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
        assert_eq!(open.head.display(), "feature/711-hardware-metadata");
        assert_eq!(open.head.branch(), None, "a fork's branch is not on origin");
        assert!(pr(14927).head.branch().is_some(), "a branch in the repo itself");
        assert_eq!(open.base.display(), "master");
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
            "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-03T03:04:05Z","headRefName":"h","isCrossRepository":false,"headRefOid":"abc","baseRefName":"b","additions":1,"deletions":2,
            "statusCheckRollup":[{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},
                                 {"__typename":"StatusContext","state":"SUCCESS"},
                                 {"__typename":"Future","x":1}],
            "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","closingIssuesReferences":[]}]"#;
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
    fn unknown_pr_state_is_an_error_naming_the_pr() {
        let bad = PRS.replacen("\"state\": \"MERGED\"", "\"state\": \"WEIRD\"", 1);
        let err = parse_prs(&bad).expect_err("must fail");
        assert_eq!(err.to_string(), r#"PR #1002 has unknown state "WEIRD""#);
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
    }

    /// The fixtures are real gh output for today's queries. A field dropped
    /// from a query must leave its fixture too, or the parser tests read data
    /// that gh no longer sends (863cca9 left `body` behind).
    #[test]
    fn fixtures_hold_exactly_the_fields_each_query_asks_for() {
        for (fixture, fields) in [(ISSUES, ISSUE_FIELDS), (PRS, PR_FIELDS)] {
            let rows: Vec<serde_json::Map<String, serde_json::Value>> = serde_json::from_str(fixture).expect("fixture");
            let mut want: Vec<&str> = fields.split(',').collect();
            want.sort_unstable();
            for row in rows {
                let mut got: Vec<&str> = row.keys().map(String::as_str).collect();
                got.sort_unstable();
                assert_eq!(got, want);
            }
        }
    }

    #[test]
    fn bad_json_and_bad_dates_are_errors() {
        assert!(parse_issues("[{\"number\":1}]").is_err());
        let err = parse_issues("nope").expect_err("not JSON");
        assert!(err.to_string().starts_with("unexpected gh issue list output: "), "{err}");
        let bad_date = ISSUES.replace("\"createdAt\": \"2", "\"createdAt\": \"x2");
        assert!(parse_issues(&bad_date).is_err());
        let bad_update = PRS.replacen("\"updatedAt\": \"2", "\"updatedAt\": \"x2", 1);
        let err = parse_prs(&bad_update).expect_err("must fail");
        assert!(err.to_string().starts_with(r#"PR #1028: bad updatedAt "x2"#), "{err}");
    }
}
