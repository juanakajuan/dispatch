use super::*;

/// What a terminal sends for `S`.
fn shift_s() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
}

fn numbers(state: &State) -> Vec<u32> {
    visible_rows(state)
        .into_iter()
        .map(|k| match k {
            RowKey::Issue(n) | RowKey::Session(SessionKey::Issue(n)) => n.0,
            RowKey::Pr(n) | RowKey::Session(SessionKey::Pr(n)) => n.0,
        })
        .collect()
}

/// `app()` plus: #1 has the most comments, #2 was updated last. No PR
/// closes an issue and no session is live, so every issue is in the
/// "Open" group and only the sort orders them.
fn sortable() -> State {
    let (mut state, _) = app();
    for p in &mut state.prs.items {
        p.closes.clear();
    }
    state.issues.items[2].comment_count = 5;
    state.issues.items[1].updated_at = jiff::Timestamp::from_second(60).expect("in range");
    state
}

#[test]
fn s_cycles_every_issue_mode_in_order_and_wraps() {
    let mut state = sortable();
    let mut seen = vec![(state.tabs.sort_label(Tab::Issues), numbers(&state))];
    for _ in 0..IssueSort::ALL.len() {
        assert!(press(&mut state, shift_s()).is_empty());
        seen.push((state.tabs.sort_label(Tab::Issues), numbers(&state)));
    }
    assert_eq!(
        seen,
        [
            ("attention", vec![3, 2, 1]),
            ("newest", vec![3, 2, 1]),
            ("oldest", vec![1, 2, 3]),
            ("recently updated", vec![2, 3, 1]),
            ("number", vec![1, 2, 3]),
            ("most comments", vec![1, 3, 2]),
            ("attention", vec![3, 2, 1]),
        ]
    );
}

#[test]
fn rows_group_by_state_before_the_sort_orders_them() {
    let mut state = sortable();
    state.sessions.insert(issue_key(1), session(1, SessionState::NeedsYou));
    state.prs.items[0].closes = vec![IssueNumber(3)];
    for _ in 0..=IssueSort::ALL.len() {
        assert_eq!(numbers(&state), [1, 3, 2], "needs you, in review, open: {}", state.tabs.sort_label(Tab::Issues));
        press(&mut state, shift_s());
    }
}

#[test]
fn each_tab_keeps_its_own_mode() {
    let mut state = sortable();
    press(&mut state, shift_s());
    press(&mut state, key('2'));
    assert_eq!(state.tabs.sort_label(Tab::Prs), "attention");
    assert_eq!(numbers(&state), [10, 11]);
    for _ in 0..4 {
        press(&mut state, shift_s());
    }
    assert_eq!(state.tabs.sort_label(Tab::Prs), "number");
    assert_eq!(numbers(&state), [10, 11]);
    press(&mut state, shift_s());
    assert_eq!(state.tabs.sort_label(Tab::Prs), "attention", "PRs have no comment mode");

    press(&mut state, key('3'));
    state.sessions.insert(issue_key(1), session(1, SessionState::NeedsYou));
    state.sessions.insert(issue_key(9), session(9, SessionState::Running));
    state.sessions.insert(issue_key(10), session(10, SessionState::Exited));
    assert_eq!(numbers(&state), [1, 9, 10]);
    press(&mut state, shift_s());
    assert_eq!(state.tabs.sort_label(Tab::Sessions), "number");
    assert_eq!(numbers(&state), [1, 9, 10]);
    state.sessions.insert(issue_key(1), session(1, SessionState::Exited));
    assert_eq!(numbers(&state), [9, 1, 10], "the group comes first, then the number");

    press(&mut state, key('1'));
    assert_eq!(state.tabs.sort_label(Tab::Issues), "newest");
}

#[test]
fn re_sorting_keeps_the_selected_row_even_when_it_was_only_the_first() {
    let mut state = sortable();
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
    press(&mut state, shift_s());
    press(&mut state, shift_s());
    assert_eq!(numbers(&state), [1, 2, 3]);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
    press(&mut state, key('k'));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    press(&mut state, shift_s());
    assert_eq!(numbers(&state), [2, 3, 1]);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
}

#[test]
fn the_sort_applies_to_the_filtered_rows_and_typing_s_in_the_filter_is_text() {
    let mut state = sortable();
    press(&mut state, key('/'));
    press(&mut state, key('j'));
    press(&mut state, shift_s());
    assert_eq!(state.filter(), "jS");
    state.filter_mut().set("j");
    press(&mut state, code(KeyCode::Enter));
    assert_eq!(numbers(&state), [3, 1]);
    press(&mut state, shift_s());
    press(&mut state, shift_s());
    assert_eq!(numbers(&state), [1, 3]);
}
