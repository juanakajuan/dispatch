//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: tab bar and its rule (2 rows),
//! list | detail (`state.split`, 42% | 58% by default), the message row
//! (1 to 3 rows, blank when there is no message), the centered key bar
//! (1 row). The right end of the tab bar shows how old the shown list is.
//! List rows are two lines tall. The dialog is a centered overlay.

use crate::app::{
    self, DialogField, Feed, FeedStatus, Focus, HitMap, HitTarget, RowKey, StartDialog, State, Tab,
};
use crate::config::Config;
use crate::markdown;
use crate::domain::{Checks, Issue, IssueNumber, Mode, PrState, PullRequest};
use crate::rules;
use crate::session::{Session, SessionState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};
use std::time::Instant;

/// The whole palette. No other module builds a `Style`.
/// Rule: Gruvbox (dark) foregrounds only; never a background color, so the
/// terminal's own background shows through. One accent (orange) marks what
/// is active: the tab, the selected row, the focused field, the keys. The
/// other colors carry state only.
pub mod palette {
    use ratatui::style::{Color, Modifier, Style};

    pub const FG: Color = Color::Rgb(0xeb, 0xdb, 0xb2);
    pub const GRAY: Color = Color::Rgb(0x92, 0x83, 0x74);
    pub const RED: Color = Color::Rgb(0xfb, 0x49, 0x34);
    pub const GREEN: Color = Color::Rgb(0xb8, 0xbb, 0x26);
    pub const YELLOW: Color = Color::Rgb(0xfa, 0xbd, 0x2f);
    pub const BLUE: Color = Color::Rgb(0x83, 0xa5, 0x98);
    pub const PURPLE: Color = Color::Rgb(0xd3, 0x86, 0x9b);
    pub const AQUA: Color = Color::Rgb(0x8e, 0xc0, 0x7c);
    pub const ORANGE: Color = Color::Rgb(0xfe, 0x80, 0x19);

    pub const PLAIN: Style = Style::new().fg(FG);
    pub const HEADING: Style = Style::new().fg(FG).add_modifier(Modifier::BOLD);
    pub const DIM: Style = Style::new().fg(GRAY);
    pub const ACCENT: Style = Style::new().fg(ORANGE);
    pub const ACTIVE: Style = Style::new().fg(ORANGE).add_modifier(Modifier::BOLD);
    pub const CURSOR: Style = Style::new().add_modifier(Modifier::REVERSED);
    pub const RUNNING: Style = Style::new().fg(GREEN);
    pub const NEEDS_YOU: Style = Style::new().fg(YELLOW).add_modifier(Modifier::BOLD);
    pub const EXITED: Style = Style::new().fg(GRAY);
    pub const STARTING: Style = Style::new().fg(AQUA);
    pub const PASS: Style = Style::new().fg(GREEN);
    pub const FAIL: Style = Style::new().fg(RED);
    pub const PENDING: Style = Style::new().fg(YELLOW);
    pub const MERGED: Style = Style::new().fg(PURPLE);
    pub const LABEL: Style = Style::new().fg(BLUE);
    pub const ERROR: Style = Style::new().fg(RED);
    pub const WARN: Style = Style::new().fg(YELLOW);
    pub const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
    pub const ITALIC: Style = Style::new().add_modifier(Modifier::ITALIC);
    pub const STRIKE: Style = Style::new().add_modifier(Modifier::CROSSED_OUT);
    pub const CODE: Style = Style::new().fg(AQUA);
    pub const LINK: Style = Style::new().fg(BLUE).add_modifier(Modifier::UNDERLINED);
}

const ROW_HEIGHT: u16 = 2;
const DETAIL_MAX_WIDTH: u16 = 100;
const MESSAGE_MAX_LINES: usize = 3;

pub fn draw(frame: &mut Frame, state: &State, config: &Config) -> HitMap {
    let now = jiff::Timestamp::now();
    let screen = frame.area();
    // Tabs, rule, one body row, and keys always fit before the message grows.
    let room = usize::from(screen.height.saturating_sub(4)).clamp(1, MESSAGE_MAX_LINES);
    let message = message(state).map(|(text, style)| (wrap_text(&text, usize::from(screen.width), room), style));
    let message_height = message.as_ref().map_or(1, |(lines, _)| lines.len().max(1));
    let [tabs, rule, body, message_area, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(u16::try_from(message_height).unwrap_or(1)),
        Constraint::Length(1),
    ])
    .areas(screen);
    let list_width = state.split.list_width(body.width);
    let list = Rect { width: list_width, ..body };
    let detail = Rect { x: body.x + list_width, width: body.width - list_width, ..body };
    let mut hits = HitMap { panes: body, ..HitMap::default() };
    // The border plus the blank column left of it: one column is hard to hit.
    let divider = Rect { x: detail.x.saturating_sub(1), width: 2, ..body }.intersection(body);
    hits.targets.push((divider, HitTarget::Divider));
    frame.buffer_mut().set_style(screen, palette::PLAIN);
    draw_tabs(frame, tabs, rule, state, Instant::now(), &mut hits);
    draw_list(frame, list, state, now, &mut hits);
    draw_detail(frame, detail, state, now);
    if let Some((lines, style)) = message {
        let lines: Vec<Line<'static>> = lines.into_iter().map(|l| centered(vec![span(l, style)], message_area.width)).collect();
        frame.render_widget(Paragraph::new(lines), message_area);
    }
    draw_keys(frame, keys, state);
    if let Focus::Dialog(dialog) = &state.focus {
        draw_dialog(frame, frame.area(), state, dialog, config);
    }
    hits
}

