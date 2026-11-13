//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: the header (tab row, logo, repo name
//! with the age of the shown list, session counts), the grouped rows (one
//! line each), the message row (only when there is a message), the prompt
//! between two rules, and the footer keys. `v` toggles the details of the
//! selected row under the rows. The dialog is a centered overlay.

use crate::app::{
    self, BINDINGS, Binding, DialogField, Feed, FeedStatus, Focus, HitMap, HitTarget, KeyGroup, RowKey, SettingsMenu,
    StartDialog, State, Tab, View,
};
use crate::config::{Config, SETTINGS};
use crate::domain::{Checks, Issue, IssueNumber, Mode, PrState, PullRequest};
use crate::glyphs::Icons;
use crate::markdown;
use crate::rules;
use crate::session::{Session, SessionState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};
use std::time::Instant;

/// The whole palette. No other module builds a `Style`.
/// Rule: Gruvbox (dark) foregrounds, and a background color only on the tab
/// number badges, so the terminal's own background shows through elsewhere.
/// One accent (blue) marks what is active: the tab, the selected row, the
/// focused field, the keys. The other colors carry state only.
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
    pub const BG0: Color = Color::Rgb(0x28, 0x28, 0x28);
    pub const BG2: Color = Color::Rgb(0x50, 0x49, 0x45);

    pub const PLAIN: Style = Style::new().fg(FG);
    pub const HEADING: Style = Style::new().fg(FG).add_modifier(Modifier::BOLD);
    pub const DIM: Style = Style::new().fg(GRAY);
    pub const ACCENT: Style = Style::new().fg(BLUE);
    pub const ACTIVE: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
    /// The number badge of the shown tab, and of the other tabs.
    pub const TAB_KEY_ON: Style = Style::new().fg(BG0).bg(BLUE).add_modifier(Modifier::BOLD);
    pub const TAB_KEY_OFF: Style = Style::new().fg(FG).bg(BG2);
    pub const CURSOR: Style = Style::new().add_modifier(Modifier::REVERSED);
    pub const RUNNING: Style = Style::new().fg(GREEN);
    pub const NEEDS_YOU: Style = Style::new().fg(YELLOW).add_modifier(Modifier::BOLD);
    pub const EXITED: Style = Style::new().fg(GRAY);
    pub const STARTING: Style = Style::new().fg(AQUA);
    pub const PASS: Style = Style::new().fg(GREEN);
    pub const FAIL: Style = Style::new().fg(RED);
    pub const PENDING: Style = Style::new().fg(YELLOW);
    pub const MERGED: Style = Style::new().fg(PURPLE);
    pub const LABEL: Style = Style::new().fg(AQUA);
    pub const ERROR: Style = Style::new().fg(RED);
    pub const WARN: Style = Style::new().fg(YELLOW);
    pub const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
    pub const ITALIC: Style = Style::new().add_modifier(Modifier::ITALIC);
    pub const STRIKE: Style = Style::new().add_modifier(Modifier::CROSSED_OUT);
    pub const CODE: Style = Style::new().fg(AQUA);
    pub const LINK: Style = Style::new().fg(BLUE).add_modifier(Modifier::UNDERLINED);
}

const DETAIL_MAX_WIDTH: u16 = 100;

pub use agent::draw;

/// The dialog or a menu, over the screen of either view.
fn draw_overlay(frame: &mut Frame, state: &State, hits: &mut HitMap) {
    match &state.focus {
        Focus::Dialog(dialog) => draw_dialog(frame, frame.area(), state, dialog),
        Focus::Settings(menu) => draw_settings(frame, frame.area(), state, menu),
        Focus::Keybinds { scroll } => hits.keybinds_max_scroll = draw_keybinds(frame, frame.area(), *scroll),
        Focus::List | Focus::Filter => {}
    }
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

fn badge(session: Option<&Session>, starting: bool, icons: &Icons) -> Span<'static> {
    match (session.map(Session::state), starting) {
        (Some(SessionState::Running), _) => span(format!("{} running", icons.running), palette::RUNNING),
        (Some(SessionState::NeedsYou), _) => span(format!("{} needs you", icons.needs_you), palette::NEEDS_YOU),
        (Some(SessionState::Exited), _) => span(format!("{} exited", icons.exited), palette::EXITED),
        (None, true) => span(format!("{} starting", icons.starting), palette::STARTING),
        (None, false) => Span::raw(""),
    }
}

fn checks_span(checks: Checks, icons: &Icons) -> Span<'static> {
    match checks {
        Checks::Pass => span(icons.check_pass, palette::PASS),
        Checks::Fail => span(icons.check_fail, palette::FAIL),
        Checks::Pending => span(icons.check_pending, palette::PENDING),
        Checks::None => span(icons.check_none, palette::DIM),
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
fn pr_mark(pr: Option<&PullRequest>, icons: &Icons) -> Span<'static> {
    let Some(p) = pr else { return Span::raw("  ") };
    match (p.state, p.checks) {
        (PrState::Merged, _) => span(format!("{} ", icons.pr_merged), palette::MERGED),
        (PrState::Draft, _) => span(format!("{} ", icons.pr_draft), palette::DIM),
        (PrState::Open, Checks::Pass) => span(format!("{} ", icons.pr_open), palette::PASS),
        (PrState::Open, Checks::Fail) => span(format!("{} ", icons.pr_open), palette::FAIL),
        (PrState::Open, Checks::Pending) => span(format!("{} ", icons.pr_open), palette::PENDING),
        (PrState::Open, Checks::None) => span(format!("{} ", icons.pr_open), palette::PLAIN),
        (PrState::Closed, _) => Span::raw("  "),
    }
}

fn issue_of(state: &State, n: IssueNumber) -> Option<&Issue> {
    state.issues.items.iter().find(|i| i.number == n)
}

// ---- header and messages ---------------------------------------------------

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
/// feed. Drawn on the message row above the prompt.
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

// ---- list ------------------------------------------------------------------

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

// ---- detail ----------------------------------------------------------------

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

