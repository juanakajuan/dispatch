//! The second `m`: look at the PR again, because the list can be a minute
//! old, then merge it here or hand it to the session's agent.

use crate::domain::{MergeMethod, PrNumber};
use crate::rules::{self, MergePlan, MergeProblem};
use crate::{github, proc};
use std::path::Path;

/// The per-project setting, in the repo's git config.
pub const SETTING: &str = "dispatch.mergeMethod";

#[derive(Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged(MergeMethod),
    /// The agent must fix `problem`, then merge with `method`.
    Handoff { problem: MergeProblem, method: MergeMethod, base: String },
    Refused(String),
}

pub fn merge(repo_root: &Path, pr: PrNumber) -> Result<MergeOutcome, String> {
    let fresh = github::view_pr(repo_root, pr).map_err(|e| e.to_string())?;
    let problem = match rules::merge_plan(&fresh) {
        MergePlan::Refuse(why) => return Ok(MergeOutcome::Refused(why)),
        MergePlan::Direct => None,
        MergePlan::Handoff(problem) => Some(problem),
    };
    let allowed = github::allowed_merge_methods(repo_root).map_err(|e| e.to_string())?;
    let method = rules::merge_method(configured_method(repo_root)?, &allowed)?;
    match problem {
        None => {
            github::merge_pr(repo_root, pr, method).map_err(|e| e.to_string())?;
            Ok(MergeOutcome::Merged(method))
        }
        Some(problem) => Ok(MergeOutcome::Handoff { problem, method, base: fresh.base }),
    }
}

/// `git config --get` exits 1 when the key is unset.
fn configured_method(repo_root: &Path) -> Result<Option<MergeMethod>, String> {
    match proc::run("git", ["config", "--get", SETTING], repo_root) {
        Ok(raw) => MergeMethod::parse(&raw)
            .map(Some)
            .ok_or_else(|| format!("{SETTING} is {:?}. Use merge, squash, or rebase.", raw.trim())),
        Err(e) if e.exit_code() == Some(1) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;

    const ALL_ALLOWED: &str = r#"{"mergeCommitAllowed":true,"rebaseMergeAllowed":true,"squashMergeAllowed":true}"#;

    fn view(mergeable: &str, status: &str, checks: &str) -> String {
        format!(
            r#"{{"number":10,"title":"t","state":"OPEN","isDraft":false,"author":null,
            "createdAt":"2026-01-02T03:04:05Z","headRefName":"fix/3-x","baseRefName":"main","additions":1,"deletions":0,
            "statusCheckRollup":[{checks}],"mergeable":"{mergeable}","mergeStateStatus":"{status}",
            "closingIssuesReferences":[],"url":"u"}}"#
        )
    }

    /// Answers `gh pr view` with `pr`, `git config` with `setting`, `gh repo
    /// view` with `allowed`, and anything else (the merge) with success.
    fn world(pr: String, setting: Option<&'static str>, allowed: &'static str) {
        fake::install(move |call| {
            if call.starts_with("gh pr view") {
                Ok(pr.clone())
            } else if call.starts_with("git config") {
                setting.map(|s| format!("{s}\n")).ok_or_else(|| fake::fail("git", 1, ""))
            } else if call.starts_with("gh repo view") {
                Ok(allowed.to_string())
            } else {
                Ok(String::new())
            }
        });
    }

    fn merges() -> Vec<String> {
        fake::calls().into_iter().filter(|c| c.starts_with("gh pr merge")).collect()
    }

    #[test]
    fn a_clean_pr_merges_with_the_configured_method() {
        world(view("MERGEABLE", "CLEAN", ""), Some("squash"), ALL_ALLOWED);
        assert_eq!(merge(Path::new("/r"), PrNumber(10)), Ok(MergeOutcome::Merged(MergeMethod::Squash)));
        assert_eq!(merges(), ["gh pr merge 10 --squash"]);
        assert!(fake::calls().contains(&"git config --get dispatch.mergeMethod".to_string()));
    }

    #[test]
    fn without_a_setting_a_clean_pr_merges_with_the_only_allowed_method() {
        let only_rebase = r#"{"mergeCommitAllowed":false,"rebaseMergeAllowed":true,"squashMergeAllowed":false}"#;
        world(view("MERGEABLE", "CLEAN", ""), None, only_rebase);
        assert_eq!(merge(Path::new("/r"), PrNumber(10)), Ok(MergeOutcome::Merged(MergeMethod::Rebase)));
        assert_eq!(merges(), ["gh pr merge 10 --rebase"]);
    }

    #[test]
    fn a_conflicted_pr_goes_to_the_agent_with_the_method_and_is_not_merged() {
        world(view("CONFLICTING", "DIRTY", ""), Some("rebase"), ALL_ALLOWED);
        let outcome = merge(Path::new("/r"), PrNumber(10));
        let want = MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, base: "main".to_string() };
        assert_eq!(outcome, Ok(want));
        assert!(merges().is_empty());
    }

    #[test]
    fn failing_checks_go_to_the_agent() {
        let failed = r#"{"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"}"#;
        world(view("MERGEABLE", "BLOCKED", failed), None, ALL_ALLOWED);
        assert!(matches!(merge(Path::new("/r"), PrNumber(10)), Ok(MergeOutcome::Handoff { problem: MergeProblem::ChecksFailing, .. })));
        assert!(merges().is_empty());
    }

    #[test]
    fn pending_checks_refuse_without_reading_the_setting() {
        let running = r#"{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}"#;
        world(view("MERGEABLE", "BLOCKED", running), Some("nonsense"), ALL_ALLOWED);
        assert!(matches!(merge(Path::new("/r"), PrNumber(10)), Ok(MergeOutcome::Refused(why)) if why.contains("still running")));
        assert_eq!(fake::calls().len(), 1, "only the fresh view");
    }

    #[test]
    fn a_method_the_repo_does_not_allow_is_an_error_and_nothing_merges() {
        let no_squash = r#"{"mergeCommitAllowed":true,"rebaseMergeAllowed":true,"squashMergeAllowed":false}"#;
        world(view("MERGEABLE", "CLEAN", ""), Some("squash"), no_squash);
        let err = merge(Path::new("/r"), PrNumber(10)).expect_err("rejected");
        assert!(err.contains("squash") && err.contains("merge, rebase"), "{err}");
        assert!(merges().is_empty());
    }

    #[test]
    fn a_bad_setting_is_an_error_that_names_the_key() {
        world(view("MERGEABLE", "CLEAN", ""), Some("fast-forward"), ALL_ALLOWED);
        let err = merge(Path::new("/r"), PrNumber(10)).expect_err("rejected");
        assert_eq!(err, r#"dispatch.mergeMethod is "fast-forward". Use merge, squash, or rebase."#);
        assert!(merges().is_empty());
    }
}
