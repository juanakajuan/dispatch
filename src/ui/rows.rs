use super::markdown;
use super::palette;
use super::text::{age, fit_spans, span, spans_width};
use crate::app::feed::LastFetch;
use crate::app::list::{Group, IssueGroup, PrGroup, RowKey, SessionGroup, Tab};
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, State};
use crate::domain::{Checks, Issue, IssueNumber, PageUrl, PrState, PullRequest, SessionKey};
use crate::glyphs::Icons;
use crate::session::{Checkout, Push, Session, SessionState, Work};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
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
        PrState::Closed => span("closed", palette::DIM),
    }
}

/// The glyph of a PR in the row glyph column, the same on each tab. The
/// shape gives its state; the colour of an open PR gives its checks.
pub(super) fn pr_glyph(p: &PullRequest, icons: &Icons) -> Span<'static> {
    match (p.state, p.checks) {
        (PrState::Merged, _) => span(icons.pr_merged, palette::MERGED),
        (PrState::Draft, _) => span(icons.pr_draft, palette::DIM),
        (PrState::Closed, _) => span(icons.pr_closed, palette::DIM),
        (PrState::Open, Checks::Pass) => span(icons.pr_open, palette::PASS),
        (PrState::Open, Checks::Fail) => span(icons.pr_open, palette::FAIL),
        (PrState::Open, Checks::Pending) => span(icons.pr_open, palette::PENDING),
        (PrState::Open, Checks::None) => span(icons.pr_open, palette::PLAIN),
    }
}

pub(super) fn issue_of(state: &State, n: IssueNumber) -> Option<&Issue> {
    state.issues.items.iter().find(|i| i.number == n)
}

pub(super) fn empty_text(state: &State) -> String {
    let (name, loading) = match state.tab {
        Tab::Issues => ("issues", matches!(state.issues.last, LastFetch::Never)),
        Tab::Prs => ("pull requests", matches!(state.prs.last, LastFetch::Never)),
        Tab::Sessions => ("sessions", false),
    };
    match (loading, state.filter().is_empty()) {
        (true, _) => format!("Loading {name}…"),
        (false, true) if state.tab == Tab::Sessions => "No sessions. Press s on an issue to start one.".to_string(),
        (false, true) => format!("No {name}."),
        (false, false) => format!("No {name} match the filter."),
    }
}

/// What the session's Claude last said: its recap, or its last message.
pub(super) fn said(state: &State, n: SessionKey) -> Option<Vec<Span<'static>>> {
    let s = state.transcripts.get(&n)?.said.as_ref()?;
    let mut v = Vec::new();
    if s.kind == crate::transcript::SummaryKind::Recap {
        v.push(span("※ ", palette::ACCENT));
    }
    v.extend(markdown::inline(&s.text, palette::DIM));
    Some(v)
}

/// A worktree that has another branch checked out says so before its PR or
/// what Claude said: its PR and its `D` still go by the session branch.
fn off_branch(s: &Session) -> Option<Vec<Span<'static>>> {
    (s.checkout != Checkout::SessionBranch)
        .then(|| vec![span(format!("on {}, not {}", s.checked_out(), s.branch.as_str()), palette::WARN)])
}

/// `±2 ↑1 ↓3`: only the work that needs action. A branch that `origin`
/// has at the same commit, with nothing uncommitted, shows a green check.
fn work_spans(w: Work, icons: &Icons) -> Vec<Span<'static>> {
    let (to_push, to_pull) = match w.push {
        Push::NotPushed => (w.commits, 0),
        Push::Pushed => (0, 0),
        Push::Differs { to_push, to_pull } => (to_push, to_pull),
    };
    let mut parts = Vec::new();
    if w.uncommitted > 0 {
        parts.push(span(format!("{}{}", icons.uncommitted, w.uncommitted), palette::WARN));
    }
    if to_push > 0 {
        parts.push(span(format!("{}{to_push}", icons.to_push), palette::WARN));
    }
    if to_pull > 0 {
        parts.push(span(format!("{}{to_pull}", icons.to_pull), palette::WARN));
    }
    if parts.is_empty() && matches!(w.push, Push::Pushed) && w.commits > 0 {
        parts.push(span(icons.pushed, palette::PASS));
    }
    let mut v = Vec::new();
    for part in parts {
        if !v.is_empty() {
            v.push(Span::raw(" "));
        }
        v.push(part);
    }
    v
}

