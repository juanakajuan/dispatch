use super::keys::{Key, key_of};
use super::msg::Cmd;
use super::toast::say;
use super::{Focus, State};
use crate::domain::{GithubRef, Issue, IssueNumber, PrNumber, PrState, PullRequest};
use crate::session::{Session, SessionState};
use crate::sort::{self, Cycle, IssueSort, PrSort, SessionSort};
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Issues,
    Prs,
    Sessions,
}

/// The section a row sits in. Lower `rank` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub rank: u8,
    pub name: &'static str,
}

pub fn group_of(state: &State, key: RowKey) -> Group {
    let g = |rank, name| Group { rank, name };
    let by_session = |n: IssueNumber| match activity(state, n) {
        Some(SessionState::NeedsYou) => Some(g(0, "Needs you")),
        Some(SessionState::Running) => Some(g(1, "Working")),
        Some(SessionState::Exited) | None => None,
    };
    match key {
        RowKey::Session(n) => by_session(n).unwrap_or(g(2, "Exited")),
        RowKey::Issue(n) => by_session(n).unwrap_or_else(|| match lead_pr(state, n).map(|p| p.state) {
            Some(PrState::Open | PrState::Draft) => g(2, "In review"),
            Some(PrState::Merged) => g(3, "Merged"),
            _ => g(4, "Open"),
        }),
        RowKey::Pr(n) => match state.prs.items.iter().find(|p| p.number == n).map(|p| p.state) {
            Some(PrState::Open) => g(0, "Ready for review"),
            Some(PrState::Draft) => g(1, "Draft"),
            Some(PrState::Merged) => g(2, "Merged"),
            _ => g(3, "Closed"),
        },
    }
}

/// A list row's identity. Selection stores keys, never indices, so a refresh
/// that reorders or filters rows keeps the same item selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    Issue(IssueNumber),
    Pr(PrNumber),
    Session(IssueNumber),
}

/// The page `o` opens for a row. A session row opens its issue. Exhaustive,
/// so a new row kind does not compile until it says what `o` opens.
impl From<RowKey> for GithubRef {
    fn from(key: RowKey) -> GithubRef {
        match key {
            RowKey::Issue(n) | RowKey::Session(n) => GithubRef::Issue(n),
            RowKey::Pr(n) => GithubRef::Pr(n),
        }
    }
}

/// Each tab's sort mode. `S` cycles the shown tab's mode only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sorts {
    pub issues: IssueSort,
    pub prs: PrSort,
    pub sessions: SessionSort,
}

impl Sorts {
    pub(super) fn cycle(&mut self, tab: Tab) {
        match tab {
            Tab::Issues => self.issues = self.issues.next(),
            Tab::Prs => self.prs = self.prs.next(),
            Tab::Sessions => self.sessions = self.sessions.next(),
        }
    }

    pub fn label(self, tab: Tab) -> &'static str {
        match tab {
            Tab::Issues => self.issues.label(),
            Tab::Prs => self.prs.label(),
            Tab::Sessions => self.sessions.label(),
        }
    }
}

#[derive(Default)]
pub struct Selected {
    pub issue: Option<IssueNumber>,
    pub pr: Option<PrNumber>,
    pub session: Option<IssueNumber>,
}

pub(super) fn go_to_issue_of_pr(state: &mut State) {
    let Some(RowKey::Pr(n)) = selected_row(state) else { return };
    let closes = state.prs.items.iter().find(|p| p.number == n).and_then(|p| p.closes.first().copied());
    match closes {
        Some(issue) => {
            state.tab = Tab::Issues;
            state.selected.issue = Some(issue);
            state.filter.clear();
        }
        None => say(state, format!("PR #{} closes no issue.", n.0)),
    }
}

#[derive(Clone, Copy)]
pub(super) enum Move {
    Down,
    Up,
}

pub(super) fn move_selection(state: &mut State, step: Move) {
    let rows = visible_rows(state);
    if rows.is_empty() {
        return;
    }
    let at = selected_row(state).and_then(|k| rows.iter().position(|r| *r == k)).unwrap_or(0);
    let to = match step {
        Move::Down => (at + 1).min(rows.len() - 1),
        Move::Up => at.saturating_sub(1),
    };
    select(state, rows[to]);
}

pub(super) fn select(state: &mut State, key: RowKey) {
    match key {
        RowKey::Issue(n) => state.selected.issue = Some(n),
        RowKey::Pr(n) => state.selected.pr = Some(n),
        RowKey::Session(n) => state.selected.session = Some(n),
    }
}

pub(super) fn filter_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    match key_of(ev) {
        Some(Key::Code(KeyCode::Enter)) => state.focus = Focus::List,
        Some(Key::Code(KeyCode::Esc)) => {
            state.filter.clear();
            state.focus = Focus::List;
        }
        Some(Key::Code(KeyCode::Backspace)) => {
            state.filter.pop();
        }
        Some(Key::Ctrl('c')) => return vec![Cmd::Quit],
        Some(Key::Char(c)) => state.filter.push(c),
        _ => {}
    }
    Vec::new()
}

