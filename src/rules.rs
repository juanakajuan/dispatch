//! Pure product rules. No I/O. Every function here has unit tests that need
//! no `gh`, `git`, or `tmux`.

use crate::config::Config;
use crate::domain::{BranchName, Issue, IssueNumber, Mode, SkillChoice};
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
    // TODO: branch = branch_for(issue); (skill, reason) = skill_for(labels, config);
    //       mode = Mode::Implement; prompt = prompt_for(issue).
    todo!()
}

/// `fix/142-retry-queue-drops-jobs`. Prefix from the first matching label
/// (bug -> fix, docs/documentation -> docs, chore -> chore, else feat).
/// Slug: lowercase ASCII alnum words, first 4, joined by `-`.
/// Always yields a valid BranchName (the slug can be empty: `feat/142`).
pub fn branch_for(issue: &Issue) -> BranchName {
    todo!()
}

/// First rule in `config.label_rules` whose label is on the issue wins.
/// Table order is the priority, so `[bug, feature]` on one issue gives tdd.
pub fn skill_for<'c>(labels: &[String], config: &'c Config) -> (SkillChoice, Option<&'c str>) {
    todo!()
}

pub fn prompt_for(issue: &Issue) -> String {
    // TODO: "Implement GitHub issue #N: <title>\n\nRead the issue and its comments with
    //        `gh issue view N --comments`.\nAdd or update tests. Open a draft PR that
    //        closes #N when the tests pass."
    todo!()
}

/// The exact text passed to `claude` as its first prompt argument.
pub fn final_prompt(skill: &SkillChoice, prompt: &str) -> String {
    // TODO: match skill { Some(s) => format!("/{} {}", s.as_str(), prompt), None => prompt }
    todo!()
}

/// `claude` argv after the program name. Built as argv, never as a shell
/// string, so the prompt needs no quoting (tmux execs multi-arg commands
/// directly, without `sh -c`).
pub fn claude_args(mode: Mode, skill: &SkillChoice, prompt: &str, hooks_json: Option<&str>) -> Vec<String> {
    // TODO: ["--permission-mode","plan"] if PlanFirst; ["--settings", hooks_json] if Some;
    //       then final_prompt(..).
    todo!()
}

/// `<repo parent>/<repo dir name>-<issue>`. Deterministic per issue: the
/// path itself is a second guard against two worktrees for one issue,
/// because `git worktree add` refuses an existing path.
pub fn worktree_dir_for(repo_root: &Path, issue: IssueNumber) -> std::path::PathBuf {
    todo!()
}

/// Case-insensitive substring match over `#N title labels branch`.
pub fn matches_filter(haystack_parts: &[&str], needle: &str) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    // TODO: branch_for: bug label -> fix/, unicode title -> ascii slug, empty slug -> feat/N.
    // TODO: skill_for: table order wins over label order; no match -> (None, None).
    // TODO: final_prompt: skill prefix present / absent.
    // TODO: claude_args: plan mode adds --permission-mode plan; prompt with quotes and
    //       newlines comes through as one argv element unchanged.
    // TODO: worktree_dir_for: /a/orbit + 142 -> /a/orbit-142.
}
