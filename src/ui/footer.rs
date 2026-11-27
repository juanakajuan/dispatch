use super::header::feed_failure;
use super::palette;
use super::text::{cells, fit, one_line, row_line, span, spans_width, wrap};
use crate::app::filter::{Term, TokenKind, Viewer, Who, tokens};
use crate::app::keys::{Hint, Key};
use crate::app::list::Tab;
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::toast::Level;
use crate::app::{self, Focus, State};
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::ops::Range;

/// The one place for messages: the toast, or else the error of the shown
/// feed. Drawn on the message row above the prompt.
pub(super) fn message(state: &State) -> Option<(String, Style)> {
    if let Some(toast) = &state.toast {
        let style = match toast.level() {
            Level::Error => palette::ERROR,
            Level::Info => palette::PLAIN,
        };
        return Some((one_line(toast.text()), style));
    }
    let failure = match state.tab {
        Tab::Issues => feed_failure(&state.issues),
        Tab::Prs => feed_failure(&state.prs),
        Tab::Sessions => None,
    };
    failure.map(|text| (text, palette::ERROR))
}

/// The most rows a message takes. A message longer than this ends in `…`.
pub(super) const MESSAGE_ROWS: usize = 4;

/// `text` word-wrapped to `width` cells, each row after a one-cell margin,
/// in at most `MESSAGE_ROWS` rows.
pub(super) fn message_rows(text: &str, width: usize) -> Vec<String> {
    let room = width.saturating_sub(1);
    let mut rows = wrap(text, room);
    if rows.len() > MESSAGE_ROWS {
        let rest = rows.split_off(MESSAGE_ROWS - 1).join(" ");
        rows.push(fit(&rest, room));
    }
    rows.into_iter().map(|row| format!(" {row}")).collect()
}

const CLEAR_FILTER: &str = "esc to clear";

/// The prompt opens the filter on a click, and its `esc to clear` clears it.
pub(super) fn draw_prompt(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let mut spans = vec![span("❯ ", palette::ACCENT)];
    match (&state.focus, state.filter().is_empty()) {
        (Focus::Filter, _) => {
            spans.extend(filter_spans(state, Style::default()));
            spans.push(span("▏", palette::ACCENT));
        }
        (Focus::Message { key, text }, _) => {
            spans.push(span(format!("message to {key}: "), palette::LABEL));
            spans.push(Span::raw(text.clone()));
            spans.push(span("▏", palette::ACCENT));
        }
        (_, false) => {
            spans.push(span("/", palette::LABEL));
            spans.extend(filter_spans(state, palette::LABEL));
        }
        (Focus::List, true) => spans.push(span(app::keys::prompt_hint(state), palette::DIM)),
        (_, true) => {}
    }
    let used = spans_width(&spans);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if matches!(state.focus, Focus::List) {
        hits.targets.push((area, HitTarget::Key(Key::Char('/'))));
    }
    let clear_w = cells(CLEAR_FILTER);
    let clears = matches!(state.focus, Focus::List | Focus::Filter);
    if clears && !state.filter().is_empty() && clear_w <= usize::from(area.width).saturating_sub(used + 1) {
        let clear = cell_rect(area, usize::from(area.width) - clear_w, clear_w);
        frame.render_widget(Paragraph::new(span(CLEAR_FILTER, palette::DIM)), clear);
        hits.targets.push((clear, HitTarget::Key(Key::Code(KeyCode::Esc))));
    }
}

/// The filter as typed, with each term in the accent colour, so the user sees
/// which words filter by a field, with its `-` if it has one. A term key with
/// no value yet is dim, a value outside the term's set is red, and `@me` is
/// red when dispatch could not read the user's login.
fn filter_spans(state: &State, text: Style) -> Vec<Span<'static>> {
    let raw = state.filter();
    let mut spans = Vec::new();
    let mut at = 0;
    for t in tokens(raw) {
        if at < t.at.start {
            spans.push(span(raw[at..t.at.start].to_string(), text));
        }
        let style = match &t.kind {
            TokenKind::Text => text,
            TokenKind::Incomplete => palette::DIM,
            TokenKind::Invalid => palette::ERROR,
            TokenKind::Term(_, Term::Assignee(Who::Me) | Term::Author(Who::Me)) if state.viewer == Viewer::Failed => {
                palette::ERROR
            }
            TokenKind::Term(..) => palette::ACCENT,
        };
        spans.push(span(raw[t.at.clone()].to_string(), style));
        at = t.at.end;
    }
    if at < raw.len() {
        spans.push(span(raw[at..].to_string(), text));
    }
    spans
}

/// Hints that stay when the footer is too narrow for all of them.
const KEPT_HINTS: [Key; 2] = [Key::Char('?'), Key::Char('q')];

fn hints_width(hints: &[Hint]) -> usize {
    let words: usize = hints.iter().map(|h| cells(&h.keys) + " to ".len() + cells(h.desc)).sum();
    words + " · ".chars().count() * hints.len().saturating_sub(1)
}

/// Drops whole hints from the right until the rest fit in `room` cells. A
/// kept hint is never dropped, so the result can still be wider than `room`.
fn fit_hints(mut hints: Vec<Hint>, room: usize) -> Vec<Hint> {
    while hints_width(&hints) > room {
        let Some(last) = hints.iter().rposition(|h| !KEPT_HINTS.contains(&h.key)) else { break };
        hints.remove(last);
    }
    hints
}

/// Each hint drawn whole is a click target for its key. A dropped hint has no
/// target, and neither has a kept hint that `row_line` had to cut.
pub(super) fn draw_footer(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let width = usize::from(area.width);
    let mut sort = if matches!(state.focus, Focus::List) {
        vec![span("S", palette::ACCENT), span(format!(" sort: {}", state.tabs.sort_label(state.tab)), palette::DIM)]
    } else {
        Vec::new()
    };
    let offered = app::keys::footer_hints(state);
    let mut shown = fit_hints(offered.clone(), width.saturating_sub(spans_width(&sort) + 1));
    if hints_width(&shown) + spans_width(&sort) + 1 > width {
        sort.clear();
        shown = fit_hints(offered, width);
    }
    let mut spans = vec![Span::raw("")];
    let mut entries: Vec<(Range<usize>, Key)> = Vec::new();
    for (i, h) in shown.into_iter().enumerate() {
        if i > 0 {
            spans.push(span(" · ", palette::DIM));
        }
        let first = spans.len();
        spans.push(span(h.keys, palette::ACCENT));
        spans.push(span(format!(" to {}", h.desc), palette::DIM));
        entries.push((first..spans.len(), h.key));
    }
    let sort_w = spans_width(&sort);
    let line = row_line(area.width, spans.clone(), sort);
    let mut starts = vec![0usize];
    for s in &line.spans[..spans.len()] {
        starts.push(starts.last().copied().unwrap_or(0) + s.width());
    }
    for (range, key) in entries {
        if range.clone().all(|i| line.spans[i].content == spans[i].content) {
            let (from, to) = (starts[range.start], starts[range.end]);
            hits.targets.push((cell_rect(area, from, to - from), HitTarget::Key(key)));
        }
    }
    if sort_w > 0 && sort_w <= width {
        hits.targets.push((cell_rect(area, width - sort_w, sort_w), HitTarget::Key(Key::Char('S'))));
    }
    frame.render_widget(Paragraph::new(line), area);
}

/// `width` cells of `row`, starting `from` cells in.
fn cell_rect(row: Rect, from: usize, width: usize) -> Rect {
    let at = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
    Rect::new(row.x.saturating_add(at(from)), row.y, at(width), 1).intersection(row)
}
