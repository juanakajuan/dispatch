//! Pure product rules. No I/O. Every function here has unit tests that need
//! no `gh`, `git`, or `tmux`.

use crate::config::Config;
use crate::domain::{BranchName, Issue, IssueNumber, Mode, PrNumber, SkillChoice};
use std::path::Path;

/// The editable defaults the start dialog opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartDefaults {
    pub branch: BranchName,
    pub skill: SkillChoice,
    /// The label that picked `skill`, shown as the dialog hint.
    pub skill_reason: Option<String>,
    pub mode: Mode,
    pub prompt: String,
}

pub fn start_defaults(issue: &Issue, config: &Config) -> StartDefaults {
    let (skill, reason) = skill_for(&issue.labels, config);
    StartDefaults {
        branch: branch_for(issue),
        skill,
        skill_reason: reason.map(str::to_string),
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

/// First rule in `config.label_rules` whose label is on the issue wins.
/// Table order is the priority, so `[bug, feature]` on one issue gives tdd.
pub fn skill_for<'c>(labels: &[String], config: &'c Config) -> (SkillChoice, Option<&'c str>) {
    config
        .label_rules
        .iter()
        .find(|rule| labels.iter().any(|l| l.eq_ignore_ascii_case(&rule.label)))
        .map_or((None, None), |rule| (Some(rule.skill.clone()), Some(rule.label.as_str())))
}

pub fn prompt_for(issue: &Issue) -> String {
    let n = issue.number.0;
    format!(
        "Implement GitHub issue #{n}: {}\n\nRead the issue and its comments with `gh issue view {n} --comments`.\nAdd or update tests. Open a ready-for-review PR (not a draft) that closes #{n} when the tests pass.",
        issue.title
    )
}

/// The exact text passed to `claude` as its first prompt argument.
pub fn final_prompt(skill: &SkillChoice, prompt: &str) -> String {
    match skill {
        Some(s) => format!("/{} {}", s.as_str(), prompt),
        None => prompt.to_string(),
    }
}

/// `claude` argv after the program name. Built as argv, never as a shell
/// string, so the prompt needs no quoting (tmux execs multi-arg commands
/// directly, without `sh -c`).
pub fn claude_args(mode: Mode, skill: &SkillChoice, prompt: &str, hooks_json: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if mode == Mode::PlanFirst {
        args.extend(["--permission-mode".to_string(), "plan".to_string()]);
    }
    args.extend(settings_args(hooks_json));
    args.push(final_prompt(skill, prompt));
    args
}

/// `claude` argv after the program name, to continue the last conversation
/// in the worktree.
pub fn continue_args(hooks_json: Option<&str>) -> Vec<String> {
    let mut args = vec!["--continue".to_string()];
    args.extend(settings_args(hooks_json));
    args
}

/// What `m` types into the session's Claude prompt. The agent runs the merge,
/// so it can fix a red check or a conflict, or explain why it stopped.
pub fn merge_prompt(pr: PrNumber) -> String {
    format!(
        "Merge PR #{n} with `gh pr merge {n}`, using a merge method the repo allows. \
         If checks are failing or the branch does not merge cleanly, stop and tell me why instead of forcing it.",
        n = pr.0
    )
}

fn settings_args(hooks_json: Option<&str>) -> Vec<String> {
    hooks_json.map_or_else(Vec::new, |json| vec!["--settings".to_string(), json.to_string()])
}

/// `<repo parent>/<repo dir name>-<issue>`. Deterministic per issue: the
/// path itself is a second guard against two worktrees for one issue,
/// because `git worktree add` refuses an existing path.
pub fn worktree_dir_for(repo_root: &Path, issue: IssueNumber) -> std::path::PathBuf {
    let name = repo_root.file_name().map_or_else(|| "repo".into(), |n| n.to_string_lossy().into_owned());
    let dir_name = format!("{name}-{}", issue.0);
    repo_root.parent().map_or_else(|| dir_name.clone().into(), |p| p.join(&dir_name))
}

/// Case-insensitive substring match over `#N title labels branch`.
pub fn matches_filter(haystack_parts: &[&str], needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    needle.is_empty() || haystack_parts.join(" ").to_lowercase().contains(&needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SkillName;
    use crate::testutil::issue;

    fn skill(s: &str) -> SkillName {
        SkillName::parse(s).expect("valid skill in test")
    }

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
    fn skill_table_order_wins_over_label_order() {
        let config = Config::default();
        let (s, why) = skill_for(&["feature".to_string(), "bug".to_string()], &config);
        assert_eq!(s, Some(skill("pstack:tdd")));
        assert_eq!(why, Some("bug"));
        assert_eq!(skill_for(&["p1".to_string()], &config), (None, None));
        assert_eq!(skill_for(&["BUG".to_string()], &config).0, Some(skill("pstack:tdd")));
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
    fn final_prompt_prefixes_the_skill() {
        assert_eq!(final_prompt(&Some(skill("pstack:tdd")), "do it"), "/pstack:tdd do it");
        assert_eq!(final_prompt(&None, "do it"), "do it");
    }

    #[test]
    fn claude_args_modes_hooks_and_prompt_passthrough() {
        let prompt = "say \"hi\" $HOME `x`\nline two;";
        let args = claude_args(Mode::PlanFirst, &Some(skill("pstack:tdd")), prompt, None);
        assert_eq!(args, ["--permission-mode", "plan", &format!("/pstack:tdd {prompt}")]);
        let args = claude_args(Mode::Implement, &None, prompt, Some("{}"));
        assert_eq!(args, ["--settings", "{}", prompt]);
        assert_eq!(continue_args(None), ["--continue"]);
        assert_eq!(continue_args(Some("{}")), ["--continue", "--settings", "{}"]);
    }

    #[test]
    fn worktree_dir_is_a_sibling() {
        assert_eq!(worktree_dir_for(Path::new("/a/orbit"), IssueNumber(142)), Path::new("/a/orbit-142"));
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
        let d = start_defaults(&issue(3, "Fix it", &["bug"]), &Config::default());
        assert_eq!(d.branch.as_str(), "fix/3-fix-it");
        assert_eq!(d.skill, Some(skill("pstack:tdd")));
        assert_eq!(d.skill_reason.as_deref(), Some("bug"));
        assert_eq!(d.mode, Mode::Implement);
        assert!(d.prompt.starts_with("Implement GitHub issue #3: Fix it\n\n"));
        assert!(d.prompt.contains("`gh issue view 3 --comments`"));
    }
}
