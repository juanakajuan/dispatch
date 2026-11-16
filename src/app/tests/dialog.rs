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
    state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
    let cmds = press(&mut state, key('s'));
    assert!(cmds.is_empty());
    assert_eq!(state.tab, Tab::Sessions);
    assert_eq!(selected_row(&state), Some(RowKey::Session(IssueNumber(3))));
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
    assert!(dialog(&state).error.is_some());
    assert!(state.starting.is_empty());
}

#[test]
fn confirm_starts_once_and_a_second_s_emits_no_start() {
    let (mut state, _) = app();
    press(&mut state, key('s'));
    let cmds = press(&mut state, ctrl('s'));
    let reqs = starts(&cmds);
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].issue, IssueNumber(3));
    assert_eq!(reqs[0].branch.as_str(), "fix/3-retry-queue-drops-jobs");
    assert!(reqs[0].prompt.starts_with("Implement GitHub issue #3"));
    assert_eq!(state.tab, Tab::Sessions);
    assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(3))]);

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
