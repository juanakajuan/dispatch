use super::*;

/// The narrowest terminal dispatch supports.
const MIN_WIDTH: u16 = 80;

struct Footer {
    at: String,
    hints: Vec<(&'static str, &'static str)>,
    line: String,
}

/// Every tab in every focus: the footer line at `width`, with the hints the
/// state offers.
fn footers(width: u16) -> Vec<Footer> {
    let focuses: [(&str, &[KeyCode]); 7] = [
        ("list", &[]),
        ("filter", &[KeyCode::Char('/')]),
        ("dialog", &[KeyCode::Char('j'), KeyCode::Char('s')]),
        ("batch", &[KeyCode::Char(' '), KeyCode::Char('s')]),
        ("settings", &[KeyCode::Char(',')]),
        ("keybinds", &[KeyCode::Char('?')]),
        ("message", &[KeyCode::Char('t')]),
    ];
    let mut out = Vec::new();
    for tab in ['1', '2', '3'] {
        for (focus, keys) in focuses {
            let mut state = full_state();
            send(&mut state, KeyCode::Char(tab));
            for key in keys {
                send(&mut state, *key);
            }
            let screen = text(&render(&state, width, 30));
            let footer = screen.lines().last().unwrap_or_default().trim().to_string();
            out.push(Footer {
                at: format!("tab {tab}, {focus}"),
                hints: app::keys::footer_hints(&state).into_iter().map(|h| (h.keys, h.desc)).collect(),
                line: footer,
            });
        }
    }
    out
}

fn shown(hint: (&str, &str)) -> String {
    format!("{} to {}", hint.0, hint.1)
}

/// Text cut at the terminal width hid key hints three times (b0633da,
/// 4ed28c0, 24a44de). At the narrowest supported width a hint is whole or
/// absent, and the keys and quit hints stay.
#[test]
fn at_the_narrowest_width_every_shown_hint_is_whole_and_keys_and_quit_stay() {
    for Footer { at, hints, line: footer } in footers(MIN_WIDTH) {
        assert!(!footer.contains('…'), "{at}: a hint is cut in {footer:?}");
        let before_sort = footer.split("S sort:").next().unwrap_or_default().trim();
        let known: Vec<String> = hints.iter().map(|h| shown(*h)).collect();
        for part in before_sort.split(" · ").filter(|p| !p.is_empty()) {
            assert!(known.contains(&part.to_string()), "{at}: {part:?} is not one of {known:?} in {footer:?}");
        }
        for keep in [("?", "keys"), ("q", "quit")] {
            if hints.contains(&keep) {
                assert!(footer.contains(&shown(keep)), "{at}: {:?} dropped from {footer:?}", shown(keep));
            }
        }
    }
}

#[test]
fn at_a_wide_width_every_hint_is_shown() {
    for Footer { at, hints, line: footer } in footers(200) {
        for hint in hints {
            assert!(footer.contains(&shown(hint)), "{at}: {:?} missing from {footer:?}", shown(hint));
        }
    }
}

