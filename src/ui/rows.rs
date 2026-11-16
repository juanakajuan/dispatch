use super::palette;
use super::text::{age, cells, fit, one_line, span};
use crate::app::feed::FeedStatus;
use crate::app::list::RowKey;
use crate::app::list::Tab;
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, State};
use crate::domain::{Checks, Issue, IssueNumber, PrState, PullRequest};
use crate::glyphs::Icons;
use crate::session::SessionState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

pub(super) fn checks_span(checks: Checks, icons: &Icons) -> Span<'static> {
    match checks {
        Checks::Pass => span(icons.check_pass, palette::PASS),
        Checks::Fail => span(icons.check_fail, palette::FAIL),
        Checks::Pending => span(icons.check_pending, palette::PENDING),
        Checks::None => span(icons.check_none, palette::DIM),
    }
}

pub(super) fn checks_word(checks: Checks) -> &'static str {
    match checks {
        Checks::Pass => "checks passing",
        Checks::Fail => "checks failing",
        Checks::Pending => "checks running",
        Checks::None => "no checks",
    }
}

pub(super) fn pr_state_span(state: PrState) -> Span<'static> {
    match state {
        PrState::Open => span("open", palette::PASS),
        PrState::Draft => span("draft", palette::DIM),
        PrState::Merged => span("merged", palette::MERGED),
        PrState::Closed => span("closed", palette::FAIL),
    }
}

/// Two cells before an issue title: the state of its lead PR, or blank.
pub(super) fn pr_mark(pr: Option<&PullRequest>, icons: &Icons) -> Span<'static> {
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

pub(super) fn issue_of(state: &State, n: IssueNumber) -> Option<&Issue> {
    state.issues.items.iter().find(|i| i.number == n)
}

pub(super) fn empty_text(state: &State) -> String {
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

/// What the session's Claude last said: its recap, or its last message.
pub(super) fn said(state: &State, n: IssueNumber) -> Option<Vec<Span<'static>>> {
    let s = state.summaries.get(&n)?;
    let mut v = Vec::new();
    if s.kind == crate::transcript::SummaryKind::Recap {
        v.push(span("※ ", palette::ACCENT));
    }
    v.push(span(one_line(&s.text), palette::PLAIN));
    Some(v)
}

/// Glyph, name, summary, age. One line per row.
pub(super) fn row_parts(
    state: &State,
    key: RowKey,
    now: jiff::Timestamp,
    icons: &Icons,
) -> (Span<'static>, String, Vec<Span<'static>>, String) {
    let session_glyph = |n: IssueNumber| match (state.sessions.get(&n).map(|s| s.state), state.starting.contains(&n)) {
        (Some(SessionState::Running), _) => Some(span(icons.running, palette::RUNNING)),
        (Some(SessionState::NeedsYou), _) => Some(span(icons.needs_you, palette::NEEDS_YOU)),
        (Some(SessionState::Exited), _) => Some(span(icons.exited, palette::EXITED)),
        (None, true) => Some(span(icons.starting, palette::STARTING)),
        (None, false) => None,
    };
    let pr_summary = |p: &PullRequest| {
        let mut v = vec![span(format!("PR {} ", p.number), palette::DIM), pr_state_span(p.state)];
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
            (glyph, format!("{n} {}", one_line(&i.title)), summary, age(i.updated_at, now))
        }
        RowKey::Pr(n) => {
            let Some(p) = state.prs.items.iter().find(|p| p.number == n) else {
                return (Span::raw(" "), String::new(), Vec::new(), String::new());
            };
            let mut summary = vec![
                span(format!("{} → {}  ", p.head.display(), p.base.display()), palette::DIM),
                span(format!("+{}", p.additions), palette::PASS),
                span(" ", palette::DIM),
                span(format!("-{}", p.deletions), palette::FAIL),
                span(format!(" · {}", checks_word(p.checks)), palette::DIM),
            ];
            if !p.closes.is_empty() {
                let c = p.closes.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                summary.push(span(format!(" · closes {c}"), palette::DIM));
            }
            let glyph = match p.state {
                PrState::Merged => span(icons.pr_merged, palette::MERGED),
                PrState::Draft => span(icons.pr_draft, palette::DIM),
                PrState::Closed => span(icons.pr_closed, palette::DIM),
                PrState::Open => checks_span(p.checks, icons),
            };
            (glyph, format!("{n} {}", one_line(&p.title)), summary, age(p.updated_at, now))
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
            (glyph, format!("{n} {name}"), summary, age)
        }
    }
}

pub(super) fn draw_rows(
    frame: &mut Frame,
    area: Rect,
    state: &State,
    now: jiff::Timestamp,
    icons: &Icons,
    hits: &mut HitMap,
) {
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
