//! Pure product rules. No I/O. Every function here has unit tests that need
//! no `gh`, `git`, or `tmux`.

use crate::domain::{BranchName, Checks, Issue, IssueNumber, MergeMethod, Mergeability, Mode, PrNumber, PrState, PullRequest};
use std::path::Path;

/// The editable defaults the start dialog opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartDefaults {
    pub branch: BranchName,
    pub mode: Mode,
    pub prompt: String,
}

pub fn start_defaults(issue: &Issue) -> StartDefaults {
    StartDefaults {
        branch: branch_for(issue),
        mode: Mode::Implement,
        prompt: prompt_for(issue),
    }
}

/// `fix/142-retry-queue-drops-jobs`. Prefix from the first matching label
/// (bug -> fix, docs/documentation -> docs, chore -> chore, else feat).
/// Slug: lowercase ASCII alnum words, first 4, joined by `-`.
/// Always yields a valid BranchName (the slug can be empty: `feat/142`).
pub fn branch_for(issue: &Issue) -> BranchName {
    let prefix = issue
        .labels
        .iter()
        .find_map(|l| match l.to_ascii_lowercase().as_str() {
            "bug" => Some("fix"),
            "docs" | "documentation" => Some("docs"),
            "chore" => Some("chore"),
            _ => None,
        })
        .unwrap_or("feat");
    let slug = issue
        .title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let name = if slug.is_empty() {
        format!("{prefix}/{}", issue.number.0)
    } else {
        format!("{prefix}/{}-{slug}", issue.number.0)
    };
    BranchName::parse(&name).unwrap_or_else(|_| BranchName::numbered(issue.number))
}

pub fn prompt_for(issue: &Issue) -> String {
    let n = issue.number.0;
    format!(
        "Implement GitHub issue #{n}: {}\n\nRead the issue and its comments with `gh issue view {n} --comments`.\nAdd or update tests. Open a ready-for-review PR (not a draft) that closes #{n} when the tests pass.",
        issue.title
    )
}

/// `claude` argv after the program name. Built as argv, never as a shell
/// string, so the prompt needs no quoting (tmux execs multi-arg commands
/// directly, without `sh -c`).
pub fn claude_args(mode: Mode, prompt: &str, hooks_json: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if mode == Mode::PlanFirst {
        args.extend(["--permission-mode".to_string(), "plan".to_string()]);
    }
    args.extend(settings_args(hooks_json));
    args.push(prompt.to_string());
    args
}

/// `claude` argv after the program name, to continue the last conversation
/// in the worktree.
pub fn continue_args(hooks_json: Option<&str>) -> Vec<String> {
    let mut args = vec!["--continue".to_string()];
    args.extend(settings_args(hooks_json));
    args
}

