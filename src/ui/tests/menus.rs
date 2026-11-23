use super::*;
use crate::app::keys::{BINDINGS, KeyGroup};
use crate::config::settings::Setting;

#[test]
fn the_settings_menu_shows_each_key_value_and_default() {
    let mut state = full_state();
    state.config.detect_waiting = false;
    send(&mut state, KeyCode::Char(','));
    let screen = text(&render(&state, 100, 30));
    for want in [
        " Settings ",
        "detect_waiting   off   default: on",
        "icons            unicode   default: unicode",
        "harness          claude   default: claude",
        "command          claude   default: claude",
        "remove_on_merge  off   default: off",
        "Applies to sessions you start or resume after this change.",
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
    for _ in 0..4 {
        send(&mut state, KeyCode::Char('j'));
    }
    send(&mut state, KeyCode::Enter);
    for c in " 'oops".chars() {
        send(&mut state, KeyCode::Char(c));
    }
    send(&mut state, KeyCode::Enter);
    let screen = text(&render(&state, 100, 30));
    for want in ["command          claude 'oops▏", "has a quote that never closes", "enter to save · esc to cancel"]
    {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    assert!(!screen.contains("Applies to sessions"), "the error takes the note's line\n{screen}");
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
        let keys = app::keys::labels(b.keys);
        let shown = screen.lines().any(|l| l.contains(&format!(" {keys} ")) && l.contains(b.desc));
        assert!(shown, "no row shows {keys:?} with {:?} in\n{screen}", b.desc);
    }
    for want in ["j/down", "q/ctrl+c", "esc ", "j/k to scroll · esc to close"] {
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
    let max = state.hits.max_scroll;
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

#[test]
fn settings_rows_and_the_close_control_are_clickable() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    let screen = draw_into(&mut state, 100, 30);
    for (needle, setting) in [
        ("detect_waiting", Setting::DetectWaiting),
        ("icons ", Setting::Icons),
        ("multiplexer", Setting::Multiplexer),
        ("harness  ", Setting::Harness),
        ("command  ", Setting::Command),
        ("remove_on_merge  ", Setting::RemoveOnMerge),
    ] {
        assert_clickable(&state, &screen, needle, HitTarget::Setting(setting));
    }
    assert_clickable(&state, &screen, "esc ✕", HitTarget::Key(Key::Code(KeyCode::Esc)));
    let (x, y) = find(&screen, "Saved to config.json");
    assert_eq!(state.hits.at(x, y), None, "the notes under the rows are not targets");
}

#[test]
fn clicking_drawn_settings_selects_then_changes_and_the_close_control_closes() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "icons ");
    click_at(&mut state, x, y);
    let Focus::Settings(m) = &state.focus else { panic!("settings open") };
    assert_eq!(m.at, Setting::Icons);
    let before = state.config.icons;
    click_at(&mut state, x, y);
    assert_ne!(state.config.icons, before);
    let (x, y) = find(&screen, "esc ✕");
    click_at(&mut state, x, y);
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn the_keybinds_close_control_is_clickable_and_closes_the_menu() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    let screen = draw_into(&mut state, 100, 60);
    assert_clickable(&state, &screen, "esc ✕", HitTarget::Key(Key::Code(KeyCode::Esc)));
    let (x, y) = find(&screen, "esc ✕");
    click_at(&mut state, x + 1, y);
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn a_click_outside_an_overlay_does_nothing() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    let screen = draw_into(&mut state, 100, 30);
    let (x, y) = find(&screen, "#3 Ret");
    click_at(&mut state, x, y);
    click_at(&mut state, 20, 1);
    assert!(matches!(state.focus, Focus::Settings(_)), "the base screen is not clickable under an overlay");
}

fn binding_row(state: &State, screen: &str, key: Key) -> Option<(u16, Option<HitTarget>)> {
    let b = BINDINGS.iter().find(|b| b.keys.first() == Some(&key)).expect("binding");
    let y = u16::try_from(screen.lines().position(|l| l.contains(b.desc))?).expect("y");
    let hit = state.hits.at(40, y).map(|(_, t)| t);
    Some((y, hit))
}

#[test]
fn each_visible_keybinds_entry_is_a_target_at_its_drawn_row() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    let screen = draw_into(&mut state, 100, 40);
    let action = BINDINGS.iter().find(|b| b.keys.first() == Some(&Key::Char(','))).expect("binding").action;
    let (_, hit) = binding_row(&state, &screen, Key::Char(',')).expect("the settings entry is drawn");
    assert_eq!(hit, Some(HitTarget::Binding(action)));
    let entries = state.hits.targets.iter().filter(|(_, t)| matches!(t, HitTarget::Binding(_))).count();
    assert_eq!(entries, BINDINGS.len(), "one target per entry, none for headings");
}

