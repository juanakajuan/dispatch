//! A one-line text field: the filter, the message to a session, and a typed
//! setting. Keys go to the same `tui_textarea` editor as the start dialog's
//! fields, so every field moves, deletes, and undoes with the same keys.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::{CursorMove, TextArea};

#[derive(Clone, Debug, Default)]
pub struct TextField(TextArea<'static>);

impl TextField {
    /// A field that holds `text`, with the cursor after it.
    pub fn new(text: &str) -> TextField {
        let mut area = TextArea::new(vec![text.to_string()]);
        area.move_cursor(CursorMove::End);
        TextField(area)
    }

    pub fn text(&self) -> &str {
        self.0.lines().first().map_or("", String::as_str)
    }

    /// The cursor, in chars from the start of the text.
    pub fn cursor(&self) -> usize {
        self.0.cursor().1
    }

    /// Puts `text` in place of the text, with the cursor after it.
    pub fn set(&mut self, text: &str) {
        *self = TextField::new(text);
    }

    pub fn clear(&mut self) {
        self.set("");
    }

    /// Edits the text with one key, as the start dialog's fields do. A key
    /// that would break the line does nothing. Returns whether the text
    /// changed.
    pub fn edit(&mut self, ev: &KeyEvent) -> bool {
        if breaks_line(ev) {
            return false;
        }
        self.0.input(*ev)
    }
}

fn breaks_line(ev: &KeyEvent) -> bool {
    let ctrl_m = ev.code == KeyCode::Char('m') && ev.modifiers.contains(KeyModifiers::CONTROL);
    ctrl_m || matches!(ev.code, KeyCode::Enter | KeyCode::Char('\n' | '\r'))
}
