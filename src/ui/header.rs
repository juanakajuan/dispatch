use super::palette;
use super::text::{centered, one_line, row_line, span, spans_width};
use crate::app::feed::LastFetch;
use crate::app::list::Tab;
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, State};
use crate::glyphs::Icons;
use crate::session::SessionState;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::time::Instant;

pub(super) const LOGO: [&str; 3] = [" ▄██▄ ", "██▀▀██", "▀▘  ▝▀"];

/// The age of the list on the current tab, counted from when the last
/// response arrived. A failed fetch keeps the old items, so it gets its own
/// warning text: stale data never reads as fresh.
pub(super) fn freshness(state: &State, now: Instant) -> Option<(String, Style)> {
    let (last, refreshing) = match state.tab {
        Tab::Issues => (&state.issues.last, state.issues.poll.in_flight()),
        Tab::Prs => (&state.prs.last, state.prs.poll.in_flight()),
        Tab::Sessions => return None,
    };
    Some(match last {
        LastFetch::Never => ("loading…".to_string(), palette::DIM),
        _ if refreshing => ("refreshing…".to_string(), palette::DIM),
        LastFetch::Fresh { at } => (format!("updated {} ago", since(*at, now)), palette::DIM),
        LastFetch::Failed { at, .. } => (format!("refresh failed {} ago", since(*at, now)), palette::WARN),
    })
}

/// "12s", "3m", "2h". Seconds count up so the user sees the clock move.
pub(super) fn since(at: Instant, now: Instant) -> String {
    let secs = now.saturating_duration_since(at).as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

pub(super) fn feed_failure<T>(feed: &app::feed::Feed<T>) -> Option<String> {
    let LastFetch::Failed { error, .. } = &feed.last else { return None };
    let kept = if feed.items.is_empty() { "" } else { ", showing old data" };
    Some(format!("gh failed{kept}: {}", one_line(&error.to_string())))
}

pub(super) fn draw_header(frame: &mut Frame, area: Rect, state: &State, icons: &Icons, hits: &mut HitMap) {
    let [tab_row, _, brand] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(3)]).areas(area);
    let [logo, _, text] =
        Layout::horizontal([Constraint::Length(6), Constraint::Length(4), Constraint::Min(1)]).areas(brand);
    frame.render_widget(
        Paragraph::new(LOGO.iter().map(|l| Line::from(span(*l, palette::ACCENT))).collect::<Vec<_>>()),
        logo,
    );
    let first =
        vec![span("dispatch", palette::HEADING), span(format!(" v{}", env!("CARGO_PKG_VERSION")), palette::DIM)];
    let mut second = vec![span(state.context.repo_name.clone(), palette::PLAIN)];
    if let Some((text, style)) = freshness(state, Instant::now()) {
        second.push(span(format!(" · {text}"), style));
    }

    let mut tabs: Vec<Span<'static>> = Vec::new();
    let mut spans_at: Vec<(Tab, u16, u16)> = Vec::new();
    let mut x: u16 = 0;
    for (tab, key, name, count) in [
        (Tab::Issues, "1", "Issues", state.issues.items.len()),
        (Tab::Prs, "2", "Pull requests", state.prs.items.len()),
        (Tab::Sessions, "3", "Sessions", state.sessions.len()),
    ] {
        if !tabs.is_empty() {
            tabs.push(Span::raw("     "));
            x = x.saturating_add(5);
        }
        let on = tab == state.tab;
        let label = vec![
            span(format!(" {key} "), if on { palette::TAB_KEY_ON } else { palette::TAB_KEY_OFF }),
            Span::raw(" "),
            span(name, if on { palette::ACTIVE } else { palette::DIM }),
            span(format!(" {count}"), if on { palette::PLAIN } else { palette::DIM }),
        ];
        let w = u16::try_from(spans_width(&label)).unwrap_or(u16::MAX);
        spans_at.push((tab, x, w));
        x = x.saturating_add(w);
        tabs.extend(label);
    }
    let left = tab_row.x + tab_row.width.saturating_sub(x) / 2;
    for (tab, at, w) in spans_at {
        hits.targets.push((Rect::new(left + at, tab_row.y, w, 1).intersection(tab_row), HitTarget::Tab(tab)));
    }
    frame.render_widget(Paragraph::new(centered(tabs, tab_row.width)), tab_row);

    let count = |want: SessionState| state.sessions.values().filter(|s| s.state == want).count();
    let needs = count(SessionState::NeedsYou);
    let working = count(SessionState::Running) + state.starting.len();
    let exited = count(SessionState::Exited);
    let needs_style = if needs > 0 { palette::NEEDS_YOU } else { palette::DIM };
    let lit = |n: usize, style| if n > 0 { style } else { palette::DIM };
    let third = vec![
        span(format!("{} ", icons.needs_you), lit(needs, palette::NEEDS_YOU)),
        span(format!("{needs} needs you"), needs_style),
        span(format!("   {} ", icons.running), lit(working, palette::RUNNING)),
        span(format!("{working} working"), palette::DIM),
        span(format!("   {} ", icons.exited), lit(exited, palette::EXITED)),
        span(format!("{exited} exited"), palette::DIM),
    ];

    frame.render_widget(
        Paragraph::new(vec![
            row_line(text.width, first, Vec::new()),
            row_line(text.width, second, Vec::new()),
            row_line(text.width, third, Vec::new()),
        ]),
        text,
    );
}
