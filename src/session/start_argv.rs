//! The argv of each command a start runs. The executors (`git.rs`,
//! `herdr.rs`, `plan_start`) and the start dialog preview both build argv
//! here, so the preview cannot drift from a real start (cbee386).

use super::git::fork_remote;
use super::herdr;
use super::mux::Backend;
use super::request::{Launch, StartFrom};
use super::worktree::WORKTREES_DIR;
use crate::config::{AgentCommand, Harness};
use crate::domain::{BranchName, Mode, OriginRef, SessionKey};

/// The start dialog's "Runs" preview.
#[derive(Clone, Debug)]
pub struct StartPreview {
    pub(super) mux: Backend,
    pub(super) base: OriginRef,
}

impl StartPreview {
    /// Each argv a start of a new branch runs, program name first. The
    /// waiting hooks JSON shows as a placeholder.
    pub fn commands(
        &self,
        key: SessionKey,
        branch: &str,
        from: &StartFrom,
        mode: Mode,
        prompt: &str,
        launch: &Launch,
    ) -> Vec<Vec<String>> {
        let dir = format!("{}/{}", WORKTREES_DIR, key.dir_name());
        let origin = BranchName::parse(branch).ok().map(OriginRef::of);
        let base = match from {
            StartFrom::Base => Some(&self.base),
            StartFrom::Origin => origin.as_ref(),
            StartFrom::Fork { .. } => None,
        };
        let (program, args) = match &self.mux {
            Backend::Tmux(_) | Backend::Hc(_) => ("git", worktree_add_args(&dir, branch, base)),
            Backend::Herdr(place) => {
                let source = match place {
                    herdr::Place::Inside { source } => source.as_str().to_string(),
                    herdr::Place::Detached { label } => format!("<workspace {label}>"),
                };
                ("herdr", herdr_worktree_create_args(&source, &dir, branch, base, key))
            }
        };
        let create = std::iter::once(program.to_string()).chain(args).collect();
        let hooks = self.mux.hooks(launch).map(|_| "<waiting hooks>");
        let fetch = match (from, BranchName::parse(branch)) {
            (StartFrom::Fork { branch: head, .. }, Ok(local)) => Some(
                std::iter::once("git".to_string()).chain(fork_fetch_args(&fork_remote(&local), head, &local)).collect(),
            ),
            _ => None,
        };
        fetch.into_iter().chain([create, agent_argv(&launch.command, launch.harness, hooks, mode, prompt)]).collect()
    }

    #[cfg(test)]
    pub fn herdr_inside(source: &str) -> StartPreview {
        let source = serde_json::from_value(serde_json::json!(source)).expect("workspace id");
        StartPreview {
            mux: Backend::Herdr(herdr::Place::Inside { source }),
            base: OriginRef::parse("origin/main").expect("ref"),
        }
    }
}

#[cfg(test)]
impl Default for StartPreview {
    fn default() -> StartPreview {
        let place = super::tmux::Place::Detached { session_name: "dispatch-orbit".to_string() };
        StartPreview { mux: Backend::Tmux(place), base: OriginRef::parse("origin/main").expect("ref") }
    }
}

/// The line that stands in for a plan flag when the harness has none.
const PLAN_FIRST_LINE: &str = "Plan first. Do not change any files until I approve the plan.";

/// The argv that starts `harness` on a prompt. Built as argv, never as a
/// shell string, so the prompt needs no quoting (tmux execs multi-arg
/// commands directly, without `sh -c`). Only a harness that takes the
/// waiting hooks gets them. pi gets `--` before the prompt, because it
/// reads a prompt that starts with `-` as an option.
pub(super) fn agent_argv(
    command: &AgentCommand,
    harness: Harness,
    hooks: Option<&str>,
    mode: Mode,
    prompt: &str,
) -> Vec<String> {
    let plan = mode == Mode::PlanFirst;
    let tail: Vec<String> = match harness {
        Harness::Claude => {
            let mut tail = Vec::new();
            if plan {
                tail.extend(["--permission-mode", "plan"].map(String::from));
            }
            if let Some(json) = hooks {
                tail.extend(["--settings".to_string(), json.to_string()]);
            }
            tail.push(prompt.to_string());
            tail
        }
        Harness::Codex => {
            let mut tail = Vec::new();
            if plan {
                tail.extend(["--sandbox", "read-only"].map(String::from));
            }
            tail.push(prompt.to_string());
            tail
        }
        Harness::OpenCode => {
            let prompt = if plan { format!("{PLAN_FIRST_LINE}\n\n{prompt}") } else { prompt.to_string() };
            vec!["--prompt".to_string(), prompt]
        }
        Harness::Antigravity => {
            let mut tail = Vec::new();
            if plan {
                tail.extend(["--mode", "plan"].map(String::from));
            }
            tail.extend(["--prompt-interactive".to_string(), prompt.to_string()]);
            tail
        }
        Harness::Pi => {
            let prompt = if plan { format!("{PLAN_FIRST_LINE}\n\n{prompt}") } else { prompt.to_string() };
            vec!["--".to_string(), prompt]
        }
    };
    command.argv(&tail)
}

impl Launch {
    /// What a resume in `mode` runs, as one line for a toast. The waiting
    /// hooks JSON is left out.
    pub fn resume_words(&self, mode: Mode) -> String {
        resume_argv(&self.command, self.harness, None, mode).join(" ")
    }
}

/// The argv that continues the last conversation in the worktree, with the
/// plan flag of the start when the session plans first. A harness with no
/// plan flag gets none: its conversation already starts with the plan line.
pub(super) fn resume_argv(command: &AgentCommand, harness: Harness, hooks: Option<&str>, mode: Mode) -> Vec<String> {
    let plan = mode == Mode::PlanFirst;
    let tail: Vec<String> = match harness {
        Harness::Claude => {
            let mut tail = vec!["--continue".to_string()];
            if plan {
                tail.extend(["--permission-mode", "plan"].map(String::from));
            }
            if let Some(json) = hooks {
                tail.extend(["--settings".to_string(), json.to_string()]);
            }
            tail
        }
        Harness::Codex => {
            let mut tail = ["resume", "--last"].map(String::from).to_vec();
            if plan {
                tail.extend(["--sandbox", "read-only"].map(String::from));
            }
            tail
        }
        Harness::Antigravity => {
            let mut tail = vec!["--continue".to_string()];
            if plan {
                tail.extend(["--mode", "plan"].map(String::from));
            }
            tail
        }
        Harness::OpenCode | Harness::Pi => vec!["--continue".to_string()],
    };
    command.argv(&tail)
}

/// `git` argv after the program name.
pub(super) fn worktree_add_args(dir: &str, branch: &str, create_from: Option<&OriginRef>) -> Vec<String> {
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
    from: Option<&OriginRef>,
    key: SessionKey,
) -> Vec<String> {
    let mut args: Vec<String> =
        ["worktree", "create", "--workspace", source, "--branch", branch].map(String::from).to_vec();
    args.extend(from.into_iter().flat_map(|base| ["--base".to_string(), base.to_string()]));
    let label = herdr::label(key);
    args.extend(["--path", dir, "--label", label.as_str(), "--no-focus"].map(String::from));
    args
}

/// `git` argv after the program name: fetches `head` of the fork remote into
/// the new local `branch`.
pub(super) fn fork_fetch_args(remote: &str, head: &BranchName, branch: &BranchName) -> Vec<String> {
    vec![
        "fetch".to_string(),
        remote.to_string(),
        format!("refs/heads/{}:refs/heads/{}", head.as_str(), branch.as_str()),
    ]
}
