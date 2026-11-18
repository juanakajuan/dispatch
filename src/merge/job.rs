//! The second `m`: look at the PR again, because the list can be a minute
//! old, then merge it here or hand it to the session's agent.

use super::plan::{self, MergePlan, MergeProblem, MergeRefusal};
use super::{Consent, MergeAsk, MergeError};
use crate::domain::{MergeMethod, PrNumber, PrState, PullRequest};
use crate::{github, proc};
use std::path::Path;

/// The per-project setting, in the repo's git config.
pub const SETTING: &str = "dispatch.mergeMethod";

#[derive(Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged(MergeMethod),
    /// `gh pr merge` succeeded but the PR is still open: a merge queue took it.
    Queued,
    /// An agent must fix `problem`, then merge with `method`. `pr` is the PR
    /// as GitHub showed it now.
    Handoff {
        problem: MergeProblem,
        method: MergeMethod,
        pr: Box<PullRequest>,
    },
    Refused(MergeRefusal),
}

/// What the job did. `draft` stays `MarkedReady` even when `result` is not
/// a merge, because dispatch does not undo `gh pr ready`.
#[derive(Debug, PartialEq, Eq)]
pub struct MergeRun {
    pub draft: DraftStep,
    pub result: Result<MergeOutcome, MergeError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftStep {
    /// The job did not run `gh pr ready`.
    WasReady,
    MarkedReady,
}

pub fn merge(repo_root: &Path, ask: MergeAsk) -> MergeRun {
    let pr = ask.pr;
    let was_ready = |result| MergeRun { draft: DraftStep::WasReady, result };
    let fresh = match github::view_pr(repo_root, pr) {
        Ok(fresh) => fresh,
        Err(e) => return was_ready(Err(e.into())),
    };
    if plan::merge_plan(&fresh) != MergePlan::MarkReady {
        return was_ready(finish(repo_root, pr, &fresh));
    }
    if ask.consent != Consent::MarkReadyThenMerge {
        return was_ready(Ok(MergeOutcome::Refused(MergeRefusal::DraftNow(pr))));
    }
    if let Err(e) = github::mark_ready(repo_root, pr) {
        return was_ready(Err(e.into()));
    }
    let result = github::view_pr(repo_root, pr).map_err(MergeError::from).and_then(|f| finish(repo_root, pr, &f));
    MergeRun { draft: DraftStep::MarkedReady, result }
}

/// Merges `fresh`, the PR as GitHub shows it now, or says who must fix it.
fn finish(repo_root: &Path, pr: PrNumber, fresh: &PullRequest) -> Result<MergeOutcome, MergeError> {
    let problem = match plan::merge_plan(fresh) {
        MergePlan::Refuse(why) => return Ok(MergeOutcome::Refused(why)),
        MergePlan::MarkReady => return Ok(MergeOutcome::Refused(MergeRefusal::StillDraft(pr))),
        MergePlan::Direct => None,
        MergePlan::Handoff(problem) => Some(problem),
    };
    let allowed = github::allowed_merge_methods(repo_root)?;
    let method = plan::merge_method(configured_method(repo_root)?, &allowed).map_err(MergeError::Method)?;
    match problem {
        None => {
            github::merge_pr(repo_root, fresh, method)?;
            let merged = github::view_pr(repo_root, pr).map_or(true, |after| after.state == PrState::Merged);
            Ok(if merged { MergeOutcome::Merged(method) } else { MergeOutcome::Queued })
        }
        Some(problem) => Ok(MergeOutcome::Handoff { problem, method, pr: Box::new(fresh.clone()) }),
    }
}

/// `git config --get` exits 1 when the key is unset.
fn configured_method(repo_root: &Path) -> Result<Option<MergeMethod>, MergeError> {
    match proc::run("git", ["config", "--get", SETTING], repo_root) {
        Ok(raw) => MergeMethod::parse(&raw).map(Some).ok_or_else(|| MergeError::BadSetting(raw.trim().to_string())),
        Err(e) if e.exit_code() == Some(1) => Ok(None),
        Err(e) => Err(MergeError::Setting(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::GhError;
    use crate::merge::plan::MethodError;
    use crate::proc::fake;

    const ALL_ALLOWED: &str = r#"{"mergeCommitAllowed":true,"rebaseMergeAllowed":true,"squashMergeAllowed":true}"#;

    fn view(mergeable: &str, status: &str, checks: &str) -> String {
        view_in(PrState::Open, mergeable, status, checks)
    }

    fn view_in(state: PrState, mergeable: &str, status: &str, checks: &str) -> String {
        let (state, draft) = match state {
            PrState::Open => ("OPEN", false),
            PrState::Draft => ("OPEN", true),
            PrState::Merged => ("MERGED", false),
            PrState::Closed => ("CLOSED", false),
        };
        format!(
            r#"{{"number":10,"title":"t","state":"{state}","isDraft":{draft},"labels":[],"assignees":[],"milestone":null,"author":null,
            "createdAt":"2026-01-02T03:04:05Z","updatedAt":"2026-01-02T03:04:05Z","headRefName":"fix/3-x","isCrossRepository":false,"headRefOid":"abc","baseRefName":"main","additions":1,"deletions":0,
            "statusCheckRollup":[{checks}],"mergeable":"{mergeable}","mergeStateStatus":"{status}",
            "closingIssuesReferences":[],"url":"u"}}"#
        )
    }

    /// Answers `gh pr view` with `pr` before the merge and a merged PR after
    /// it, `git config` with `setting`, `gh repo view` with `allowed`, and the
    /// merge with success.
    fn world(pr: String, setting: Option<&'static str>, allowed: &'static str) {
        world_after(pr, view_in(PrState::Merged, "UNKNOWN", "UNKNOWN", ""), setting, allowed, Ok(String::new()));
    }

    fn world_after(
        pr: String,
        after: String,
        setting: Option<&'static str>,
        allowed: &'static str,
        merge: Result<String, &'static str>,
    ) {
        let mut merged = false;
        fake::install(move |call| {
            if call.starts_with("gh pr view") {
                Ok(if merged { after.clone() } else { pr.clone() })
            } else if call.starts_with("gh pr merge") {
                merged = merge.is_ok();
                merge.clone().map_err(|e| fake::fail("gh", 1, e))
            } else if call.starts_with("git config") {
                setting.map(|s| format!("{s}\n")).ok_or_else(|| fake::fail("git", 1, ""))
            } else if call.starts_with("gh repo view") {
                Ok(allowed.to_string())
            } else {
                Ok(String::new())
            }
        });
    }

    fn ask(consent: Consent) -> MergeAsk {
        MergeAsk { pr: PrNumber(10), consent }
    }

    fn outcome() -> Result<MergeOutcome, MergeError> {
        merge(Path::new("/r"), ask(Consent::MergeOnly)).result
    }

    /// The PR is a draft until `gh pr ready` runs, then it is `after`, and a
    /// merge makes it merged.
    fn draft_world(after: String, ready: Result<&'static str, &'static str>) {
        let (mut is_ready, mut merged) = (false, false);
        fake::install(move |call| {
            if call.starts_with("gh pr view") {
                Ok(if merged {
                    view_in(PrState::Merged, "UNKNOWN", "UNKNOWN", "")
                } else if is_ready {
                    after.clone()
                } else {
                    view_in(PrState::Draft, "MERGEABLE", "CLEAN", "")
                })
            } else if call.starts_with("gh pr ready") {
                is_ready = ready.is_ok();
                ready.map(str::to_string).map_err(|e| fake::fail("gh", 1, e))
            } else if call.starts_with("gh pr merge") {
                merged = true;
                Ok(String::new())
            } else if call.starts_with("gh repo view") {
                Ok(ALL_ALLOWED.to_string())
            } else if call.starts_with("git config") {
                Err(fake::fail("git", 1, ""))
            } else {
                Ok(String::new())
            }
        });
    }

    fn readies() -> Vec<String> {
        fake::calls().into_iter().filter(|c| c.starts_with("gh pr ready")).collect()
    }

    fn views() -> usize {
        fake::calls().iter().filter(|c| c.starts_with("gh pr view")).count()
    }

    #[test]
    fn a_draft_is_marked_ready_looked_at_again_and_merged_when_clean() {
        draft_world(view("MERGEABLE", "CLEAN", ""), Ok(""));
        let run = merge(Path::new("/r"), ask(Consent::MarkReadyThenMerge));
        assert_eq!(
            run,
            MergeRun { draft: DraftStep::MarkedReady, result: Ok(MergeOutcome::Merged(MergeMethod::Merge)) }
        );
        assert_eq!(readies(), ["gh pr ready 10"]);
        assert_eq!(merges(), ["gh pr merge 10 --merge --match-head-commit abc"]);
        assert_eq!(views(), 3, "before ready, after ready, after the merge");
    }

    #[test]
    fn a_draft_with_pending_checks_after_ready_is_refused_and_stays_ready() {
        let running = r#"{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}"#;
        draft_world(view("MERGEABLE", "BLOCKED", running), Ok(""));
        let run = merge(Path::new("/r"), ask(Consent::MarkReadyThenMerge));
        assert_eq!(run.draft, DraftStep::MarkedReady);
        assert_eq!(run.result, Ok(MergeOutcome::Refused(MergeRefusal::ChecksRunning(PrNumber(10)))));
        assert!(merges().is_empty());
    }

    #[test]
    fn a_draft_that_is_still_a_draft_after_ready_is_refused() {
        draft_world(view_in(PrState::Draft, "MERGEABLE", "CLEAN", ""), Ok(""));
        let run = merge(Path::new("/r"), ask(Consent::MarkReadyThenMerge));
        assert_eq!(run.draft, DraftStep::MarkedReady);
        assert_eq!(run.result, Ok(MergeOutcome::Refused(MergeRefusal::StillDraft(PrNumber(10)))));
        assert!(merges().is_empty());
    }

    #[test]
    fn a_draft_without_consent_is_refused_and_gh_pr_ready_does_not_run() {
        draft_world(view("MERGEABLE", "CLEAN", ""), Ok(""));
        let run = merge(Path::new("/r"), ask(Consent::MergeOnly));
        assert_eq!(run.draft, DraftStep::WasReady);
        assert_eq!(run.result, Ok(MergeOutcome::Refused(MergeRefusal::DraftNow(PrNumber(10)))));
        assert!(readies().is_empty() && merges().is_empty());
        assert_eq!(fake::calls().len(), 1, "only the fresh view");
    }

    #[test]
    fn a_failing_gh_pr_ready_is_an_error_and_nothing_merges() {
        draft_world(view("MERGEABLE", "CLEAN", ""), Err("not allowed"));
        let run = merge(Path::new("/r"), ask(Consent::MarkReadyThenMerge));
        assert_eq!(run.draft, DraftStep::WasReady);
        assert_eq!(run.result, Err(MergeError::Gh(GhError::Run(fake::fail("gh", 1, "not allowed")))));
        assert!(merges().is_empty());
    }

    fn merges() -> Vec<String> {
        fake::calls().into_iter().filter(|c| c.starts_with("gh pr merge")).collect()
    }

    #[test]
    fn a_clean_pr_merges_with_the_configured_method() {
        world(view("MERGEABLE", "CLEAN", ""), Some("squash"), ALL_ALLOWED);
        assert_eq!(outcome(), Ok(MergeOutcome::Merged(MergeMethod::Squash)));
        assert_eq!(merges(), ["gh pr merge 10 --squash --match-head-commit abc"]);
        assert!(fake::calls().contains(&"git config --get dispatch.mergeMethod".to_string()));
    }

    #[test]
    fn without_a_setting_a_clean_pr_merges_with_the_only_allowed_method() {
        let only_rebase = r#"{"mergeCommitAllowed":false,"rebaseMergeAllowed":true,"squashMergeAllowed":false}"#;
        world(view("MERGEABLE", "CLEAN", ""), None, only_rebase);
        assert_eq!(outcome(), Ok(MergeOutcome::Merged(MergeMethod::Rebase)));
        assert_eq!(merges(), ["gh pr merge 10 --rebase --match-head-commit abc"]);
    }

    #[test]
    fn a_conflicted_pr_goes_to_the_agent_with_the_method_and_is_not_merged() {
        world(view("CONFLICTING", "DIRTY", ""), Some("rebase"), ALL_ALLOWED);
        let outcome = outcome();
        assert!(
            matches!(&outcome, Ok(MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr }) if pr.base.display() == "main" && pr.head.display() == "fix/3-x"),
            "{outcome:?}"
        );
        assert!(merges().is_empty());
    }

    #[test]
    fn failing_checks_go_to_the_agent() {
        let failed = r#"{"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"}"#;
        world(view("MERGEABLE", "BLOCKED", failed), None, ALL_ALLOWED);
        assert!(matches!(outcome(), Ok(MergeOutcome::Handoff { problem: MergeProblem::ChecksFailing, .. })));
        assert!(merges().is_empty());
    }

    #[test]
    fn pending_checks_refuse_without_reading_the_setting() {
        let running = r#"{"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""}"#;
        world(view("MERGEABLE", "BLOCKED", running), Some("nonsense"), ALL_ALLOWED);
        assert_eq!(outcome(), Ok(MergeOutcome::Refused(MergeRefusal::ChecksRunning(PrNumber(10)))));
        assert_eq!(fake::calls().len(), 1, "only the fresh view");
    }

    #[test]
    fn a_method_the_repo_does_not_allow_is_an_error_and_nothing_merges() {
        let no_squash = r#"{"mergeCommitAllowed":true,"rebaseMergeAllowed":true,"squashMergeAllowed":false}"#;
        world(view("MERGEABLE", "CLEAN", ""), Some("squash"), no_squash);
        let err = outcome().expect_err("rejected");
        assert!(matches!(err, MergeError::Method(MethodError::NotAllowed { configured: MergeMethod::Squash, .. })));
        assert!(merges().is_empty());
    }

    #[test]
    fn a_bad_setting_is_an_error_that_names_the_key() {
        world(view("MERGEABLE", "CLEAN", ""), Some("fast-forward"), ALL_ALLOWED);
        let err = outcome().expect_err("rejected");
        assert_eq!(err.to_string(), r#"dispatch.mergeMethod is "fast-forward". Use merge, squash, or rebase."#);
        assert!(merges().is_empty());
    }

    #[test]
    fn a_merge_queue_that_takes_the_pr_reads_as_queued_not_merged() {
        let still_open = view("MERGEABLE", "CLEAN", "");
        world_after(view("MERGEABLE", "CLEAN", ""), still_open, None, ALL_ALLOWED, Ok(String::new()));
        assert_eq!(outcome(), Ok(MergeOutcome::Queued));
    }

    #[test]
    fn a_failed_gh_merge_is_an_error() {
        let after = view_in(PrState::Merged, "UNKNOWN", "UNKNOWN", "");
        world_after(view("MERGEABLE", "CLEAN", ""), after, None, ALL_ALLOWED, Err("Head branch was modified"));
        let err = outcome().expect_err("fails");
        assert_eq!(err, MergeError::Gh(GhError::Run(fake::fail("gh", 1, "Head branch was modified"))));
    }

    #[test]
    fn behind_and_blocked_prs_go_to_the_agent() {
        for (status, problem) in [("BEHIND", MergeProblem::Behind), ("BLOCKED", MergeProblem::Blocked)] {
            let passed = r#"{"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"}"#;
            world(view("MERGEABLE", status, passed), None, ALL_ALLOWED);
            let outcome = outcome();
            assert!(
                matches!(outcome, Ok(MergeOutcome::Handoff { problem: p, method: MergeMethod::Merge, .. }) if p == problem),
                "{status}"
            );
            assert!(merges().is_empty());
        }
    }
}
