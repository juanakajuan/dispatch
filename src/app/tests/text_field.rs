use super::*;
use crate::app::text_field::TextField;

/// Opens one text field, empty, and reads its text and cursor.
struct Field {
    name: &'static str,
    open: fn(&mut State),
    read: fn(&State) -> (String, usize),
}

fn on(field: &TextField) -> (String, usize) {
    (field.text().to_string(), field.cursor())
}

fn fields() -> [Field; 4] {
    [
        Field {
            name: "filter",
            open: |state| {
                press(state, key('/'));
            },
            read: |state| on(state.tabs.filter_field(state.tab)),
        },
        Field {
            name: "message",
            open: |state| {
                state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
                press(state, key('t'));
            },
            read: |state| match &state.focus {
                Focus::Message { text, .. } => on(text),
                _ => panic!("the message field is closed"),
            },
        },
        Field {
            name: "settings command",
            open: |state| {
                press(state, key(','));
                while !matches!(&state.focus, Focus::Settings(m) if m.at == Setting::Command) {
                    press(state, key('j'));
                }
                press(state, code(KeyCode::Enter));
                press(state, ctrl('j'));
            },
            read: |state| match &state.focus {
                Focus::Settings(m) => on(&m.edit.as_ref().expect("the editor is open").input),
                _ => panic!("the settings menu is closed"),
            },
        },
        Field {
            name: "start dialog branch",
            open: |state| {
                press(state, key('s'));
                press(state, ctrl('j'));
            },
            read: |state| {
                let branch = &dialog(state).branch;
                (branch.lines().join(""), branch.cursor().1)
            },
        },
    ]
}

fn typed(state: &mut State, text: &str) {
    for c in text.chars() {
        press(state, key(c));
    }
}

fn times(state: &mut State, n: usize, code: KeyCode) {
    for _ in 0..n {
        press(state, super::code(code));
    }
}

#[test]
fn every_text_field_moves_types_and_deletes_with_the_same_keys() {
    for field in fields() {
        let (mut state, _) = app();
        (field.open)(&mut state);
        assert_eq!((field.read)(&state), (String::new(), 0), "{} opens empty", field.name);

        typed(&mut state, "fix teh typo");
        times(&mut state, 7, KeyCode::Left);
        typed(&mut state, "X");
        assert_eq!((field.read)(&state), ("fix tXeh typo".to_string(), 6), "{}: left, then type", field.name);

        press(&mut state, code(KeyCode::Home));
        typed(&mut state, "A");
        assert_eq!((field.read)(&state), ("Afix tXeh typo".to_string(), 1), "{}: home, then type", field.name);

        press(&mut state, code(KeyCode::Right));
        press(&mut state, code(KeyCode::Delete));
        assert_eq!((field.read)(&state), ("Afx tXeh typo".to_string(), 2), "{}: delete", field.name);

        press(&mut state, code(KeyCode::End));
        press(&mut state, ctrl('w'));
        assert_eq!((field.read)(&state), ("Afx tXeh ".to_string(), 9), "{}: end, then ctrl+w", field.name);

        press(&mut state, ctrl('u'));
        assert_eq!((field.read)(&state).0, "Afx tXeh typo", "{}: ctrl+u undoes", field.name);
    }
}

#[test]
fn enter_never_breaks_a_one_line_field() {
    let mut field = TextField::new("ab");
    press_on(&mut field, KeyCode::Left, KeyModifiers::NONE);
    for (code, modifiers) in [
        (KeyCode::Enter, KeyModifiers::NONE),
        (KeyCode::Char('m'), KeyModifiers::CONTROL),
        (KeyCode::Char('\n'), KeyModifiers::NONE),
        (KeyCode::Char('\r'), KeyModifiers::NONE),
    ] {
        assert!(!press_on(&mut field, code, modifiers), "{code:?} {modifiers:?}");
        assert_eq!(on(&field), ("ab".to_string(), 1));
    }
}

#[test]
fn a_set_text_puts_the_cursor_after_it() {
    let mut field = TextField::new("claude");
    assert_eq!(on(&field), ("claude".to_string(), 6));
    press_on(&mut field, KeyCode::Home, KeyModifiers::NONE);
    field.set("label:bug");
    assert_eq!(on(&field), ("label:bug".to_string(), 9));
    field.clear();
    assert_eq!(on(&field), (String::new(), 0));
}

fn press_on(field: &mut TextField, code: KeyCode, modifiers: KeyModifiers) -> bool {
    field.edit(&KeyEvent::new(code, modifiers))
}

#[test]
fn tab_completes_the_last_word_and_puts_the_cursor_after_it() {
    let (mut state, _) = app();
    press(&mut state, key('/'));
    typed(&mut state, "retry ha");
    times(&mut state, 3, KeyCode::Left);
    press(&mut state, code(KeyCode::Tab));
    assert_eq!(on(state.tabs.filter_field(state.tab)), ("retry has:".to_string(), 10));
}

#[test]
fn a_saved_filter_puts_the_cursor_after_it() {
    let (mut state, _) = app();
    state.config.saved_filters = vec!["label:bug".to_string()];
    press(&mut state, key('/'));
    press(&mut state, code(KeyCode::Down));
    assert_eq!(on(state.tabs.filter_field(state.tab)), ("label:bug".to_string(), 9));
}
