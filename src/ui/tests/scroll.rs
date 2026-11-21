use super::*;

/// 30 plain issues with no sessions: one "Open" group, so the list has a
/// heading line and then one line per issue.
fn long_list() -> State {
    let mut state = full_state();
    state.sessions = crate::session::Sessions::new();
    state.issues.items = (1..=30).rev().map(|n| issue(n, &format!("Issue {n}"), &[])).collect();
    state
}

/// The screen row of the `❯` marker on a list row.
fn selector_row(screen: &str) -> usize {
    screen.lines().position(|l| l.contains('❯') && l.contains('#')).unwrap_or_else(|| panic!("no ❯ in\n{screen}"))
}

#[test]
fn the_selector_moves_to_the_center_then_the_list_scrolls_under_it_then_it_moves_to_the_end() {
    let mut state = long_list();
    let mut rows = Vec::new();
    for _ in 0..30 {
        let screen = draw_into(&mut state, 100, 17);
        rows.push(selector_row(&screen));
        send(&mut state, KeyCode::Char('j'));
    }
    let first = rows[0];
    let max = *rows.iter().max().expect("rows");
    let center = rows.iter().copied().filter(|&r| r == rows[15]).count();
    assert!(rows.windows(2).all(|w| w[0] <= w[1]), "the selector never moves up on j: {rows:?}");
    assert!(center > 10, "the selector stays on one center row for most of the list: {rows:?}");
    assert!(rows[15] > first && rows[15] < max, "the center is between the top and the last row: {rows:?}");
    assert_eq!(rows[29], max, "at the end the selector is on the last row: {rows:?}");
}

#[test]
fn the_center_has_the_same_number_of_list_lines_above_and_below() {
    let mut state = long_list();
    for _ in 0..15 {
        send(&mut state, KeyCode::Char('j'));
    }
    let _ = draw_into(&mut state, 100, 17);
    let ys: Vec<u16> =
        state.hits.targets.iter().filter_map(|(r, t)| matches!(t, HitTarget::Row(_)).then_some(r.y)).collect();
    let screen = draw_into(&mut state, 100, 17);
    let at = u16::try_from(selector_row(&screen)).expect("y");
    let (top, bottom) = (*ys.first().expect("rows"), *ys.last().expect("rows"));
    assert!((at - top).abs_diff(bottom - at) <= 1, "rows {top}..={bottom}, selector at {at}");
    assert!(screen.contains("#15 Issue 15"), "the selected issue is drawn\n{screen}");
}

#[test]
fn a_click_selects_the_row_drawn_under_it_after_a_scroll() {
    let mut state = long_list();
    for _ in 0..20 {
        send(&mut state, KeyCode::Char('j'));
    }
    let screen = draw_into(&mut state, 100, 17);
    assert_clickable(&state, &screen, "Issue 9", HitTarget::Row(RowKey::Issue(IssueNumber(9))));
    let (x, y) = find(&screen, "Issue 9");
    click_at(&mut state, x, y);
    let screen = draw_into(&mut state, 100, 17);
    let line = screen.lines().nth(selector_row(&screen)).expect("line");
    assert!(line.contains("#9 Issue 9"), "the clicked row is selected: {line:?}");
}
