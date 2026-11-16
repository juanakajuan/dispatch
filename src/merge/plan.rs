//! Pure merge rules. No I/O.

use crate::domain::{Checks, MergeMethod, Mergeability, PrNumber, PrState, PullRequest};

/// What `m` does with a PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergePlan {
    /// dispatch runs `gh pr merge` itself.
    Direct,
    /// The session's agent fixes the problem, then merges.
    Handoff(MergeProblem),
    /// A draft. dispatch marks it ready, looks at it again, and plans from that.
    MarkReady,
    /// Nothing to do now. The text says why.
    Refuse(String),
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
    let n = pr.number.0;
    match pr.state {
        PrState::Open => {}
        PrState::Draft => return MergePlan::MarkReady,
        PrState::Merged => return MergePlan::Refuse(format!("PR #{n} is already merged.")),
        PrState::Closed => return MergePlan::Refuse(format!("PR #{n} is closed.")),
    }
    match (pr.mergeability, pr.checks) {
        (Mergeability::Conflict, _) => MergePlan::Handoff(MergeProblem::Conflict),
        #[expect(clippy::match_same_arms, reason = "order matters: `Pending` must refuse before `Unstable` hands off")]
        (_, Checks::Fail) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (_, Checks::Pending) => {
            MergePlan::Refuse(format!("PR #{n} has checks that are still running. Press m again when they finish."))
        }
        (Mergeability::Unstable, _) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (Mergeability::Unknown, _) => {
            MergePlan::Refuse(format!("GitHub does not know yet if PR #{n} can merge. Try again in a moment."))
        }
        (Mergeability::Behind, _) => MergePlan::Handoff(MergeProblem::Behind),
        (Mergeability::Blocked, Checks::None) => MergePlan::Refuse(format!(
            "PR #{n} is blocked and has no checks yet. If checks are about to start, press m again soon. If it needs a review, get one first."
        )),
        (Mergeability::Blocked, _) => MergePlan::Handoff(MergeProblem::Blocked),
        (Mergeability::Clean, _) => MergePlan::Direct,
    }
}

/// One sentence that names the problem, for the toast and the agent prompt.
pub fn problem_sentence(pr: PrNumber, base: &str, problem: MergeProblem) -> String {
    let n = pr.0;
    match problem {
        MergeProblem::Conflict => format!("PR #{n} has a merge conflict with {base}."),
        MergeProblem::ChecksFailing => format!("PR #{n} has failing checks."),
        MergeProblem::Behind => format!("PR #{n} is behind {base}, and the repo requires an up-to-date branch."),
        MergeProblem::Blocked => format!("PR #{n} is blocked by a branch rule, such as a required review."),
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
pub fn merge_method(configured: Option<MergeMethod>, allowed: &[MergeMethod]) -> Result<MergeMethod, String> {
    let names = || allowed.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ");
    match configured {
        Some(m) if allowed.contains(&m) => Ok(m),
        Some(m) => Err(format!("dispatch.mergeMethod is {m}, but the repo allows only: {}", names())),
        None => allowed.first().copied().ok_or_else(|| "the repo allows no merge method".to_string()),
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
        assert!(
            matches!(plan(Mergeability::Blocked, Checks::Pending), MergePlan::Refuse(why) if why.contains("still running"))
        );
        assert!(
            matches!(plan(Mergeability::Unknown, Checks::Pass), MergePlan::Refuse(why) if why.contains("Try again"))
        );
        assert!(
            matches!(plan(Mergeability::Blocked, Checks::None), MergePlan::Refuse(why) if why.contains("no checks yet")),
            "nothing for the agent to fix"
        );
    }

    #[test]
    fn merge_plan_marks_a_draft_ready_and_refuses_a_closed_pr() {
        let plan = |state| merge_plan(&PullRequest { state, ..pr(10, "h", &[]) });
        assert_eq!(plan(PrState::Draft), MergePlan::MarkReady);
        assert_eq!(plan(PrState::Merged), MergePlan::Refuse("PR #10 is already merged.".to_string()));
        assert_eq!(plan(PrState::Closed), MergePlan::Refuse("PR #10 is closed.".to_string()));
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
        assert_eq!(
            merge_method(Some(Squash), &[Merge, Rebase]),
            Err("dispatch.mergeMethod is squash, but the repo allows only: merge, rebase".to_string())
        );
        assert!(merge_method(None, &[]).is_err());
    }
}
