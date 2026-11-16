//! The argv of each command a start runs. The executors (`git.rs`,
//! `herdr.rs`, `plan_start`) and the start dialog preview both build argv
//! here, so the preview cannot drift from a real start (cbee386).

use super::herdr;
use super::mux::Backend;
use super::request::Launch;
use super::worktree::WORKTREES_DIR;
use crate::config::ClaudeCommand;
use crate::domain::{IssueNumber, Mode};

/// The start dialog's "Runs" preview.
#[derive(Clone, Debug)]
pub struct StartPreview {
    pub(super) mux: Backend,
    pub(super) base: String,
}

impl StartPreview {
    /// Each argv a start of a new branch runs, program name first. The
    /// waiting hooks JSON shows as a placeholder.
    pub fn commands(
        &self,
        issue: IssueNumber,
        branch: &str,
        mode: Mode,
        prompt: &str,
        launch: &Launch,
    ) -> Vec<Vec<String>> {
        let dir = format!("{}/{}", WORKTREES_DIR, issue.0);
        let base = Some(self.base.as_str());
        let (program, args) = match &self.mux {
            Backend::Tmux(_) | Backend::Hc(_) => ("git", worktree_add_args(&dir, branch, base)),
            Backend::Herdr(place) => {
                let source = match place {
                    herdr::Place::Inside { source } => source.as_str().to_string(),
                    herdr::Place::Detached { label } => format!("<workspace {label}>"),
                };
                ("herdr", herdr_worktree_create_args(&source, &dir, branch, base, issue))
            }
        };
        let create = std::iter::once(program.to_string()).chain(args).collect();
        let hooks = self.mux.hooks(launch).map(|_| "<waiting hooks>");
        vec![create, claude_argv(&launch.claude, hooks, mode, prompt)]
    }

    #[cfg(test)]
    pub fn herdr_inside(source: &str) -> StartPreview {
        let source = serde_json::from_value(serde_json::json!(source)).expect("workspace id");
        StartPreview { mux: Backend::Herdr(herdr::Place::Inside { source }), base: "origin/main".to_string() }
    }
}

#[cfg(test)]
impl Default for StartPreview {
    fn default() -> StartPreview {
        let place = super::tmux::Place::Detached { session_name: "dispatch-orbit".to_string() };
        StartPreview { mux: Backend::Tmux(place), base: "origin/main".to_string() }
    }
}

pub(super) fn claude_argv(claude: &ClaudeCommand, hooks: Option<&str>, mode: Mode, prompt: &str) -> Vec<String> {
    claude.argv(&claude_args(mode, prompt, hooks))
}

/// `git` argv after the program name.
pub(super) fn worktree_add_args(dir: &str, branch: &str, create_from: Option<&str>) -> Vec<String> {
    let mut args = vec!["worktree".to_string(), "add".to_string(), dir.to_string()];
    match create_from {
        Some(base) => args.extend(["-b".to_string(), branch.to_string(), base.to_string()]),
        None => args.push(branch.to_string()),
    }
    args
}

/// `herdr` argv after the program name.
pub(super) fn herdr_worktree_create_args(
    source: &str,
    dir: &str,
    branch: &str,
    from: Option<&str>,
    issue: IssueNumber,
) -> Vec<String> {
    let mut args = vec!["worktree", "create", "--workspace", source, "--branch", branch];
    args.extend(from.into_iter().flat_map(|base| ["--base", base]));
    let label = herdr::label(issue);
    args.extend(["--path", dir, "--label", label.as_str(), "--no-focus"]);
    args.into_iter().map(str::to_string).collect()
}

/// `claude` argv after the program name. Built as argv, never as a shell
/// string, so the prompt needs no quoting (tmux execs multi-arg commands
/// directly, without `sh -c`).
pub(super) fn claude_args(mode: Mode, prompt: &str, hooks_json: Option<&str>) -> Vec<String> {
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
pub(super) fn continue_args(hooks_json: Option<&str>) -> Vec<String> {
    let mut args = vec!["--continue".to_string()];
    args.extend(settings_args(hooks_json));
    args
}

fn settings_args(hooks_json: Option<&str>) -> Vec<String> {
    hooks_json.map_or_else(Vec::new, |json| vec!["--settings".to_string(), json.to_string()])
}