#[test]
fn a_toast_is_red_for_a_failed_job_and_plain_for_a_warning_whatever_its_text() {
    let mut state = full_state();
    let error = crate::proc::fake::fail("git", 128, "no network");
    let base = crate::domain::OriginRef::parse("origin/main").expect("ref");
    let warning = crate::session::error::StartWarning::FetchFailed { base, error };
    let session = state.sessions[&IssueNumber(3)].clone();
    let outcome = crate::session::StartOutcome { session, warnings: vec![warning] };
    let _ = app::update(&mut state, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
    let (text, style) = super::footer::message(&state).expect("toast");
    assert!(text.contains("fetch failed"), "{text}");
    assert_eq!(style, palette::PLAIN, "a start that went on is not an error");

    let error = crate::github::GhError::Run(crate::proc::fake::fail("gh", 1, "no browser"));
    let target = crate::domain::GithubRef::Issue(IssueNumber(2));
    let _ = app::update(&mut state, Msg::Opened { target, result: Err(error) });
    let (text, style) = super::footer::message(&state).expect("toast");
    assert_eq!(text, "Open issue #2 failed: gh failed with code 1: no browser");
    assert_eq!(style, palette::ERROR);
}

/// The `Key` targets on the footer row of a `h`-row screen.
fn footer_keys(state: &State, h: u16) -> Vec<Key> {
    state
        .hits
        .targets
        .iter()
        .filter(|(r, _)| r.y == h - 1)
        .filter_map(|(_, t)| if let HitTarget::Key(k) = t { Some(*k) } else { None })
        .collect()
}

#[test]
fn key_bar_entries_and_the_sort_footer_are_clickable() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 200, 24);
    for (needle, key) in [
        ("? to keys", Key::Char('?')),
        ("s to start session", Key::Char('s')),
        ("q to quit", Key::Char('q')),
        ("S sort: attention", Key::Char('S')),
    ] {
        assert_clickable(&state, &screen, needle, HitTarget::Key(key));
    }
    let (x, y) = find(&screen, "? to keys");
    let gap = x + u16::try_from(cells("? to keys")).expect("w");
    assert_eq!(state.hits.at(gap, y).map(|(_, t)| t), None, "the separator is not part of an entry");
}

/// The footer drops hints from the right to keep `? to keys` and `q to
/// quit`. A dropped hint has no target, a kept one does, and a kept one that
/// the edge still cuts has none.
#[test]
fn a_key_bar_entry_cut_by_a_narrow_terminal_is_not_clickable() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 40, 24);
    assert!(screen.contains("? to keys") && screen.contains("q to quit"), "{screen}");
    assert!(!screen.contains("s to start session") && !screen.contains("S sort"), "{screen}");
    assert_clickable(&state, &screen, "q to quit", HitTarget::Key(Key::Char('q')));
    assert_eq!(footer_keys(&state, 24), [Key::Char('?'), Key::Char('q')], "dropped hints and the sort are not targets");

    let screen = draw_into(&mut state, 20, 24);
    assert!(screen.contains("? to keys") && !screen.contains("q to quit"), "{screen}");
    assert_eq!(footer_keys(&state, 24), [Key::Char('?')], "the cut q hint is not a target");
}

#[test]
fn the_prompt_opens_the_filter_and_the_clear_control_clears_it() {
    let mut state = full_state();
    let screen = draw_into(&mut state, 100, 24);
    assert_clickable(&state, &screen, "/ to filter issues", HitTarget::Key(Key::Char('/')));
    assert!(!screen.contains("esc to clear"), "nothing to clear yet\n{screen}");

    state.filter = "retry".to_string();
    let screen = draw_into(&mut state, 100, 24);
    assert_clickable(&state, &screen, "esc to clear", HitTarget::Key(Key::Code(KeyCode::Esc)));
    let (x, y) = find(&screen, "/retry");
    assert_eq!(state.hits.at(x, y).map(|(_, t)| t), Some(HitTarget::Key(Key::Char('/'))));

    send(&mut state, KeyCode::Char('/'));
    let screen = draw_into(&mut state, 100, 24);
    assert_clickable(&state, &screen, "esc to clear", HitTarget::Key(Key::Code(KeyCode::Esc)));
    let (x, y) = find(&screen, "retry");
    assert_eq!(state.hits.at(x, y), None, "typing in the filter is not a click target");
}

