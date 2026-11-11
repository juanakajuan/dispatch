//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: tab bar (1 row), list | detail
//! (40% | 60%), key bar (1 row). The dialog is a centered overlay.

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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// The whole palette. No other module builds a `Style`.
/// Rule: foreground from the 16 ANSI colors only; never a background color;
/// selection is `Modifier::REVERSED`, so it uses the terminal's own colors and
/// the user's theme applies everywhere.
pub mod palette {
    use ratatui::style::{Color, Modifier, Style};

    pub const PLAIN: Style = Style::new();
    pub const HEADING: Style = Style::new().add_modifier(Modifier::BOLD);
    pub const DIM: Style = Style::new().fg(Color::DarkGray);
    pub const NUMBER: Style = Style::new().fg(Color::Cyan);
    pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
    pub const RUNNING: Style = Style::new().fg(Color::Green);
    pub const NEEDS_YOU: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    pub const EXITED: Style = Style::new().fg(Color::DarkGray);
    pub const STARTING: Style = Style::new().fg(Color::Cyan);
    pub const PASS: Style = Style::new().fg(Color::Green);
    pub const FAIL: Style = Style::new().fg(Color::Red);
    pub const PENDING: Style = Style::new().fg(Color::Yellow);
    pub const MERGED: Style = Style::new().fg(Color::Magenta);
    pub const LABEL: Style = Style::new().fg(Color::Magenta);
    pub const KEY: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
    pub const ERROR: Style = Style::new().fg(Color::Red);
}

pub fn draw(frame: &mut Frame, state: &State, config: &Config) -> HitMap {
    let now = jiff::Timestamp::now();
    let [tabs, body, keys] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    let [list, detail] = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(body);
    let mut hits = HitMap::default();
    draw_tabs(frame, tabs, state, &mut hits);
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

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

/// One list row: `left` is cut to fit, `right` stays whole at the edge.
fn row_line(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>, selected: bool) -> Line<'static> {
    let width = usize::from(width);
    let right_w: usize = right.iter().map(Span::width).sum();
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
    let line = Line::from(spans);
    if selected { line.style(palette::SELECTED) } else { line }
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

fn pr_state_span(state: PrState) -> Span<'static> {
    match state {
        PrState::Open => span("open", palette::PASS),
        PrState::Draft => span("draft", palette::DIM),
        PrState::Merged => span("merged", palette::MERGED),
        PrState::Closed => span("closed", palette::FAIL),
    }
}

fn labels_spans(labels: &[String]) -> Vec<Span<'static>> {
    labels.iter().flat_map(|l| [span(format!("[{l}]"), palette::LABEL), Span::raw(" ")]).collect()
}

fn issue_of(state: &State, n: IssueNumber) -> Option<&Issue> {
    state.issues.items.iter().find(|i| i.number == n)
}

// ---- tabs and key bar ------------------------------------------------------

