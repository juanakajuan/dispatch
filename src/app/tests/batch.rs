use super::*;

/// Marks #3 and #1 (the first and third rows), and leaves the selection on #1.
fn mark_3_and_1(state: &mut State) {
    press(state, key(' '));
    press(state, key('j'));
    press(state, key('j'));
    press(state, key(' '));
}

#[test]
fn space_marks_an_issue_and_a_second_space_unmarks_it() {
    let (mut state, _) = app();
    assert!(press(&mut state, key(' ')).is_empty());
    assert_eq!(state.marked, [IssueNumber(3)].into());
    press(&mut state, key(' '));
    assert!(state.marked.is_empty());
}

#[test]
fn space_does_nothing_on_the_other_tabs() {
    let (mut state, _) = app();
    press(&mut state, key('2'));
    press(&mut state, key(' '));
    press(&mut state, key('3'));
    press(&mut state, key(' '));
    assert!(state.marked.is_empty());
    assert!(state.toast.is_none());
}

#[test]
fn space_refuses_an_issue_that_has_a_session_or_is_starting() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key(' '));
    assert!(state.marked.is_empty());
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("#3 already has a session."));

    let (mut state, _) = app();
    state.starting.insert(issue_key(3));
    press(&mut state, key(' '));
    assert!(state.marked.is_empty());
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("#3 already has a session."));
}

#[test]
fn s_with_marks_opens_the_batch_dialog_listing_them() {
    let (mut state, _) = app();
    mark_3_and_1(&mut state);
    assert!(press(&mut state, key('s')).is_empty());
    let b = batch(&state);
    assert_eq!(
        b.issues,
        [(IssueNumber(1), "Add json output".to_string()), (IssueNumber(3), "Retry queue drops jobs".to_string())]
    );
    assert_eq!(b.mode, Mode::Implement);
}

#[test]
fn s_when_no_marked_issue_can_start_clears_the_marks() {
    let (mut state, _) = app();
    press(&mut state, key(' '));
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('s'));
    assert!(matches!(state.focus, Focus::List));
    assert!(state.marked.is_empty());
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("No marked issue can start."));
}

#[test]
fn batch_mode_toggles_with_left_right_space_tab_and_backtab() {
    let (mut state, _) = app();
    press(&mut state, key(' '));
    press(&mut state, key('s'));
    for (expected, msg) in [
        (Mode::PlanFirst, code(KeyCode::Left)),
        (Mode::Implement, code(KeyCode::Right)),
        (Mode::PlanFirst, key(' ')),
        (Mode::Implement, code(KeyCode::Tab)),
        (Mode::PlanFirst, code(KeyCode::BackTab)),
    ] {
        press(&mut state, msg);
        assert_eq!(batch(&state).mode, expected);
    }
}

#[test]
fn confirm_starts_each_marked_issue_with_its_defaults_and_the_chosen_mode() {
    let (mut state, _) = app();
    mark_3_and_1(&mut state);
    press(&mut state, key('s'));
    press(&mut state, key(' '));
    let cmds = press(&mut state, ctrl('s'));
    let reqs = starts(&cmds);
    assert_eq!(reqs.len(), 2);
    assert_eq!((reqs[0].key, reqs[0].branch.as_str()), (issue_key(1), "feat/1-add-json-output"));
    assert_eq!((reqs[1].key, reqs[1].branch.as_str()), (issue_key(3), "fix/3-retry-queue-drops-jobs"));
    for r in &reqs {
        assert_eq!(r.mode, Mode::PlanFirst);
        assert!(r.prompt.starts_with(&format!("Implement GitHub issue {}:", r.key)));
        assert_eq!(r.from, StartFrom::Base);
    }
    assert!(state.marked.is_empty());
    assert_eq!(state.starting, [issue_key(1), issue_key(3)].into());
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(state.tab, Tab::Sessions);
    assert_eq!(selected_row(&state), Some(RowKey::Session(issue_key(1))));
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("Starting #1, #3…"));
}

#[test]
fn enter_confirms_the_batch_too() {
    let (mut state, _) = app();
    press(&mut state, key(' '));
    press(&mut state, key('s'));
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(starts(&cmds).len(), 1);
}

#[test]
fn esc_closes_the_batch_dialog_and_keeps_the_marks() {
    let (mut state, _) = app();
    mark_3_and_1(&mut state);
    press(&mut state, key('s'));
    assert!(press(&mut state, code(KeyCode::Esc)).is_empty());
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(state.marked, [IssueNumber(1), IssueNumber(3)].into());
    assert!(state.starting.is_empty());
}

#[test]
fn an_issue_that_gained_a_session_while_the_dialog_was_open_is_skipped() {
    let (mut state, _) = app();
    mark_3_and_1(&mut state);
    press(&mut state, key('s'));
    state.sessions.insert(issue_key(1), session(1, SessionState::Running));
    let cmds = press(&mut state, ctrl('s'));
    let reqs = starts(&cmds);
    assert_eq!(reqs.iter().map(|r| r.key).collect::<Vec<_>>(), [issue_key(3)]);
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("Starting #3…"));
}

#[test]
fn a_batch_where_every_issue_gained_a_session_starts_nothing() {
    let (mut state, _) = app();
    press(&mut state, key(' '));
    press(&mut state, key('s'));
    state.starting.insert(issue_key(3));
    let cmds = press(&mut state, ctrl('s'));
    assert!(cmds.is_empty());
    assert!(state.marked.is_empty());
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("No marked issue can start."));
}

#[test]
fn a_single_start_of_a_marked_issue_clears_its_mark() {
    let (mut state, _) = app();
    press(&mut state, key(' '));
    let issue = state.issues.items[0].clone();
    state.focus = Focus::Dialog(Box::new(open_dialog(&issue)));
    let cmds = press(&mut state, ctrl('s'));
    assert_eq!(starts(&cmds).len(), 1);
    assert!(state.marked.is_empty());
}

#[test]
fn clicking_a_mode_option_sets_the_batch_mode() {
    let (mut state, now) = app();
    press(&mut state, key(' '));
    press(&mut state, key('s'));
    state.hits.targets = vec![(Rect::new(0, 0, 10, 1), HitTarget::Mode(Mode::PlanFirst))];
    press(&mut state, click(1, 0, now));
    assert_eq!(batch(&state).mode, Mode::PlanFirst);
}
