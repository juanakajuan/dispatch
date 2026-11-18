//! `gh` JSON as serde reads it, and the parse into domain types.

use super::GhError;
use crate::domain::{
    CommitOid, ForkPush, GithubRef, Issue, IssueNumber, MergeMethod, PrBranch, PrNumber, PrState, PullRequest, RepoSlug,
};
use checks::{WireCheck, mergeability, rollup};
use serde::de::DeserializeOwned;

mod checks;

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

fn names(labels: Vec<WireLabel>) -> Vec<String> {
    labels.into_iter().map(|l| l.name).collect()
}

fn logins(users: Vec<WireAuthor>) -> Vec<String> {
    users.into_iter().map(|u| u.login).collect()
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
                labels: names(w.labels),
                assignees: logins(w.assignees),
                milestone: w.milestone.map(|m| m.title),
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
        ("OPEN", Readiness::Draft) => PrState::Draft,
        ("OPEN", Readiness::Ready) => PrState::Open,
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
        labels: names(w.labels),
        assignees: logins(w.assignees),
        milestone: w.milestone.map(|m| m.title),
        author: login(w.author),
        head: match w.is_cross_repository {
            HeadRepo::Fork => {
                PrBranch::fork(number, w.head_ref_name, fork_push(w.maintainer_can_modify, w.head_repository))
            }
            HeadRepo::Same => PrBranch::parse(w.head_ref_name),
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

/// A deleted fork has no `headRepository`, so no one can push to it.
fn fork_push(edits: Edits, repo: Option<WireRepo>) -> ForkPush {
    match (edits, repo.and_then(|r| RepoSlug::parse(&r.name_with_owner))) {
        (Edits::Allowed, Some(slug)) => ForkPush::Allowed(slug),
        (Edits::Allowed | Edits::Denied, _) => ForkPush::Denied,
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireIssue {
    number: u32,
    title: String,
    labels: Vec<WireLabel>,
    assignees: Vec<WireAuthor>,
    milestone: Option<WireMilestone>,
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
struct WireMilestone {
    title: String,
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
    is_draft: Readiness,
    labels: Vec<WireLabel>,
    assignees: Vec<WireAuthor>,
    milestone: Option<WireMilestone>,
    #[serde(default)]
    author: Option<WireAuthor>,
    created_at: String,
    updated_at: String,
    head_ref_name: String,
    is_cross_repository: HeadRepo,
    head_repository: Option<WireRepo>,
    maintainer_can_modify: Edits,
    head_ref_oid: String,
    base_ref_name: String,
    additions: u32,
    deletions: u32,
    status_check_rollup: Vec<WireCheck>,
    mergeable: String,
    merge_state_status: String,
    closing_issues_references: Vec<WireIssueRef>,
}

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(from = "bool")]
enum Readiness {
    Draft,
    Ready,
}

impl From<bool> for Readiness {
    fn from(is_draft: bool) -> Self {
        if is_draft { Readiness::Draft } else { Readiness::Ready }
    }
}

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(from = "bool")]
enum HeadRepo {
    Fork,
    Same,
}

impl From<bool> for HeadRepo {
    fn from(is_cross_repository: bool) -> Self {
        if is_cross_repository { HeadRepo::Fork } else { HeadRepo::Same }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRepo {
    name_with_owner: String,
}

/// `maintainerCanModify`: whether the PR lets maintainers of the base repo
/// push to its head.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(from = "bool")]
enum Edits {
    Allowed,
    Denied,
}

impl From<bool> for Edits {
    fn from(maintainer_can_modify: bool) -> Self {
        if maintainer_can_modify { Edits::Allowed } else { Edits::Denied }
    }
}

#[derive(serde::Deserialize)]
pub(super) struct WireRepoMerge {
    #[serde(rename = "mergeCommitAllowed")]
    merge: Allowed,
    #[serde(rename = "squashMergeAllowed")]
    squash: Allowed,
    #[serde(rename = "rebaseMergeAllowed")]
    rebase: Allowed,
}

impl WireRepoMerge {
    pub(super) fn methods(&self) -> Vec<MergeMethod> {
        [(self.merge, MergeMethod::Merge), (self.squash, MergeMethod::Squash), (self.rebase, MergeMethod::Rebase)]
            .into_iter()
            .filter_map(|(allowed, method)| (allowed == Allowed::Yes).then_some(method))
            .collect()
    }
}

#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(from = "bool")]
enum Allowed {
    Yes,
    No,
}

impl From<bool> for Allowed {
    fn from(allowed: bool) -> Self {
        if allowed { Allowed::Yes } else { Allowed::No }
    }
}

#[derive(serde::Deserialize)]
struct WireIssueRef {
    number: u32,
}

/// An open issue and the PRs GitHub links to it, from `gh issue list`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireIssueLinks {
    url: String,
    closed_by_pull_requests_references: Vec<WireLinkedPr>,
}

#[derive(serde::Deserialize)]
struct WireLinkedPr {
    number: u32,
    url: String,
}

/// The PRs that close the issues, once each, in number order. A PR in
/// another repo is left out: `gh pr view` looks only in this repo, so its
/// number would name a different PR.
pub(super) fn linked_prs(wire: Vec<WireIssueLinks>) -> Vec<PrNumber> {
    let mut prs: Vec<PrNumber> = wire
        .into_iter()
        .flat_map(|issue| {
            let repo = issue.url.rsplit_once("/issues/").map(|(repo, _)| format!("{repo}/pull/"));
            issue
                .closed_by_pull_requests_references
                .into_iter()
                .filter(move |pr| repo.as_ref().is_some_and(|repo| pr.url.starts_with(repo.as_str())))
                .map(|pr| PrNumber(pr.number))
        })
        .collect();
    prs.sort_unstable();
    prs.dedup();
    prs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{BranchName, Checks, ForkHead, Mergeability};
    use crate::github::{ISSUE_FIELDS, PR_FIELDS};

    const PRS: &str = include_str!("../../../fixtures/prs.json");
    const ISSUES: &str = include_str!("../../../fixtures/issues.json");

    fn parse_issues(json: &str) -> Result<Vec<Issue>, GhError> {
        decode(json, "gh issue list").and_then(issues_from_wire)
    }

    fn parse_prs(json: &str) -> Result<Vec<PullRequest>, GhError> {
        decode(json, "gh pr list").and_then(prs_from_wire)
    }

    fn pr(n: u32) -> PullRequest {
        parse_prs(PRS).expect("fixture parses").into_iter().find(|p| p.number == PrNumber(n)).expect("pr in fixture")
    }

    #[test]
    fn real_prs_map_state_checks_and_closes() {
        let open = pr(1028);
        assert_eq!((open.state, open.checks, open.mergeability), (PrState::Open, Checks::Pass, Mergeability::Clean));
        assert_eq!(open.closes, [IssueNumber(711)]);
        assert_eq!(open.head.display(), "feature/711-hardware-metadata");
        assert_eq!(
            open.head.branch().map(BranchName::as_str),
            Some("fork/pr-1028"),
            "a fork's branch is not on origin"
        );
        assert!(matches!(pr(14927).head, PrBranch::Valid(_)), "a branch in the repo itself");
        assert_eq!(open.base.display(), "master");
        assert_eq!(pr(1002).updated_at.to_string(), "2026-10-12T08:33:48Z");
        assert_eq!(pr(1002).state, PrState::Merged);
        let draft = pr(14927);
        assert_eq!((draft.state, draft.checks), (PrState::Draft, Checks::Fail));
        let none = pr(14035);
        assert_eq!((none.state, none.checks), (PrState::Draft, Checks::None));
    }

    #[test]
    fn mixed_check_shapes_parse_by_typename() {
        let json = r#"[{"number":1,"title":"t","state":"OPEN","isDraft":false,"labels":[],"assignees":[],"milestone":null,"author":{"login":"a"},
            "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-03T03:04:05Z","headRefName":"h","isCrossRepository":false,"headRepository":null,"maintainerCanModify":false,"headRefOid":"abc","baseRefName":"b","additions":1,"deletions":2,
            "statusCheckRollup":[{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},
                                 {"__typename":"StatusContext","state":"SUCCESS"},
                                 {"__typename":"Future","x":1}],
            "mergeable":"MERGEABLE","mergeStateStatus":"BLOCKED","closingIssuesReferences":[]}]"#;
        assert_eq!(parse_prs(json).expect("parses")[0].checks, Checks::Pending);
    }

    #[test]
    fn unknown_pr_state_is_an_error_naming_the_pr() {
        let bad = PRS.replacen("\"state\": \"MERGED\"", "\"state\": \"WEIRD\"", 1);
        let err = parse_prs(&bad).expect_err("must fail");
        assert_eq!(err.to_string(), r#"PR #1002 has unknown state "WEIRD""#);
    }

    #[test]
    fn linked_prs_keep_this_repo_once_each_in_order() {
        let json = r#"[
            {"url":"https://github.com/o/r/issues/5","closedByPullRequestsReferences":[
                {"number":9,"url":"https://github.com/o/r/pull/9"},
                {"number":3,"url":"https://github.com/o/r/pull/3"}]},
            {"url":"https://github.com/o/r/issues/6","closedByPullRequestsReferences":[
                {"number":9,"url":"https://github.com/o/r/pull/9"},
                {"number":4,"url":"https://github.com/other/r/pull/4"},
                {"number":7,"url":"https://github.com/o/r2/pull/7"}]},
            {"url":"https://github.com/o/r/issues/8","closedByPullRequestsReferences":[]}]"#;
        let wire = decode(json, "gh issue list").expect("parses");
        assert_eq!(linked_prs(wire), [PrNumber(3), PrNumber(9)]);
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
        assert!(first.assignees.is_empty() && first.milestone.is_none());
        let assigned = issues.iter().find(|i| i.number == IssueNumber(788)).expect("issue in fixture");
        assert_eq!(assigned.assignees, ["sharkdp"]);
        assert_eq!(assigned.milestone.as_deref(), Some("hyperfine 2.0"));
    }

    #[test]
    fn real_prs_map_labels() {
        assert_eq!(pr(14035).labels, ["needs-triage", "external"]);
        assert!(pr(1028).labels.is_empty() && pr(1028).assignees.is_empty() && pr(1028).milestone.is_none());
    }

    #[test]
    fn a_fork_pr_can_take_a_push_only_when_it_allows_maintainer_edits() {
        let push = |n| match pr(n).head {
            PrBranch::Fork(ForkHead { push, .. }) => push,
            other => panic!("#{n} is from a fork: {other:?}"),
        };
        assert_eq!(push(14035), ForkPush::Allowed(RepoSlug::parse("maxbeizer/cli").expect("slug")));
        assert_eq!(push(1002), ForkPush::Denied, "maintainerCanModify is false");
        assert_eq!(push(1028), ForkPush::Denied, "the fork is gone");
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
