//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: the header (tab row, logo, repo name
//! with the age of the shown list, session counts), the grouped rows (one
//! line each), the message row (only when there is a message), the prompt
//! between two rules, and the footer keys. The dialog is a centered overlay.

mod dialog;
mod footer;
mod header;
mod menus;
mod palette;
mod rows;
pub(crate) mod text;

use crate::app::mouse::HitMap;
use crate::app::{Focus, State};
use dialog::draw_dialog;
use footer::{draw_footer, draw_prompt, message};
use header::draw_header;
use menus::{draw_keybinds, draw_settings};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use rows::draw_rows;
use text::{fit, span};

pub fn draw(frame: &mut Frame, state: &State) -> HitMap {
    let now = jiff::Timestamp::now();
    let full = frame.area();
    let icons = state.config.icons.icons();
    frame.buffer_mut().set_style(full, palette::PLAIN);
    let screen = Rect {
        x: full.x + 2,
        y: full.y + 1,
        width: full.width.saturating_sub(4),
        height: full.height.saturating_sub(1),
    };
    let message = message(state);
    let [header, _, body, msg, rule1, prompt, rule2, footer] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(u16::from(message.is_some())),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(screen);
    let mut hits = HitMap::default();
    draw_header(frame, header, state, icons, &mut hits);
    draw_rows(frame, body, state, now, icons, &mut hits);
    if let Some((text, style)) = message {
        frame.render_widget(Paragraph::new(span(format!(" {}", fit(&text, usize::from(msg.width))), style)), msg);
    }
    let rule = || Paragraph::new(span("─".repeat(usize::from(screen.width)), palette::DIM));
    frame.render_widget(rule(), rule1);
    frame.render_widget(rule(), rule2);
    draw_prompt(frame, prompt, state);
    draw_footer(frame, footer, state);
    draw_overlay(frame, state, &mut hits);
    hits
}

/// The dialog or a menu, over the screen of either view.
fn draw_overlay(frame: &mut Frame, state: &State, hits: &mut HitMap) {
    match &state.focus {
        Focus::Dialog(dialog) => draw_dialog(frame, frame.area(), state, dialog),
        Focus::Settings(menu) => draw_settings(frame, frame.area(), state, menu),
        Focus::Keybinds { scroll } => hits.keybinds_max_scroll = draw_keybinds(frame, frame.area(), *scroll),
        Focus::List | Focus::Filter => {}
    }
}

#[cfg(test)]
mod tests;
