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
    let focuses: [(&str, &[KeyCode]); 5] = [
        ("list", &[]),
        ("filter", &[KeyCode::Char('/')]),
        ("dialog", &[KeyCode::Char('j'), KeyCode::Char('s')]),
        ("settings", &[KeyCode::Char(',')]),
        ("keybinds", &[KeyCode::Char('?')]),
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
                hints: app::keys::footer_hints(&state),
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
