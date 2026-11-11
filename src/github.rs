//! `gh` adapter. Wire structs are private; callers only see domain types.
//! `gh` infers the repo from the working directory, so no owner/name here.

use crate::domain::{Checks, Issue, IssueNumber, PrNumber, PrState, PullRequest};
use std::path::Path;

#[derive(Debug)]
pub struct FetchError(pub String);

pub fn fetch_issues(repo_root: &Path) -> Result<Vec<Issue>, FetchError> {
    // TODO: proc::run("gh", ["issue","list","--state","open","--limit","200",
    //       "--json","number,title,labels,author,createdAt,comments,body,url"], repo_root)
    //       then parse_issues.
    todo!()
}

pub fn fetch_prs(repo_root: &Path) -> Result<Vec<PullRequest>, FetchError> {
    // TODO: gh pr list --state all --limit 100 --json number,title,state,isDraft,author,
    //       createdAt,headRefName,baseRefName,additions,deletions,statusCheckRollup,
    //       closingIssuesReferences,url  then parse_prs.
    todo!()
}

/// Pure. Tested against fixture text captured from real `gh` output.
fn parse_issues(json: &str) -> Result<Vec<Issue>, FetchError> {
    todo!()
}

fn parse_prs(json: &str) -> Result<Vec<PullRequest>, FetchError> {
    // TODO: state: "OPEN" + isDraft -> Draft, "OPEN" -> Open, "MERGED", "CLOSED";
    //       any other string is a FetchError naming the PR (no silent default).
    //       closes: closingIssuesReferences[].number.
    //       checks: rollup(statusCheckRollup).
    todo!()
}

/// Any failure -> Fail, else any not completed -> Pending, else Pass.
/// Empty -> Checks::None.
fn rollup(items: &[WireCheck]) -> Checks {
    todo!()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireIssue {
    number: u32,
    title: String,
    labels: Vec<WireLabel>,
    author: WireAuthor,
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
    author: WireAuthor,
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
    CheckRun { status: String, conclusion: String },
    StatusContext { state: String },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    // TODO: fixture from `gh pr list` in a real repo: MERGED, OPEN+isDraft, empty rollup,
    //       mixed CheckRun IN_PROGRESS + SUCCESS -> Pending, any FAILURE -> Fail.
    // TODO: unknown PR state -> error, not a default.
    // TODO: issue with 6 comments -> comment_count 6; labels -> names.
}
