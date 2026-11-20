use crate::config::ClaudeCommand;
use crate::domain::{BranchName, Mode, RepoSlug, SessionKey};
use crate::session::StartPreview;
use crate::session::herdr::label as herdr_label;
use crate::session::request::{Launch, StartFrom};
use crate::session::start_argv::{claude_args, continue_args};
use crate::testutil::{issue_key, pr_key};

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

fn preview(mux: &StartPreview, key: SessionKey, branch: &str, from: &StartFrom) -> Vec<String> {
    let launch =
        Launch { detect_waiting: false, claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() } };
    let all = mux.commands(key, branch, from, Mode::Implement, "p", &launch);
    all[..all.len() - 1].iter().map(|argv| argv.join(" ")).collect()
}

#[test]
fn the_preview_for_a_pr_shows_the_pr_dir_and_the_commands_that_origin_runs() {
    let tmux = StartPreview::default();
    assert_eq!(
        preview(&tmux, issue_key(7), "docs/7-x", &StartFrom::Base),
        ["git worktree add .claude/worktrees/7 -b docs/7-x origin/main"]
    );
    assert_eq!(
        preview(&tmux, pr_key(7), "chore/dep", &StartFrom::Origin),
        ["git worktree add .claude/worktrees/pr-7 -b chore/dep origin/chore/dep"]
    );
    let fork = StartFrom::Fork {
        repo: RepoSlug::parse("maxbeizer/cli").expect("slug"),
        branch: BranchName::parse("dep-bump").expect("branch"),
    };
    assert_eq!(
        preview(&tmux, pr_key(7), "fork/pr-7", &fork),
        [
            "git fetch dispatch-fork-pr-7 refs/heads/dep-bump:refs/heads/fork/pr-7",
            "git worktree add .claude/worktrees/pr-7 fork/pr-7",
        ]
    );
}

#[test]
fn the_herdr_preview_and_label_for_a_pr_do_not_collide_with_the_issue() {
    let herdr = StartPreview::herdr_inside("w-orbit");
    assert_eq!(
        preview(&herdr, pr_key(7), "chore/dep", &StartFrom::Origin),
        ["herdr worktree create --workspace w-orbit --branch chore/dep --base origin/chore/dep \
             --path .claude/worktrees/pr-7 --label PR #7 --no-focus"]
    );
    assert_eq!((herdr_label(issue_key(7)), herdr_label(pr_key(7))), ("#7".to_string(), "PR #7".to_string()));
}