/// Rows of the current tab after the filter, in the tab's sort order. The
/// single definition of "what is in the list"; ui renders it and update moves
/// through it.
pub fn visible_rows(state: &State) -> Vec<RowKey> {
    let mut rows = sorted_rows(state);
    rows.sort_by_key(|k| group_of(state, *k).rank);
    rows
}

pub(super) fn sorted_rows(state: &State) -> Vec<RowKey> {
    let f = state.filter.as_str();
    let activity = |n| activity(state, n);
    match state.tab {
        Tab::Issues => {
            let mut rows: Vec<&Issue> = state
                .issues
                .items
                .iter()
                .filter(|i| {
                    let number = format!("#{}", i.number.0);
                    let mut parts = vec![number.as_str(), i.title.as_str()];
                    parts.extend(i.labels.iter().map(String::as_str));
                    matches_filter(&parts, f)
                })
                .collect();
            sort::issues(&mut rows, state.sort.issues, activity);
            rows.into_iter().map(|i| RowKey::Issue(i.number)).collect()
        }
        Tab::Prs => {
            let mut rows: Vec<&PullRequest> = state
                .prs
                .items
                .iter()
                .filter(|p| matches_filter(&[&format!("#{}", p.number.0), &p.title, &p.head], f))
                .collect();
            sort::prs(&mut rows, state.sort.prs);
            rows.into_iter().map(|p| RowKey::Pr(p.number)).collect()
        }
        Tab::Sessions => {
            let numbers: BTreeSet<IssueNumber> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            let mut rows: Vec<IssueNumber> = numbers
                .into_iter()
                .filter(|n| {
                    let title = state.issues.items.iter().find(|i| i.number == *n).map_or("", |i| i.title.as_str());
                    let branch = state.sessions.get(n).map_or("", |s| s.branch.as_str());
                    matches_filter(&[&format!("#{}", n.0), title, branch], f)
                })
                .collect();
            sort::sessions(&mut rows, state.sort.sessions, activity);
            rows.into_iter().map(RowKey::Session).collect()
        }
    }
}

/// The state of issue `n`'s session for sorting. A start in flight counts as
/// running, so the row does not jump when the window appears.
pub(super) fn activity(state: &State, n: IssueNumber) -> Option<SessionState> {
    match state.sessions.get(&n) {
        Some(s) => Some(s.state),
        None => state.starting.contains(&n).then_some(SessionState::Running),
    }
}

/// The stored key if it is visible, else the first visible row.
pub fn selected_row(state: &State) -> Option<RowKey> {
    let rows = visible_rows(state);
    let stored = match state.tab {
        Tab::Issues => state.selected.issue.map(RowKey::Issue),
        Tab::Prs => state.selected.pr.map(RowKey::Pr),
        Tab::Sessions => state.selected.session.map(RowKey::Session),
    };
    stored.filter(|k| rows.contains(k)).or_else(|| rows.first().copied())
}

/// The session the `a`/`x` keys act on: the selected session, the selected
/// issue's session, or the session of the issue the selected PR closes.
pub fn current_session(state: &State) -> Option<&Session> {
    match selected_row(state)? {
        RowKey::Issue(n) | RowKey::Session(n) => state.sessions.get(&n),
        RowKey::Pr(n) => {
            let pr = state.prs.items.iter().find(|p| p.number == n)?;
            pr.closes
                .iter()
                .find_map(|c| state.sessions.get(c))
                .or_else(|| state.sessions.values().find(|s| s.branch.as_str() == pr.head))
        }
    }
}

/// PRs that name issue `n` in their closing references.
pub fn prs_closing(state: &State, n: IssueNumber) -> impl Iterator<Item = &PullRequest> {
    state.prs.items.iter().filter(move |p| p.closes.contains(&n))
}

/// The one PR an issue row shows: merged beats open beats draft, then the
/// newest number wins. A closed PR never shows.
pub fn lead_pr(state: &State, n: IssueNumber) -> Option<&PullRequest> {
    let rank = |p: &PullRequest| match p.state {
        PrState::Merged => Some(3),
        PrState::Open => Some(2),
        PrState::Draft => Some(1),
        PrState::Closed => None,
    };
    prs_closing(state, n).filter_map(|p| Some(((rank(p)?, p.number), p))).max_by_key(|(k, _)| *k).map(|(_, p)| p)
}

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    state.prs.items.iter().find(|p| p.head == session.branch.as_str())
}

/// Case-insensitive substring match over `#N title labels branch`.
pub(super) fn matches_filter(haystack_parts: &[&str], needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    needle.is_empty() || haystack_parts.join(" ").to_lowercase().contains(&needle)
}