fn draw_issue_detail(
    frame: &mut Frame,
    area: Rect,
    state: &State,
    n: IssueNumber,
    now: jiff::Timestamp,
    icons: &Icons,
) {
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
            lines.push(field(
                "session",
                vec![badge(Some(s), false, icons), span(format!("  {}", s.branch.as_str()), palette::DIM)],
            ));
            let width = usize::from(area.width).saturating_sub(2);
            lines.extend(
                last_n(tail_lines(state, s), 3)
                    .iter()
                    .map(|l| Line::from(vec![span("│ ", palette::DIM), span(fit(l, width), palette::DIM)])),
            );
            lines.push(Line::raw(""));
        }
        (None, true) => lines.extend([field("session", vec![badge(None, true, icons)]), Line::raw("")]),
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
                checks_span(p.checks, icons),
                Span::raw(" "),
                pr_state_span(p.state),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_pr_detail(
    frame: &mut Frame,
    area: Rect,
    state: &State,
    n: crate::domain::PrNumber,
    now: jiff::Timestamp,
    icons: &Icons,
) {
    let Some(p) = state.prs.items.iter().find(|p| p.number == n) else { return };
    let mut lines = vec![
        Line::from(span(p.title.clone(), palette::HEADING)),
        Line::from(span(format!("#{} · opened {} ago by {}", n.0, age(p.created_at, now), p.author), palette::DIM)),
        Line::raw(""),
        field("state", vec![pr_state_span(p.state)]),
        field("checks", vec![checks_span(p.checks, icons), Span::raw(format!(" {}", checks_word(p.checks)))]),
        field("branch", vec![Span::raw(format!("{} → {}", p.head, p.base))]),
        field(
            "diff",
            vec![
                span(format!("+{}", p.additions), palette::PASS),
                Span::raw(" "),
                span(format!("-{}", p.deletions), palette::FAIL),
            ],
        ),
    ];
    if !p.closes.is_empty() {
        let closes = p.closes.iter().map(|c| format!("#{}", c.0)).collect::<Vec<_>>().join(", ");
        lines.push(field("closes", vec![Span::raw(closes)]));
    }
    if let Some(s) = state.sessions.values().find(|s| s.branch.as_str() == p.head) {
        lines.push(field("session", vec![Span::raw(format!("#{} ", s.issue.0)), badge(Some(s), false, icons)]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_session_detail(frame: &mut Frame, area: Rect, state: &State, n: IssueNumber, icons: &Icons) {
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
        field("state", vec![badge(Some(s), false, icons)]),
        field("branch", vec![Span::raw(s.branch.as_str().to_string())]),
        field(
            "worktree",
            vec![Span::raw(fit_path(
                &s.worktree.as_path().display().to_string(),
                width.saturating_sub(FIELD_KEY_CELLS),
            ))],
        ),
    ];
    if let Some(p) = app::pr_for_session(state, s) {
        head.push(field(
            "pr",
            vec![
                Span::raw(format!("#{} ", p.number.0)),
                checks_span(p.checks, icons),
                Span::raw(" "),
                pr_state_span(p.state),
            ],
        ));
    }
    head.push(Line::raw(""));
    head.push(Line::from(span(format!("output {}", "─".repeat(width.saturating_sub(7))), palette::DIM)));
    let head_h = u16::try_from(head.len()).unwrap_or(u16::MAX).min(area.height);
    let [top, rest] = Layout::vertical([Constraint::Length(head_h), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(head), top);
    let tail: Vec<Line<'static>> =
        last_n(tail_lines(state, s), usize::from(rest.height)).iter().map(|l| Line::raw(fit(l, width))).collect();
    frame.render_widget(Paragraph::new(tail), rest);
}

// ---- dialog ----------------------------------------------------------------

/// The exact commands the dialog will run, for the "Runs" preview.
fn preview(state: &State, dialog: &StartDialog) -> Vec<String> {
    let branch = dialog.branch.lines().join("");
    let prompt = dialog.prompt.lines().join("\n");
    let args = rules::claude_args(dialog.mode, prompt.trim(), None);
    let shown: Vec<String> = state
        .config
        .claude_command
        .argv(&args)
        .into_iter()
        .map(|a| if a.contains([' ', '\n', '"']) { format!("{:?}", one_line(&a)) } else { a })
        .collect();
    let base = if state.context.base.is_empty() { "origin/main" } else { state.context.base.as_str() };
    vec![
        format!("git worktree add {}/{} -b {} {base}", rules::WORKTREES_DIR, dialog.issue.0, branch.trim()),
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

fn draw_dialog(frame: &mut Frame, screen: Rect, state: &State, d: &StartDialog) {
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

    let icons = state.config.icons.icons();
    let radio = |on: bool, text: &str| -> Vec<Span<'static>> {
        if on {
            vec![span(format!("{} ", icons.radio_on), palette::ACCENT), Span::raw(text.to_string())]
        } else {
            vec![span(format!("{} {text}", icons.radio_off), palette::DIM)]
        }
    };
    let mut modes = vec![Span::raw("  ")];
    modes.extend(radio(d.mode == Mode::PlanFirst, "Plan first, then wait for approval"));
    modes.push(Span::raw("    "));
    modes.extend(radio(d.mode == Mode::Implement, "Implement directly"));
    frame.render_widget(Paragraph::new(vec![field_label("Mode", focus(DialogField::Mode)), Line::from(modes)]), mode);
    frame.render_widget(Paragraph::new(field_label("Prompt", focus(DialogField::Prompt))), prompt_l);
    frame.render_widget(&styled_input(&d.prompt, focus(DialogField::Prompt)), indent(prompt));

    let mut run_lines = vec![Line::from(span("  Runs", palette::DIM))];
    let width = usize::from(runs.width).saturating_sub(4);
    run_lines
        .extend(preview(state, d).iter().map(|c| Line::from(span(format!("  $ {}", fit(c, width)), palette::DIM))));
    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(format!("  {e}"), palette::ERROR)), error);
    }
}

// ---- keybinds --------------------------------------------------------------

/// The keybinds menu lines, made from `BINDINGS` and grouped by `KeyGroup`.
fn keybinds_lines() -> Vec<Line<'static>> {
    let keys = |b: &Binding| b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
    let key_width = BINDINGS.iter().map(|b| cells(&keys(b))).max().unwrap_or(0);
    let mut lines = Vec::new();
    for group in KeyGroup::ALL {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(span(group.title(), palette::HEADING)));
        for b in BINDINGS.iter().filter(|b| b.group == group) {
            lines.push(Line::from(vec![
                span(format!("  {:key_width$}  ", keys(b)), palette::ACCENT),
                span(b.desc, palette::PLAIN),
            ]));
        }
    }
    lines
}

