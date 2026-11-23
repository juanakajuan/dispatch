use super::*;
use crate::app::dialog::DialogField;
use ratatui::layout::Rect;

fn point(state: &mut State, x: u16, y: u16) {
    let ev = MouseEvent { kind: MouseEventKind::Moved, column: x, row: y, modifiers: KeyModifiers::NONE };
    let _ = app::update(state, Msg::Mouse(ev, Instant::now()));
}

/// The cells with the hover background.
fn tinted(buf: &Buffer) -> Vec<(u16, u16)> {
    buf.area.positions().filter(|&p| buf[p].bg == palette::BG1).map(|p| (p.x, p.y)).collect()
}

/// Point at the first cell of `needle`, redraw, and check that the target
/// under it is tinted and nothing else is. A badge keeps its own color.
fn assert_hover_tints(state: &mut State, w: u16, h: u16, needle: &str) {
    let screen = draw_into(state, w, h);
    let (x, y) = find(&screen, needle);
    point(state, x, y);
    let buf = frame(state, w, h);
    let (rect, _) = state.hits.at(x, y).unwrap_or_else(|| panic!("{needle:?} is a target"));
    let badge = |p: Position| [palette::BLUE, palette::BG2].contains(&buf[p].bg);
    let want: Vec<(u16, u16)> = rect.positions().filter(|&p| !badge(p)).map(|p| (p.x, p.y)).collect();
    assert_eq!(tinted(&buf), want, "{needle:?}: only its target is tinted");
}

#[test]
fn the_target_under_the_pointer_is_tinted_on_the_list() {
    for needle in ["#2 Docs", "Pull requests", "q to quit", "S sort"] {
        let mut state = full_state();
        assert_hover_tints(&mut state, 100, 30, needle);
    }
}

#[test]
fn a_hovered_tab_keeps_its_badge_color() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "Pull requests");
    point(&mut state, x, y);
    let buf = frame(&mut state, 100, 30);
    let (rect, _) = state.hits.at(x, y).expect("tab target");
    assert_eq!(buf[(rect.x + 1, y)].bg, palette::BG2, "the badge is not tinted");
    assert_eq!(buf[(x, y)].bg, palette::BG1, "the label is tinted");
}

#[test]
fn a_hovered_row_does_not_look_selected() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "#2 Docs");
    point(&mut state, x, y);
    let screen = draw_into(&mut state, 100, 30);
    let marked: Vec<&str> = screen.lines().filter(|l| l.contains('❯') && l.contains('#')).collect();
    assert_eq!(marked.len(), 1, "one selected row in\n{screen}");
    assert!(marked[0].contains("#3 Retry"), "the selection stays on #3: {marked:?}");
}

#[test]
fn moving_off_every_target_clears_the_tint() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "#2 Docs");
    point(&mut state, x, y);
    assert!(!tinted(&frame(&mut state, 100, 30)).is_empty());
    point(&mut state, 0, 0);
    assert_eq!(tinted(&frame(&mut state, 100, 30)), Vec::new(), "the corner is not a target");
    point(&mut state, x, y);
    let _ = app::update(&mut state, Msg::FocusLost);
    assert_eq!(tinted(&frame(&mut state, 100, 30)), Vec::new(), "focus lost clears the hover");
}

#[test]
fn overlay_targets_are_tinted() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    for needle in ["icons ", "esc ✕", "esc to cancel"] {
        assert_hover_tints(&mut state, 100, 30, needle);
    }
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    assert_hover_tints(&mut state, 100, 40, "open the settings menu");
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    for needle in ["Branch", "Implement directly", "[ Start ]", "[ Cancel ]"] {
        assert_hover_tints(&mut state, 120, 40, needle);
    }
}

#[test]
fn the_base_screen_under_an_overlay_is_not_tinted() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "#3 Ret");
    point(&mut state, x, y);
    assert_eq!(tinted(&frame(&mut state, 100, 30)), Vec::new());
}

#[test]
fn a_text_input_is_not_tinted() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let _ = frame(&mut state, 120, 40);
    let rect: Rect = state
        .hits
        .targets
        .iter()
        .find(|(_, t)| matches!(t, HitTarget::Text { field: DialogField::Prompt, .. }))
        .map(|(r, _)| *r)
        .expect("prompt input");
    point(&mut state, rect.x, rect.y);
    assert_eq!(tinted(&frame(&mut state, 120, 40)), Vec::new());
}
