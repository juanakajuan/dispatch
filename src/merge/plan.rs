//! Pure merge rules. No I/O.

use crate::domain::{Checks, MergeMethod, Mergeability, PrNumber, PrState, PullRequest};
use std::fmt;

/// What `m` does with a PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergePlan {
    /// dispatch runs `gh pr merge` itself.
    Direct,
    /// The session's agent fixes the problem, then merges.
    Handoff(MergeProblem),
    /// A draft. dispatch marks it ready, looks at it again, and plans from that.
    MarkReady,
    /// Nothing to do now.
    Refuse(MergeRefusal),
}

/// Why `m` does nothing now. Each is one sentence for the toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeRefusal {
    AlreadyMerged(PrNumber),
    Closed(PrNumber),
    ChecksRunning(PrNumber),
    MergeabilityUnknown(PrNumber),
    BlockedWithoutChecks(PrNumber),
    /// The PR became a draft after the first `m`, which did not ask to mark it ready.
    DraftNow(PrNumber),
    /// `gh pr ready` ran, and GitHub still shows a draft.
    StillDraft(PrNumber),
}

impl fmt::Display for MergeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeRefusal::AlreadyMerged(n) => write!(f, "PR {n} is already merged."),
            MergeRefusal::Closed(n) => write!(f, "PR {n} is closed."),
            MergeRefusal::ChecksRunning(n) => {
                write!(f, "PR {n} has checks that are still running. Press m again when they finish.")
            }
            MergeRefusal::MergeabilityUnknown(n) => {
                write!(f, "GitHub does not know yet if PR {n} can merge. Try again in a moment.")
            }
            MergeRefusal::BlockedWithoutChecks(n) => write!(
                f,
                "PR {n} is blocked and has no checks yet. If checks are about to start, press m again soon. If it needs a review, get one first."
            ),
            MergeRefusal::DraftNow(n) => {
                write!(f, "PR {n} is a draft now. Press m twice again to mark it ready and merge it.")
            }
            MergeRefusal::StillDraft(n) => write!(f, "PR {n} is still a draft."),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeProblem {
    Conflict,
    ChecksFailing,
    Behind,
    Blocked,
}

/// Pure. Order matters: a conflict needs the agent even while checks run,
/// and pending checks wait rather than hand off, because they may pass.
pub fn merge_plan(pr: &PullRequest) -> MergePlan {
    let n = pr.number;
    match pr.state {
        PrState::Open => {}
        PrState::Draft => return MergePlan::MarkReady,
        PrState::Merged => return MergePlan::Refuse(MergeRefusal::AlreadyMerged(n)),
        PrState::Closed => return MergePlan::Refuse(MergeRefusal::Closed(n)),
    }
    match (pr.mergeability, pr.checks) {
        (Mergeability::Conflict, _) => MergePlan::Handoff(MergeProblem::Conflict),
        #[expect(clippy::match_same_arms, reason = "order matters: `Pending` must refuse before `Unstable` hands off")]
        (_, Checks::Fail) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (_, Checks::Pending) => MergePlan::Refuse(MergeRefusal::ChecksRunning(n)),
        (Mergeability::Unstable, _) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (Mergeability::Unknown, _) => MergePlan::Refuse(MergeRefusal::MergeabilityUnknown(n)),
        (Mergeability::Behind, _) => MergePlan::Handoff(MergeProblem::Behind),
        (Mergeability::Blocked, Checks::None) => MergePlan::Refuse(MergeRefusal::BlockedWithoutChecks(n)),
        (Mergeability::Blocked, _) => MergePlan::Handoff(MergeProblem::Blocked),
        (Mergeability::Clean, _) => MergePlan::Direct,
    }
}

/// One sentence that names the problem, for the toast and the agent prompt.
pub fn problem_sentence(pr: PrNumber, base: &str, problem: MergeProblem) -> String {
    match problem {
        MergeProblem::Conflict => format!("PR {pr} has a merge conflict with {base}."),
        MergeProblem::ChecksFailing => format!("PR {pr} has failing checks."),
        MergeProblem::Behind => format!("PR {pr} is behind {base}, and the repo requires an up-to-date branch."),
        MergeProblem::Blocked => format!("PR {pr} is blocked by a branch rule, such as a required review."),
    }
}

/// What `m` types into the session's Claude prompt when the PR has a problem.
pub fn handoff_prompt(pr: PrNumber, base: &str, problem: MergeProblem, method: MergeMethod) -> String {
    format!(
        "{} Fix it and push. When the checks pass, merge it with `gh pr merge {n} --{method}`.",
        problem_sentence(pr, base, problem),
        n = pr.0
    )
}

/// The first prompt of a new session that `m` starts to fix a PR. The
/// worktree's branch can be older than the PR, so it syncs first.
pub fn fix_session_prompt(pr: PrNumber, base: &str, problem: MergeProblem, method: MergeMethod) -> String {
    format!(
        "This worktree is on the branch of PR #{n}. Run `gh pr checkout {n}` to get its latest commits. {}",
        handoff_prompt(pr, base, problem, method),
        n = pr.0
    )
}

