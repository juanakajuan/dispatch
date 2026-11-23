use super::keys::{Key, key_of};
use super::list::current_session;
use super::msg::{Cmd, Job, SendKind, WriteJob};
use super::toast;
use super::{Focus, State};
use crate::domain::SessionKey;
use crate::session::error::SessionError;
use crossterm::event::{KeyCode, KeyEvent};

/// `t`: open the message field for the selected live session. The field keeps
/// the key, so a list that changes under it can not move the message.
pub(super) fn open_message(state: &mut State) -> Vec<Cmd> {
    match current_session(state).filter(|s| s.state.is_live()) {
        Some(session) => state.focus = Focus::Message { key: session.key, text: String::new() },
        None => toast::info(state, "No running session here.".to_string()),
    }
    Vec::new()
}

pub(super) fn message_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Message { key, text } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Code(KeyCode::Enter)) => {
            let (key, text) = (*key, text.trim().to_string());
            if text.is_empty() {
                toast::info(state, "Type a message first.".to_string());
                return Vec::new();
            }
            state.focus = Focus::List;
            toast::info(state, format!("Sending to {key}…"));
            return vec![Cmd::Job(Job::Write(WriteJob::Send { key, kind: SendKind::Message, text }))];
        }
        Some(Key::Code(KeyCode::Esc)) => state.focus = Focus::List,
        Some(Key::Code(KeyCode::Backspace)) => {
            text.pop();
        }
        Some(Key::Ctrl('c')) => return vec![Cmd::Quit],
        Some(Key::Char(c)) => text.push(c),
        _ => {}
    }
    Vec::new()
}

pub(super) fn sent(state: &mut State, key: SessionKey, kind: SendKind, result: Result<(), SessionError>) -> Vec<Cmd> {
    match (kind, result) {
        (SendKind::MergeFix, Ok(())) => toast::info(state, format!("Asked {key}'s agent to fix and merge its PR.")),
        (SendKind::MergeFix, Err(e)) => {
            toast::error(state, toast::failed(format!("Sending the merge request to {key}"), e));
        }
        (SendKind::Message, Ok(())) => toast::info(state, format!("Sent your message to {key}.")),
        (SendKind::Message, Err(e)) => toast::error(state, toast::failed(format!("Sending the message to {key}"), e)),
    }
    Vec::new()
}
