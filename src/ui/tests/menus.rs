use super::*;

#[test]
fn the_settings_menu_shows_each_key_value_and_default() {
    let mut state = full_state();
    state.config.detect_waiting = false;
    send(&mut state, KeyCode::Char(','));
    let screen = text(&render(&state, 100, 30));
    for want in [
        " Settings ",
        "detect_waiting  off   default: on",
        "icons           unicode   default: unicode",
        "claude_command  claude   default: claude",
        "Applies to sessions you start or resume after this change.",
        "j/k move · enter change · esc close",
        "Saved to config.json",
    ] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    assert!(screen.contains("› detect_waiting"), "the selected row is marked\n{screen}");
}

#[test]
fn the_settings_menu_shows_the_editor_and_its_error() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    for _ in 0..3 {
        send(&mut state, KeyCode::Char('j'));
    }
    send(&mut state, KeyCode::Enter);
    for c in " 'oops".chars() {
        send(&mut state, KeyCode::Char(c));
    }
    send(&mut state, KeyCode::Enter);
    let screen = text(&render(&state, 100, 30));
    for want in ["claude_command  claude 'oops▏", "has a quote that never closes", "enter save · esc cancel"] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    assert!(!screen.contains("Applies to sessions"), "the error takes the note's line\n{screen}");
}

/// Draws like the main loop does, so the reducer sees this frame's size.
fn draw_into(state: &mut State, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    let mut hits = HitMap::default();
    terminal.draw(|f| hits = draw(f, state)).expect("draw");
    state.hits = hits;
    text(terminal.backend().buffer())
}

#[test]
fn the_keybinds_menu_shows_every_key_in_bindings() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    let screen = draw_into(&mut state, 100, 60);
    for group in app::keys::KeyGroup::ALL {
        assert!(screen.contains(group.title()), "missing heading {:?} in\n{screen}", group.title());
    }
    for b in app::keys::BINDINGS {
        let keys = b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
        let shown = screen.lines().any(|l| l.contains(&format!(" {keys} ")) && l.contains(b.desc));
        assert!(shown, "no row shows {keys:?} with {:?} in\n{screen}", b.desc);
    }
    for want in ["j / down", "q / ctrl+c", "esc ", "j/k to scroll · esc to close"] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
}

#[test]
fn a_short_terminal_scrolls_the_keybinds_menu_with_j_and_k() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    let screen = draw_into(&mut state, 100, 16);
    assert!(screen.contains("Navigation"), "the menu starts at the top\n{screen}");
    assert!(!screen.contains("show all keys"), "the end is below the menu\n{screen}");
    let max = state.hits.keybinds_max_scroll;
    assert!(max > 0);
    for _ in 0..u32::from(max) + 5 {
        send(&mut state, KeyCode::Char('j'));
    }
    assert!(matches!(state.focus, Focus::Keybinds { scroll } if scroll == max), "j stops at the end");
    let screen = draw_into(&mut state, 100, 16);
    assert!(screen.contains("show all keys"), "j scrolls to the end\n{screen}");
    assert!(!screen.contains("Navigation"), "the top scrolled out\n{screen}");
    send(&mut state, KeyCode::Char('k'));
    assert!(matches!(state.focus, Focus::Keybinds { scroll } if scroll == max - 1), "one k moves back one line");
}
