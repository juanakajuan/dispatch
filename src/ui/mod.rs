//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: the header (tab row, logo, repo name
//! with the age of the shown list, session counts), the grouped rows (one
//! line each), the message row (only when there is a message), the prompt
//! between two rules, and the footer keys. The dialog is a centered overlay.

use crate::app::feed::FeedStatus;
use crate::app::list::{RowKey, Tab};
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, Focus, State, dialog::DialogField};
use crate::config::{Config, SETTINGS};
use crate::domain::{Checks, Issue, IssueNumber, Mode, PrState, PullRequest};
use crate::glyphs::Icons;
use crate::session::{Session, SessionState, request::Launch};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
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
}

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

/// Word-wraps `text` to rows of at most `width` cells. A word wider than a
/// whole row is split by cell.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cells(&cur) + 1 + cells(word) > width {
            rows.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        for c in word.chars() {
            let mut buf = [0; 4];
            if !cur.is_empty() && cells(&cur) + cells(c.encode_utf8(&mut buf)) > width {
                rows.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows
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

fn feed_failure<T>(feed: &app::feed::Feed<T>) -> Option<String> {
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

// ---- dialog ----------------------------------------------------------------

/// The exact commands the dialog will run, for the "Runs" preview.
fn preview(state: &State, dialog: &app::dialog::StartDialog) -> Vec<String> {
    let branch = dialog.branch.lines().join("");
    let prompt = dialog.prompt.lines().join("\n");
    let launch = Launch::from_config(&state.config);
    let commands =
        state.context.start_preview.commands(dialog.issue, branch.trim(), dialog.mode, prompt.trim(), &launch);
    commands
        .into_iter()
        .map(|argv| {
            let shown: Vec<String> = argv
                .into_iter()
                .map(|a| if a.contains([' ', '\n', '"']) { format!("{:?}", one_line(&a)) } else { a })
                .collect();
            shown.join(" ")
        })
        .collect()
}

/// The "Runs" label and each preview command, word-wrapped to `width` cells.
/// A wrapped row hangs under the start of its command.
fn run_lines(state: &State, dialog: &app::dialog::StartDialog, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(span("  Runs", palette::DIM))];
    for command in preview(state, dialog) {
        for (i, row) in wrap(&command, width.saturating_sub(4).max(1)).into_iter().enumerate() {
            let indent = if i == 0 { "  $ " } else { "    " };
            lines.push(Line::from(span(format!("{indent}{row}"), palette::DIM)));
        }
    }
    lines
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

/// Rows of the start dialog that are not the prompt or the "Runs" preview.
const DIALOG_FIXED_ROWS: u16 = 9;
/// The prompt input keeps at least this many rows when the preview is tall.
const PROMPT_MIN_ROWS: u16 = 3;

fn draw_dialog(frame: &mut Frame, screen: Rect, state: &State, d: &app::dialog::StartDialog) {
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
    let run_lines = run_lines(state, d, usize::from(inner.width));
    let room = inner.height.saturating_sub(DIALOG_FIXED_ROWS + PROMPT_MIN_ROWS);
    let runs_h = u16::try_from(run_lines.len()).unwrap_or(u16::MAX).min(room);
    let [branch_l, branch, _, mode, _, prompt_l, prompt, _, runs, error] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(PROMPT_MIN_ROWS),
        Constraint::Length(1),
        Constraint::Length(runs_h),
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

    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(format!("  {e}"), palette::ERROR)), error);
    }
}

// ---- keybinds --------------------------------------------------------------

/// The keybinds menu lines, made from `BINDINGS` and grouped by `KeyGroup`.
fn keybinds_lines() -> Vec<Line<'static>> {
    let keys = |b: &app::keys::Binding| b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
    let key_width = app::keys::BINDINGS.iter().map(|b| cells(&keys(b))).max().unwrap_or(0);
    let mut lines = Vec::new();
    for group in app::keys::KeyGroup::ALL {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(span(group.title(), palette::HEADING)));
        for b in app::keys::BINDINGS.iter().filter(|b| b.group == group) {
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

fn draw_settings(frame: &mut Frame, screen: Rect, state: &State, menu: &app::menus::SettingsMenu) {
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
        None => menu.at.applies_note().map_or_else(|| Line::raw(""), |note| Line::from(span(note, palette::DIM))),
    });
    lines.push(Line::from(span(
        if menu.edit.is_some() { "enter save · esc cancel" } else { "j/k move · enter change · esc close" },
        palette::DIM,
    )));
    lines.push(Line::from(span("Saved to config.json", palette::DIM)));
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---- layout ----------------------------------------------------------------

mod agent {
    use super::{
        Checks, Constraint, Focus, Frame, HitMap, HitTarget, Icons, Instant, IssueNumber, Layout, Line, Paragraph,
        PrState, PullRequest, Rect, RowKey, Session, SessionState, Span, State, Tab, age, app, cells, centered,
        checks_span, checks_word, draw_overlay, empty_text, fit, freshness, issue_of, message, one_line, palette,
        pr_mark, pr_state_span, row_line, span, spans_width,
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
        draw_rows(frame, body, state, now, icons, &mut hits);
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
                    let m = pr_mark(app::list::lead_pr(state, n), icons);
                    if m.content.trim().is_empty() {
                        span(icons.idle, palette::DIM)
                    } else {
                        span(m.content.trim().to_string(), m.style)
                    }
                });
                let summary = match (said(state, n), app::list::lead_pr(state, n)) {
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
                    .or_else(|| session.and_then(|s| app::list::pr_for_session(state, s)).map(pr_summary))
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
        let rows = app::list::visible_rows(state);
        if rows.is_empty() {
            frame.render_widget(Paragraph::new(span(format!(" {}", empty_text(state)), palette::DIM)), area);
            return;
        }
        let selected = app::list::selected_row(state);
        let width = usize::from(area.width);
        let name_w = (width * 36 / 100).clamp(20, 64);
        let age_w = 5;
        let summary_w = width.saturating_sub(name_w + age_w + 6);
        let mut lines: Vec<(Option<RowKey>, Line<'static>)> = Vec::new();
        let mut last_group = None;
        for key in &rows {
            let group = app::list::group_of(state, *key);
            if last_group != Some(group) {
                if last_group.is_some() {
                    lines.push((None, Line::raw("")));
                }
                let style = match group.rank {
                    0 if state.tab != Tab::Prs => palette::NEEDS_YOU,
                    0 | 1 => palette::HEADING,
                    _ => palette::DIM,
                };
                let n = rows.iter().filter(|k| app::list::group_of(state, **k) == group).count();
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
        let sort =
            vec![span("S", palette::ACCENT), span(format!(" sort: {}", state.sort.label(state.tab)), palette::DIM)];
        frame.render_widget(Paragraph::new(row_line(area.width, spans, sort)), area);
    }
}

#[cfg(test)]
mod tests;
