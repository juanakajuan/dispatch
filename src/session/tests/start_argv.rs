use crate::config::{AgentCommand, Harness};
use crate::domain::{BranchName, Mode, RepoSlug, SessionKey};
use crate::session::StartPreview;
use crate::session::herdr::label as herdr_label;
use crate::session::request::{Launch, StartFrom};
use crate::session::start_argv::{agent_argv, resume_argv};
use crate::testutil::{issue_key, pr_key};

const PROMPT: &str = "say \"hi\" $HOME `x`\nline two;";
const PLAN_LINE: &str = "Plan first. Do not change any files until I approve the plan.";

fn start(harness: Harness, mode: Mode, hooks: Option<&str>) -> Vec<String> {
    agent_argv(&AgentCommand::default_for(harness), harness, hooks, mode, PROMPT)
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

#[test]
fn claude_start_argv() {
    assert_eq!(start(Harness::Claude, Mode::Implement, None), words(&["claude", PROMPT]));
    assert_eq!(start(Harness::Claude, Mode::PlanFirst, None), words(&["claude", "--permission-mode", "plan", PROMPT]));
    assert_eq!(start(Harness::Claude, Mode::Implement, Some("{}")), words(&["claude", "--settings", "{}", PROMPT]));
    assert_eq!(
        start(Harness::Claude, Mode::PlanFirst, Some("{}")),
        words(&["claude", "--permission-mode", "plan", "--settings", "{}", PROMPT])
    );
}

#[test]
fn codex_start_argv_ignores_hooks() {
    for hooks in [None, Some("{}")] {
        assert_eq!(start(Harness::Codex, Mode::Implement, hooks), words(&["codex", PROMPT]));
        assert_eq!(start(Harness::Codex, Mode::PlanFirst, hooks), words(&["codex", "--sandbox", "read-only", PROMPT]));
    }
}

#[test]
fn opencode_start_argv_ignores_hooks_and_plans_in_the_prompt() {
    let planned = format!("{PLAN_LINE}\n\n{PROMPT}");
    for hooks in [None, Some("{}")] {
        assert_eq!(start(Harness::OpenCode, Mode::Implement, hooks), words(&["opencode", "--prompt", PROMPT]));
        assert_eq!(start(Harness::OpenCode, Mode::PlanFirst, hooks), words(&["opencode", "--prompt", &planned]));
    }
}

#[test]
fn antigravity_start_argv_ignores_hooks() {
    for hooks in [None, Some("{}")] {
        assert_eq!(
            start(Harness::Antigravity, Mode::Implement, hooks),
            words(&["agy", "--prompt-interactive", PROMPT])
        );
        assert_eq!(
            start(Harness::Antigravity, Mode::PlanFirst, hooks),
            words(&["agy", "--mode", "plan", "--prompt-interactive", PROMPT])
        );
    }
}

#[test]
fn pi_start_argv_ignores_hooks_ends_options_and_plans_in_the_prompt() {
    let planned = format!("{PLAN_LINE}\n\n{PROMPT}");
    for hooks in [None, Some("{}")] {
        assert_eq!(start(Harness::Pi, Mode::Implement, hooks), words(&["pi", "--", PROMPT]));
        assert_eq!(start(Harness::Pi, Mode::PlanFirst, hooks), words(&["pi", "--", &planned]));
    }
}

#[test]
fn a_command_override_leads_the_argv() {
    let command = AgentCommand::from_words("my-codex --x").expect("valid");
    assert_eq!(agent_argv(&command, Harness::Codex, None, Mode::Implement, "p"), words(&["my-codex", "--x", "p"]));
    assert_eq!(
        resume_argv(&command, Harness::Codex, None, Mode::Implement),
        words(&["my-codex", "--x", "resume", "--last"])
    );
}

#[test]
fn resume_argv_per_harness() {
    let resume = |harness, hooks| resume_argv(&AgentCommand::default_for(harness), harness, hooks, Mode::Implement);
    assert_eq!(resume(Harness::Claude, None), words(&["claude", "--continue"]));
    assert_eq!(resume(Harness::Claude, Some("{}")), words(&["claude", "--continue", "--settings", "{}"]));
    assert_eq!(resume(Harness::Codex, Some("{}")), words(&["codex", "resume", "--last"]));
    assert_eq!(resume(Harness::OpenCode, Some("{}")), words(&["opencode", "--continue"]));
    assert_eq!(resume(Harness::Antigravity, Some("{}")), words(&["agy", "--continue"]));
    assert_eq!(resume(Harness::Pi, Some("{}")), words(&["pi", "--continue"]));
}

#[test]
fn a_plan_first_resume_keeps_the_plan_flag_of_the_start() {
    let resume = |harness, hooks| resume_argv(&AgentCommand::default_for(harness), harness, hooks, Mode::PlanFirst);
    assert_eq!(
        resume(Harness::Claude, Some("{}")),
        words(&["claude", "--continue", "--permission-mode", "plan", "--settings", "{}"])
    );
    assert_eq!(resume(Harness::Codex, None), words(&["codex", "resume", "--last", "--sandbox", "read-only"]));
    assert_eq!(resume(Harness::Antigravity, None), words(&["agy", "--continue", "--mode", "plan"]));
    assert_eq!(resume(Harness::OpenCode, None), words(&["opencode", "--continue"]), "the plan line is in the prompt");
    assert_eq!(resume(Harness::Pi, None), words(&["pi", "--continue"]), "the plan line is in the prompt");
}

fn preview(mux: &StartPreview, key: SessionKey, branch: &str, from: &StartFrom) -> Vec<String> {
    let launch =
        Launch { detect_waiting: false, harness: Harness::Claude, command: AgentCommand::default_for(Harness::Claude) };
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