/// The `#3` cells of a row and the page they link to. `x` counts from the
/// left edge of the row.
pub(super) struct NumberLink {
    x: u16,
    width: u16,
    url: PageUrl,
}

/// The name of a row: `#3 ` in front of its title.
pub(super) struct Name {
    spans: Vec<Span<'static>>,
    link: Option<NumberLink>,
}

impl Name {
    fn numbered(n: impl std::fmt::Display, url: Option<&PageUrl>, title: Vec<Span<'static>>) -> Name {
        let number = n.to_string();
        let width = u16::try_from(number.chars().count()).unwrap_or(u16::MAX);
        let link = url.map(|url| NumberLink { x: 0, width, url: url.clone() });
        let mut spans = vec![Span::raw(format!("{number} "))];
        spans.extend(title);
        Name { spans, link }
    }

    fn empty() -> Name {
        Name { spans: Vec::new(), link: None }
    }
}

/// A line that draws a row. A group heading, or the gap before it, draws none.
struct DrawnRow {
    key: RowKey,
    number: Option<NumberLink>,
}

pub(super) struct RowParts {
    glyph: Span<'static>,
    name: Name,
    summary: Vec<Span<'static>>,
    work: Vec<Span<'static>>,
    age: String,
}

impl RowParts {
    fn new(glyph: Span<'static>, name: Name, summary: Vec<Span<'static>>, age: String) -> RowParts {
        RowParts { glyph, name, summary, work: Vec::new(), age }
    }
}

/// What a session row shows of its issue or PR.
struct Subject<'a> {
    title: &'a str,
    updated_at: jiff::Timestamp,
    url: Option<&'a PageUrl>,
}

/// The issue or PR that session `key` is for, when the list has it.
fn subject_of(state: &State, key: SessionKey) -> Option<Subject<'_>> {
    match key {
        SessionKey::Issue(i) => {
            issue_of(state, i).map(|i| Subject { title: &i.title, updated_at: i.updated_at, url: i.url.as_ref() })
        }
        SessionKey::Pr(p) => state.prs.items.iter().find(|r| r.number == p).map(|r| Subject {
            title: &r.title,
            updated_at: r.updated_at,
            url: r.url.as_ref(),
        }),
    }
}

