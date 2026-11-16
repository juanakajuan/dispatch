use super::header::feed_failure;
use super::palette;
use super::text::{one_line, row_line, span};
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

pub(super) fn draw_footer(frame: &mut Frame, area: Rect, state: &State) {
    let pairs: Vec<(&str, &str)> = match &state.focus {
        Focus::Filter => vec![("enter", "keep"), ("esc", "clear")],
        Focus::Dialog(_) => vec![("ctrl-s", "start"), ("tab", "next field"), ("esc", "cancel")],
        Focus::Settings(menu) if menu.edit.is_some() => vec![("enter", "save"), ("esc", "cancel")],
        Focus::Settings(_) => vec![("j/k", "move"), ("enter", "change"), ("esc", "close")],
        Focus::Keybinds { .. } => vec![("j/k", "scroll"), ("esc", "close")],
        Focus::List => app::keys::key_bar(state),
    };
    let mut spans = vec![Span::raw("")];
    for (i, (k, d)) in pairs.into_iter().enumerate() {
        if i > 0 {
            spans.push(span(" · ", palette::DIM));
        }
        spans.push(span(k.to_string(), palette::ACCENT));
        spans.push(span(format!(" to {d}"), palette::DIM));
    }
    let sort = vec![span("S", palette::ACCENT), span(format!(" sort: {}", state.sort.label(state.tab)), palette::DIM)];
    frame.render_widget(Paragraph::new(row_line(area.width, spans, sort)), area);
}
