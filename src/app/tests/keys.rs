use super::*;

#[test]
fn key_bar_follows_the_context() {
    let (mut state, _) = app();
    let names = |s: &State| key_bar(s).iter().map(|(k, d)| format!("{k} {d}")).collect::<Vec<_>>();
    assert!(names(&state).contains(&"s start session".to_string()));
    assert!(names(&state).contains(&"o open on GitHub".to_string()));
    assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
    state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
    assert!(names(&state).contains(&"a attach".to_string()));
    assert!(names(&state).contains(&"x stop".to_string()));
    state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
    assert!(names(&state).contains(&"a resume".to_string()));
    press(&mut state, key('2'));
    assert!(names(&state).contains(&"enter go to issue".to_string()));
    assert_eq!(names(&state), ["? keys", "enter go to issue", "a resume", "m merge", "o open on GitHub", "q quit"]);
}

#[test]
fn every_binding_has_a_description() {
    for b in BINDINGS {
        assert!(!b.desc.trim().is_empty(), "{:?} has no description", b.action);
    }
}

#[test]
fn every_binding_key_is_unique() {
    let mut seen: Vec<Key> = Vec::new();
    for k in BINDINGS.iter().flat_map(|b| b.keys) {
        assert!(!seen.contains(k), "{k:?} is bound twice");
        seen.push(*k);
    }
}

#[test]
fn release_events_are_ignored() {
    let (mut state, _) = app();
    let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    ev.kind = KeyEventKind::Release;
    assert!(press(&mut state, Msg::Key(ev)).is_empty());
}
