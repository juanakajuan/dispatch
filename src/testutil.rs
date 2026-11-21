//! Shared fixtures for unit tests.

use crate::domain::{
    Checks, CommitOid, Issue, IssueNumber, Mergeability, PageUrl, PrBranch, PrNumber, PrState, PullRequest, SessionKey,
};
use std::path::PathBuf;

pub fn issue(number: u32, title: &str, labels: &[&str]) -> Issue {
    Issue {
        number: IssueNumber(number),
        title: title.to_string(),
        labels: labels.iter().map(ToString::to_string).collect(),
        assignees: Vec::new(),
        milestone: None,
        author: "octo".to_string(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
        comment_count: 0,
        url: PageUrl::parse(&format!("https://github.com/o/r/issues/{number}")),
    }
}

pub fn pr(number: u32, head: &str, closes: &[u32]) -> PullRequest {
    PullRequest {
        number: PrNumber(number),
        title: format!("PR {number}"),
        state: PrState::Open,
        labels: Vec::new(),
        assignees: Vec::new(),
        milestone: None,
        author: "octo".to_string(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        updated_at: jiff::Timestamp::UNIX_EPOCH,
        head: PrBranch::parse(head.to_string()),
        head_oid: CommitOid::new(format!("sha{number}")),
        base: PrBranch::parse("main".to_string()),
        additions: 1,
        deletions: 0,
        checks: Checks::None,
        mergeability: Mergeability::Clean,
        closes: closes.iter().map(|n| IssueNumber(*n)).collect(),
        url: PageUrl::parse(&format!("https://github.com/o/r/pull/{number}")),
    }
}

/// A fresh, empty, canonical directory under the system temp dir.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dispatch-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonicalize temp dir")
}

pub fn issue_key(n: u32) -> SessionKey {
    SessionKey::Issue(IssueNumber(n))
}

pub fn pr_key(n: u32) -> SessionKey {
    SessionKey::Pr(PrNumber(n))
}
