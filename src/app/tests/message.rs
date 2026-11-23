use super::*;

fn typed(state: &mut State, text: &str) {
    for c in text.chars() {
        press(state, key(c));
    }
}

fn sends_of(cmds: &[Cmd]) -> Vec<(SessionKey, SendKind, &str)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Send { key, kind, text })) => Some((*key, *kind, text.as_str())),
            _ => None,
        })
        .collect()
}

#[test]
fn t_on_a_row_with_no_running_session_refuses() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('t')).is_empty());
    assert_eq!(toast(&state), "No running session here.", "no session");
    assert!(matches!(state.focus, Focus::List));

    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    press(&mut state, key('t'));
    assert_eq!(toast(&state), "No running session here.", "an exited session");
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn the_key_bar_offers_t_only_for_a_live_session() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert!(!shows(&state, "t", "send message"));
    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    assert!(shows(&state, "t", "send message"));
}

#[test]
fn enter_sends_the_typed_line_to_the_session_and_closes_the_field() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    press(&mut state, key('t'));
    typed(&mut state, "yes, use plan Bx");
    press(&mut state, code(KeyCode::Backspace));
    assert!(
        matches!(&state.focus, Focus::Message { key: SessionKey::Issue(IssueNumber(3)), text } if text == "yes, use plan B"),
        "list keys type into the field, and backspace deletes"
    );
    assert_eq!(
        footer_hints(&state).iter().map(|h| (h.keys.as_str(), h.desc)).collect::<Vec<_>>(),
        [("enter", "send"), ("esc", "cancel")]
    );

    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(sends_of(&cmds), [(issue_key(3), SendKind::Message, "yes, use plan B")]);
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(toast(&state), "Sending to #3");
}

#[test]
fn the_message_goes_to_the_session_it_was_opened_on() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('t'));
    typed(&mut state, "go");
    state.tabs.issues.selected = Some(IssueNumber(2));
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(sends_of(&cmds), [(issue_key(3), SendKind::Message, "go")]);
}

#[test]
fn enter_on_a_blank_message_keeps_the_field_open() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    press(&mut state, key('t'));
    typed(&mut state, "  ");
    assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
    assert_eq!(toast(&state), "Type a message first.");
    assert!(matches!(state.focus, Focus::Message { .. }));
}

#[test]
fn esc_cancels_without_a_send() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    press(&mut state, key('t'));
    typed(&mut state, "never mind");
    assert!(press(&mut state, code(KeyCode::Esc)).is_empty());
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn the_send_result_shows_a_toast() {
    let (mut state, _) = app();
    press(&mut state, Msg::Sent { key: issue_key(3), kind: SendKind::Message, result: Ok(()) });
    assert_eq!(toast(&state), "Sent your message to #3");
    assert_eq!(state.toast.as_ref().map(Toast::level), Some(Level::Info));

    let result = Err(SessionError::NotRunning(issue_key(3)));
    press(&mut state, Msg::Sent { key: issue_key(3), kind: SendKind::Message, result });
    assert_eq!(toast(&state), "Message to #3 failed: #3 has no running session");
    assert_eq!(state.toast.as_ref().map(Toast::level), Some(Level::Error));
}