/// The method to merge with. `configured` is the `dispatch.mergeMethod`
/// setting; `allowed` is what the repo allows, in gh order. Without a
/// setting, the first allowed method wins, so a repo that allows one method
/// gets that one and a repo that allows all gets `merge`.
pub fn merge_method(configured: Option<MergeMethod>, allowed: &[MergeMethod]) -> Result<MergeMethod, MethodError> {
    match configured {
        Some(m) if allowed.contains(&m) => Ok(m),
        Some(m) => Err(MethodError::NotAllowed { configured: m, allowed: allowed.to_vec() }),
        None => allowed.first().copied().ok_or(MethodError::NoneAllowed),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MethodError {
    NotAllowed { configured: MergeMethod, allowed: Vec<MergeMethod> },
    NoneAllowed,
}

impl fmt::Display for MethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MethodError::NotAllowed { configured, allowed } => {
                let names = allowed.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ");
                write!(f, "dispatch.mergeMethod is {configured}, but the repo allows only: {names}")
            }
            MethodError::NoneAllowed => f.write_str("the repo allows no merge method"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::pr;

    #[test]
    fn merge_plan_merges_clean_prs_and_hands_off_problems() {
        let plan = |mergeability, checks| merge_plan(&PullRequest { checks, mergeability, ..pr(10, "h", &[]) });
        assert_eq!(plan(Mergeability::Clean, Checks::Pass), MergePlan::Direct);
        assert_eq!(plan(Mergeability::Clean, Checks::None), MergePlan::Direct);
        assert_eq!(plan(Mergeability::Conflict, Checks::Pending), MergePlan::Handoff(MergeProblem::Conflict));
        assert_eq!(plan(Mergeability::Blocked, Checks::Fail), MergePlan::Handoff(MergeProblem::ChecksFailing));
        assert_eq!(plan(Mergeability::Unstable, Checks::Pass), MergePlan::Handoff(MergeProblem::ChecksFailing));
        assert_eq!(plan(Mergeability::Behind, Checks::Pass), MergePlan::Handoff(MergeProblem::Behind));
        assert_eq!(plan(Mergeability::Blocked, Checks::Pass), MergePlan::Handoff(MergeProblem::Blocked));
        let n = PrNumber(10);
        assert_eq!(plan(Mergeability::Blocked, Checks::Pending), MergePlan::Refuse(MergeRefusal::ChecksRunning(n)));
        assert_eq!(plan(Mergeability::Unknown, Checks::Pass), MergePlan::Refuse(MergeRefusal::MergeabilityUnknown(n)));
        assert_eq!(
            plan(Mergeability::Blocked, Checks::None),
            MergePlan::Refuse(MergeRefusal::BlockedWithoutChecks(n)),
            "nothing for the agent to fix"
        );
    }

    #[test]
    fn merge_plan_marks_a_draft_ready_and_refuses_a_closed_pr() {
        let plan = |state| merge_plan(&PullRequest { state, ..pr(10, "h", &[]) });
        assert_eq!(plan(PrState::Draft), MergePlan::MarkReady);
        assert_eq!(plan(PrState::Merged), MergePlan::Refuse(MergeRefusal::AlreadyMerged(PrNumber(10))));
        assert_eq!(plan(PrState::Closed), MergePlan::Refuse(MergeRefusal::Closed(PrNumber(10))));
    }

    #[test]
    fn each_refusal_is_one_sentence_for_the_toast() {
        let n = PrNumber(10);
        let cases = [
            (MergeRefusal::AlreadyMerged(n), "PR #10 is already merged."),
            (MergeRefusal::Closed(n), "PR #10 is closed."),
            (
                MergeRefusal::ChecksRunning(n),
                "PR #10 has checks that are still running. Press m again when they finish.",
            ),
            (
                MergeRefusal::MergeabilityUnknown(n),
                "GitHub does not know yet if PR #10 can merge. Try again in a moment.",
            ),
            (
                MergeRefusal::BlockedWithoutChecks(n),
                "PR #10 is blocked and has no checks yet. If checks are about to start, press m again soon. If it needs a review, get one first.",
            ),
            (MergeRefusal::DraftNow(n), "PR #10 is a draft now. Press m twice again to mark it ready and merge it."),
            (MergeRefusal::StillDraft(n), "PR #10 is still a draft."),
        ];
        for (refusal, sentence) in cases {
            assert_eq!(refusal.to_string(), sentence);
        }
    }

    #[test]
    fn handoff_prompt_names_the_problem_and_the_method() {
        assert_eq!(
            handoff_prompt(PrNumber(12), "main", MergeProblem::Conflict, MergeMethod::Squash),
            "PR #12 has a merge conflict with main. Fix it and push. When the checks pass, merge it with `gh pr merge 12 --squash`."
        );
        assert!(
            handoff_prompt(PrNumber(12), "main", MergeProblem::Behind, MergeMethod::Rebase)
                .starts_with("PR #12 is behind main,")
        );
        assert_eq!(
            fix_session_prompt(PrNumber(12), "main", MergeProblem::ChecksFailing, MergeMethod::Merge),
            "This worktree is on the branch of PR #12. Run `gh pr checkout 12` to get its latest commits. \
             PR #12 has failing checks. Fix it and push. When the checks pass, merge it with `gh pr merge 12 --merge`."
        );
    }

    #[test]
    fn merge_method_uses_the_setting_or_the_first_allowed() {
        use MergeMethod::{Merge, Rebase, Squash};
        assert_eq!(merge_method(None, &[Merge, Squash, Rebase]), Ok(Merge));
        assert_eq!(merge_method(None, &[Rebase]), Ok(Rebase), "the only allowed method");
        assert_eq!(merge_method(None, &[Squash, Rebase]), Ok(Squash));
        assert_eq!(merge_method(Some(Rebase), &[Merge, Rebase]), Ok(Rebase));
        let err = merge_method(Some(Squash), &[Merge, Rebase]).expect_err("not allowed");
        assert_eq!(err.to_string(), "dispatch.mergeMethod is squash, but the repo allows only: merge, rebase");
        assert_eq!(merge_method(None, &[]), Err(MethodError::NoneAllowed));
        assert_eq!(MethodError::NoneAllowed.to_string(), "the repo allows no merge method");
    }
}
