use super::*;

fn hit_state() -> (State, Instant) {
    let (mut state, now) = app();
    state.hits.targets = vec![
        (Rect::new(0, 0, 10, 1), HitTarget::Tab(Tab::Issues)),
        (Rect::new(10, 0, 10, 1), HitTarget::Tab(Tab::Prs)),
        (Rect::new(0, 2, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(3)))),
        (Rect::new(0, 3, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(2)))),
    ];
    (state, now)
}

#[test]
fn click_selects_a_row_and_a_tab() {
    let (mut state, now) = hit_state();
    press(&mut state, click(5, 3, now));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    press(&mut state, click(12, 0, now));
    assert_eq!(state.tab, Tab::Prs);
}

#[test]
fn double_click_inside_the_window_opens_the_dialog_and_outside_only_selects() {
    let (mut state, now) = hit_state();
    press(&mut state, click(5, 3, now));
    press(&mut state, click(5, 3, now + DOUBLE_CLICK.saturating_sub(Duration::from_millis(1))));
    assert_eq!(dialog(&state).issue, IssueNumber(2));

    let (mut state, now) = hit_state();
    press(&mut state, click(5, 3, now));
    press(&mut state, click(5, 3, now + DOUBLE_CLICK + Duration::from_millis(1)));
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
}

#[test]
fn wheel_moves_the_selection() {
    let (mut state, now) = app();
    let wheel = |kind| Msg::Mouse(MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE }, now);
    press(&mut state, wheel(MouseEventKind::ScrollDown));
    press(&mut state, wheel(MouseEventKind::ScrollDown));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
    press(&mut state, wheel(MouseEventKind::ScrollDown));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))), "clamped at the end");
    press(&mut state, wheel(MouseEventKind::ScrollUp));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
}

#[test]
fn jk_move_and_stay_inside_the_list() {
    let (mut state, _) = app();
    press(&mut state, key('k'));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
    press(&mut state, key('j'));
    press(&mut state, code(KeyCode::Down));
    press(&mut state, key('j'));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
}

#[test]
fn a_filter_that_hides_the_selection_selects_the_first_visible_and_clearing_restores_it() {
    let (mut state, _) = app();
    press(&mut state, key('j'));
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    press(&mut state, key('/'));
    assert!(matches!(state.focus, Focus::Filter));
    for c in "json".chars() {
        press(&mut state, key(c));
    }
    press(&mut state, code(KeyCode::Enter));
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(1))]);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
    press(&mut state, code(KeyCode::Esc));
    assert_eq!(visible_rows(&state).len(), 3);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
}

#[test]
fn filter_esc_clears_and_backspace_edits() {
    let (mut state, _) = app();
    press(&mut state, key('/'));
    press(&mut state, key('a'));
    press(&mut state, key('b'));
    press(&mut state, code(KeyCode::Backspace));
    assert_eq!(state.filter(), "a");
    press(&mut state, code(KeyCode::Esc));
    assert_eq!((state.filter(), matches!(state.focus, Focus::List)), ("", true));
}

#[test]
fn pr_enter_goes_to_the_closed_issue_and_pr_rows_show_the_session() {
    let (mut state, _) = app();
    press(&mut state, key('2'));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    assert_eq!(current_session(&state).map(|s| s.issue), Some(IssueNumber(3)), "via closes");
    press(&mut state, code(KeyCode::Enter));
    assert_eq!(state.tab, Tab::Issues);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));

    press(&mut state, key('2'));
    press(&mut state, key('j'));
    assert_eq!(selected_row(&state), Some(RowKey::Pr(PrNumber(11))));
    press(&mut state, code(KeyCode::Enter));
    assert_eq!(state.tab, Tab::Prs, "a PR without closes stays");
}

#[test]
fn lead_pr_prefers_merged_then_open_then_draft_and_skips_closed() {
    let (mut state, _) = app();
    let with = |n: u32, s: PrState| PullRequest { state: s, ..pr(n, &format!("b{n}"), &[7]) };
    state.prs.items = vec![with(20, PrState::Closed)];
    assert!(lead_pr(&state, IssueNumber(7)).is_none(), "a closed PR never shows");
    state.prs.items.push(with(21, PrState::Draft));
    assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(21)));
    state.prs.items.push(with(22, PrState::Open));
    state.prs.items.push(with(23, PrState::Open));
    assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(23)), "newest open wins a tie");
    state.prs.items.push(with(19, PrState::Merged));
    assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(19)));
    assert!(lead_pr(&state, IssueNumber(8)).is_none());
}

