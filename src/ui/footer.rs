use super::header::feed_failure;
use super::palette;
use super::text::{cells, one_line, row_line, span, spans_width};
use crate::app::list::Tab;
use crate::app::{self, Focus, State};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The one place for messages: the toast, or else the error of the shown
/// feed. Drawn on the message row above the prompt.
pub(super) fn message(state: &State) -> Option<(String, Style)> {
    if let Some(toast) = &state.toast {
        let style = if toast.text.contains("failed") { palette::ERROR } else { palette::PLAIN };
        return Some((one_line(&toast.text), style));
    }
    let failure = match state.tab {
        Tab::Issues => feed_failure(&state.issues),
        Tab::Prs => feed_failure(&state.prs),
        Tab::Sessions => None,
    };
    failure.map(|text| (text, palette::ERROR))
}

pub(super) fn draw_prompt(frame: &mut Frame, area: Rect, state: &State) {
    let mut spans = vec![span("❯ ", palette::ACCENT)];
    match (&state.focus, state.filter.is_empty()) {
        (Focus::Filter, _) => {
            spans.push(Span::raw(state.filter.clone()));
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
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Hints that stay when the footer is too narrow for all of them.
const KEPT_HINTS: [&str; 2] = ["?", "q"];

fn hints_width(hints: &[(&str, &str)]) -> usize {
    let words: usize = hints.iter().map(|(k, d)| cells(k) + " to ".len() + cells(d)).sum();
    words + " · ".chars().count() * hints.len().saturating_sub(1)
}

/// Drops whole hints from the right until the rest fit in `room` cells. A
/// kept hint is never dropped, so the result can still be wider than `room`.
fn fit_hints(mut hints: Vec<(&'static str, &'static str)>, room: usize) -> Vec<(&'static str, &'static str)> {
    while hints_width(&hints) > room {
        let Some(last) = hints.iter().rposition(|(k, _)| !KEPT_HINTS.contains(k)) else { break };
        hints.remove(last);
    }
    hints
}

pub(super) fn draw_footer(frame: &mut Frame, area: Rect, state: &State) {
    let width = usize::from(area.width);
    let mut sort =
        vec![span("S", palette::ACCENT), span(format!(" sort: {}", state.sort.label(state.tab)), palette::DIM)];
    let hints = app::keys::footer_hints(state);
    let mut shown = fit_hints(hints.clone(), width.saturating_sub(spans_width(&sort) + 1));
    if hints_width(&shown) + spans_width(&sort) + 1 > width {
        sort.clear();
        shown = fit_hints(hints, width);
    }
    let mut spans = vec![Span::raw("")];
    for (i, (k, d)) in shown.into_iter().enumerate() {
        if i > 0 {
            spans.push(span(" · ", palette::DIM));
        }
        spans.push(span(k.to_string(), palette::ACCENT));
        spans.push(span(format!(" to {d}"), palette::DIM));
    }
    frame.render_widget(Paragraph::new(row_line(area.width, spans, sort)), area);
}
