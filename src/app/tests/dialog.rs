use super::merge::{ASK_FIX, handoff, with_ask};
use super::*;

#[test]
fn s_on_an_issue_opens_the_dialog_with_label_defaults() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('s')).is_empty());
    let d = dialog(&state);
    assert_eq!(d.branch.lines(), ["fix/3-retry-queue-drops-jobs"]);
    assert!(d.prompt.lines()[0].starts_with("Implement GitHub issue #3: Retry queue drops jobs"));
}

#[test]
fn s_on_an_issue_with_a_session_goes_to_it_and_opens_no_dialog() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let cmds = press(&mut state, key('s'));
    assert!(cmds.is_empty());
    assert_eq!(state.tab, Tab::Sessions);
    assert_eq!(selected_row(&state), Some(RowKey::Session(issue_key(3))));
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn confirm_with_an_invalid_branch_keeps_the_dialog_open_with_an_error() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    press(&mut state, key('x'));
    let Focus::Dialog(d) = &mut state.focus else { panic!("dialog") };
    d.branch.insert_str(" bad name");
    let cmds = press(&mut state, ctrl('s'));
    assert!(cmds.is_empty());
    assert_eq!(dialog(&state).error.as_deref(), Some("The branch name can not have a space."));
    assert!(state.starting.is_empty());
}

#[test]
fn confirm_starts_once_and_a_second_s_emits_no_start() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    let cmds = press(&mut state, ctrl('s'));
    let reqs = starts(&cmds);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].key, issue_key(3));
    assert_eq!(reqs[0].branch.as_str(), "fix/3-retry-queue-drops-jobs");
    assert!(reqs[0].prompt.starts_with("Implement GitHub issue #3"));
    assert_eq!(state.tab, Tab::Sessions);
    assert_eq!(visible_rows(&state), [RowKey::Session(issue_key(3))]);

    press(&mut state, key('1'));
    let again = press(&mut state, key('s'));
    assert!(again.is_empty());
    assert!(matches!(state.focus, Focus::List), "no dialog while starting");
}

#[test]
fn enter_confirms_on_the_branch_field_but_adds_a_newline_in_the_prompt() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    for _ in 0..2 {
        press(&mut state, code(KeyCode::Tab));
    }
    assert_eq!(dialog(&state).field, DialogField::Prompt);
    let lines = dialog(&state).prompt.lines().len();
    assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
    assert_eq!(dialog(&state).prompt.lines().len(), lines + 1);
    press(&mut state, code(KeyCode::Tab));
    assert_eq!(dialog(&state).field, DialogField::Branch);
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(starts(&cmds).len(), 1);
}

#[test]
fn dialog_mode_toggles_and_reaches_the_request() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    press(&mut state, code(KeyCode::BackTab));
    assert_eq!(dialog(&state).field, DialogField::Prompt);
    press(&mut state, code(KeyCode::Tab));
    press(&mut state, code(KeyCode::Tab));
    assert_eq!(dialog(&state).field, DialogField::Mode);
    press(&mut state, code(KeyCode::Right));
    assert_eq!(dialog(&state).mode, Mode::PlanFirst);
    let cmds = press(&mut state, ctrl('s'));
    assert_eq!(starts(&cmds)[0].mode, Mode::PlanFirst);
}

#[test]
fn esc_closes_the_dialog_without_a_command() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    assert!(press(&mut state, code(KeyCode::Esc)).is_empty());
    assert!(matches!(state.focus, Focus::List));
    assert!(state.starting.is_empty());
}

/// The start dialog for issue #3, with `targets` as the drawn hit map.
fn dialog_with(targets: Vec<(Rect, HitTarget)>) -> (State, Instant) {
    let (mut state, now) = app();
    press(&mut state, key('s'));
    state.hits.targets = targets;
    (state, now)
}

