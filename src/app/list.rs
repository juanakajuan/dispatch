use super::State;
use super::filter::{Facets, Query};
use super::toast;
use crate::domain::{GithubRef, Issue, IssueNumber, PrNumber, PrState, PullRequest};
use crate::session::{Session, SessionState};
use crate::sort::{self, Cycle, IssueSort, PrSort, SessionSort};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Issues,
    Prs,
    Sessions,
}

/// The section a row sits in. Each tab has its own sections, and the
/// declaration order of each enum is the order on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Issue(IssueGroup),
    Pr(PrGroup),
    Session(SessionGroup),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueGroup {
    NeedsYou,
    Working,
    InReview,
    Merged,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrGroup {
    ReadyForReview,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionGroup {
    NeedsYou,
    Working,
    Exited,
    Merged,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Issue(IssueGroup::NeedsYou) | Group::Session(SessionGroup::NeedsYou) => "Needs you",
            Group::Issue(IssueGroup::Working) | Group::Session(SessionGroup::Working) => "Working",
            Group::Issue(IssueGroup::InReview) => "In review",
            Group::Issue(IssueGroup::Merged) | Group::Pr(PrGroup::Merged) | Group::Session(SessionGroup::Merged) => {
                "Merged"
            }
            Group::Issue(IssueGroup::Open) => "Open",
            Group::Pr(PrGroup::ReadyForReview) => "Ready for review",
            Group::Pr(PrGroup::Draft) => "Draft",
            Group::Pr(PrGroup::Closed) => "Closed",
            Group::Session(SessionGroup::Exited) => "Exited",
        }
    }
}