/// "3h", "2d", "now". Pure, tested.
pub fn age(created: jiff::Timestamp, now: jiff::Timestamp) -> String {
    let secs = now.as_second().saturating_sub(created.as_second()).max(0);
    match secs {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=2_591_999 => format!("{}d", secs / 86_400),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

// ---- small text helpers ------------------------------------------------------

pub(crate) fn cells(text: &str) -> usize {
    Span::raw(text).width()
}

/// Cuts `text` to at most `width` cells and ends it with an ellipsis when cut.
fn fit(text: &str, width: usize) -> String {
    if cells(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = cells(c.encode_utf8(&mut [0; 4]));
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// Cuts `path` from the start to at most `width` cells, so the last segments
/// stay visible. The cut starts at a `/` when the rest still fits.
fn fit_path(path: &str, width: usize) -> String {
    if cells(path) <= width {
        return path.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut used = 0;
    for c in path.chars().rev() {
        let w = cells(c.encode_utf8(&mut [0; 4]));
        if used + w + 1 > width {
            break;
        }
        kept.push(c);
        used += w;
    }
    let tail: String = kept.into_iter().rev().collect();
    let tail = match tail.find('/') {
        Some(i) if i + 1 < tail.len() => &tail[i..],
        _ => &tail,
    };
    format!("…{tail}")
}

/// Word-wraps `text` to `width` cells in at most `max` lines. The last line
/// ends with an ellipsis when text is left over; a word wider than `width`
/// is cut.
fn wrap_text(text: &str, width: usize, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && cells(&line) + 1 + cells(word) > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.len() > max {
        let rest = lines.split_off(max).join(" ");
        if let Some(last) = lines.last_mut() {
            *last = format!("{last} {rest}");
        }
    }
    lines.iter().map(|l| fit(l, width)).collect()
}

/// Pads `spans` on the left so they sit in the middle of `width` cells. A
/// line wider than `width` stays at the left edge and is cut at the right.
fn centered(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let pad = usize::from(width).saturating_sub(spans_width(&spans)) / 2;
    let mut line = vec![Span::raw(" ".repeat(pad))];
    line.extend(spans);
    Line::from(line)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// One row line: `left` is cut to fit, `right` stays whole at the edge.
fn row_line(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>) -> Line<'static> {
    let width = usize::from(width);
    let right_w = spans_width(&right);
    let budget = width.saturating_sub(right_w + 1);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for s in left {
        let room = budget.saturating_sub(used);
        let text = fit(&s.content, room);
        used += cells(&text);
        spans.push(Span::styled(text, s.style));
    }
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w))));
    spans.extend(right);
    Line::from(spans)
}

/// A detail row with a dim key in a fixed column.
/// Cells taken by the key column of a detail field.
const FIELD_KEY_CELLS: usize = 9;

fn field(key: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![span(format!("{key:<FIELD_KEY_CELLS$}"), palette::DIM)];
    spans.extend(value);
    Line::from(spans)
}

fn badge(session: Option<&Session>, starting: bool) -> Span<'static> {
    match (session.map(Session::state), starting) {
        (Some(SessionState::Running), _) => span("● running", palette::RUNNING),
        (Some(SessionState::NeedsYou), _) => span("▲ needs you", palette::NEEDS_YOU),
        (Some(SessionState::Exited), _) => span("○ exited", palette::EXITED),
        (None, true) => span("… starting", palette::STARTING),
        (None, false) => Span::raw(""),
    }
}

fn checks_span(checks: Checks) -> Span<'static> {
    match checks {
        Checks::Pass => span("✓", palette::PASS),
        Checks::Fail => span("✗", palette::FAIL),
        Checks::Pending => span("●", palette::PENDING),
        Checks::None => span("·", palette::DIM),
    }
}

fn checks_word(checks: Checks) -> &'static str {
    match checks {
        Checks::Pass => "checks passing",
        Checks::Fail => "checks failing",
        Checks::Pending => "checks running",
        Checks::None => "no checks",
    }
}

fn pr_state_span(state: PrState) -> Span<'static> {
    match state {
        PrState::Open => span("open", palette::PASS),
        PrState::Draft => span("draft", palette::DIM),
        PrState::Merged => span("merged", palette::MERGED),
        PrState::Closed => span("closed", palette::FAIL),
    }
}

/// Two cells before an issue title: the state of its lead PR, or blank.
fn pr_mark(pr: Option<&PullRequest>) -> Span<'static> {
    let Some(p) = pr else { return Span::raw("  ") };
    match (p.state, p.checks) {
        (PrState::Merged, _) => span("✓ ", palette::MERGED),
        (PrState::Draft, _) => span("◌ ", palette::DIM),
        (PrState::Open, Checks::Pass) => span("● ", palette::PASS),
        (PrState::Open, Checks::Fail) => span("● ", palette::FAIL),
        (PrState::Open, Checks::Pending) => span("● ", palette::PENDING),
        (PrState::Open, Checks::None) => span("● ", palette::PLAIN),
        (PrState::Closed, _) => Span::raw("  "),
    }
}

fn issue_of(state: &State, n: IssueNumber) -> Option<&Issue> {
    state.issues.items.iter().find(|i| i.number == n)
}

// ---- tabs and key bar ------------------------------------------------------

fn draw_tabs(frame: &mut Frame, area: Rect, rule: Rect, state: &State, now: Instant, hits: &mut HitMap) {
    let needs = state.sessions.values().filter(|s| s.state() == SessionState::NeedsYou).count();
    let tabs = [
        (Tab::Issues, "1", "Issues", state.issues.items.len(), 0),
        (Tab::Prs, "2", "Pull requests", state.prs.items.len(), 0),
        (Tab::Sessions, "3", "Sessions", state.sessions.len(), needs),
    ];
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x.saturating_add(1);
    let mut active: (u16, u16) = (area.x, 0);
    for (tab, key, name, count, needs) in tabs {
        let on = tab == state.tab;
        let mut label = vec![
            span(format!("{key} "), palette::DIM),
            span(name, if on { palette::ACTIVE } else { palette::PLAIN }),
            span(format!(" {count}"), palette::DIM),
        ];
        if needs > 0 {
            label.push(span(format!(" ▲{needs}"), palette::NEEDS_YOU));
        }
        let w = u16::try_from(spans_width(&label)).unwrap_or(u16::MAX);
        hits.targets.push((Rect::new(x, area.y, w, 1).intersection(area), HitTarget::Tab(tab)));
        if on {
            active = (x, w);
        }
        spans.extend(label);
        spans.push(Span::raw("   "));
        x = x.saturating_add(w).saturating_add(3);
    }
    let used = usize::from(x.saturating_sub(area.x));
    let room = usize::from(area.width).saturating_sub(used + 1);
    let repo = &state.context.repo_name;
    let right = match freshness(state, now) {
        Some((text, style)) if cells(&text) + 3 + cells(repo) <= room => {
            vec![span(text, style), Span::raw("   "), span(repo.clone(), palette::DIM)]
        }
        Some((text, style)) => vec![span(fit(&text, room), style)],
        None => vec![span(fit(repo, room), palette::DIM)],
    };
    spans.push(Span::raw(" ".repeat(room.saturating_sub(spans_width(&right)))));
    spans.extend(right);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    let total = usize::from(rule.width);
    let before = usize::from(active.0.saturating_sub(rule.x)).min(total);
    let on = usize::from(active.1).min(total - before);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            span("─".repeat(before), palette::DIM),
            span("━".repeat(on), palette::ACCENT),
            span("─".repeat(total - before - on), palette::DIM),
        ])),
        rule,
    );
}

