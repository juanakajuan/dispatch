use super::*;

#[test]
fn key_bar_follows_the_context() {
    let (mut state, _) = app();
    let names = |s: &State| key_bar(s).iter().map(|b| format!("{} {}", b.keys, b.desc)).collect::<Vec<_>>();
    assert!(names(&state).contains(&"s start session".to_string()));
    assert!(names(&state).contains(&"o open on GitHub".to_string()));
    assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    assert!(names(&state).contains(&"a attach".to_string()));
    assert!(names(&state).contains(&"x stop".to_string()));
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Exited));
    assert!(names(&state).contains(&"a resume".to_string()));
    press(&mut state, key('2'));
    assert!(names(&state).contains(&"enter go to issue".to_string()));
    assert_eq!(
        names(&state),
        ["? keys", "enter go to issue", "a resume", "D remove", "m merge", "o open on GitHub", "q quit"]
    );
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

#[test]
fn every_bound_key_round_trips_through_its_key_event() {
    for key in BINDINGS.iter().flat_map(|b| b.keys) {
        assert_eq!(key_of(&key.event()), Some(*key), "{}", key.label());
    }
}

#[test]
fn every_key_bar_entry_carries_the_first_key_of_its_binding() {
    let (mut state, _) = app();
    let bar_keys = |state: &State| key_bar(state).iter().map(|b| (b.keys, b.key)).collect::<Vec<_>>();
    assert_eq!(
        bar_keys(&state),
        [
            ("?", Key::Char('?')),
            ("s", Key::Char('s')),
            ("space", Key::Char(' ')),
            ("m", Key::Char('m')),
            ("o", Key::Char('o')),
            ("q", Key::Char('q'))
        ]
    );
    press(&mut state, key('2'));
    press(&mut state, key('j'));
    assert!(bar_keys(&state).contains(&("enter", Key::Code(KeyCode::Enter))));
}

#[test]
fn clicking_a_key_does_what_pressing_it_does() {
    let (mut state, now) = with_target(HitTarget::Key(Key::Char('?')));
    press(&mut state, click(3, 0, now));
    assert!(matches!(state.focus, Focus::Keybinds { .. }), "the ? key bar entry opens the keys menu");

    let (mut state, now) = with_target(HitTarget::Key(Key::Char('q')));
    assert!(matches!(press(&mut state, click(0, 0, now)).as_slice(), [Cmd::Quit]));

    let (mut pressed, _) = app();
    press(&mut pressed, key('S'));
    let (mut state, now) = with_target(HitTarget::Key(Key::Char('S')));
    press(&mut state, click(0, 0, now));
    assert_eq!(
        state.tabs.sort_label(Tab::Issues),
        pressed.tabs.sort_label(Tab::Issues),
        "the sort footer cycles the sort"
    );
    assert_ne!(state.tabs.sort_label(Tab::Issues), app().0.tabs.sort_label(Tab::Issues));
}

#[test]
fn clicking_a_keybinds_entry_closes_the_menu_and_runs_its_action() {
    let (mut state, now) = app();
    press(&mut state, key('?'));
    state.hits.targets = vec![(Rect::new(0, 0, 10, 1), HitTarget::Binding(Action::Settings))];
    press(&mut state, click(1, 0, now));
    assert!(matches!(state.focus, Focus::Settings(_)), "the , entry opens settings");

    let (mut state, now) = app();
    press(&mut state, key('?'));
    state.hits.targets = vec![(Rect::new(0, 0, 10, 1), HitTarget::Binding(Action::Refresh))];
    let cmds = press(&mut state, click(1, 0, now));
    assert!(matches!(state.focus, Focus::List));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))), "the r entry refreshes");
}