/// Glyph, name, summary, work, age. One line per row.
pub(super) fn row_parts(state: &State, key: RowKey, now: jiff::Timestamp, icons: &Icons) -> RowParts {
    let session_glyph = |n: SessionKey| match (state.sessions.get(&n).map(|s| s.state), state.starting.contains(&n)) {
        _ if app::list::session_merged(state, n) => Some(span(icons.pr_merged, palette::MERGED)),
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
                return RowParts::new(Span::raw(" "), Name::empty(), Vec::new(), String::new());
            };
            let glyph = session_glyph(n.into()).unwrap_or_else(|| {
                if state.marked.contains(&n) {
                    return span(icons.marked, palette::ACCENT);
                }
                app::list::lead_pr(state, n).map_or_else(|| span(icons.idle, palette::DIM), |p| pr_glyph(p, icons))
            });
            let summary = match (said(state, n.into()), app::list::lead_pr(state, n)) {
                (Some(said), _) => said,
                (None, Some(p)) => pr_summary(p),
                (None, None) if !i.labels.is_empty() => {
                    vec![span(format!("{} · {}", i.author, i.labels.join(", ")), palette::DIM)]
                }
                (None, None) => vec![span(format!("{} · {} comments", i.author, i.comment_count), palette::DIM)],
            };
            RowParts::new(
                glyph,
                Name::numbered(n, i.url.as_ref(), markdown::title(&i.title, Style::default())),
                summary,
                age(i.updated_at, now),
            )
        }
        RowKey::Pr(n) => {
            let Some(p) = state.prs.items.iter().find(|p| p.number == n) else {
                return RowParts::new(Span::raw(" "), Name::empty(), Vec::new(), String::new());
            };
            let mut summary = vec![
                span(format!("{} → {}  ", p.head.display(), p.base.display()), palette::DIM),
                span(format!("+{}", p.additions), palette::PASS),
                span(" ", palette::DIM),
                span(format!("-{}", p.deletions), palette::FAIL),
                span(" · ", palette::DIM),
            ];
            if p.checks != Checks::None {
                summary.extend([checks_span(p.checks, icons), Span::raw(" ")]);
            }
            summary.push(span(checks_word(p.checks), palette::DIM));
            if !p.closes.is_empty() {
                let c = p.closes.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                summary.push(span(format!(" · closes {c}"), palette::DIM));
            }
            let glyph = pr_glyph(p, icons);
            RowParts::new(
                glyph,
                Name::numbered(n, p.url.as_ref(), markdown::title(&p.title, Style::default())),
                summary,
                age(p.updated_at, now),
            )
        }
        RowKey::Session(n) => {
            let glyph = session_glyph(n).unwrap_or(span(icons.idle, palette::DIM));
            let session = state.sessions.get(&n);
            let subject = subject_of(state, n);
            let title = subject
                .as_ref()
                .map(|s| markdown::title(s.title, Style::default()))
                .or_else(|| session.map(|s| vec![Span::raw(s.branch.as_str().to_string())]))
                .unwrap_or_default();
            let name = Name::numbered(n, subject.as_ref().and_then(|s| s.url), title);
            let summary = session
                .and_then(off_branch)
                .or_else(|| said(state, n))
                .or_else(|| session.and_then(|s| app::list::pr_for_session(state, s)).map(pr_summary))
                .or_else(|| session.map(|s| vec![span(s.branch.as_str().to_string(), palette::DIM)]))
                .unwrap_or_else(|| vec![span("creating worktree…", palette::STARTING)]);
            let age = subject.map(|s| age(s.updated_at, now)).unwrap_or_default();
            let work = state.work.get(&n).map(|w| work_spans(*w, icons)).unwrap_or_default();
            RowParts { work, ..RowParts::new(glyph, name, summary, age) }
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
    let age_w = 5;
    let parts: Vec<_> = rows.iter().map(|key| row_parts(state, *key, now, icons)).collect();
    let work_w = parts.iter().map(|p| spans_width(&p.work)).max().unwrap_or(0);
    let work_gap = usize::from(work_w > 0);
    let avail = usize::from(area.width).saturating_sub(age_w + 6 + work_w + work_gap);
    let groups: Vec<Group> = rows.iter().map(|key| app::list::group_of(state, *key)).collect();
    let in_group = |g: Group| parts.iter().zip(&groups).filter(move |(_, rg)| **rg == g).map(|(p, _)| p);
    let name_ws: Vec<usize> = groups
        .iter()
        .map(|g| {
            let name_need = in_group(*g).map(|p| spans_width(&p.name.spans)).max().unwrap_or(0);
            let summary_need = in_group(*g).map(|p| spans_width(&p.summary)).max().unwrap_or(0);
            name_width(usize::from(area.width), avail, name_need, summary_need)
        })
        .collect();
    let mut lines: Vec<(Option<DrawnRow>, Line<'static>)> = Vec::new();
    let mut last_group = None;
    for (((key, RowParts { glyph, name, summary, work, age }), group), name_w) in
        rows.iter().zip(parts).zip(groups.iter().copied()).zip(name_ws)
    {
        let summary_w = avail - name_w;
        if last_group != Some(group) {
            if last_group.is_some() {
                lines.push((None, Line::raw("")));
            }
            let style = group_style(group);
            let n = rows.iter().filter(|k| app::list::group_of(state, **k) == group).count();
            lines.push((
                None,
                Line::from(vec![Span::raw("  "), span(group.name(), style), span(format!(" {n}"), palette::DIM)]),
            ));
            last_group = Some(group);
        }
        let on = selected == Some(*key);
        let marker = if on { span("❯", palette::ACCENT) } else { Span::raw(" ") };
        let name_style = if on { palette::HEADING } else { palette::PLAIN };
        let summary_text = fit_spans(summary, summary_w);
        let used = spans_width(&summary_text);
        let Name { spans: name, link } = name;
        let name =
            fit_spans(name.into_iter().map(|s| Span::styled(s.content, name_style.patch(s.style))).collect(), name_w);
        let name_pad = name_w.saturating_sub(spans_width(&name));
        let mut line = vec![marker, Span::raw(" "), glyph, Span::raw(" ")];
        // A name cut short can cut the number too: link only what is drawn.
        let x = u16::try_from(spans_width(&line)).unwrap_or(u16::MAX);
        let drawn = u16::try_from(spans_width(&name)).unwrap_or(u16::MAX);
        let number = link.map(|l| NumberLink { x, width: l.width.min(drawn), url: l.url }).filter(|l| l.width > 0);
        line.extend(name);
        line.push(span(" ".repeat(name_pad + 2), name_style));
        line.extend(summary_text);
        line.push(Span::raw(" ".repeat(summary_w.saturating_sub(used) + work_gap + work_w - spans_width(&work))));
        line.extend(work);
        line.push(span(format!("{age:>age_w$}"), palette::DIM));
        lines.push((Some(DrawnRow { key: *key, number }), Line::from(line)));
    }
    let height = usize::from(area.height);
    let at = lines.iter().position(|(row, _)| row.as_ref().is_some_and(|r| Some(r.key) == selected)).unwrap_or(0);
    // Keep the selected line at the center, as `scrolloff=999` does in Neovim,
    // but never scroll past the last line.
    let top = at.saturating_sub(height / 2).min(lines.len().saturating_sub(height));
    for (i, (row, line)) in lines.into_iter().skip(top).take(height).enumerate() {
        let rect = Rect::new(area.x, area.y + u16::try_from(i).unwrap_or(0), area.width, 1);
        frame.render_widget(Paragraph::new(line), rect);
        let Some(DrawnRow { key, number }) = row else { continue };
        hits.targets.push((rect, HitTarget::Row(key)));
        if let Some(NumberLink { x, width, url }) = number {
            hits.links.push((Rect::new(rect.x.saturating_add(x), rect.y, width, 1).intersection(rect), url));
        }
    }
}

/// The name column width of one group: each group sizes its own columns, so
/// a long summary in one group does not narrow the names of another. It gets at least two fifths of the full row
/// `width`, less the 4 prefix cells and 2 gap cells, with a floor of 20 cells
/// on narrow terminals. When the widest summary leaves more of `avail` free,
/// the name grows into it, up to the widest name.
fn name_width(width: usize, avail: usize, name_need: usize, summary_need: usize) -> usize {
    let share = (width * 2 / 5).saturating_sub(6).max(avail.min(20));
    name_need.min(avail.saturating_sub(summary_need)).max(share).min(avail)
}

fn group_style(group: Group) -> Style {
    match group {
        Group::Issue(IssueGroup::NeedsYou) | Group::Session(SessionGroup::NeedsYou) => palette::NEEDS_YOU,
        Group::Issue(IssueGroup::Working)
        | Group::Session(SessionGroup::Working)
        | Group::Pr(PrGroup::ReadyForReview | PrGroup::Draft) => palette::HEADING,
        Group::Issue(IssueGroup::InReview | IssueGroup::Merged | IssueGroup::Open)
        | Group::Pr(PrGroup::Merged | PrGroup::Closed)
        | Group::Session(SessionGroup::Exited | SessionGroup::Merged) => palette::DIM,
    }
}