/// The age of the list on the current tab, counted from when the last
/// response arrived. A failed fetch keeps the old items, so it gets its own
/// warning text: stale data never reads as fresh.
fn freshness(state: &State, now: Instant) -> Option<(String, Style)> {
    let (status, refreshing) = match state.tab {
        Tab::Issues => (&state.issues.status, state.inflight.issues),
        Tab::Prs => (&state.prs.status, state.inflight.prs),
        Tab::Sessions => return None,
    };
    Some(match status {
        FeedStatus::Loading => ("loading…".to_string(), palette::DIM),
        _ if refreshing => ("refreshing…".to_string(), palette::DIM),
        FeedStatus::Fresh { at } => (format!("updated {} ago", since(*at, now)), palette::DIM),
        FeedStatus::Failed { at, .. } => (format!("refresh failed {} ago", since(*at, now)), palette::WARN),
    })
}

/// "12s", "3m", "2h". Seconds count up so the user sees the clock move.
fn since(at: Instant, now: Instant) -> String {
    let secs = now.saturating_duration_since(at).as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h", secs / 3600),
    }
}

/// The one place for messages: the toast, or else the error of the shown
/// feed. Drawn on the message row above the key bar.
fn message(state: &State) -> Option<(String, Style)> {
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

fn feed_failure<T>(feed: &Feed<T>) -> Option<String> {
    let FeedStatus::Failed { error, .. } = &feed.status else { return None };
    let kept = if feed.items.is_empty() { "" } else { ", showing old data" };
    Some(format!("gh failed{kept}: {}", one_line(error)))
}

fn draw_keys(frame: &mut Frame, area: Rect, state: &State) {
    let hint = |k: &str, d: &str| vec![span(k.to_string(), palette::ACCENT), span(format!(" {d}"), palette::DIM)];
    let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
    match &state.focus {
        Focus::Filter => {
            parts.push(vec![span("/ ", palette::ACCENT), Span::raw(state.filter.clone()), span("▏", palette::DIM)]);
            parts.push(hint("enter", "keep"));
            parts.push(hint("esc", "clear"));
        }
        Focus::Dialog(_) => {
            for (k, d) in [("ctrl-s", "start"), ("tab", "next field"), ("←/→", "change"), ("esc", "cancel")] {
                parts.push(hint(k, d));
            }
        }
        Focus::List => {
            if !state.filter.is_empty() {
                parts.push(vec![span(format!("/{}", state.filter), palette::LABEL)]);
            }
            for (k, d) in app::key_bar(state) {
                parts.push(hint(k, d));
            }
        }
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.extend(part);
    }
    frame.render_widget(Paragraph::new(centered(spans, area.width)), area);
}

// ---- list ------------------------------------------------------------------

fn row_number(key: RowKey) -> String {
    match key {
        RowKey::Issue(n) | RowKey::Session(n) => format!("#{}", n.0),
        RowKey::Pr(n) => format!("#{}", n.0),
    }
}

fn draw_list(frame: &mut Frame, area: Rect, state: &State, now: jiff::Timestamp, hits: &mut HitMap) {
    let area = Rect { width: area.width.saturating_sub(1), ..area };
    let rows = app::visible_rows(state);
    let selected = app::selected_row(state);
    if rows.is_empty() {
        frame.render_widget(Paragraph::new(span(format!("  {}", empty_text(state)), palette::DIM)), area);
        return;
    }
    let number_width = rows.iter().map(|k| row_number(*k).len()).max().unwrap_or(0);
    let visible = usize::from(area.height / ROW_HEIGHT).max(1);
    let at = selected.and_then(|k| rows.iter().position(|r| *r == k)).unwrap_or(0);
    let top = (at + 1).saturating_sub(visible);
    let mut y = area.y;
    for key in rows.iter().skip(top).take(visible) {
        let height = area.bottom().saturating_sub(y).min(ROW_HEIGHT);
        if height == 0 {
            break;
        }
        let rect = Rect::new(area.x, y, area.width, height);
        let lines = list_row(state, *key, now, area.width, number_width, selected == Some(*key));
        frame.render_widget(Paragraph::new(lines.to_vec()), rect);
        hits.targets.push((rect, HitTarget::Row(*key)));
        y = y.saturating_add(ROW_HEIGHT);
    }
}

fn empty_text(state: &State) -> String {
    let (name, loading) = match state.tab {
        Tab::Issues => ("issues", matches!(state.issues.status, FeedStatus::Loading)),
        Tab::Prs => ("pull requests", matches!(state.prs.status, FeedStatus::Loading)),
        Tab::Sessions => return "No sessions. Press s on an issue to start one.".to_string(),
    };
    match (loading, state.filter.is_empty()) {
        (true, _) => format!("Loading {name}…"),
        (false, true) => format!("No {name}."),
        (false, false) => format!("No {name} match the filter."),
    }
}

/// Line 1: marker, number, title, and the state at the right edge.
/// Line 2: the quiet facts (author, age, labels or branch) under the title.
fn list_row(
    state: &State,
    key: RowKey,
    now: jiff::Timestamp,
    width: u16,
    number_width: usize,
    selected: bool,
) -> [Line<'static>; 2] {
    let mark = match key {
        RowKey::Issue(n) => vec![pr_mark(app::lead_pr(state, n))],
        RowKey::Pr(_) | RowKey::Session(_) => Vec::new(),
    };
    let (title, right, meta): (String, Span<'static>, Vec<Span<'static>>) = match key {
        RowKey::Issue(n) => {
            let Some(i) = issue_of(state, n) else { return [Line::raw(""), Line::raw("")] };
            let mut facts = format!("{} · {}", i.author, age(i.created_at, now));
            if !i.labels.is_empty() {
                facts.push_str(&format!(" · {}", i.labels.join(", ")));
            }
            (one_line(&i.title), badge(state.sessions.get(&n), state.starting.contains(&n)), vec![span(facts, palette::DIM)])
        }
        RowKey::Pr(n) => {
            let Some(p) = state.prs.items.iter().find(|p| p.number == n) else { return [Line::raw(""), Line::raw("")] };
            let mut meta = vec![span(format!("{} · {}", p.author, age(p.created_at, now)), palette::DIM)];
            if p.checks != Checks::None {
                meta.extend([span(" · ", palette::DIM), checks_span(p.checks), span(format!(" {}", checks_word(p.checks)), palette::DIM)]);
            }
            (one_line(&p.title), pr_state_span(p.state), meta)
        }
        RowKey::Session(n) => {
            let session = state.sessions.get(&n);
            let title = issue_of(state, n).map(|i| one_line(&i.title));
            let branch = session.map(|s| s.branch.as_str().to_string());
            let meta = if title.is_some() { branch.clone().unwrap_or_default() } else { String::new() };
            (title.or(branch).unwrap_or_default(), badge(session, state.starting.contains(&n)), vec![span(meta, palette::DIM)])
        }
    };
    let marker = || if selected { span("▌ ", palette::ACCENT) } else { Span::raw("  ") };
    let number = span(format!("{:<number_width$}  ", row_number(key)), palette::DIM);
    let title = span(title, if selected { palette::HEADING } else { palette::PLAIN });
    let mut second = vec![marker(), Span::raw(" ".repeat(number_width + 2 + spans_width(&mark)))];
    second.extend(meta);
    let mut first = vec![marker(), number];
    first.extend(mark);
    first.push(title);
    [row_line(width, first, vec![right]), row_line(width, second, Vec::new())]
}

// ---- detail ----------------------------------------------------------------

fn draw_detail(frame: &mut Frame, area: Rect, state: &State, now: jiff::Timestamp) {
    let border = if state.dragging_split { palette::ACCENT } else { palette::DIM };
    let block = Block::new().borders(Borders::LEFT).border_style(border).padding(Padding::horizontal(2));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect { width: inner.width.min(DETAIL_MAX_WIDTH), ..inner };
    let Some(key) = app::selected_row(state) else { return };
    match key {
        RowKey::Issue(n) => draw_issue_detail(frame, inner, state, n, now),
        RowKey::Pr(n) => draw_pr_detail(frame, inner, state, n, now),
        RowKey::Session(n) => draw_session_detail(frame, inner, state, n),
    }
}

fn tail_lines<'a>(state: &'a State, session: &Session) -> &'a [String] {
    match (&state.tail, &session.window) {
        (Some(t), Some(w)) if t.window == w.id => &t.lines,
        _ => &[],
    }
}

