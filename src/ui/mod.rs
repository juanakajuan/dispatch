//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: the header (tab row, logo, repo name
//! with the age of the shown list, session counts), the grouped rows (one
//! line each), the message row (only when there is a message), the prompt
//! between two rules, and the footer keys. The dialog is a centered overlay.

mod batch;
mod dialog;
mod footer;
mod header;
mod markdown;
mod menus;
mod palette;
mod rows;
pub(crate) mod text;

use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{Focus, State};
use batch::draw_batch;
use dialog::draw_dialog;
use footer::{draw_footer, draw_prompt, message};
use header::draw_header;
use menus::{draw_keybinds, draw_settings, draw_summary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Color;
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
    draw_prompt(frame, prompt, state, &mut hits);
    if !matches!(state.focus, Focus::List | Focus::Filter) {
        // An overlay owns the mouse: only its own targets and the footer stay live.
        hits.targets.clear();
    }
    draw_footer(frame, footer, state, &mut hits);
    draw_overlay(frame, state, &mut hits);
    draw_hover(frame, &hits, state.pointer);
    hits
}

/// Tint the target under the pointer, so the user sees what a click hits.
/// A text input shows its own cursor instead, and a tab badge keeps its color.
fn draw_hover(frame: &mut Frame, hits: &HitMap, pointer: Option<Position>) {
    let Some((rect, target)) = pointer.and_then(|p| hits.at(p.x, p.y)) else { return };
    if matches!(target, HitTarget::Text { .. }) {
        return;
    }
    let buf = frame.buffer_mut();
    for pos in rect.intersection(buf.area).positions() {
        let cell = &mut buf[pos];
        if cell.bg == Color::Reset {
            cell.set_style(palette::HOVER);
        }
    }
}

/// The dialog or a menu, over the screen of either view.
fn draw_overlay(frame: &mut Frame, state: &State, hits: &mut HitMap) {
    match &state.focus {
        Focus::Dialog(dialog) => draw_dialog(frame, frame.area(), state, dialog, hits),
        Focus::Batch(batch) => draw_batch(frame, frame.area(), state, batch, hits),
        Focus::Settings(menu) => draw_settings(frame, frame.area(), state, menu, hits),
        Focus::Keybinds { scroll } => hits.max_scroll = draw_keybinds(frame, frame.area(), *scroll, hits),
        Focus::Summary { issue, scroll } => {
            hits.max_scroll = draw_summary(frame, frame.area(), state, *issue, *scroll, hits);
        }
        Focus::List | Focus::Filter | Focus::Message { .. } => {}
    }
}

#[cfg(test)]
mod tests;