/// What `m` does with a PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergePlan {
    /// dispatch runs `gh pr merge` itself.
    Direct,
    /// The session's agent fixes the problem, then merges.
    Handoff(MergeProblem),
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
        PrState::Draft => return MergePlan::Refuse(format!("PR #{n} is a draft.")),
        PrState::Merged => return MergePlan::Refuse(format!("PR #{n} is already merged.")),
        PrState::Closed => return MergePlan::Refuse(format!("PR #{n} is closed.")),
    }
    match (pr.mergeability, pr.checks) {
        (Mergeability::Conflict, _) => MergePlan::Handoff(MergeProblem::Conflict),
        (_, Checks::Fail) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (_, Checks::Pending) => MergePlan::Refuse(format!("PR #{n} has checks that are still running. Press m again when they finish.")),
        (Mergeability::Unstable, _) => MergePlan::Handoff(MergeProblem::ChecksFailing),
        (Mergeability::Unknown, _) => MergePlan::Refuse(format!("GitHub does not know yet if PR #{n} can merge. Try again in a moment.")),
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

fn settings_args(hooks_json: Option<&str>) -> Vec<String> {
    hooks_json.map_or_else(Vec::new, |json| vec!["--settings".to_string(), json.to_string()])
}

/// Where session worktrees live, relative to the repo root. The same place
/// Claude Code uses for its own worktrees.
pub const WORKTREES_DIR: &str = ".claude/worktrees";

/// The `.git/info/exclude` line that hides `WORKTREES_DIR` from `git status`
/// in the main checkout.
pub const WORKTREES_EXCLUDE: &str = ".claude/worktrees/";

/// `<repo root>/.claude/worktrees/<issue>`. Deterministic per issue: the
/// path itself is a second guard against two worktrees for one issue,
/// because `git worktree add` refuses an existing path.
pub fn worktree_dir_for(repo_root: &Path, issue: IssueNumber) -> std::path::PathBuf {
    repo_root.join(WORKTREES_DIR).join(issue.0.to_string())
}

/// Case-insensitive substring match over `#N title labels branch`.
pub fn matches_filter(haystack_parts: &[&str], needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    needle.is_empty() || haystack_parts.join(" ").to_lowercase().contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{issue, pr};

    #[test]
    fn branch_prefix_follows_labels() {
        let b = |labels: &[&str]| branch_for(&issue(142, "Retry queue drops jobs when Redis reconnects", labels));
        assert_eq!(b(&["bug", "p1"]).as_str(), "fix/142-retry-queue-drops-jobs");
        assert_eq!(b(&["documentation"]).as_str(), "docs/142-retry-queue-drops-jobs");
        assert_eq!(b(&["chore"]).as_str(), "chore/142-retry-queue-drops-jobs");
        assert_eq!(b(&["feature"]).as_str(), "feat/142-retry-queue-drops-jobs");
        assert_eq!(b(&[]).as_str(), "feat/142-retry-queue-drops-jobs");
    }

    #[test]
    fn branch_slug_is_ascii_and_may_be_empty() {
        assert_eq!(branch_for(&issue(7, "Größe `--json` ändern!", &[])).as_str(), "feat/7-gr-e-json-ndern");
        assert_eq!(branch_for(&issue(9, "日本語 ?!", &["bug"])).as_str(), "fix/9");
    }

    #[test]
    fn prompt_asks_for_a_ready_pr_not_a_draft() {
        assert_eq!(
            prompt_for(&issue(142, "Retry queue drops jobs", &[])),
            "Implement GitHub issue #142: Retry queue drops jobs\n\n\
             Read the issue and its comments with `gh issue view 142 --comments`.\n\
             Add or update tests. Open a ready-for-review PR (not a draft) that closes #142 when the tests pass."
        );
    }

    #[test]
    fn claude_args_modes_hooks_and_prompt_passthrough() {
        let prompt = "say \"hi\" $HOME `x`\nline two;";
        let args = claude_args(Mode::PlanFirst, prompt, None);
        assert_eq!(args, ["--permission-mode", "plan", prompt]);
        let args = claude_args(Mode::Implement, prompt, Some("{}"));
        assert_eq!(args, ["--settings", "{}", prompt]);
        assert_eq!(continue_args(None), ["--continue"]);
        assert_eq!(continue_args(Some("{}")), ["--continue", "--settings", "{}"]);
    }

    #[test]
    fn worktree_dir_is_inside_the_repo() {
        assert_eq!(worktree_dir_for(Path::new("/a/orbit"), IssueNumber(142)), Path::new("/a/orbit/.claude/worktrees/142"));
        assert!(Path::new(WORKTREES_EXCLUDE).starts_with(WORKTREES_DIR));
    }

    #[test]
    fn filter_is_case_insensitive_over_all_parts() {
        let parts = ["#142", "Retry queue", "bug", "fix/142-retry"];
        assert!(matches_filter(&parts, ""));
        assert!(matches_filter(&parts, "RETRY"));
        assert!(matches_filter(&parts, "#142"));
        assert!(matches_filter(&parts, "bug fix"));
        assert!(!matches_filter(&parts, "docs"));
    }

    #[test]
    fn defaults_combine_the_rules() {
        let d = start_defaults(&issue(3, "Fix it", &["bug"]));
        assert_eq!(d.branch.as_str(), "fix/3-fix-it");
        assert_eq!(d.mode, Mode::Implement);
        assert!(d.prompt.starts_with("Implement GitHub issue #3: Fix it\n\n"));
        assert!(d.prompt.contains("`gh issue view 3 --comments`"));
    }

    #[test]
    fn merge_plan_merges_clean_prs_and_hands_off_problems() {
        let plan = |mergeability, checks| merge_plan(&PullRequest { mergeability, checks, ..pr(10, "h", &[]) });
        assert_eq!(plan(Mergeability::Clean, Checks::Pass), MergePlan::Direct);
        assert_eq!(plan(Mergeability::Clean, Checks::None), MergePlan::Direct);
        assert_eq!(plan(Mergeability::Conflict, Checks::Pending), MergePlan::Handoff(MergeProblem::Conflict));
        assert_eq!(plan(Mergeability::Blocked, Checks::Fail), MergePlan::Handoff(MergeProblem::ChecksFailing));
        assert_eq!(plan(Mergeability::Unstable, Checks::Pass), MergePlan::Handoff(MergeProblem::ChecksFailing));
        assert_eq!(plan(Mergeability::Behind, Checks::Pass), MergePlan::Handoff(MergeProblem::Behind));
        assert_eq!(plan(Mergeability::Blocked, Checks::Pass), MergePlan::Handoff(MergeProblem::Blocked));
        assert!(matches!(plan(Mergeability::Blocked, Checks::Pending), MergePlan::Refuse(why) if why.contains("still running")));
        assert!(matches!(plan(Mergeability::Unknown, Checks::Pass), MergePlan::Refuse(why) if why.contains("Try again")));
        assert!(matches!(plan(Mergeability::Blocked, Checks::None), MergePlan::Refuse(why) if why.contains("no checks yet")), "nothing for the agent to fix");
    }

    #[test]
    fn merge_plan_refuses_a_pr_that_is_not_open() {
        let plan = |state| merge_plan(&PullRequest { state, ..pr(10, "h", &[]) });
        assert_eq!(plan(PrState::Draft), MergePlan::Refuse("PR #10 is a draft.".to_string()));
        assert_eq!(plan(PrState::Merged), MergePlan::Refuse("PR #10 is already merged.".to_string()));
        assert_eq!(plan(PrState::Closed), MergePlan::Refuse("PR #10 is closed.".to_string()));
    }

    #[test]
    fn handoff_prompt_names_the_problem_and_the_method() {
        assert_eq!(
            handoff_prompt(PrNumber(12), "main", MergeProblem::Conflict, MergeMethod::Squash),
            "PR #12 has a merge conflict with main. Fix it and push. When the checks pass, merge it with `gh pr merge 12 --squash`."
        );
        assert!(handoff_prompt(PrNumber(12), "main", MergeProblem::Behind, MergeMethod::Rebase).starts_with("PR #12 is behind main,"));
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
