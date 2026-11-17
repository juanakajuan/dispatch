use super::dialog::{Focus, draw_buttons, draw_mode_options, field_label};
use super::palette;
use super::text::{fit, span};
use crate::app::State;
use crate::app::batch::BatchStart;
use crate::app::mouse::HitMap;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

/// Rows that are not issue lines: label, blank, mode label and options, blank, note, blank, buttons.
const FIXED_ROWS: u16 = 8;

pub(super) fn draw_batch(frame: &mut Frame, screen: Rect, state: &State, batch: &BatchStart, hits: &mut HitMap) {
    let count = u16::try_from(batch.issues.len()).unwrap_or(u16::MAX);
    let width = screen.width.min(96);
    // Borders and padding take 3 rows.
    let height = count.saturating_add(FIXED_ROWS + 3).min(screen.height.min(28));
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(format!(" Start {} sessions ", batch.issues.len()), palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let room = inner.height.saturating_sub(FIXED_ROWS);
    let [label, list, _, mode_l, mode_options, _, note, _, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(room),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(field_label("Issues", Focus::Off)), label);
    frame.render_widget(Paragraph::new(issue_lines(batch, usize::from(room), usize::from(list.width))), list);
    frame.render_widget(Paragraph::new(field_label("Mode", Focus::On)), mode_l);
    draw_mode_options(frame, state, batch.mode, mode_options, hits);
    frame.render_widget(Paragraph::new(span("  Each session uses its default branch and prompt.", palette::DIM)), note);
    draw_buttons(frame, buttons, hits);
}

/// One line per issue. When they do not all fit, the last line says how many are left out.
fn issue_lines(batch: &BatchStart, room: usize, width: usize) -> Vec<Line<'static>> {
    let line = |n: &crate::domain::IssueNumber, title: &str| {
        Line::from(span(fit(&format!("  {n} {title}"), width), palette::PLAIN))
    };
    if batch.issues.len() <= room {
        return batch.issues.iter().map(|(n, t)| line(n, t)).collect();
    }
    let shown = room.saturating_sub(1);
    let mut lines: Vec<Line<'static>> = batch.issues.iter().take(shown).map(|(n, t)| line(n, t)).collect();
    if room > 0 {
        lines.push(Line::from(span(format!("  … and {} more", batch.issues.len() - shown), palette::DIM)));
    }
    lines
}