fn last_n(lines: &[String], n: usize) -> &[String] {
    &lines[lines.len().saturating_sub(n)..]
}

fn labels_line(labels: &[String]) -> Line<'static> {
    Line::from(labels.iter().flat_map(|l| [span(l.clone(), palette::LABEL), Span::raw("  ")]).collect::<Vec<_>>())
}

fn draw_issue_detail(frame: &mut Frame, area: Rect, state: &State, n: IssueNumber, now: jiff::Timestamp) {
    let Some(i) = issue_of(state, n) else { return };
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(span(i.title.clone(), palette::HEADING)),
        Line::from(span(
            format!(
                "#{} · opened {} ago by {} · {} comment{}",
                n.0,
                age(i.created_at, now),
                i.author,
                i.comment_count,
                if i.comment_count == 1 { "" } else { "s" }
            ),
            palette::DIM,
        )),
    ];
    if !i.labels.is_empty() {
        lines.push(labels_line(&i.labels));
    }
    lines.push(Line::raw(""));
    match (state.sessions.get(&n), state.starting.contains(&n)) {
        (Some(s), _) => {
            lines.push(field("session", vec![badge(Some(s), false), span(format!("  {}", s.branch.as_str()), palette::DIM)]));
            let width = usize::from(area.width).saturating_sub(2);
            lines.extend(last_n(tail_lines(state, s), 3).iter().map(|l| Line::from(vec![span("│ ", palette::DIM), span(fit(l, width), palette::DIM)])));
            lines.push(Line::raw(""));
        }
        (None, true) => lines.extend([field("session", vec![badge(None, true)]), Line::raw("")]),
        (None, false) => {}
    }
    lines.extend(markdown::render(&i.body, area.width));
    let linked: Vec<&PullRequest> = app::prs_closing(state, n).collect();
    if !linked.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(span("Linked pull requests", palette::HEADING)));
        for p in linked {
            lines.push(Line::from(vec![
                span(format!("#{}  ", p.number.0), palette::DIM),
                Span::raw(one_line(&p.title)),
                Span::raw("  "),
                checks_span(p.checks),
                Span::raw(" "),
                pr_state_span(p.state),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_pr_detail(frame: &mut Frame, area: Rect, state: &State, n: crate::domain::PrNumber, now: jiff::Timestamp) {
    let Some(p) = state.prs.items.iter().find(|p| p.number == n) else { return };
    let mut lines = vec![
        Line::from(span(p.title.clone(), palette::HEADING)),
        Line::from(span(format!("#{} · opened {} ago by {}", n.0, age(p.created_at, now), p.author), palette::DIM)),
        Line::raw(""),
        field("state", vec![pr_state_span(p.state)]),
        field("checks", vec![checks_span(p.checks), Span::raw(format!(" {}", checks_word(p.checks)))]),
        field("branch", vec![Span::raw(format!("{} → {}", p.head, p.base))]),
        field("diff", vec![span(format!("+{}", p.additions), palette::PASS), Span::raw(" "), span(format!("-{}", p.deletions), palette::FAIL)]),
    ];
    if !p.closes.is_empty() {
        let closes = p.closes.iter().map(|c| format!("#{}", c.0)).collect::<Vec<_>>().join(", ");
        lines.push(field("closes", vec![Span::raw(closes)]));
    }
    if let Some(s) = state.sessions.values().find(|s| s.branch.as_str() == p.head) {
        lines.push(field("session", vec![Span::raw(format!("#{} ", s.issue.0)), badge(Some(s), false)]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_session_detail(frame: &mut Frame, area: Rect, state: &State, n: IssueNumber) {
    let Some(s) = state.sessions.get(&n) else {
        let text = if state.starting.contains(&n) { "Creating the worktree and the tmux window…" } else { "" };
        frame.render_widget(Paragraph::new(span(text, palette::STARTING)), area);
        return;
    };
    let title = issue_of(state, n).map_or_else(|| s.branch.as_str().to_string(), |i| i.title.clone());
    let width = usize::from(area.width);
    let mut head = vec![
        Line::from(span(fit(&title, width), palette::HEADING)),
        Line::from(span(format!("#{}", n.0), palette::DIM)),
        Line::raw(""),
        field("state", vec![badge(Some(s), false)]),
        field("branch", vec![Span::raw(s.branch.as_str().to_string())]),
        field("worktree", vec![Span::raw(fit_path(&s.worktree.as_path().display().to_string(), width.saturating_sub(FIELD_KEY_CELLS)))]),
    ];
    if let Some(p) = app::pr_for_session(state, s) {
        head.push(field("pr", vec![Span::raw(format!("#{} ", p.number.0)), checks_span(p.checks), Span::raw(" "), pr_state_span(p.state)]));
    }
    head.push(Line::raw(""));
    head.push(Line::from(span(format!("output {}", "─".repeat(width.saturating_sub(7))), palette::DIM)));
    let head_h = u16::try_from(head.len()).unwrap_or(u16::MAX).min(area.height);
    let [top, rest] = Layout::vertical([Constraint::Length(head_h), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(head), top);
    let tail: Vec<Line<'static>> = last_n(tail_lines(state, s), usize::from(rest.height))
        .iter()
        .map(|l| Line::raw(fit(l, width)))
        .collect();
    frame.render_widget(Paragraph::new(tail), rest);
}

// ---- dialog ----------------------------------------------------------------

/// The exact commands the dialog will run, for the "Runs" preview.
fn preview(state: &State, dialog: &StartDialog, config: &Config) -> Vec<String> {
    let branch = dialog.branch.lines().join("");
    let prompt = dialog.prompt.lines().join("\n");
    let args = rules::claude_args(dialog.mode, prompt.trim(), None);
    let shown: Vec<String> = config
        .claude_command
        .argv(&args)
        .into_iter()
        .map(|a| if a.contains([' ', '\n', '"']) { format!("{:?}", one_line(&a)) } else { a })
        .collect();
    let repo = if state.context.repo_name.is_empty() { "repo" } else { state.context.repo_name.as_str() };
    let base = if state.context.base.is_empty() { "origin/main" } else { state.context.base.as_str() };
    vec![
        format!("git worktree add ../{repo}-{} -b {} {base}", dialog.issue.0, branch.trim()),
        shown.join(" "),
    ]
}

fn styled_input(source: &tui_textarea::TextArea<'static>, focused: bool) -> tui_textarea::TextArea<'static> {
    let mut input = source.clone();
    input.set_style(palette::PLAIN);
    input.set_cursor_line_style(palette::PLAIN);
    input.set_cursor_style(if focused { palette::CURSOR } else { palette::PLAIN });
    input
}

fn field_label(text: &str, focused: bool) -> Line<'static> {
    if focused {
        Line::from(vec![span("› ", palette::ACCENT), span(text.to_string(), palette::ACTIVE)])
    } else {
        Line::from(span(format!("  {text}"), palette::DIM))
    }
}

fn draw_dialog(frame: &mut Frame, screen: Rect, state: &State, d: &StartDialog, config: &Config) {
    let width = screen.width.min(96);
    let height = screen.height.min(28);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(format!(" Start session for #{} {} ", d.issue.0, fit(&d.title, 50)), palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [branch_l, branch, _, mode, _, prompt_l, prompt, _, runs, error] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    let indent = |r: Rect| Rect { x: r.x.saturating_add(2), width: r.width.saturating_sub(2), ..r };
    let focus = |f: DialogField| d.field == f;
    frame.render_widget(Paragraph::new(field_label("Branch", focus(DialogField::Branch))), branch_l);
    frame.render_widget(&styled_input(&d.branch, focus(DialogField::Branch)), indent(branch));

    let radio = |on: bool, text: &str| -> Vec<Span<'static>> {
        if on {
            vec![span("● ", palette::ACCENT), Span::raw(text.to_string())]
        } else {
            vec![span(format!("○ {text}"), palette::DIM)]
        }
    };
    let mut modes = vec![Span::raw("  ")];
    modes.extend(radio(d.mode == Mode::PlanFirst, "Plan first, then wait for approval"));
    modes.push(Span::raw("    "));
    modes.extend(radio(d.mode == Mode::Implement, "Implement directly"));
    frame.render_widget(
        Paragraph::new(vec![field_label("Mode", focus(DialogField::Mode)), Line::from(modes)]),
        mode,
    );
    frame.render_widget(Paragraph::new(field_label("Prompt", focus(DialogField::Prompt))), prompt_l);
    frame.render_widget(&styled_input(&d.prompt, focus(DialogField::Prompt)), indent(prompt));

    let mut run_lines = vec![Line::from(span("  Runs", palette::DIM))];
    let width = usize::from(runs.width).saturating_sub(4);
    run_lines.extend(preview(state, d, config).iter().map(|c| Line::from(span(format!("  $ {}", fit(c, width)), palette::DIM))));
    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(format!("  {e}"), palette::ERROR)), error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Context, Msg};
    use crate::domain::{BranchName, WorktreePath};
    use crate::session::{Window, WindowId};
    use crate::testutil::{issue, pr};
    use crate::domain::Split;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn ts(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(secs).expect("valid timestamp")
    }

    #[test]
    fn age_buckets() {
        let now = ts(10_000_000);
        for (secs, want) in [(0, "now"), (59, "now"), (60, "1m"), (3599, "59m"), (3600, "1h"), (86_399, "23h"), (86_400, "1d"), (9 * 86_400, "9d"), (40 * 86_400, "1mo"), (400 * 86_400, "1y")] {
            assert_eq!(age(ts(10_000_000 - secs), now), want, "{secs}s");
        }
        assert_eq!(age(ts(10_000_100), now), "now", "a clock skew never prints a negative age");
    }

    #[test]
    fn fit_cuts_by_cells() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 6), "hello…");
        assert_eq!(fit("anything", 0), "");
    }

    #[test]
    fn fit_path_keeps_the_last_segments() {
        assert_eq!(fit_path("/w/orbit-3", 20), "/w/orbit-3");
        assert_eq!(fit_path("/home/me/scratchpad/smoke/hf-1017", 26), "…/scratchpad/smoke/hf-1017");
        assert_eq!(fit_path("/home/me/scratchpad/smoke/hf-1017", 24), "…/smoke/hf-1017", "the cut starts at a slash");
        assert_eq!(fit_path("/a/very-long-final-segment", 10), "…l-segment", "a segment wider than the space is cut by cells");
        assert_eq!(fit_path("/anything", 0), "");
    }

    #[test]
    fn sessions_detail_shows_the_end_of_a_long_worktree_path() {
        let (mut state, config) = full_state();
        let long = "/home/someone/projects/a-very-long-directory-name/another-long-directory/scratchpad/smoke/hf-1017";
        if let Some(s) = state.sessions.get_mut(&IssueNumber(3)) {
            s.worktree = WorktreePath::planned(PathBuf::from(long));
        }
        send(&mut state, &config, KeyCode::Char('3'));
        let screen = text(&render(&state, &config, 80, 24));
        let line = screen.lines().find(|l| l.contains("worktree ")).expect("worktree line drawn");
        assert!(line.contains("…/") && line.trim_end().ends_with("/smoke/hf-1017"), "path end not visible in {line:?}\n{screen}");
    }

    fn full_state() -> (State, Config) {
        let config = Config::default();
        let (mut state, _) = app::init(Instant::now(), &config, Context { repo_name: "orbit".to_string(), base: "origin/main".to_string() });
        state.issues.items = vec![
            issue(3, "Retry queue drops jobs when Redis reconnects after a very long title that must be cut", &["bug", "p1"]),
            issue(2, "Docs", &["docs"]),
        ];
        state.issues.items[0].body = "First line of the body.\n\n```\ncode block\n```".to_string();
        state.issues.status = FeedStatus::Fresh { at: Instant::now() };
        state.prs.items = vec![pr(10, "fix/3-x", &[3])];
        state.prs.status = FeedStatus::Fresh { at: Instant::now() };
        state.inflight.issues = false;
        state.inflight.prs = false;
        let id = WindowId::parse("@3").expect("id");
        state.sessions.insert(
            IssueNumber(3),
            Session {
                issue: IssueNumber(3),
                branch: BranchName::parse("fix/3-x").expect("branch"),
                worktree: WorktreePath::planned(PathBuf::from("/w/orbit-3")),
                window: Some(Window { id: id.clone(), alive: true, waiting: true }),
            },
        );
        state.tail = Some(app::Tail { window: id, lines: vec!["fake output".to_string(), "second line".to_string()] });
        (state, config)
    }

    fn render(state: &State, config: &Config, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| {
            let _ = draw(f, state, config);
        })
        .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        buf.content().chunks(usize::from(buf.area.width)).map(|row| row.iter().map(|c| c.symbol()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn send(state: &mut State, config: &Config, code: KeyCode) {
        let _ = app::update(state, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)), config);
    }

    fn in_palette(c: Color) -> bool {
        use palette::{AQUA, BLUE, FG, GRAY, GREEN, ORANGE, PURPLE, RED, YELLOW};
        [FG, GRAY, RED, GREEN, YELLOW, BLUE, PURPLE, AQUA, ORANGE].contains(&c)
    }

    /// Every screen the app can show: no cell has a background, and every
    /// foreground is a Gruvbox palette color.
    #[test]
    fn no_cell_has_a_background_and_every_foreground_is_gruvbox() {
        let (mut state, config) = full_state();
        let mut screens: Vec<(String, Buffer)> = Vec::new();
        for (name, key) in [("issues", '1'), ("prs", '2'), ("sessions", '3')] {
            send(&mut state, &config, KeyCode::Char(key));
            screens.push((name.to_string(), render(&state, &config, 100, 30)));
        }
        send(&mut state, &config, KeyCode::Char('1'));
        send(&mut state, &config, KeyCode::Char('j'));
        send(&mut state, &config, KeyCode::Char('s'));
        screens.push(("dialog".to_string(), render(&state, &config, 100, 30)));
        screens.push(("narrow".to_string(), render(&state, &config, 30, 12)));
        for (name, buf) in screens {
            for cell in buf.content() {
                assert_eq!(cell.bg, Color::Reset, "{name}: background at {:?}", cell.symbol());
                assert_eq!(cell.underline_color, Color::Reset, "{name}: underline color");
                assert!(in_palette(cell.fg), "{name}: fg {:?} at {:?}", cell.fg, cell.symbol());
            }
        }
    }

    #[test]
    fn selection_is_an_accent_marker_on_both_lines_of_one_row() {
        let (state, config) = full_state();
        let buf = render(&state, &config, 100, 30);
        let width = usize::from(buf.area.width);
        let marked: Vec<usize> = buf.content().iter().enumerate().filter(|(_, c)| c.symbol() == "▌").map(|(i, _)| i / width).collect();
        assert_eq!(marked, [2, 3], "the first row is selected, and it is two lines tall");
        assert!(buf.content().iter().all(|c| !c.modifier.contains(Modifier::REVERSED)), "no reversed block outside a text cursor");
        assert!(buf.content().iter().filter(|c| c.symbol() == "▌").all(|c| c.fg == palette::ORANGE));
    }

    #[test]
    fn issues_screen_shows_rows_badges_detail_and_key_bar() {
        let (state, config) = full_state();
        let screen = text(&render(&state, &config, 120, 24));
        for want in [
            "1 Issues 2",
            "2 Pull requests 1",
            "3 Sessions 1 ▲1",
            "#3  ● Retry queue",
            "▲ needs you",
            "bug, p1",
            "fake output",
            "First line of the body.",
            "Linked pull requests",
            "s start session",
            "a attach",
            "x stop",
            "q quit",
        ] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    /// The cell that holds the PR mark on the first line of issue `n`'s row.
    fn mark_cell(buf: &Buffer, n: u32) -> (String, Color) {
        let screen = text(buf);
        let (y, line) = screen.lines().enumerate().find(|(_, l)| l.contains(&format!("#{n} "))).expect("row drawn");
        let x = line.chars().position(|c| c == '#').expect("number") + 4;
        let cell = &buf[(u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))];
        (cell.symbol().to_string(), cell.fg)
    }

    #[test]
    fn issue_rows_show_the_lead_pr_state_and_follow_a_refresh() {
        let (mut state, config) = full_state();
        let mark = |state: &State| (mark_cell(&render(state, &config, 100, 20), 3), mark_cell(&render(state, &config, 100, 20), 2));
        assert_eq!(mark(&state), (("●".to_string(), palette::FG), (" ".to_string(), palette::FG)), "open, no checks; #2 has no PR");
        for (pr_state, checks, want, color) in [
            (PrState::Draft, Checks::Pass, "◌", palette::GRAY),
            (PrState::Open, Checks::Pending, "●", palette::YELLOW),
            (PrState::Open, Checks::Fail, "●", palette::RED),
            (PrState::Open, Checks::Pass, "●", palette::GREEN),
            (PrState::Merged, Checks::Fail, "✓", palette::PURPLE),
            (PrState::Closed, Checks::Pass, " ", palette::FG),
        ] {
            let fresh = vec![PullRequest { state: pr_state, checks, ..pr(10, "fix/3-x", &[3]) }];
            let _ = app::update(&mut state, Msg::Prs { result: Ok(fresh), at: Instant::now() }, &config);
            assert_eq!(mark(&state).0, (want.to_string(), color), "{pr_state:?} {checks:?}");
        }
        let screen = text(&render(&state, &config, 100, 20));
        assert!(screen.contains("#3    Retry queue") && screen.contains("#2    Docs"), "titles stay in one column\n{screen}");
    }

    #[test]
    fn sessions_detail_shows_the_facts_and_the_tail_under_the_output_rule() {
        let (mut state, config) = full_state();
        send(&mut state, &config, KeyCode::Char('3'));
        let screen = text(&render(&state, &config, 120, 24));
        assert!(screen.contains("worktree /w/orbit-3"));
        let lines: Vec<&str> = screen.lines().collect();
        let rule = lines.iter().position(|l| l.contains("output ─")).expect("output rule drawn");
        assert!(lines[rule + 1].contains("fake output"));
        assert!(lines[rule + 2].contains("second line"));
    }

    #[test]
    fn dialog_shows_fields_and_the_exact_commands() {
        let (mut state, config) = full_state();
        send(&mut state, &config, KeyCode::Char('j'));
        send(&mut state, &config, KeyCode::Char('s'));
        let screen = text(&render(&state, &config, 120, 30));
        for want in ["Start session for #2 Docs", "Branch", "docs/2-docs", "Implement directly", "Runs", "$ git worktree add ../orbit-2 -b docs/2-docs origin/main", "claude \"Implement GitHub issue #2: Docs", "ctrl-s start"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn draw_returns_hits_for_tabs_and_visible_rows() {
        let (state, config) = full_state();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = draw(f, &state, &config)).expect("draw");
        let tabs = hits.targets.iter().filter(|(_, t)| matches!(t, HitTarget::Tab(_))).count();
        let rows: Vec<_> = hits.targets.iter().filter_map(|(r, t)| match t { HitTarget::Row(k) => Some((r.y, *k)), _ => None }).collect();
        assert_eq!(tabs, 3);
        assert_eq!(rows, [(2, RowKey::Issue(IssueNumber(3))), (4, RowKey::Issue(IssueNumber(2)))]);
    }

    /// Draws like the shell does: the frame's hits go back into `state`.
    fn frame(state: &mut State, config: &Config, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = draw(f, state, config)).expect("draw");
        state.hits = hits;
        terminal.backend().buffer().clone()
    }

    /// The column of the list | detail border on the first body row.
    fn border(buf: &Buffer) -> Option<u16> {
        (0..buf.area.width).find(|&x| buf[(x, 2)].symbol() == "│")
    }

    fn mouse(state: &mut State, config: &Config, kind: MouseEventKind, column: u16) {
        let ev = MouseEvent { kind, column, row: 4, modifiers: KeyModifiers::NONE };
        let _ = app::update(state, Msg::Mouse(ev, Instant::now()), config);
    }

    fn drag(state: &mut State, config: &Config, from: u16, to: u16) {
        mouse(state, config, MouseEventKind::Down(MouseButton::Left), from);
        mouse(state, config, MouseEventKind::Drag(MouseButton::Left), to);
        mouse(state, config, MouseEventKind::Up(MouseButton::Left), to);
    }

    #[test]
    fn dragging_the_border_and_the_resize_keys_move_it() {
        let (mut state, config) = full_state();
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(42));
        drag(&mut state, &config, 42, 60);
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(60));
        drag(&mut state, &config, 59, 50); // the blank column left of the border also grabs it
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(50));
        send(&mut state, &config, KeyCode::Char('<'));
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(45));
        send(&mut state, &config, KeyCode::Char('>'));
        send(&mut state, &config, KeyCode::Char('>'));
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(55));
        send(&mut state, &config, KeyCode::Char('3'));
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(55), "the split applies to every tab");
        send(&mut state, &config, KeyCode::Char('='));
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(42));
    }

    #[test]
    fn a_drag_stops_at_the_minimum_pane_widths() {
        let (mut state, config) = full_state();
        let _ = frame(&mut state, &config, 100, 20);
        drag(&mut state, &config, 42, 0);
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(Split::MIN_LIST));
        drag(&mut state, &config, Split::MIN_LIST, 99);
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(100 - Split::MIN_DETAIL));
    }

    #[test]
    fn a_drag_that_starts_on_a_row_selects_it_and_leaves_the_border() {
        let (mut state, config) = full_state();
        let _ = frame(&mut state, &config, 100, 20);
        drag(&mut state, &config, 10, 70);
        assert_eq!(app::selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(42));
    }

    #[test]
    fn resize_keys_typed_into_the_filter_are_text() {
        let (mut state, config) = full_state();
        let _ = frame(&mut state, &config, 100, 20);
        send(&mut state, &config, KeyCode::Char('/'));
        send(&mut state, &config, KeyCode::Char('<'));
        assert_eq!(state.filter, "<");
        assert_eq!(border(&frame(&mut state, &config, 100, 20)), Some(42));
    }

    #[test]
    fn narrow_panes_still_draw_two_line_rows_and_the_worktree_tail() {
        let (mut state, config) = full_state();
        let long = "/home/someone/projects/a-very-long-directory-name/another-long-directory/scratchpad/smoke/hf-1017";
        if let Some(s) = state.sessions.get_mut(&IssueNumber(3)) {
            s.worktree = WorktreePath::planned(PathBuf::from(long));
        }
        let _ = frame(&mut state, &config, 100, 20);
        drag(&mut state, &config, 42, 0);
        let screen = text(&frame(&mut state, &config, 100, 20));
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[2].contains("#3") && rows[3].contains("octo"), "two-line row broken:\n{screen}");
        assert!(rows[4].contains("#2"), "second row not drawn:\n{screen}");
        drag(&mut state, &config, Split::MIN_LIST, 99);
        send(&mut state, &config, KeyCode::Char('3'));
        let screen = text(&frame(&mut state, &config, 100, 20));
        let line = screen.lines().find(|l| l.contains("worktree ")).expect("worktree line drawn");
        assert!(line.contains('…') && line.trim_end().ends_with("hf-1017"), "path end not visible in {line:?}\n{screen}");
    }

    #[test]
    fn small_terminals_keep_both_panes() {
        for (w, h) in [(54, 12), (40, 10), (20, 6), (3, 5)] {
            for percent in [10, 42, 90] {
                let (mut state, mut config) = full_state();
                config.split = Split::from_percent(percent).expect("in range");
                state.split = config.split;
                let buf = frame(&mut state, &config, w, h);
                let x = border(&buf).unwrap_or_else(|| panic!("no border at {w}x{h} {percent}%:\n{}", text(&buf)));
                assert!(x >= 1 && x < w, "{w}x{h} {percent}%: border at {x}");
            }
        }
        let (mut state, config) = full_state();
        let screen = text(&frame(&mut state, &config, 40, 10));
        let row = screen.lines().nth(2).unwrap_or_default();
        assert!(row.contains("▌ #") && row.contains("│  Retry"), "a pane is empty:\n{screen}");
    }

    #[test]
    fn a_long_message_wraps_on_its_own_row_above_the_centered_keys() {
        let (mut state, config) = full_state();
        state.toast = Some(app::Toast { text: "Press m again to ask #3's agent to merge PR #10".to_string(), shown_at: None });
        let screen = text(&render(&state, &config, 30, 12));
        let lines: Vec<&str> = screen.lines().collect();
        let message = format!("{} {}", lines[9].trim(), lines[10].trim());
        assert_eq!(message, "Press m again to ask #3's agent to merge PR #10", "message not whole in\n{screen}");
        assert!(!lines[11].contains("Press"), "message on the key bar:\n{screen}");

        state.toast = None;
        let screen = text(&render(&state, &config, 160, 24));
        let keys = screen.lines().last().unwrap_or_default();
        let left = keys.len() - keys.trim_start().len();
        let right = keys.len() - keys.trim_end().len();
        assert!(left > 1 && left.abs_diff(right) <= 1, "key bar not centered: {keys:?}");
    }

    #[test]
    fn a_failed_feed_shows_on_the_message_row() {
        let (mut state, config) = full_state();
        state.issues.status = FeedStatus::Failed { error: "rate limited".to_string(), at: Instant::now() };
        let screen = text(&render(&state, &config, 100, 20));
        let row = screen.lines().nth(18).unwrap_or_default();
        assert!(row.contains("gh failed, showing old data: rate limited"), "missing failure in\n{screen}");
        assert!(screen.lines().next().unwrap_or_default().trim_end().ends_with("orbit"));
    }

    #[test]
    fn wrap_text_breaks_at_words_and_cuts_what_does_not_fit() {
        assert_eq!(wrap_text("aa bb cc", 5, 3), ["aa bb", "cc"]);
        assert_eq!(wrap_text("aa bb cc dd", 2, 2), ["aa", "b…"]);
        assert_eq!(wrap_text("abcdefgh", 4, 3), ["abc…"]);
    }

    #[test]
    fn since_counts_seconds_then_minutes_then_hours() {
        let at = Instant::now();
        for (secs, want) in [(0, "0s"), (12, "12s"), (59, "59s"), (60, "1m"), (3599, "59m"), (3600, "1h"), (7300, "2h")] {
            assert_eq!(since(at, at + Duration::from_secs(secs)), want, "{secs}s");
        }
        assert_eq!(since(at + Duration::from_secs(5), at), "0s", "a response stamped after now is not negative");
    }

    /// The tab row's right end, and the style of its first character.
    fn freshness_on_screen(state: &State, config: &Config) -> (String, Style) {
        let buf = render(state, config, 120, 20);
        let row = text(&buf).lines().next().unwrap_or_default().to_string();
        let words = ["loading", "refreshing", "updated", "refresh failed"];
        let Some(x) = words.iter().filter_map(|w| row.find(w)).min() else { return (row.trim().to_string(), palette::PLAIN) };
        let column = row[..x].chars().count();
        let cell = &buf[(u16::try_from(column).expect("x"), 0)];
        (row[x..].trim_end().to_string(), Style::new().fg(cell.fg))
    }

    fn ago(secs: u64) -> Instant {
        Instant::now().checked_sub(Duration::from_secs(secs)).expect("uptime above a minute")
    }

    #[test]
    fn the_tab_row_shows_how_old_the_list_is_and_each_fetch_state() {
        let (mut state, config) = full_state();
        state.issues.status = FeedStatus::Fresh { at: ago(12) };
        assert_eq!(freshness_on_screen(&state, &config), ("updated 12s ago   orbit".to_string(), palette::DIM));

        state.inflight.issues = true;
        assert_eq!(freshness_on_screen(&state, &config), ("refreshing…   orbit".to_string(), palette::DIM));

        state.inflight.issues = false;
        state.issues.status = FeedStatus::Failed { error: "offline".to_string(), at: ago(40) };
        assert_eq!(freshness_on_screen(&state, &config), ("refresh failed 40s ago   orbit".to_string(), palette::WARN));

        state.prs.status = FeedStatus::Fresh { at: ago(125) };
        send(&mut state, &config, KeyCode::Char('2'));
        assert_eq!(freshness_on_screen(&state, &config).0, "updated 2m ago   orbit", "each tab shows its own feed");

        send(&mut state, &config, KeyCode::Char('3'));
        let row = freshness_on_screen(&state, &config).0;
        assert!(row.starts_with("1 Issues") && row.ends_with(" orbit"), "sessions are not fetched from GitHub, so no age: {row:?}");
    }

    #[test]
    fn a_narrow_tab_row_keeps_the_age_and_drops_the_repo_name() {
        let (mut state, config) = full_state();
        state.issues.status = FeedStatus::Fresh { at: ago(12) };
        let row = text(&render(&state, &config, 72, 10)).lines().next().unwrap_or_default().to_string();
        assert!(row.trim_end().ends_with("updated 12s ago") && !row.contains("orbit"), "row: {row:?}");
    }

    #[test]
    fn empty_and_loading_lists_say_so() {
        let config = Config::default();
        let (state, _) = app::init(Instant::now(), &config, Context::default());
        assert!(text(&render(&state, &config, 80, 10)).contains("Loading issues…"));
    }
}