pub fn group_of(state: &State, key: RowKey) -> Group {
    match key {
        RowKey::Session(n) => Group::Session(match activity(state, n) {
            _ if session_merged(state, n) => SessionGroup::Merged,
            Some(SessionState::NeedsYou) => SessionGroup::NeedsYou,
            Some(SessionState::Running) => SessionGroup::Working,
            Some(SessionState::Exited) | None => SessionGroup::Exited,
        }),
        RowKey::Issue(n) => Group::Issue(match activity(state, n) {
            _ if session_merged(state, n) => IssueGroup::Merged,
            Some(SessionState::NeedsYou) => IssueGroup::NeedsYou,
            Some(SessionState::Running) => IssueGroup::Working,
            Some(SessionState::Exited) | None => match lead_pr(state, n).map(|p| p.state) {
                Some(PrState::Open | PrState::Draft) => IssueGroup::InReview,
                Some(PrState::Merged) => IssueGroup::Merged,
                Some(PrState::Closed) | None => IssueGroup::Open,
            },
        }),
        RowKey::Pr(n) => Group::Pr(match state.prs.items.iter().find(|p| p.number == n).map(|p| p.state) {
            Some(PrState::Open) => PrGroup::ReadyForReview,
            Some(PrState::Draft) => PrGroup::Draft,
            Some(PrState::Merged) => PrGroup::Merged,
            Some(PrState::Closed) | None => PrGroup::Closed,
        }),
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

/// What each tab remembers while another tab is shown.
#[derive(Default)]
pub struct Tabs {
    pub issues: TabState<IssueNumber, IssueSort>,
    pub prs: TabState<PrNumber, PrSort>,
    pub sessions: TabState<IssueNumber, SessionSort>,
}

/// A tab's selected row and sort mode. Selection stores the key, see `RowKey`.
pub struct TabState<K, S> {
    pub selected: Option<K>,
    pub sort: S,
}

impl<K, S: Default> Default for TabState<K, S> {
    fn default() -> Self {
        TabState { selected: None, sort: S::default() }
    }
}

impl Tabs {
    /// `S` cycles the shown tab's mode only.
    pub(super) fn cycle_sort(&mut self, tab: Tab) {
        match tab {
            Tab::Issues => self.issues.sort = self.issues.sort.next(),
            Tab::Prs => self.prs.sort = self.prs.sort.next(),
            Tab::Sessions => self.sessions.sort = self.sessions.sort.next(),
        }
    }

    pub fn sort_label(&self, tab: Tab) -> &'static str {
        match tab {
            Tab::Issues => self.issues.sort.label(),
            Tab::Prs => self.prs.sort.label(),
            Tab::Sessions => self.sessions.sort.label(),
        }
    }

    pub(super) fn select(&mut self, key: RowKey) {
        match key {
            RowKey::Issue(n) => self.issues.selected = Some(n),
            RowKey::Pr(n) => self.prs.selected = Some(n),
            RowKey::Session(n) => self.sessions.selected = Some(n),
        }
    }

    fn selected(&self, tab: Tab) -> Option<RowKey> {
        match tab {
            Tab::Issues => self.issues.selected.map(RowKey::Issue),
            Tab::Prs => self.prs.selected.map(RowKey::Pr),
            Tab::Sessions => self.sessions.selected.map(RowKey::Session),
        }
    }
}

pub(super) fn go_to_issue_of_pr(state: &mut State) {
    let Some(RowKey::Pr(n)) = selected_row(state) else { return };
    let closes = state.prs.items.iter().find(|p| p.number == n).and_then(|p| p.closes.first().copied());
    match closes {
        Some(issue) => {
            state.tab = Tab::Issues;
            state.tabs.issues.selected = Some(issue);
            state.filter.clear();
        }
        None => toast::info(state, format!("PR {n} closes no issue.")),
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
    state.tabs.select(key);
}

/// Rows of the current tab after the filter, in the tab's sort order. The
/// single definition of "what is in the list"; ui renders it and update moves
/// through it.
pub fn visible_rows(state: &State) -> Vec<RowKey> {
    let mut rows = sorted_rows(state);
    rows.sort_by_key(|k| group_of(state, *k));
    rows
}

pub(super) fn sorted_rows(state: &State) -> Vec<RowKey> {
    let query = Query::parse(&state.filter, state.viewer.login());
    let activity = |n| activity(state, n);
    match state.tab {
        Tab::Issues => {
            let mut rows: Vec<&Issue> = state
                .issues
                .items
                .iter()
                .filter(|i| {
                    let number = i.number.to_string();
                    let mut text = vec![number.as_str(), i.title.as_str()];
                    text.extend(i.labels.iter().map(String::as_str));
                    query.matches(&text, Facets::from(*i))
                })
                .collect();
            sort::issues(&mut rows, state.tabs.issues.sort, activity);
            rows.into_iter().map(|i| RowKey::Issue(i.number)).collect()
        }
        Tab::Prs => {
            let mut rows: Vec<&PullRequest> = state
                .prs
                .items
                .iter()
                .filter(|p| query.matches(&[&p.number.to_string(), &p.title, p.head.display()], Facets::from(*p)))
                .collect();
            sort::prs(&mut rows, state.tabs.prs.sort);
            rows.into_iter().map(|p| RowKey::Pr(p.number)).collect()
        }
        Tab::Sessions => {
            let numbers: BTreeSet<IssueNumber> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            let mut rows: Vec<IssueNumber> = numbers
                .into_iter()
                .filter(|n| {
                    let issue = state.issues.items.iter().find(|i| i.number == *n);
                    let title = issue.map_or("", |i| i.title.as_str());
                    let session = state.sessions.get(n);
                    let branch = session.map_or("", |s| s.branch.as_str());
                    let checked_out = session.map_or("", Session::checked_out);
                    query.matches(
                        &[&n.to_string(), title, branch, checked_out],
                        issue.map(Facets::from).unwrap_or_default(),
                    )
                })
                .collect();
            sort::sessions(&mut rows, state.tabs.sessions.sort, activity);
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
    let stored = state.tabs.selected(state.tab);
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
                .or_else(|| state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)))
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

/// Issue `n` has a session whose branch has a merged PR. Its work is done,
/// whatever its window still does.
pub fn session_merged(state: &State, n: IssueNumber) -> bool {
    state.sessions.get(&n).and_then(|s| pr_for_session(state, s)).is_some_and(|pr| pr.state == PrState::Merged)
}

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    state.prs.items.iter().find(|p| p.head.branch() == Some(&session.branch))
}