#[test]
fn a_scrolled_out_keybinds_entry_has_no_target() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    draw_into(&mut state, 100, 16);
    let max = state.hits.max_scroll;
    for _ in 0..u32::from(max) + 5 {
        send(&mut state, KeyCode::Char('j'));
    }
    let screen = draw_into(&mut state, 100, 16);
    let entries: Vec<_> =
        state.hits.targets.iter().filter_map(|(r, t)| matches!(t, HitTarget::Binding(_)).then_some(r.y)).collect();
    assert!(!entries.is_empty() && entries.len() < BINDINGS.len(), "only the visible entries are targets");
    let first = BINDINGS.iter().find(|b| b.group == KeyGroup::ALL[0]).expect("first binding");
    let first_key = *first.keys.first().expect("key");
    assert!(!screen.contains(first.desc), "the first entry is scrolled out\n{screen}");
    assert!(binding_row(&state, &screen, first_key).is_none());
    let last = BINDINGS.last().expect("binding");
    let (y, hit) = binding_row(&state, &screen, *last.keys.first().expect("key")).expect("the last entry shows");
    assert_eq!(hit, Some(HitTarget::Binding(last.action)));
    assert!(entries.contains(&y));
}

#[test]
fn the_summary_shows_all_of_what_claude_said_wrapped_and_scrolled() {
    use crate::transcript::{Summary, SummaryKind, Transcript};
    let mut state = full_state();
    let long = "word ".repeat(40);
    let text = format!("Fixed the retry queue.\n\n{long}\nEnd of the recap.");
    let said = Summary { kind: SummaryKind::Recap, text };
    state.transcripts.insert(issue_key(3), Transcript { said: Some(said), active_at: jiff::Timestamp::UNIX_EPOCH });
    send(&mut state, KeyCode::Char('3'));
    send(&mut state, KeyCode::Char('i'));
    let screen = draw_into(&mut state, 60, 40);
    for want in [" #3 recap ", "Fixed the retry queue.", "End of the recap.", "esc ✕", "j/k to scroll · esc to close"]
    {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    assert_eq!(state.hits.max_scroll, 0, "it fits");

    let screen = draw_into(&mut state, 60, 9);
    assert!(screen.contains("Fixed the retry queue."), "it starts at the top\n{screen}");
    assert!(!screen.contains("End of the recap."), "the end is below the box\n{screen}");
    let max = state.hits.max_scroll;
    assert!(max > 0);
    for _ in 0..max {
        send(&mut state, KeyCode::Char('j'));
    }
    let screen = draw_into(&mut state, 60, 9);
    assert!(screen.contains("End of the recap."), "j scrolls to the end\n{screen}");
    send(&mut state, KeyCode::Esc);
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn the_diff_overlay_reads_then_shows_colored_lines_and_untracked_files() {
    use crate::app::DiffBody;
    use crate::session::Diff;
    let mut state = full_state();
    send(&mut state, KeyCode::Char('3'));
    send(&mut state, KeyCode::Char('d'));
    let screen = draw_into(&mut state, 60, 20);
    for want in [" #3 diff ", "Reading the diff…", "esc ✕"] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }

    let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n context\n";
    let long = format!("+{}", "x".repeat(200));
    let patch = format!("{patch}{long}\n");
    let diff = Diff { untracked: vec!["notes.txt".to_string()], patch };
    state.focus = Focus::Diff { key: issue_key(3), scroll: 0, body: DiffBody::Read(diff) };
    let buf = frame(&mut state, 60, 20);
    let screen = text(&buf);
    assert!(!screen.contains("Reading the diff"), "{screen}");
    let fg_of = |needle: &str| {
        let (x, y) = find(&screen, needle);
        buf[(x, y)].fg
    };
    assert_eq!(fg_of("untracked: notes.txt"), palette::FG);
    assert_eq!(fg_of("+new"), palette::GREEN);
    assert_eq!(fg_of("-old"), palette::RED);
    assert_eq!(fg_of("@@ -1 +1 @@"), palette::BLUE);
    assert_eq!(fg_of("+++ b/f"), palette::FG, "a file header is not an added line");
    assert_eq!(fg_of("--- a/f"), palette::FG, "a file header is not a removed line");
    assert_eq!(fg_of("context"), palette::FG);
    assert!(screen.contains('…'), "a long line is cut, not wrapped\n{screen}");
    assert!(!screen.contains("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
    assert_eq!(state.hits.max_scroll, 0, "it fits");

    let _ = draw_into(&mut state, 60, 8);
    assert!(state.hits.max_scroll > 0, "a short box scrolls");
}