fn draw_tabs(frame: &mut Frame, area: Rect, state: &State, hits: &mut HitMap) {
    let needs = state.sessions.values().filter(|s| s.state() == SessionState::NeedsYou).count();
    let sessions_n = state.sessions.len();
    let tabs = [
        (Tab::Issues, format!("1 Issues {}", state.issues.items.len())),
        (Tab::Prs, format!("2 Pull requests {}", state.prs.items.len())),
        (
            Tab::Sessions,
            if needs > 0 { format!("3 Sessions {sessions_n} ▲{needs}") } else { format!("3 Sessions {sessions_n}") },
        ),
    ];
    let mut spans = Vec::new();
    let mut x = area.x;
    for (tab, label) in tabs {
        let text = format!(" {label} ");
        let w = u16::try_from(cells(&text)).unwrap_or(u16::MAX);
        hits.targets.push((Rect::new(x, area.y, w, 1).intersection(area), HitTarget::Tab(tab)));
        spans.push(span(text, if tab == state.tab { palette::SELECTED } else { palette::PLAIN }));
        spans.push(Span::raw(" "));
        x = x.saturating_add(w + 1);
    }
    let status = match state.tab {
        Tab::Issues => feed_status(&state.issues),
        Tab::Prs => feed_status(&state.prs),
        Tab::Sessions => None,
    };
    if let Some((text, style)) = status {
        let used = usize::from(x.saturating_sub(area.x));
        let room = usize::from(area.width).saturating_sub(used + 1);
        spans.push(span(fit(&text, room), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn feed_status<T>(feed: &Feed<T>) -> Option<(String, Style)> {
    match &feed.status {
        FeedStatus::Loading => Some(("loading…".to_string(), palette::DIM)),
        FeedStatus::Fresh => None,
        FeedStatus::Failed(e) => Some((format!("gh failed, showing old data: {}", one_line(e)), palette::ERROR)),
    }
}

fn draw_keys(frame: &mut Frame, area: Rect, state: &State) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    match &state.focus {
        Focus::Filter => {
            spans.push(span("/ ", palette::KEY));
            spans.push(Span::raw(state.filter.clone()));
            spans.push(span("▏", palette::DIM));
            spans.push(span("   enter keep   esc clear", palette::DIM));
        }
        Focus::Dialog(_) => {
            for (k, d) in [("ctrl-s", "start"), ("esc", "cancel"), ("tab", "next field")] {
                spans.extend([span(k, palette::KEY), span(format!(" {d}  "), palette::DIM)]);
            }
        }
        Focus::List => {
            if !state.filter.is_empty() {
                spans.push(span(format!("filter: {}  ", state.filter), palette::LABEL));
            }
            for (k, d) in app::key_bar(state) {
                spans.extend([span(k, palette::KEY), span(format!(" {d}  "), palette::DIM)]);
            }
        }
    }
    if let Some(toast) = &state.toast {
        let used: usize = spans.iter().map(Span::width).sum();
        let room = usize::from(area.width).saturating_sub(used + 1);
        let style = if toast.text.contains("failed") { palette::ERROR } else { palette::PLAIN };
        spans.push(span(fit(&one_line(&toast.text), room), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---- list ------------------------------------------------------------------

fn draw_list(frame: &mut Frame, area: Rect, state: &State, now: jiff::Timestamp, hits: &mut HitMap) {
    let rows = app::visible_rows(state);
    let selected = app::selected_row(state);
    if rows.is_empty() {
        frame.render_widget(Paragraph::new(span(empty_text(state), palette::DIM)), area);
        return;
    }
    let height = usize::from(area.height);
    let at = selected.and_then(|k| rows.iter().position(|r| *r == k)).unwrap_or(0);
    let top = (at + 1).saturating_sub(height);
    for (i, key) in rows.iter().enumerate().skip(top).take(height) {
        let y = area.y + u16::try_from(i - top).unwrap_or(0);
        let rect = Rect::new(area.x, y, area.width, 1);
        let line = list_row(state, *key, now, area.width, selected == Some(*key));
        frame.render_widget(Paragraph::new(line), rect);
        hits.targets.push((rect, HitTarget::Row(*key)));
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

fn list_row(state: &State, key: RowKey, now: jiff::Timestamp, width: u16, selected: bool) -> Line<'static> {
    match key {
        RowKey::Issue(n) => {
            let Some(i) = issue_of(state, n) else { return Line::raw("") };
            let left = vec![span(format!("#{} ", n.0), palette::NUMBER), Span::raw(one_line(&i.title))];
            let mut right = labels_spans(&i.labels);
            let starting = state.starting.contains(&n);
            right.push(badge(state.sessions.get(&n), starting));
            right.push(span(format!(" {:>3}", age(i.created_at, now)), palette::DIM));
            row_line(width, left, right, selected)
        }
        RowKey::Pr(n) => {
            let Some(p) = state.prs.items.iter().find(|p| p.number == n) else { return Line::raw("") };
            let left = vec![span(format!("#{} ", n.0), palette::NUMBER), Span::raw(one_line(&p.title))];
            let right = vec![
                checks_span(p.checks),
                Span::raw(" "),
                pr_state_span(p.state),
                span(format!(" {:>3}", age(p.created_at, now)), palette::DIM),
            ];
            row_line(width, left, right, selected)
        }
        RowKey::Session(n) => {
            let session = state.sessions.get(&n);
            let title = match (issue_of(state, n), session) {
                (Some(i), _) => one_line(&i.title),
                (None, Some(s)) => s.branch.as_str().to_string(),
                (None, None) => String::new(),
            };
            let left = vec![span(format!("#{} ", n.0), palette::NUMBER), Span::raw(title)];
            let right = vec![badge(session, state.starting.contains(&n))];
            row_line(width, left, right, selected)
        }
    }
}

// ---- detail ----------------------------------------------------------------

fn draw_detail(frame: &mut Frame, area: Rect, state: &State, now: jiff::Timestamp) {
    let block = Block::new().borders(Borders::LEFT).border_style(palette::DIM);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect { x: inner.x + 1, width: inner.width.saturating_sub(1), ..inner };
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

fn draw_issue_detail(frame: &mut Frame, area: Rect, state: &State, n: IssueNumber, now: jiff::Timestamp) {
    let Some(i) = issue_of(state, n) else { return };
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![span(format!("#{} ", n.0), palette::NUMBER), span(i.title.clone(), palette::HEADING)]),
        Line::from(span(
            format!("opened {} ago by {} · {} comments", age(i.created_at, now), i.author, i.comment_count),
            palette::DIM,
        )),
        Line::from(labels_spans(&i.labels)),
        Line::raw(""),
    ];
    let session = state.sessions.get(&n);
    match (session, state.starting.contains(&n)) {
        (Some(s), _) => {
            lines.push(Line::from(vec![badge(Some(s), false), span(format!("  {}", s.branch.as_str()), palette::DIM)]));
            lines.extend(last_n(tail_lines(state, s), 3).iter().map(|l| Line::from(span(format!("  {}", fit(l, usize::from(area.width).saturating_sub(2))), palette::DIM))));
            lines.push(Line::from(vec![span("a", palette::KEY), span(" attach or resume   ", palette::DIM), span("x", palette::KEY), span(" stop", palette::DIM)]));
        }
        (None, true) => lines.push(Line::from(badge(None, true))),
        (None, false) => lines.push(Line::from(vec![span("s", palette::KEY), Span::raw(" Start Claude session")])),
    }
    lines.push(Line::raw(""));
    lines.extend(i.body.lines().take(200).map(|l| Line::raw(l.to_string())));
    let linked: Vec<&PullRequest> = state.prs.items.iter().filter(|p| p.closes.contains(&n)).collect();
    if !linked.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(span("Linked pull requests", palette::HEADING)));
        for p in linked {
            lines.push(Line::from(vec![
                span(format!("#{} ", p.number.0), palette::NUMBER),
                Span::raw(one_line(&p.title)),
                Span::raw(" "),
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
    let checks = match p.checks {
        Checks::Pass => "checks passing",
        Checks::Fail => "checks failing",
        Checks::Pending => "checks running",
        Checks::None => "no checks",
    };
    let mut lines = vec![
        Line::from(vec![span(format!("#{} ", n.0), palette::NUMBER), span(p.title.clone(), palette::HEADING)]),
        Line::from(vec![
            pr_state_span(p.state),
            Span::raw(" · "),
            checks_span(p.checks),
            Span::raw(format!(" {checks}")),
        ]),
        Line::from(span(format!("opened {} ago by {}", age(p.created_at, now), p.author), palette::DIM)),
        Line::from(format!("{} → {}", p.head, p.base)),
        Line::from(vec![span(format!("+{}", p.additions), palette::PASS), Span::raw(" "), span(format!("-{}", p.deletions), palette::FAIL)]),
    ];
    for c in &p.closes {
        lines.push(Line::from(format!("closes #{}", c.0)));
    }
    if let Some(s) = state.sessions.values().find(|s| s.branch.as_str() == p.head) {
        lines.push(Line::from(vec![Span::raw(format!("session #{} ", s.issue.0)), badge(Some(s), false)]));
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
    let mut head = vec![
        Line::from(vec![span(format!("#{} ", n.0), palette::NUMBER), span(title, palette::HEADING)]),
        Line::from(vec![badge(Some(s), false), span(format!("  {}", s.branch.as_str()), palette::DIM)]),
        Line::from(span(format!("worktree {}", s.worktree.as_path().display()), palette::DIM)),
        Line::from(span(
            s.skill.as_ref().map_or_else(|| "automatic skill".to_string(), |k| format!("skill {}", k.as_str())),
            palette::DIM,
        )),
    ];
    if let Some(p) = app::pr_for_session(state, s) {
        head.push(Line::from(vec![
            Span::raw(format!("PR #{} ", p.number.0)),
            checks_span(p.checks),
            Span::raw(" "),
            pr_state_span(p.state),
        ]));
    }
    let head_h = u16::try_from(head.len() + 1).unwrap_or(u16::MAX).min(area.height);
    let [top, rest] = Layout::vertical([Constraint::Length(head_h), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(head), top);
    let width = usize::from(rest.width);
    let tail: Vec<Line<'static>> = last_n(tail_lines(state, s), usize::from(rest.height))
        .iter()
        .map(|l| Line::raw(fit(l, width)))
        .collect();
    let pad = usize::from(rest.height).saturating_sub(tail.len());
    let mut lines: Vec<Line<'static>> = vec![Line::raw(""); pad];
    lines.extend(tail);
    frame.render_widget(Paragraph::new(lines), rest);
}

// ---- dialog ----------------------------------------------------------------

/// The exact commands the dialog will run, for the "Runs:" preview.
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
    input.set_cursor_style(if focused { palette::SELECTED } else { palette::PLAIN });
    input
}

fn field_label(text: &str, focused: bool) -> Line<'static> {
    Line::from(span(text.to_string(), if focused { palette::KEY } else { palette::DIM }))
}

fn draw_dialog(frame: &mut Frame, screen: Rect, state: &State, d: &StartDialog, config: &Config) {
    let width = screen.width.min(100);
    let height = screen.height.min(24);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let block = Block::bordered().title(format!(" Start session for #{} {} ", d.issue.0, fit(&d.title, 50)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [branch_l, branch, skill, mode, prompt_l, prompt, runs, error] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    let focus = |f: DialogField| d.field == f;
    frame.render_widget(Paragraph::new(field_label("Branch", focus(DialogField::Branch))), branch_l);
    frame.render_widget(&styled_input(&d.branch, focus(DialogField::Branch)), branch);

    let chosen = d.choices.get(d.skill).cloned().flatten();
    let hint = match (&chosen, &d.skill_reason) {
        (Some(_), Some(label)) => format!("default from the [{label}] label"),
        (None, _) => "no /skill prefix; the pstack session hook decides".to_string(),
        _ => "the skill goes at the start of the prompt".to_string(),
    };
    let name = chosen.as_ref().map_or("automatic", |s| s.as_str());
    frame.render_widget(
        Paragraph::new(vec![
            field_label("Skill  (left/right)", focus(DialogField::Skill)),
            Line::from(vec![Span::raw(format!("< {name} >  ")), span(hint, palette::DIM)]),
        ]),
        skill,
    );
    let radio = |on: bool, text: &str| format!("({}) {text}", if on { "•" } else { " " });
    frame.render_widget(
        Paragraph::new(vec![
            field_label("Mode  (left/right)", focus(DialogField::Mode)),
            Line::raw(format!(
                "{}   {}",
                radio(d.mode == Mode::PlanFirst, "Plan first, then wait for approval"),
                radio(d.mode == Mode::Implement, "Implement directly")
            )),
        ]),
        mode,
    );
    frame.render_widget(Paragraph::new(field_label("Prompt  (enter adds a line)", focus(DialogField::Prompt))), prompt_l);
    frame.render_widget(&styled_input(&d.prompt, focus(DialogField::Prompt)), prompt);

    let mut run_lines = vec![Line::from(span("Runs", palette::DIM))];
    let width = usize::from(runs.width).saturating_sub(2);
    run_lines.extend(preview(state, d, config).iter().map(|c| Line::from(span(format!("  {}", fit(c, width)), palette::DIM))));
    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(e.clone(), palette::ERROR)), error);
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

    fn is_ansi16(c: Color) -> bool {
        !matches!(c, Color::Rgb(..) | Color::Indexed(_))
    }

    /// Every screen the app can show: no cell has a background, and every
    /// foreground is one of the 16 ANSI colors or the terminal default.
    #[test]
    fn no_cell_has_a_background_or_a_non_ansi_color() {
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
                assert!(is_ansi16(cell.fg), "{name}: fg {:?}", cell.fg);
            }
        }
    }

    #[test]
    fn selection_uses_reversed_and_nothing_else() {
        let (state, config) = full_state();
        let buf = render(&state, &config, 100, 30);
        let reversed: Vec<_> = buf.content().iter().filter(|c| c.modifier.contains(Modifier::REVERSED)).collect();
        assert!(!reversed.is_empty());
        let row_of = |c: &&ratatui::buffer::Cell| c.symbol().to_string();
        assert!(reversed.iter().any(|c| row_of(c) == "#"), "the selected row is reversed");
    }

    #[test]
    fn issues_screen_shows_rows_badges_detail_and_key_bar() {
        let (state, config) = full_state();
        let screen = text(&render(&state, &config, 120, 24));
        for want in [
            "1 Issues 2",
            "2 Pull requests 1",
            "3 Sessions 1 ▲1",
            "#3 Retry queue",
            "▲ needs you",
            "[bug]",
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

    #[test]
    fn sessions_detail_anchors_the_tail_at_the_bottom() {
        let (mut state, config) = full_state();
        send(&mut state, &config, KeyCode::Char('3'));
        let screen = text(&render(&state, &config, 120, 24));
        assert!(screen.contains("worktree /w/orbit-3"));
        assert!(screen.contains("automatic skill"));
        let lines: Vec<&str> = screen.lines().collect();
        let last_tail = lines.iter().rposition(|l| l.contains("second line")).expect("tail drawn");
        assert_eq!(last_tail, lines.len() - 2, "the newest tail line sits just above the key bar");
    }

    #[test]
    fn dialog_shows_fields_and_the_exact_commands() {
        let (mut state, config) = full_state();
        send(&mut state, &config, KeyCode::Char('j'));
        send(&mut state, &config, KeyCode::Char('s'));
        let screen = text(&render(&state, &config, 120, 30));
        for want in ["Start session for #2 Docs", "Branch", "docs/2-docs", "< pstack:technical-writing >", "default from the [docs] label", "Implement directly", "Runs", "git worktree add ../orbit-2 -b docs/2-docs origin/main", "claude \"/pstack:technical-writing Implement GitHub issue #2: Docs", "ctrl-s start"] {
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
        assert_eq!(rows, [(1, RowKey::Issue(IssueNumber(3))), (2, RowKey::Issue(IssueNumber(2)))]);
    }

    #[test]
    fn empty_and_loading_lists_say_so() {
        let config = Config::default();
        let (state, _) = app::init(Instant::now(), &config, Context::default());
        assert!(text(&render(&state, &config, 80, 10)).contains("Loading issues…"));
    }
}