#[test]
fn session_pr_is_found_by_head_branch() {
    let (state, _) = app();
    let s = session(3, SessionState::Exited);
    assert_eq!(pr_for_session(&state, &s).map(|p| p.number), Some(PrNumber(10)));
    assert!(pr_for_session(&state, &session(4, SessionState::Exited)).is_none());
}

#[test]
fn sessions_tab_lists_running_and_starting_before_exited_by_number() {
    let (mut state, _) = app();
    state.sessions.insert(IssueNumber(1), session(1, SessionState::Exited));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    state.starting.insert(IssueNumber(2));
    press(&mut state, key('3'));
    assert_eq!(
        visible_rows(&state),
        [RowKey::Session(IssueNumber(2)), RowKey::Session(IssueNumber(3)), RowKey::Session(IssueNumber(1))]
    );
    press(&mut state, key('/'));
    for c in "docs".chars() {
        press(&mut state, key(c));
    }
    assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(2))], "filters by the issue title");
}

#[test]
fn clicking_the_filter_prompt_opens_the_filter_and_clicking_clear_removes_it() {
    let (mut state, now) = with_target(HitTarget::Key(Key::Char('/')));
    press(&mut state, click(0, 0, now));
    assert!(matches!(state.focus, Focus::Filter));

    let (mut state, now) = with_target(HitTarget::Key(Key::Code(KeyCode::Esc)));
    *state.filter_mut() = "json".to_string();
    press(&mut state, click(0, 0, now));
    assert!(state.filter().is_empty());

    let (mut state, now) = with_target(HitTarget::Key(Key::Code(KeyCode::Esc)));
    *state.filter_mut() = "json".to_string();
    state.focus = Focus::Filter;
    press(&mut state, click(0, 0, now));
    assert!(state.filter().is_empty() && matches!(state.focus, Focus::List), "in the filter, clear also closes it");
}

#[test]
fn tabs_and_rows_ignore_clicks_while_something_else_has_focus() {
    let (mut state, now) = hit_state();
    state.focus = Focus::Filter;
    press(&mut state, click(12, 0, now));
    press(&mut state, click(5, 3, now));
    assert_eq!(state.tab, Tab::Issues);
    assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
}

#[test]
fn double_click_on_a_pr_row_goes_to_its_issue() {
    let (mut state, now) = with_target(HitTarget::Row(RowKey::Pr(PrNumber(10))));
    state.tab = Tab::Prs;
    press(&mut state, click(0, 0, now));
    press(&mut state, click(0, 0, now + Duration::from_millis(100)));
    assert_eq!((state.tab, state.tabs.issues.selected), (Tab::Issues, Some(IssueNumber(3))));
}

#[test]
fn double_click_on_a_session_row_attaches_to_it() {
    let (mut state, now) = with_target(HitTarget::Row(RowKey::Session(IssueNumber(3))));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    state.tab = Tab::Sessions;
    assert!(press(&mut state, click(0, 0, now)).is_empty(), "one click only selects");
    let cmds = press(&mut state, click(0, 0, now + Duration::from_millis(100)));
    assert!(matches!(cmds.as_slice(), [Cmd::Attach(n)] if *n == IssueNumber(3)));
}

#[test]
fn a_session_whose_branch_pr_merged_sits_in_a_merged_group_after_exited() {
    let (mut state, _) = app();
    state.sessions.insert(IssueNumber(1), session(1, SessionState::Exited));
    state.sessions.insert(IssueNumber(2), session(2, SessionState::Running));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::NeedsYou));
    state.prs.items[0].state = PrState::Merged;
    press(&mut state, key('3'));
    assert_eq!(
        visible_rows(&state),
        [RowKey::Session(IssueNumber(2)), RowKey::Session(IssueNumber(1)), RowKey::Session(IssueNumber(3))],
        "a merged PR outranks the window's state"
    );
    let group = group_of(&state, RowKey::Session(IssueNumber(3)));
    assert_eq!((group, group.name()), (Group::Session(SessionGroup::Merged), "Merged"));
    assert_eq!(group_of(&state, RowKey::Issue(IssueNumber(3))), Group::Issue(IssueGroup::Merged));
    press(&mut state, key('S'));
    assert_eq!(visible_rows(&state).last(), Some(&RowKey::Session(IssueNumber(3))), "and in number order");
}

#[test]
fn the_session_filter_matches_the_branch_its_worktree_has_checked_out() {
    let (mut state, _) = app();
    let mut s = session(3, SessionState::Running);
    s.checkout = crate::session::Checkout::Other(BranchName::parse("try-other").expect("branch"));
    state.sessions.insert(IssueNumber(3), s);
    press(&mut state, key('3'));
    press(&mut state, key('/'));
    for c in "try-other".chars() {
        press(&mut state, key(c));
    }
    assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(3))]);
}
