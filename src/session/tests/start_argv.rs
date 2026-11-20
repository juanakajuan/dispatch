use crate::config::{AgentCommand, Harness};
use crate::domain::Mode;
use crate::session::start_argv::{agent_argv, resume_argv};

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
fn a_command_override_leads_the_argv() {
    let command = AgentCommand::from_words("my-codex --x").expect("valid");
    assert_eq!(agent_argv(&command, Harness::Codex, None, Mode::Implement, "p"), words(&["my-codex", "--x", "p"]));
    assert_eq!(resume_argv(&command, Harness::Codex, None), words(&["my-codex", "--x", "resume", "--last"]));
}

#[test]
fn resume_argv_per_harness() {
    let resume = |harness, hooks| resume_argv(&AgentCommand::default_for(harness), harness, hooks);
    assert_eq!(resume(Harness::Claude, None), words(&["claude", "--continue"]));
    assert_eq!(resume(Harness::Claude, Some("{}")), words(&["claude", "--continue", "--settings", "{}"]));
    assert_eq!(resume(Harness::Codex, Some("{}")), words(&["codex", "resume", "--last"]));
    assert_eq!(resume(Harness::OpenCode, Some("{}")), words(&["opencode", "--continue"]));
    assert_eq!(resume(Harness::Antigravity, Some("{}")), words(&["agy", "--continue"]));
}
