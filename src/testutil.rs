//! Shared fixtures for unit tests.

use crate::domain::{Checks, Issue, IssueNumber, Mergeability, PrNumber, PrState, PullRequest};
use std::path::PathBuf;

pub fn issue(number: u32, title: &str, labels: &[&str]) -> Issue {
    Issue {
        number: IssueNumber(number),
        title: title.to_string(),
        labels: labels.iter().map(ToString::to_string).collect(),
        author: "octo".to_string(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        comment_count: 0,
        body: String::new(),
        url: String::new(),
    }
}

pub fn pr(number: u32, head: &str, closes: &[u32]) -> PullRequest {
    PullRequest {
        number: PrNumber(number),
        title: format!("PR {number}"),
        state: PrState::Open,
        author: "octo".to_string(),
        created_at: jiff::Timestamp::UNIX_EPOCH,
        head: head.to_string(),
        base: "main".to_string(),
        additions: 1,
        deletions: 0,
        checks: Checks::None,
        mergeability: Mergeability::Clean,
        closes: closes.iter().map(|n| IssueNumber(*n)).collect(),
        url: String::new(),
    }
}

/// A fresh, empty, canonical directory under the system temp dir.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dispatch-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::canonicalize(&dir).expect("canonicalize temp dir")
}
