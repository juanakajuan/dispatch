use super::*;
use crate::app::dialog::DialogField;
use crate::domain::Mode;
use ratatui::layout::Rect;

#[test]
fn dialog_shows_fields_and_the_exact_commands() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    for want in ["Start session for #2 Docs", "Branch", "docs/2-docs", "Implement directly", "Runs", "ctrl+s to start"]
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
fn dialog_previews_the_chosen_harness_with_no_hooks() {
    let mut state = full_state();
    state.config.harness = crate::config::Harness::Codex;
    open_dialog(&mut state);
    let screen = text(&render(&state, 120, 30));
    let commands = previewed_commands(&screen);
    let run = commands.last().expect("a run line");
    assert!(run.starts_with("codex \"Implement GitHub issue #2"), "{run}\n{screen}");
    assert!(!screen.contains("waiting hooks"), "{screen}");
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

fn open_dialog(state: &mut State) {
    send(state, KeyCode::Char('j'));
    send(state, KeyCode::Char('s'));
}

#[test]
fn dialog_labels_radio_options_and_buttons_are_clickable() {
    let mut state = full_state();
    open_dialog(&mut state);
    let screen = draw_into(&mut state, 120, 40);
    for (needle, want) in [
        ("Branch", HitTarget::Field(DialogField::Branch)),
        ("Mode", HitTarget::Field(DialogField::Mode)),
        ("Prompt", HitTarget::Field(DialogField::Prompt)),
        ("Plan first, then wait for approval", HitTarget::Mode(Mode::PlanFirst)),
        ("Implement directly", HitTarget::Mode(Mode::Implement)),
        ("[ Start ]", HitTarget::Key(Key::Ctrl('s'))),
        ("[ Cancel ]", HitTarget::Key(Key::Code(KeyCode::Esc))),
    ] {
        assert_clickable(&state, &screen, needle, want);
    }
}

#[test]
fn clicking_the_drawn_dialog_changes_the_field_and_mode() {
    let mut state = full_state();
    open_dialog(&mut state);
    let screen = draw_into(&mut state, 120, 40);
    let (x, y) = find(&screen, "Plan first");
    click_at(&mut state, x + 3, y);
    let Focus::Dialog(d) = &state.focus else { panic!("dialog open") };
    assert_eq!((d.mode, d.field), (Mode::PlanFirst, DialogField::Mode));
    let (x, y) = find(&screen, "Prompt");
    click_at(&mut state, x, y);
    let Focus::Dialog(d) = &state.focus else { panic!("dialog open") };
    assert_eq!(d.field, DialogField::Prompt);
    let (x, y) = find(&screen, "[ Cancel ]");
    click_at(&mut state, x + 2, y);
    assert!(matches!(state.focus, Focus::List), "the Cancel button closes the dialog");
}

/// The cell under a click target's top-left corner.
fn first_cell(state: &State, buf: &Buffer, field: DialogField) -> (Rect, char) {
    let (rect, _) = state
        .hits
        .targets
        .iter()
        .find(|(_, t)| matches!(t, HitTarget::Text { field: f, .. } if *f == field))
        .expect("text target");
    (*rect, buf[(rect.x, rect.y)].symbol().chars().next().expect("a symbol"))
}

#[test]
fn a_click_in_a_scrolled_prompt_lands_on_the_line_under_the_pointer() {
    let mut state = full_state();
    open_dialog(&mut state);
    let Focus::Dialog(d) = &mut state.focus else { panic!("dialog open") };
    d.prompt = tui_textarea::TextArea::new((0..40).map(|i| format!("{i:02} line")).collect());
    d.prompt.move_cursor(tui_textarea::CursorMove::Bottom);
    let buf = frame(&mut state, 120, 40);
    let (rect, _) = first_cell(&state, &buf, DialogField::Prompt);
    let first: usize = (0..2)
        .map(|dx| buf[(rect.x + dx, rect.y)].symbol().to_string())
        .collect::<String>()
        .parse()
        .expect("line number");
    assert!(first > 0, "the prompt is scrolled down");
    click_at(&mut state, rect.x + 3, rect.y + 1);
    let Focus::Dialog(d) = &state.focus else { panic!("dialog open") };
    assert_eq!((d.field, d.prompt.cursor()), (DialogField::Prompt, (first + 1, 3)));
}

#[test]
fn a_click_in_a_scrolled_branch_lands_on_the_character_under_the_pointer() {
    let mut state = full_state();
    open_dialog(&mut state);
    let Focus::Dialog(d) = &mut state.focus else { panic!("dialog open") };
    let alphabet: String = ('!'..='~').collect();
    d.branch = tui_textarea::TextArea::new(vec![alphabet.clone()]);
    d.branch.move_cursor(tui_textarea::CursorMove::End);
    let buf = frame(&mut state, 120, 40);
    let (rect, shown) = first_cell(&state, &buf, DialogField::Branch);
    let first = alphabet.find(shown).expect("a character of the branch");
    assert!(first > 0, "the branch is scrolled right");
    click_at(&mut state, rect.x + 4, rect.y);
    let Focus::Dialog(d) = &state.focus else { panic!("dialog open") };
    assert_eq!(d.branch.cursor(), (0, first + 4));
}

#[test]
fn the_pr_dialog_names_the_pr_marks_the_branch_as_fixed_and_previews_that_origin() {
    let mut state = full_state();
    state.prs.items.push(crate::testutil::pr(11, "chore/dep", &[]));
    send(&mut state, KeyCode::Char('2'));
    state.tabs.prs.selected = Some(crate::domain::PrNumber(11));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    for want in ["Start session for PR #11 PR 11", "chore/dep (PR branch)", "Work on GitHub PR #11: PR 11"] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    assert_eq!(
        previewed_commands(&screen).first().map(String::as_str),
        Some("git worktree add .claude/worktrees/pr-11 -b chore/dep origin/chore/dep"),
        "{screen}"
    );
}
