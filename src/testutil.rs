//! Shared fixtures for unit tests.

use crate::domain::{Checks, CommitOid, Issue, IssueNumber, Mergeability, PrBranch, PrNumber, PrState, PullRequest};
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
    }
}

/// A fresh, empty, canonical directory under the system temp dir.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dispatch-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonicalize temp dir")
}
