use super::*;

#[test]
fn dialog_shows_fields_and_the_exact_commands() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    for want in ["Start session for #2 Docs", "Branch", "docs/2-docs", "Implement directly", "Runs", "ctrl-s to start"]
    {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    let prompt = "Implement GitHub issue #2: Docs Read the issue and its comments with `gh issue view 2 --comments`. \
                  Add or update tests. Open a ready-for-review PR (not a draft) that closes #2 when the tests pass.";
    assert_eq!(
        previewed_commands(&screen),
        [
            "git worktree add .claude/worktrees/2 -b docs/2-docs origin/main".to_string(),
            format!("claude --settings \"<waiting hooks>\" {prompt:?}"),
        ],
        "the whole commands, end of the prompt included, in\n{screen}"
    );
    assert!(!screen.contains('…'), "no command is cut\n{screen}");
}

#[test]
fn dialog_previews_the_herdr_command_on_herdr() {
    let mut state = full_state();
    state.context.start_preview = StartPreview::herdr_inside("w-orbit");
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    let want = "herdr worktree create --workspace w-orbit --branch docs/2-docs --base origin/main \
                --path .claude/worktrees/2 --label #2 --no-focus";
    assert_eq!(previewed_commands(&screen).first().map(String::as_str), Some(want), "{screen}");
    assert!(!screen.contains("git worktree add"), "{screen}");
}

#[test]
fn dialog_keeps_the_prompt_input_when_the_preview_is_taller_than_the_dialog() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let Focus::Dialog(d) = &mut state.focus else { panic!("dialog open") };
    d.prompt.insert_str("word ".repeat(400));
    let screen = text(&render(&state, 120, 30));
    let lines: Vec<&str> = screen.lines().collect();
    let label = lines.iter().position(|l| l.contains("  Prompt  ")).expect("prompt label drawn");
    let runs = lines.iter().position(|l| l.contains("  Runs")).expect("runs label drawn");
    assert!(runs > label + usize::from(PROMPT_MIN_ROWS), "prompt input squeezed in\n{screen}");
    assert!(screen.contains("$ claude"), "{screen}");
}