#[test]
fn clicking_a_label_focuses_its_field_and_a_radio_option_picks_its_mode() {
    let (mut state, now) = dialog_with(vec![
        (Rect::new(0, 0, 20, 1), HitTarget::Field(DialogField::Prompt)),
        (Rect::new(0, 1, 20, 2), HitTarget::Field(DialogField::Mode)),
        (Rect::new(0, 2, 10, 1), HitTarget::Mode(Mode::PlanFirst)),
        (Rect::new(12, 2, 10, 1), HitTarget::Mode(Mode::Implement)),
    ]);
    press(&mut state, click(1, 0, now));
    assert_eq!(dialog(&state).field, DialogField::Prompt);
    press(&mut state, click(1, 1, now));
    assert_eq!(dialog(&state).field, DialogField::Mode);
    press(&mut state, code(KeyCode::Tab));
    press(&mut state, click(13, 2, now));
    assert_eq!((dialog(&state).mode, dialog(&state).field), (Mode::Implement, DialogField::Mode));
    press(&mut state, click(2, 2, now));
    assert_eq!(dialog(&state).mode, Mode::PlanFirst);
}

#[test]
fn clicking_a_text_input_focuses_it_and_puts_the_cursor_under_the_click() {
    let target = HitTarget::Text { field: DialogField::Prompt, top: 2, left: 5 };
    let (mut state, now) = dialog_with(vec![(Rect::new(10, 4, 30, 3), target)]);
    let Focus::Dialog(d) = &mut state.focus else { panic!("dialog open") };
    d.prompt = tui_textarea::TextArea::new((0..8).map(|i| format!("0123456789 line {i}")).collect());
    press(&mut state, click(13, 5, now));
    assert_eq!(dialog(&state).field, DialogField::Prompt);
    assert_eq!(dialog(&state).prompt.cursor(), (3, 8), "row 2 + (5 - 4), col 5 + (13 - 10)");

    press(&mut state, click(39, 6, now + Duration::from_secs(1)));
    assert_eq!(dialog(&state).prompt.cursor(), (4, 17), "a click past the end of a line stops at its end");
}

#[test]
fn the_start_button_starts_and_the_cancel_button_closes() {
    let buttons = vec![
        (Rect::new(0, 0, 9, 1), HitTarget::Key(Key::Ctrl('s'))),
        (Rect::new(11, 0, 10, 1), HitTarget::Key(Key::Code(KeyCode::Esc))),
    ];
    let (mut state, now) = dialog_with(buttons.clone());
    let cmds = press(&mut state, click(2, 0, now));
    assert_eq!(starts(&cmds).len(), 1);
    assert!(matches!(state.focus, Focus::List));

    let (mut state, now) = dialog_with(buttons);
    let cmds = press(&mut state, click(12, 0, now));
    assert!(cmds.is_empty() && matches!(state.focus, Focus::List), "cancel closes without a start");
}

#[test]
fn a_click_outside_the_dialog_keeps_it_open_with_its_text() {
    let (mut state, now) = dialog_with(vec![(Rect::new(0, 0, 10, 1), HitTarget::Field(DialogField::Prompt))]);
    press(&mut state, key('x'));
    let typed = dialog(&state).branch.lines().join("");
    press(&mut state, click(50, 20, now));
    assert_eq!(dialog(&state).branch.lines().join(""), typed);
    assert_eq!(dialog(&state).field, DialogField::Branch);
}

#[test]
fn list_targets_do_nothing_while_the_dialog_is_open() {
    let (mut state, now) = dialog_with(vec![
        (Rect::new(0, 0, 10, 1), HitTarget::Tab(Tab::Prs)),
        (Rect::new(0, 1, 10, 1), HitTarget::Row(RowKey::Issue(IssueNumber(2)))),
    ]);
    press(&mut state, click(1, 0, now));
    press(&mut state, click(1, 1, now));
    press(&mut state, click(1, 1, now + Duration::from_millis(50)));
    assert_eq!(state.tab, Tab::Issues);
    assert_eq!(dialog(&state).key, issue_key(3), "the same dialog is still open");
}

#[test]
fn the_wheel_does_not_move_the_list_behind_the_dialog() {
    let (mut state, now) = app();
    press(&mut state, key('s'));
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
    assert!(matches!(state.focus, Focus::Dialog(_)));
}

#[test]
fn confirm_after_a_fix_session_took_the_issue_starts_nothing_and_goes_to_it() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    press(&mut state, with_ask(handoff(MergeProblem::Conflict), ASK_FIX));
    assert!(state.starting.contains(&issue_key(3)), "the fix session is starting");
    let cmds = press(&mut state, ctrl('s'));
    assert!(starts(&cmds).is_empty());
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(selected_row(&state), Some(RowKey::Session(issue_key(3))));
    assert_eq!(toast(&state), "#3 is already starting.");
}
