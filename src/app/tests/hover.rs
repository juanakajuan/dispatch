use super::*;
use ratatui::layout::Position;

fn moved(col: u16, row: u16, at: Instant) -> Msg {
    Msg::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: col, row, modifiers: KeyModifiers::NONE }, at)
}

#[test]
fn moving_the_mouse_tracks_the_pointer_and_clicks_nothing() {
    let (mut state, now) = app();
    state.hits.targets = vec![
        (Rect::new(10, 0, 10, 1), HitTarget::Tab(Tab::Prs)),
        (Rect::new(0, 3, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(2)))),
        (Rect::new(0, 5, 10, 1), HitTarget::Key(Key::Char(','))),
    ];
    let before = selected_row(&state);
    for (x, y) in [(12, 0), (5, 3), (2, 5)] {
        let cmds = press(&mut state, moved(x, y, now));
        assert_eq!(state.pointer, Some(Position { x, y }));
        assert!(cmds.is_empty());
    }
    assert_eq!(state.tab, Tab::Issues, "hovering a tab does not switch to it");
    assert_eq!(selected_row(&state), before, "hovering a row does not select it");
    assert!(matches!(state.focus, Focus::List), "hovering a key does not press it");
    assert!(state.last_click.is_none());
}

#[test]
fn a_click_moves_the_pointer_too() {
    let (mut state, now) = app();
    press(&mut state, click(4, 7, now));
    assert_eq!(state.pointer, Some(Position { x: 4, y: 7 }));
}

#[test]
fn losing_focus_clears_the_pointer() {
    let (mut state, now) = app();
    press(&mut state, moved(3, 3, now));
    press(&mut state, Msg::FocusLost);
    assert_eq!(state.pointer, None);
}