/// Draws the menu scrolled down by `scroll` lines, and returns how far it
/// can scroll at this size.
fn draw_keybinds(frame: &mut Frame, screen: Rect, scroll: u16) -> u16 {
    let lines = keybinds_lines();
    let wanted: u16 = lines.len().try_into().unwrap_or(u16::MAX);
    let width = screen.width.min(64);
    // The margin keeps the footer and its `esc to close` in view.
    let height = screen.height.saturating_sub(2).min(wanted.saturating_add(3));
    let area = Rect::new(screen.x + (screen.width - width) / 2, screen.y + (screen.height - height) / 2, width, height);
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(" Keys ", palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let max_scroll = wanted.saturating_sub(inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.min(max_scroll), 0)), inner);
    max_scroll
}

// ---- settings --------------------------------------------------------------

fn draw_settings(frame: &mut Frame, screen: Rect, state: &State, menu: &SettingsMenu) {
    let width = screen.width.min(76);
    let height = screen.height.min(SETTINGS.len().try_into().unwrap_or(4) + 8);
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
        .title(span(" Settings ", palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let default = Config::default();
    let name_width = SETTINGS.iter().map(|s| s.key().len()).max().unwrap_or(0);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for setting in SETTINGS {
        let on = setting == menu.at;
        let room = usize::from(inner.width).saturating_sub(4 + name_width);
        let mut spans = vec![
            span(if on { "› " } else { "  " }, palette::ACCENT),
            span(format!("{:name_width$}  ", setting.key()), if on { palette::ACCENT } else { palette::PLAIN }),
        ];
        if let (Some(edit), true) = (&menu.edit, on) {
            let text: Vec<char> = edit.input.lines().join("").chars().collect();
            let shown: String = text[text.len().saturating_sub(room.saturating_sub(2))..].iter().collect();
            spans.push(span(format!("{shown}▏"), palette::ACTIVE));
        } else {
            let default_note = format!("   default: {}", setting.show(&default));
            let value = fit(&setting.show(&state.config), room.saturating_sub(cells(&default_note)).max(1));
            spans.push(span(value, if on { palette::ACTIVE } else { palette::PLAIN }));
            spans.push(span(default_note, palette::DIM));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    let error = menu.edit.as_ref().and_then(|e| e.error.as_ref());
    lines.push(match error {
        Some(e) => Line::from(span(fit(e, usize::from(inner.width)), palette::ERROR)),
        None if menu.at.new_sessions_only() => {
            Line::from(span("Applies to sessions you start or resume after this change.", palette::DIM))
        }
        None => Line::raw(""),
    });
    lines.push(Line::from(span(
        if menu.edit.is_some() { "enter save · esc cancel" } else { "j/k move · enter change · esc close" },
        palette::DIM,
    )));
    lines.push(Line::from(span("Saved to config.json", palette::DIM)));
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Context, Msg};
    use crate::domain::{BranchName, WorktreePath};
    use crate::glyphs::IconSet;
    use crate::session::{Window, WindowId};
    use crate::testutil::{issue, pr};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
        for (secs, want) in [
            (0, "now"),
            (59, "now"),
            (60, "1m"),
            (3599, "59m"),
            (3600, "1h"),
            (86_399, "23h"),
            (86_400, "1d"),
            (9 * 86_400, "9d"),
            (40 * 86_400, "1mo"),
            (400 * 86_400, "1y"),
        ] {
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
        assert_eq!(
            fit_path("/a/very-long-final-segment", 10),
            "…l-segment",
            "a segment wider than the space is cut by cells"
        );
        assert_eq!(fit_path("/anything", 0), "");
    }

    #[test]
    fn sessions_detail_shows_the_end_of_a_long_worktree_path() {
        let mut state = full_state();
        let long = "/home/someone/projects/a-very-long-directory-name/another-long-directory/scratchpad/smoke/hf-1017";
        if let Some(s) = state.sessions.get_mut(&IssueNumber(3)) {
            s.worktree = WorktreePath::planned(PathBuf::from(long));
        }
        state.view = View::AgentPreview;
        send(&mut state, KeyCode::Char('3'));
        let screen = text(&render(&state, 80, 40));
        let line = screen.lines().find(|l| l.contains("worktree ")).expect("worktree line drawn");
        assert!(
            line.contains("…/") && line.trim_end().ends_with("/smoke/hf-1017"),
            "path end not visible in {line:?}\n{screen}"
        );
    }

    fn full_state() -> State {
        let (mut state, _) = app::init(
            Instant::now(),
            Config::default(),
            Context { repo_name: "orbit".to_string(), base: "origin/main".to_string(), ..Context::default() },
        );
        state.issues.items = vec![
            issue(
                3,
                "Retry queue drops jobs when Redis reconnects after a very long title that must be cut",
                &["bug", "p1"],
            ),
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
        state
    }

    fn render(state: &State, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal
            .draw(|f| {
                let _ = draw(f, state);
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        buf.content()
            .chunks(usize::from(buf.area.width))
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn send(state: &mut State, code: KeyCode) {
        let _ = app::update(state, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn in_palette(c: Color) -> bool {
        use palette::{AQUA, BG0, BLUE, FG, GRAY, GREEN, PURPLE, RED, YELLOW};
        [FG, GRAY, RED, GREEN, YELLOW, BLUE, PURPLE, AQUA, BG0].contains(&c)
    }

    /// Every screen the app can show: no cell has a background, and every
    /// foreground is a Gruvbox palette color.
    #[test]
    fn no_cell_has_a_background_and_every_foreground_is_gruvbox() {
        let mut state = full_state();
        let mut screens: Vec<(String, Buffer)> = Vec::new();
        for view in [View::Agent, View::AgentPreview] {
            state.view = view;
            for (name, key) in [("issues", '1'), ("prs", '2'), ("sessions", '3')] {
                send(&mut state, KeyCode::Char(key));
                screens.push((format!("{view:?} {name}"), render(&state, 100, 30)));
            }
        }
        send(&mut state, KeyCode::Char('1'));
        send(&mut state, KeyCode::Char('j'));
        send(&mut state, KeyCode::Char('s'));
        screens.push(("dialog".to_string(), render(&state, 100, 30)));
        screens.push(("narrow".to_string(), render(&state, 30, 12)));
        for (name, buf) in screens {
            for cell in buf.content() {
                let badge = [palette::BLUE, palette::BG2].contains(&cell.bg);
                assert!(cell.bg == Color::Reset || badge, "{name}: background at {:?}", cell.symbol());
                assert_eq!(cell.underline_color, Color::Reset, "{name}: underline color");
                assert!(in_palette(cell.fg), "{name}: fg {:?} at {:?}", cell.fg, cell.symbol());
            }
        }
    }

    #[test]
    fn selection_is_an_accent_marker_on_one_row() {
        let state = full_state();
        let buf = render(&state, 100, 30);
        let screen = text(&buf);
        let rows: Vec<(usize, &str)> =
            screen.lines().enumerate().filter(|(_, l)| l.contains("❯") && l.contains('#')).collect();
        assert_eq!(rows.len(), 1, "one selected row in\n{screen}");
        let (y, line) = rows[0];
        assert!(line.contains("#3 Retry"), "the first row is selected: {line:?}");
        let x = u16::try_from(cells(&line[..line.find('❯').expect("marker")])).expect("x");
        assert_eq!(buf[(x, u16::try_from(y).expect("y"))].fg, palette::BLUE);
        assert!(
            buf.content().iter().all(|c| !c.modifier.contains(Modifier::REVERSED)),
            "no reversed block outside a text cursor"
        );
    }

    #[test]
    fn issues_screen_shows_groups_rows_detail_and_key_bar() {
        let mut state = full_state();
        state.view = View::AgentPreview;
        let screen = text(&render(&state, 120, 30));
        for want in [
            "1  Issues 2",
            "2  Pull requests 1",
            "3  Sessions 1",
            "▲ 1 needs you",
            "Needs you 1",
            "▲ #3 Retry queue",
            "PR #10 open",
            "Open 1",
            "· #2 Docs",
            "── #3 ──",
            "bug  p1",
            "session  ▲ needs you  fix/3-x",
            "│ fake output",
            "s to start session",
            "a to attach",
            "x to stop",
            "S sort: attention · v layout",
        ] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    /// The glyph cell left of issue `n`'s number on its row.
    fn mark_cell(buf: &Buffer, n: u32) -> (String, Color) {
        let screen = text(buf);
        let needle = format!("#{n} ");
        let (y, line) =
            screen.lines().enumerate().find(|(_, l)| l.contains(&needle) && !l.contains("──")).expect("row drawn");
        let x = cells(&line[..line.find(&needle).expect("number")]) - 2;
        let cell = &buf[(u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))];
        (cell.symbol().to_string(), cell.fg)
    }

    #[test]
    fn issue_rows_show_the_lead_pr_state_and_follow_a_refresh() {
        let mut state = full_state();
        state.sessions = crate::session::Sessions::new();
        let mark = |state: &State| (mark_cell(&render(state, 100, 20), 3), mark_cell(&render(state, 100, 20), 2));
        assert_eq!(
            mark(&state),
            (("●".to_string(), palette::FG), ("·".to_string(), palette::GRAY)),
            "open, no checks; #2 has no PR"
        );
        for (pr_state, checks, want, color) in [
            (PrState::Draft, Checks::Pass, "◌", palette::GRAY),
            (PrState::Open, Checks::Pending, "●", palette::YELLOW),
            (PrState::Open, Checks::Fail, "●", palette::RED),
            (PrState::Open, Checks::Pass, "●", palette::GREEN),
            (PrState::Merged, Checks::Fail, "✓", palette::PURPLE),
            (PrState::Closed, Checks::Pass, "·", palette::GRAY),
        ] {
            let fresh = vec![PullRequest { state: pr_state, checks, ..pr(10, "fix/3-x", &[3]) }];
            let _ = app::update(&mut state, Msg::Prs { result: Ok(fresh), at: Instant::now() });
            assert_eq!(mark(&state).0, (want.to_string(), color), "{pr_state:?} {checks:?}");
        }
        let screen = text(&render(&state, 100, 20));
        assert_eq!(column_of(&screen, "#3 ", "#3 "), column_of(&screen, "#2 ", "#2 "), "titles stay in one column");
    }

    /// The column of `needle` on the first screen line that holds `anchor`.
    fn column_of(screen: &str, anchor: &str, needle: &str) -> usize {
        let line = screen.lines().find(|l| l.contains(anchor)).unwrap_or_else(|| panic!("no {anchor:?} in\n{screen}"));
        let byte = line.find(needle).unwrap_or_else(|| panic!("no {needle:?} in {line:?}"));
        cells(&line[..byte])
    }

    #[test]
    fn the_nerd_font_set_draws_its_glyphs_in_the_same_columns_as_unicode() {
        let mut state = full_state();
        state.view = View::AgentPreview;
        let plain = text(&render(&state, 120, 24));
        state.config.icons = IconSet::NerdFont;
        let icons = state.config.icons.icons();
        let buf = render(&state, 120, 24);
        let screen = text(&buf);
        assert_eq!(mark_cell(&buf, 3), (icons.needs_you.to_string(), palette::YELLOW), "the session glyph cell");
        assert_eq!(mark_cell(&buf, 2).0, icons.idle, "the idle glyph cell");
        assert!(screen.contains(&format!("{} 1 needs you", icons.needs_you)), "header count\n{screen}");
        for (anchor, needle) in [("#3 ", "Retry queue"), ("#2 ", "Docs"), ("#3 ", "PR #10"), ("needs you", "needs you")]
        {
            assert_eq!(column_of(&screen, anchor, needle), column_of(&plain, anchor, needle), "{needle:?} moved");
        }
    }

    #[test]
    fn sessions_detail_shows_the_facts_and_the_tail_under_the_output_rule() {
        let mut state = full_state();
        state.view = View::AgentPreview;
        send(&mut state, KeyCode::Char('3'));
        let screen = text(&render(&state, 120, 40));
        assert!(screen.contains("worktree /w/orbit-3"), "{screen}");
        let lines: Vec<&str> = screen.lines().collect();
        let rule = lines.iter().position(|l| l.contains("output ─")).expect("output rule drawn");
        assert!(lines[rule + 1].contains("fake output"));
        assert!(lines[rule + 2].contains("second line"));
    }

    #[test]
    fn dialog_shows_fields_and_the_exact_commands() {
        let mut state = full_state();
        send(&mut state, KeyCode::Char('j'));
        send(&mut state, KeyCode::Char('s'));
        let screen = text(&render(&state, 120, 30));
        for want in [
            "Start session for #2 Docs",
            "Branch",
            "docs/2-docs",
            "Implement directly",
            "Runs",
            "$ git worktree add .claude/worktrees/2 -b docs/2-docs origin/main",
            "claude \"Implement GitHub issue #2: Docs",
            "ctrl-s to start",
        ] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn draw_returns_hits_for_tabs_and_visible_rows() {
        let state = full_state();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = draw(f, &state)).expect("draw");
        let screen = text(terminal.backend().buffer());
        let line_of =
            |needle: &str| u16::try_from(screen.lines().position(|l| l.contains(needle)).expect(needle)).expect("y");
        let tabs = hits.targets.iter().filter(|(_, t)| matches!(t, HitTarget::Tab(_))).count();
        let rows: Vec<_> = hits
            .targets
            .iter()
            .filter_map(|(r, t)| match t {
                HitTarget::Row(k) => Some((r.y, *k)),
                HitTarget::Tab(_) => None,
            })
            .collect();
        assert_eq!(tabs, 3);
        assert_eq!(
            rows,
            [(line_of("#3 Retry"), RowKey::Issue(IssueNumber(3))), (line_of("#2 Docs"), RowKey::Issue(IssueNumber(2)))],
            "group headings and blank lines are not rows"
        );
    }

    #[test]
    fn tab_keys_look_like_key_hints_and_counts_like_counts() {
        let mut state = full_state();
        state.issues.items.clear();
        let buf = frame(&mut state, 120, 20);
        let y = 1;
        let row = text(&buf).lines().nth(usize::from(y)).expect("tab row").to_string();
        let column = |needle: &str| u16::try_from(cells(&row[..row.find(needle).expect(needle)])).expect("x");
        for (label, tab) in
            [(" 1  Issues 0", Tab::Issues), (" 2  Pull requests 1", Tab::Prs), (" 3  Sessions 1", Tab::Sessions)]
        {
            let x = column(label);
            let badge = if tab == state.tab { palette::BLUE } else { palette::BG2 };
            assert_eq!(buf[(x + 1, y)].bg, badge, "{label}: the key sits on a badge, blue for the shown tab");
            let last = x + u16::try_from(cells(label)).expect("w") - 1;
            if tab != state.tab {
                assert_eq!(buf[(last, y)].fg, palette::GRAY, "{label}: the count of another tab is dim");
            }
            let hit = |cx: u16| state.hits.targets.iter().find(|(r, _)| r.contains((cx, y).into())).map(|(_, t)| *t);
            assert_eq!(hit(x), Some(HitTarget::Tab(tab)), "{label}: a click on the badge selects the tab");
            assert_eq!(hit(last), Some(HitTarget::Tab(tab)), "{label}: a click on the count selects the tab");
            assert_eq!(hit(last + 1), None, "{label}: the hit area ends at the label");
        }
    }

    /// Draws like the shell does: the frame's hits go back into `state`.
    fn frame(state: &mut State, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = draw(f, state)).expect("draw");
        state.hits = hits;
        terminal.backend().buffer().clone()
    }

    #[test]
    fn a_failed_feed_shows_on_the_message_row_above_the_prompt() {
        let mut state = full_state();
        state.issues.status = FeedStatus::Failed { error: "rate limited".to_string(), at: Instant::now() };
        let screen = text(&render(&state, 100, 20));
        let lines: Vec<&str> = screen.lines().collect();
        let at = lines.iter().position(|l| l.contains("gh failed, showing old data: rate limited"));
        let at = at.unwrap_or_else(|| panic!("missing failure in\n{screen}"));
        assert!(lines[at + 1].contains("────") && lines[at + 2].contains("❯ /"), "not above the prompt:\n{screen}");
    }

    #[test]
    fn the_footer_shows_the_shown_tabs_sort_mode() {
        let mut state = full_state();
        let footer = |state: &State| text(&render(state, 120, 24)).lines().last().unwrap_or_default().to_string();
        assert!(footer(&state).trim_end().ends_with("S sort: attention · v layout"), "{}", footer(&state));
        let _ = app::update(&mut state, Msg::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)));
        assert!(footer(&state).contains("S sort: newest"));
        send(&mut state, KeyCode::Char('2'));
        assert!(footer(&state).contains("S sort: attention"), "each tab has its own mode");
    }

    #[test]
    fn since_counts_seconds_then_minutes_then_hours() {
        let at = Instant::now();
        for (secs, want) in [(0, "0s"), (12, "12s"), (59, "59s"), (60, "1m"), (3599, "59m"), (3600, "1h"), (7300, "2h")]
        {
            assert_eq!(since(at, at + Duration::from_secs(secs)), want, "{secs}s");
        }
        assert_eq!(since(at + Duration::from_secs(5), at), "0s", "a response stamped after now is not negative");
    }

    /// The header text after the repo name, and the style of its first character.
    fn freshness_on_screen(state: &State) -> (String, Style) {
        let buf = render(state, 120, 20);
        let screen = text(&buf);
        let (y, row) = screen.lines().enumerate().find(|(_, l)| l.contains("orbit")).expect("repo line");
        let Some(x) = row.find(" · ").map(|x| x + " · ".len()) else {
            return (String::new(), palette::PLAIN);
        };
        let cell = &buf[(u16::try_from(cells(&row[..x])).expect("x"), u16::try_from(y).expect("y"))];
        (row[x..].trim_end().to_string(), Style::new().fg(cell.fg))
    }

    fn ago(secs: u64) -> Instant {
        Instant::now().checked_sub(Duration::from_secs(secs)).expect("uptime above a minute")
    }

    #[test]
    fn the_header_shows_how_old_the_list_is_and_each_fetch_state() {
        let mut state = full_state();
        state.issues.status = FeedStatus::Fresh { at: ago(12) };
        assert_eq!(freshness_on_screen(&state), ("updated 12s ago".to_string(), palette::DIM));

        state.inflight.issues = true;
        assert_eq!(freshness_on_screen(&state), ("refreshing…".to_string(), palette::DIM));

        state.inflight.issues = false;
        state.issues.status = FeedStatus::Failed { error: "offline".to_string(), at: ago(40) };
        assert_eq!(freshness_on_screen(&state), ("refresh failed 40s ago".to_string(), palette::WARN));

        state.prs.status = FeedStatus::Fresh { at: ago(125) };
        send(&mut state, KeyCode::Char('2'));
        assert_eq!(freshness_on_screen(&state).0, "updated 2m ago", "each tab shows its own feed");

        send(&mut state, KeyCode::Char('3'));
        assert_eq!(freshness_on_screen(&state).0, "", "sessions are not fetched from GitHub, so no age");
    }

    #[test]
    fn empty_and_loading_lists_say_so() {
        let (state, _) = app::init(Instant::now(), Config::default(), Context::default());
        assert!(text(&render(&state, 80, 10)).contains("Loading issues…"));
    }

    #[test]
    fn the_settings_menu_shows_each_key_value_and_default_in_both_views() {
        for view in [View::Agent, View::AgentPreview] {
            let mut state = full_state();
            state.view = view;
            state.config.detect_waiting = false;
            send(&mut state, KeyCode::Char(','));
            let screen = text(&render(&state, 100, 30));
            for want in [
                " Settings ",
                "detect_waiting  off   default: on",
                "icons           unicode   default: unicode",
                "claude_command  claude   default: claude",
                "Applies to sessions you start or resume after this change.",
                "j/k move · enter change · esc close",
                "Saved to config.json",
            ] {
                assert!(screen.contains(want), "{view:?} missing {want:?} in\n{screen}");
            }
            assert!(screen.contains("› detect_waiting"), "the selected row is marked\n{screen}");
        }
    }

    #[test]
    fn the_settings_menu_shows_the_editor_and_its_error() {
        let mut state = full_state();
        send(&mut state, KeyCode::Char(','));
        send(&mut state, KeyCode::Char('j'));
        send(&mut state, KeyCode::Char('j'));
        send(&mut state, KeyCode::Enter);
        for c in " 'oops".chars() {
            send(&mut state, KeyCode::Char(c));
        }
        send(&mut state, KeyCode::Enter);
        let screen = text(&render(&state, 100, 30));
        for want in ["claude_command  claude 'oops▏", "has a quote that never closes", "enter save · esc cancel"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
        assert!(!screen.contains("Applies to sessions"), "the error takes the note's line\n{screen}");
    }

    /// Draws like the main loop does, so the reducer sees this frame's size.
    fn draw_into(state: &mut State, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        let mut hits = HitMap::default();
        terminal.draw(|f| hits = draw(f, state)).expect("draw");
        state.hits = hits;
        text(terminal.backend().buffer())
    }

    #[test]
    fn the_keybinds_menu_shows_every_key_in_bindings() {
        let mut state = full_state();
        send(&mut state, KeyCode::Char('?'));
        let screen = draw_into(&mut state, 100, 60);
        for group in KeyGroup::ALL {
            assert!(screen.contains(group.title()), "missing heading {:?} in\n{screen}", group.title());
        }
        for b in BINDINGS {
            let keys = b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
            let shown = screen.lines().any(|l| l.contains(&format!(" {keys} ")) && l.contains(b.desc));
            assert!(shown, "no row shows {keys:?} with {:?} in\n{screen}", b.desc);
        }
        for want in ["j / down", "q / ctrl+c", "esc ", "j/k to scroll · esc to close"] {
            assert!(screen.contains(want), "missing {want:?} in\n{screen}");
        }
    }

    #[test]
    fn a_short_terminal_scrolls_the_keybinds_menu_with_j_and_k() {
        let mut state = full_state();
        send(&mut state, KeyCode::Char('?'));
        let screen = draw_into(&mut state, 100, 16);
        assert!(screen.contains("Navigation"), "the menu starts at the top\n{screen}");
        assert!(!screen.contains("show all keys"), "the end is below the menu\n{screen}");
        let max = state.hits.keybinds_max_scroll;
        assert!(max > 0);
        for _ in 0..u32::from(max) + 5 {
            send(&mut state, KeyCode::Char('j'));
        }
        assert!(matches!(state.focus, Focus::Keybinds { scroll } if scroll == max), "j stops at the end");
        let screen = draw_into(&mut state, 100, 16);
        assert!(screen.contains("show all keys"), "j scrolls to the end\n{screen}");
        assert!(!screen.contains("Navigation"), "the top scrolled out\n{screen}");
        send(&mut state, KeyCode::Char('k'));
        assert!(matches!(state.focus, Focus::Keybinds { scroll } if scroll == max - 1), "one k moves back one line");
    }
}

// ---- layout ----------------------------------------------------------------

mod agent {
    use super::{
        Checks, Constraint, DETAIL_MAX_WIDTH, Focus, Frame, HitMap, HitTarget, Icons, Instant, IssueNumber, Layout,
        Line, Paragraph, PrState, PullRequest, Rect, RowKey, Session, SessionState, Span, State, Tab, View, age, app,
        cells, centered, checks_span, checks_word, draw_issue_detail, draw_overlay, draw_pr_detail,
        draw_session_detail, empty_text, fit, freshness, issue_of, message, one_line, palette, pr_mark, pr_state_span,
        row_line, span, spans_width,
    };

    const LOGO: [&str; 3] = [" ▄██▄ ", "██▀▀██", "▀▘  ▝▀"];

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
        let list = if state.view == View::AgentPreview {
            let [list, sep, detail] =
                Layout::vertical([Constraint::Percentage(55), Constraint::Length(1), Constraint::Min(3)]).areas(body);
            draw_separator(frame, sep, state);
            let detail =
                Rect { x: detail.x + 2, width: detail.width.saturating_sub(4).min(DETAIL_MAX_WIDTH), ..detail };
            match app::selected_row(state) {
                Some(RowKey::Issue(n)) => draw_issue_detail(frame, detail, state, n, now, icons),
                Some(RowKey::Pr(n)) => draw_pr_detail(frame, detail, state, n, now, icons),
                Some(RowKey::Session(n)) => draw_session_detail(frame, detail, state, n, icons),
                None => {}
            }
            list
        } else {
            body
        };
        draw_rows(frame, list, state, now, icons, &mut hits);
        if let Some((text, style)) = message {
            frame.render_widget(Paragraph::new(span(format!(" {}", fit(&text, usize::from(msg.width))), style)), msg);
        }
        let rule = || Paragraph::new(span("─".repeat(usize::from(screen.width)), palette::DIM));
        frame.render_widget(rule(), rule1);
        frame.render_widget(rule(), rule2);
        draw_prompt(frame, prompt, state);
        draw_footer(frame, footer, state);
        draw_overlay(frame, state, &mut hits);
        hits
    }

    fn draw_header(frame: &mut Frame, area: Rect, state: &State, icons: &Icons, hits: &mut HitMap) {
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

        let count = |want: SessionState| state.sessions.values().filter(|s| s.state() == want).count();
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

    fn draw_separator(frame: &mut Frame, area: Rect, state: &State) {
        let label = match app::selected_row(state) {
            Some(RowKey::Issue(n) | RowKey::Session(n)) => format!(" #{} ", n.0),
            Some(RowKey::Pr(n)) => format!(" PR #{} ", n.0),
            None => String::new(),
        };
        let rest = usize::from(area.width).saturating_sub(2 + cells(&label));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                span("──", palette::DIM),
                span(label, palette::ACCENT),
                span("─".repeat(rest), palette::DIM),
            ])),
            area,
        );
    }

    /// Glyph, name, summary, age. One line per row.
    fn row_parts(
        state: &State,
        key: RowKey,
        now: jiff::Timestamp,
        icons: &Icons,
    ) -> (Span<'static>, String, Vec<Span<'static>>, String) {
        let session_glyph =
            |n: IssueNumber| match (state.sessions.get(&n).map(Session::state), state.starting.contains(&n)) {
                (Some(SessionState::Running), _) => Some(span(icons.running, palette::RUNNING)),
                (Some(SessionState::NeedsYou), _) => Some(span(icons.needs_you, palette::NEEDS_YOU)),
                (Some(SessionState::Exited), _) => Some(span(icons.exited, palette::EXITED)),
                (None, true) => Some(span(icons.starting, palette::STARTING)),
                (None, false) => None,
            };
        let pr_summary = |p: &PullRequest| {
            let mut v = vec![span(format!("PR #{} ", p.number.0), palette::DIM), pr_state_span(p.state)];
            if p.checks != Checks::None {
                v.extend([
                    span(" · ", palette::DIM),
                    checks_span(p.checks, icons),
                    span(format!(" {}", checks_word(p.checks)), palette::DIM),
                ]);
            }
            v
        };
        match key {
            RowKey::Issue(n) => {
                let Some(i) = issue_of(state, n) else {
                    return (Span::raw(" "), String::new(), Vec::new(), String::new());
                };
                let glyph = session_glyph(n).unwrap_or_else(|| {
                    let m = pr_mark(app::lead_pr(state, n), icons);
                    if m.content.trim().is_empty() {
                        span(icons.idle, palette::DIM)
                    } else {
                        span(m.content.trim().to_string(), m.style)
                    }
                });
                let summary = match (said(state, n), app::lead_pr(state, n)) {
                    (Some(said), _) => said,
                    (None, Some(p)) => pr_summary(p),
                    (None, None) if !i.labels.is_empty() => {
                        vec![span(format!("{} · {}", i.author, i.labels.join(", ")), palette::DIM)]
                    }
                    (None, None) => vec![span(format!("{} · {} comments", i.author, i.comment_count), palette::DIM)],
                };
                (glyph, format!("#{} {}", n.0, one_line(&i.title)), summary, age(i.updated_at, now))
            }
            RowKey::Pr(n) => {
                let Some(p) = state.prs.items.iter().find(|p| p.number == n) else {
                    return (Span::raw(" "), String::new(), Vec::new(), String::new());
                };
                let mut summary = vec![
                    span(format!("{} → {}  ", p.head, p.base), palette::DIM),
                    span(format!("+{}", p.additions), palette::PASS),
                    span(" ", palette::DIM),
                    span(format!("-{}", p.deletions), palette::FAIL),
                    span(format!(" · {}", checks_word(p.checks)), palette::DIM),
                ];
                if !p.closes.is_empty() {
                    let c = p.closes.iter().map(|c| format!("#{}", c.0)).collect::<Vec<_>>().join(", ");
                    summary.push(span(format!(" · closes {c}"), palette::DIM));
                }
                let glyph = match p.state {
                    PrState::Merged => span(icons.pr_merged, palette::MERGED),
                    PrState::Draft => span(icons.pr_draft, palette::DIM),
                    PrState::Closed => span(icons.pr_closed, palette::DIM),
                    PrState::Open => checks_span(p.checks, icons),
                };
                (glyph, format!("#{} {}", n.0, one_line(&p.title)), summary, age(p.updated_at, now))
            }
            RowKey::Session(n) => {
                let glyph = session_glyph(n).unwrap_or(span(icons.idle, palette::DIM));
                let issue = issue_of(state, n);
                let session = state.sessions.get(&n);
                let name = issue
                    .map(|i| one_line(&i.title))
                    .or_else(|| session.map(|s| s.branch.as_str().to_string()))
                    .unwrap_or_default();
                let summary = said(state, n)
                    .or_else(|| session.and_then(|s| app::pr_for_session(state, s)).map(pr_summary))
                    .or_else(|| session.map(|s| vec![span(s.branch.as_str().to_string(), palette::DIM)]))
                    .unwrap_or_else(|| vec![span("creating worktree…", palette::STARTING)]);
                let age = issue.map(|i| age(i.updated_at, now)).unwrap_or_default();
                (glyph, format!("#{} {}", n.0, name), summary, age)
            }
        }
    }

    /// What the session's Claude last said: its recap, or its last message.
    fn said(state: &State, n: IssueNumber) -> Option<Vec<Span<'static>>> {
        let s = state.summaries.get(&n)?;
        let mut v = Vec::new();
        if s.kind == crate::transcript::SummaryKind::Recap {
            v.push(span("※ ", palette::ACCENT));
        }
        v.push(span(one_line(&s.text), palette::PLAIN));
        Some(v)
    }

    fn draw_rows(frame: &mut Frame, area: Rect, state: &State, now: jiff::Timestamp, icons: &Icons, hits: &mut HitMap) {
        let rows = app::visible_rows(state);
        if rows.is_empty() {
            frame.render_widget(Paragraph::new(span(format!(" {}", empty_text(state)), palette::DIM)), area);
            return;
        }
        let selected = app::selected_row(state);
        let width = usize::from(area.width);
        let name_w = (width * 36 / 100).clamp(20, 64);
        let age_w = 5;
        let summary_w = width.saturating_sub(name_w + age_w + 6);
        let mut lines: Vec<(Option<RowKey>, Line<'static>)> = Vec::new();
        let mut last_group = None;
        for key in &rows {
            let group = app::group_of(state, *key);
            if last_group != Some(group) {
                if last_group.is_some() {
                    lines.push((None, Line::raw("")));
                }
                let style = match group.rank {
                    0 if state.tab != Tab::Prs => palette::NEEDS_YOU,
                    0 | 1 => palette::HEADING,
                    _ => palette::DIM,
                };
                let n = rows.iter().filter(|k| app::group_of(state, **k) == group).count();
                lines.push((None, Line::from(vec![span(group.name, style), span(format!(" {n}"), palette::DIM)])));
                last_group = Some(group);
            }
            let on = selected == Some(*key);
            let (glyph, name, summary, age) = row_parts(state, *key, now, icons);
            let marker = if on { span("❯", palette::ACCENT) } else { Span::raw(" ") };
            let name_style = if on { palette::HEADING } else { palette::PLAIN };
            let mut summary_text: Vec<Span<'static>> = Vec::new();
            let mut used = 0;
            for s in summary {
                let t = fit(&s.content, summary_w.saturating_sub(used));
                used += cells(&t);
                summary_text.push(Span::styled(t, s.style));
            }
            let mut line = vec![
                marker,
                Span::raw(" "),
                glyph,
                Span::raw(" "),
                span(format!("{:<name_w$}", fit(&name, name_w)), name_style),
                Span::raw("  "),
            ];
            line.extend(summary_text);
            line.push(Span::raw(" ".repeat(summary_w.saturating_sub(used))));
            line.push(span(format!("{age:>age_w$}"), palette::DIM));
            lines.push((Some(*key), Line::from(line)));
        }
        let height = usize::from(area.height);
        let at = lines.iter().position(|(k, _)| k.is_some() && *k == selected).unwrap_or(0);
        let top = (at + 1).saturating_sub(height);
        for (i, (key, line)) in lines.into_iter().skip(top).take(height).enumerate() {
            let rect = Rect::new(area.x, area.y + u16::try_from(i).unwrap_or(0), area.width, 1);
            frame.render_widget(Paragraph::new(line), rect);
            if let Some(key) = key {
                hits.targets.push((rect, HitTarget::Row(key)));
            }
        }
    }

    fn draw_prompt(frame: &mut Frame, area: Rect, state: &State) {
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

    fn draw_footer(frame: &mut Frame, area: Rect, state: &State) {
        let pairs: Vec<(&str, &str)> = match &state.focus {
            Focus::Filter => vec![("enter", "keep"), ("esc", "clear")],
            Focus::Dialog(_) => vec![("ctrl-s", "start"), ("tab", "next field"), ("esc", "cancel")],
            Focus::Settings(menu) if menu.edit.is_some() => vec![("enter", "save"), ("esc", "cancel")],
            Focus::Settings(_) => vec![("j/k", "move"), ("enter", "change"), ("esc", "close")],
            Focus::Keybinds { .. } => vec![("j/k", "scroll"), ("esc", "close")],
            Focus::List => app::key_bar(state),
        };
        let mut spans = vec![Span::raw("")];
        for (i, (k, d)) in pairs.into_iter().enumerate() {
            if i > 0 {
                spans.push(span(" · ", palette::DIM));
            }
            spans.push(span(k.to_string(), palette::ACCENT));
            spans.push(span(format!(" to {d}"), palette::DIM));
        }
        let sort = vec![
            span("S", palette::ACCENT),
            span(format!(" sort: {} · ", state.sort.label(state.tab)), palette::DIM),
            span("v", palette::ACCENT),
            span(" layout", palette::DIM),
        ];
        frame.render_widget(Paragraph::new(row_line(area.width, spans, sort)), area);
    }
}
