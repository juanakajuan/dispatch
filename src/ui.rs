//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: tab bar and its rule (2 rows),
//! list | detail (42% | 58%), a blank row, key bar (1 row). List rows are two
//! lines tall. The dialog is a centered overlay.

use crate::app::{
    self, DialogField, Feed, FeedStatus, Focus, HitMap, HitTarget, RowKey, StartDialog, State, Tab,
};
use crate::config::Config;
use crate::domain::{Checks, Issue, IssueNumber, Mode, PrState, PullRequest};
use crate::rules;
use crate::session::{Session, SessionState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

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
}

const ROW_HEIGHT: u16 = 2;
const DETAIL_MAX_WIDTH: u16 = 100;

pub fn draw(frame: &mut Frame, state: &State, config: &Config) -> HitMap {
    let now = jiff::Timestamp::now();
    let [tabs, rule, body, _gap, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [list, detail] = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(body);
    let mut hits = HitMap::default();
    let screen = frame.area();
    frame.buffer_mut().set_style(screen, palette::PLAIN);
    draw_tabs(frame, tabs, rule, state, &mut hits);
    draw_list(frame, list, state, now, &mut hits);
    draw_detail(frame, detail, state, now);
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

fn cells(text: &str) -> usize {
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

fn draw_tabs(frame: &mut Frame, area: Rect, rule: Rect, state: &State, hits: &mut HitMap) {
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
    let status = match state.tab {
        Tab::Issues => feed_status(&state.issues),
        Tab::Prs => feed_status(&state.prs),
        Tab::Sessions => None,
    };
    let (text, style) = status.unwrap_or_else(|| (state.context.repo_name.clone(), palette::DIM));
    let used = usize::from(x.saturating_sub(area.x));
    let room = usize::from(area.width).saturating_sub(used + 1);
    let text = fit(&text, room);
    spans.push(Span::raw(" ".repeat(room.saturating_sub(cells(&text)))));
    spans.push(span(text, style));
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

fn feed_status<T>(feed: &Feed<T>) -> Option<(String, Style)> {
    match &feed.status {
        FeedStatus::Loading => Some(("loading…".to_string(), palette::DIM)),
        FeedStatus::Fresh => None,
        FeedStatus::Failed(e) => {
            let kept = if feed.items.is_empty() { "" } else { ", showing old data" };
            Some((format!("gh failed{kept}: {}", one_line(e)), palette::ERROR))
        }
    }
}

fn draw_keys(frame: &mut Frame, area: Rect, state: &State) {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];
    let hint = |spans: &mut Vec<Span<'static>>, k: &str, d: &str| {
        spans.extend([span(k.to_string(), palette::ACCENT), span(format!(" {d}   "), palette::DIM)]);
    };
    match &state.focus {
        Focus::Filter => {
            spans.push(span("/ ", palette::ACCENT));
            spans.push(Span::raw(state.filter.clone()));
            spans.push(span("▏   ", palette::DIM));
            hint(&mut spans, "enter", "keep");
            hint(&mut spans, "esc", "clear");
        }
        Focus::Dialog(_) => {
            for (k, d) in [("ctrl-s", "start"), ("tab", "next field"), ("←/→", "change"), ("esc", "cancel")] {
                hint(&mut spans, k, d);
            }
        }
        Focus::List => {
            if !state.filter.is_empty() {
                spans.push(span(format!("/{}   ", state.filter), palette::LABEL));
            }
            for (k, d) in app::key_bar(state) {
                hint(&mut spans, k, d);
            }
        }
    }
    if let Some(toast) = &state.toast {
        let room = usize::from(area.width).saturating_sub(spans_width(&spans) + 1);
        let style = if toast.text.contains("failed") { palette::ERROR } else { palette::PLAIN };
        let text = fit(&one_line(&toast.text), room);
        spans.push(Span::raw(" ".repeat(room.saturating_sub(cells(&text)))));
        spans.push(span(text, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
    let block = Block::new().borders(Borders::LEFT).border_style(palette::DIM).padding(Padding::horizontal(2));
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

/// A light Markdown pass for issue bodies: headings are bold without their
/// `#` marks, code fences become a dim gutter, and runs of blank lines
/// collapse to one. Everything else is shown as written.
fn body_lines(body: &str) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut in_code = false;
    let mut last_blank = true;
    for raw in body.lines().take(300) {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(Line::from(vec![span("│ ", palette::DIM), Span::raw(line.to_string())]));
            last_blank = false;
            continue;
        }
        if line.is_empty() {
            if !last_blank {
                out.push(Line::raw(""));
            }
            last_blank = true;
            continue;
        }
        last_blank = false;
        let heading = line.trim_start_matches('#');
        if heading.len() < line.len() && heading.starts_with(' ') {
            out.push(Line::from(span(heading.trim().to_string(), palette::HEADING)));
        } else {
            out.push(Line::raw(line.to_string()));
        }
    }
    out
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
    lines.extend(body_lines(&i.body));
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
        field("skill", vec![Span::raw(s.skill.as_ref().map_or_else(|| "automatic".to_string(), |k| k.as_str().to_string()))]),
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
    let skill = dialog.choices.get(dialog.skill).cloned().flatten();
    let prompt = dialog.prompt.lines().join("\n");
    let args = rules::claude_args(dialog.mode, &skill, prompt.trim(), None);
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
    let [branch_l, branch, _, skill, _, mode, _, prompt_l, prompt, _, runs, error] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
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

    let chosen = d.choices.get(d.skill).cloned().flatten();
    let hint = match (&chosen, &d.skill_reason) {
        (Some(_), Some(label)) => format!("default from the {label} label"),
        (None, _) => "no /skill prefix; the pstack session hook decides".to_string(),
        _ => "the skill goes at the start of the prompt".to_string(),
    };
    let name = chosen.as_ref().map_or("automatic", |s| s.as_str());
    frame.render_widget(
        Paragraph::new(vec![
            field_label("Skill", focus(DialogField::Skill)),
            Line::from(vec![
                span("  ‹ ", palette::DIM),
                Span::raw(name.to_string()),
                span(" ›   ", palette::DIM),
                span(hint, palette::DIM),
            ]),
        ]),
        skill,
    );
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier};
    use std::path::PathBuf;
    use std::time::Instant;

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
        state.issues.status = FeedStatus::Fresh;
        state.prs.items = vec![pr(10, "fix/3-x", &[3])];
        state.prs.status = FeedStatus::Fresh;
        let id = WindowId::parse("@3").expect("id");
        state.sessions.insert(
            IssueNumber(3),
            Session {
                issue: IssueNumber(3),
                branch: BranchName::parse("fix/3-x").expect("branch"),
                worktree: WorktreePath::planned(PathBuf::from("/w/orbit-3")),
                skill: None,
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
            let _ = app::update(&mut state, Msg::Prs(Ok(fresh)), &config);
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
        assert!(screen.contains("skill    automatic"));
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
        for want in ["Start session for #2 Docs", "Branch", "docs/2-docs", "‹ pstack:technical-writing ›", "default from the docs label", "Implement directly", "Runs", "$ git worktree add ../orbit-2 -b docs/2-docs origin/main", "claude \"/pstack:technical-writing Implement GitHub issue #2: Docs", "ctrl-s start"] {
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

    #[test]
    fn empty_and_loading_lists_say_so() {
        let config = Config::default();
        let (state, _) = app::init(Instant::now(), &config, Context::default());
        assert!(text(&render(&state, &config, 80, 10)).contains("Loading issues…"));
    }
}
