use super::header::feed_failure;
use super::palette;
use super::text::{cells, one_line, row_line, span, spans_width};
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

const CLEAR_FILTER: &str = "esc to clear";

/// The prompt opens the filter on a click, and its `esc to clear` clears it.
pub(super) fn draw_prompt(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let mut spans = vec![span("❯ ", palette::ACCENT)];
    match (&state.focus, state.filter.is_empty()) {
        (Focus::Filter, _) => {
            spans.push(Span::raw(state.filter.clone()));
            spans.push(span("▏", palette::ACCENT));
        }
        (Focus::Message { issue, text }, _) => {
            spans.push(span(format!("message to {issue}: "), palette::LABEL));
            spans.push(Span::raw(text.clone()));
            spans.push(span("▏", palette::ACCENT));
        }
        (_, false) => spans.push(span(format!("/{}", state.filter), palette::LABEL)),
        (_, true) => spans.push(span(
            match state.tab {
                Tab::Issues => "/ to filter issues · s to start a session",
                Tab::Prs => "/ to filter pull requests",
                Tab::Sessions => "/ to filter sessions · enter to attach",
            },
            palette::DIM,
        )),
    }
    let used = spans_width(&spans);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if matches!(state.focus, Focus::List) {
        hits.targets.push((area, HitTarget::Key(Key::Char('/'))));
    }
    let clear_w = cells(CLEAR_FILTER);
    if !state.filter.is_empty() && clear_w <= usize::from(area.width).saturating_sub(used + 1) {
        let clear = cell_rect(area, usize::from(area.width) - clear_w, clear_w);
        frame.render_widget(Paragraph::new(span(CLEAR_FILTER, palette::DIM)), clear);
        hits.targets.push((clear, HitTarget::Key(Key::Code(KeyCode::Esc))));
    }
}

/// Hints that stay when the footer is too narrow for all of them.
const KEPT_HINTS: [&str; 2] = ["?", "q"];

fn hints_width(hints: &[Hint]) -> usize {
    let words: usize = hints.iter().map(|h| cells(h.keys) + " to ".len() + cells(h.desc)).sum();
    words + " · ".chars().count() * hints.len().saturating_sub(1)
}

/// Drops whole hints from the right until the rest fit in `room` cells. A
/// kept hint is never dropped, so the result can still be wider than `room`.
fn fit_hints(mut hints: Vec<Hint>, room: usize) -> Vec<Hint> {
    while hints_width(&hints) > room {
        let Some(last) = hints.iter().rposition(|h| !KEPT_HINTS.contains(&h.keys)) else { break };
        hints.remove(last);
    }
    hints
}

/// Each hint drawn whole is a click target for its key. A dropped hint has no
/// target, and neither has a kept hint that `row_line` had to cut.
pub(super) fn draw_footer(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let width = usize::from(area.width);
    let mut sort =
        vec![span("S", palette::ACCENT), span(format!(" sort: {}", state.tabs.sort_label(state.tab)), palette::DIM)];
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
