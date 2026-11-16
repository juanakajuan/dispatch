use crate::domain::Mode;
use crate::session::start_argv::{claude_args, continue_args};

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