#[test]
fn overlay_footers_are_clickable_and_the_base_screen_is_not() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('?'));
    let screen = draw_into(&mut state, 100, 60);
    for (needle, key) in [("j/k to scroll", Key::Char('j')), ("esc to close", Key::Code(KeyCode::Esc))] {
        assert_clickable(&state, &screen, needle, HitTarget::Key(key));
    }
    assert!(
        state.hits.targets.iter().all(|(_, t)| matches!(t, HitTarget::Key(_) | HitTarget::Binding(_))),
        "tabs and rows are not live under an overlay"
    );

    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    let screen = draw_into(&mut state, 120, 40);
    for (needle, key) in [
        ("ctrl-s to start", Key::Ctrl('s')),
        ("tab to next field", Key::Code(KeyCode::Tab)),
        ("esc to cancel", Key::Code(KeyCode::Esc)),
    ] {
        assert_clickable(&state, &screen, needle, HitTarget::Key(key));
    }

    let mut state = full_state();
    send(&mut state, KeyCode::Char(','));
    let screen = draw_into(&mut state, 100, 30);
    for (needle, key) in [
        ("j/k to move", Key::Char('j')),
        ("enter to change", Key::Code(KeyCode::Enter)),
        ("esc to close", Key::Code(KeyCode::Esc)),
    ] {
        assert_clickable(&state, &screen, needle, HitTarget::Key(key));
    }
    for _ in 0..4 {
        send(&mut state, KeyCode::Char('j'));
    }
    send(&mut state, KeyCode::Enter);
    let screen = draw_into(&mut state, 100, 30);
    assert_clickable(&state, &screen, "enter to save", HitTarget::Key(Key::Code(KeyCode::Enter)));
    assert_clickable(&state, &screen, "esc to cancel", HitTarget::Key(Key::Code(KeyCode::Esc)));
}

#[test]
fn the_filter_footer_is_clickable() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('/'));
    let screen = draw_into(&mut state, 100, 24);
    assert_clickable(&state, &screen, "enter to keep", HitTarget::Key(Key::Code(KeyCode::Enter)));
    assert_clickable(&state, &screen, "esc to clear", HitTarget::Key(Key::Code(KeyCode::Esc)));
}

#[test]
fn the_prompt_shows_each_filter_term_in_the_accent_colour() {
    let mut state = full_state();
    state.filter = "retry label:bug assignee:@me milestone:".to_string();
    let fg = |state: &mut State, needle: &str| {
        let screen = draw_into(state, 120, 24);
        let (x, y) = find(&screen, needle);
        Some(render(state, 120, 24)[(x, y)].fg)
    };
    assert_eq!(fg(&mut state, "retry"), palette::LABEL.fg, "free text");
    assert_eq!(fg(&mut state, "label:bug"), palette::ACCENT.fg);
    assert_eq!(fg(&mut state, "assignee:@me"), palette::ACCENT.fg);
    assert_eq!(fg(&mut state, "milestone:"), palette::DIM.fg, "a term with no value yet");

    let _ = app::update(&mut state, Msg::Viewer(Err(crate::github::GhError::EmptyLogin)));
    assert_eq!(fg(&mut state, "assignee:@me"), palette::ERROR.fg, "@me can not match without the login");

    send(&mut state, KeyCode::Char('/'));
    assert_eq!(fg(&mut state, "label:bug"), palette::ACCENT.fg, "while typing too");
}

#[test]
fn sessions_tab_with_merged_session_keeps_keys_and_quit_at_min_width_and_clicks_clean_when_wide() {
    let mut state = full_state();
    state.prs.items[0].state = PrState::Merged;
    send(&mut state, KeyCode::Char('3'));

    let narrow = text(&render(&state, MIN_WIDTH, 30));
    let footer = narrow.lines().last().unwrap_or_default().trim().to_string();
    assert!(!footer.contains('…'), "a hint is cut in {footer:?}");
    let hints: Vec<_> = app::keys::footer_hints(&state).into_iter().map(|h| (h.keys, h.desc)).collect();
    assert!(hints.contains(&("D", "remove")));
    assert!(hints.contains(&("c", "clean merged")));
    let before_sort = footer.split("S sort:").next().unwrap_or_default().trim();
    let known: Vec<String> = hints.iter().map(|h| shown(*h)).collect();
    for part in before_sort.split(" · ").filter(|p| !p.is_empty()) {
        assert!(known.contains(&part.to_string()), "{part:?} is not one of {known:?} in {footer:?}");
    }
    for keep in [("?", "keys"), ("q", "quit")] {
        assert!(footer.contains(&shown(keep)), "{:?} dropped from {footer:?}", shown(keep));
    }

    let wide = draw_into(&mut state, 200, 30);
    assert_clickable(&state, &wide, "c to clean merged", HitTarget::Key(Key::Char('c')));
}
