use super::*;
use crate::transcript::{Summary, SummaryKind, Transcript};

fn said(state: &mut State, n: u32, text: &str) {
    let said = Summary { kind: SummaryKind::Message, text: text.to_string() };
    state.transcripts.insert(IssueNumber(n), Transcript { said: Some(said) });
}

#[test]
fn i_opens_the_summary_of_the_selected_session_and_esc_or_i_closes_it() {
    let (mut state, now) = app();
    assert!(press(&mut state, key('i')).is_empty());
    assert_eq!(
        state.toast.as_ref().map(Toast::text),
        Some("No session for this issue. Press s to start one."),
        "an issue with no session"
    );

    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    press(&mut state, key('i'));
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("Claude has not said anything in #3 yet."));
    assert!(!key_bar(&state).iter().any(|h| h.keys == "i"), "the key bar hides i until Claude says something");

    said(&mut state, 3, "First line.\nSecond line.");
    assert!(key_bar(&state).iter().any(|h| (h.keys, h.desc) == ("i", "read summary")));
    press(&mut state, key('i'));
    assert!(matches!(state.focus, Focus::Summary { issue: IssueNumber(3), scroll: 0 }));
    assert!(press(&mut state, key('q')).is_empty(), "list keys do not act while it is open");
    assert!(matches!(state.focus, Focus::Summary { .. }));

    state.hits.max_scroll = 2;
    for _ in 0..3 {
        press(&mut state, key('j'));
    }
    assert!(matches!(state.focus, Focus::Summary { scroll: 2, .. }), "j stops at the end");
    press(&mut state, wheel(MouseEventKind::ScrollUp, now));
    assert!(matches!(state.focus, Focus::Summary { scroll: 1, .. }), "the wheel scrolls it");

    press(&mut state, code(KeyCode::Esc));
    assert!(matches!(state.focus, Focus::List), "esc closes it");
    press(&mut state, key('i'));
    press(&mut state, key('i'));
    assert!(matches!(state.focus, Focus::List), "i closes it again");
}
