use super::State;
use super::filter::{Facets, Has, Is, Query};
use super::text_field::TextField;
use super::toast;
use crate::domain::{GithubRef, Issue, IssueNumber, PrBranch, PrNumber, PrState, PullRequest, SessionKey};
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
        RowKey::Issue(n) => Group::Issue(match activity(state, n.into()) {
            _ if session_merged(state, n.into()) => IssueGroup::Merged,
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
    Session(SessionKey),
}

/// The page `o` opens for a row. A session row opens its issue or PR. Exhaustive,
/// so a new row kind does not compile until it says what `o` opens.
impl From<RowKey> for GithubRef {
    fn from(key: RowKey) -> GithubRef {
        match key {
            RowKey::Issue(n) => GithubRef::Issue(n),
            RowKey::Pr(n) => GithubRef::Pr(n),
            RowKey::Session(key) => key.into(),
        }
    }
}

/// What each tab remembers while another tab is shown.
#[derive(Default)]
pub struct Tabs {
    pub issues: TabState<IssueNumber, IssueSort>,
    pub prs: TabState<PrNumber, PrSort>,
    pub sessions: TabState<SessionKey, SessionSort>,
}

/// A tab's selected row, sort mode, and `/` filter. Selection stores the
/// key, see `RowKey`. Each tab has its own filter, so a filter on one tab
/// never hides rows on another.
pub struct TabState<K, S> {
    pub selected: Option<K>,
    pub sort: S,
    pub filter: TextField,
}

impl<K, S: Default> Default for TabState<K, S> {
    fn default() -> Self {
        TabState { selected: None, sort: S::default(), filter: TextField::default() }
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

    pub fn filter(&self, tab: Tab) -> &str {
        self.filter_field(tab).text()
    }

    pub fn filter_field(&self, tab: Tab) -> &TextField {
        match tab {
            Tab::Issues => &self.issues.filter,
            Tab::Prs => &self.prs.filter,
            Tab::Sessions => &self.sessions.filter,
        }
    }

    pub(super) fn filter_mut(&mut self, tab: Tab) -> &mut TextField {
        match tab {
            Tab::Issues => &mut self.issues.filter,
            Tab::Prs => &mut self.prs.filter,
            Tab::Sessions => &mut self.sessions.filter,
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

pub(super) fn closed_issue_of_pr(state: &State) -> Option<IssueNumber> {
    let Some(RowKey::Pr(n)) = selected_row(state).filter(|_| state.tab == Tab::Prs) else { return None };
    state.prs.items.iter().find(|p| p.number == n).and_then(|p| p.closes.first().copied())
}

pub(super) fn go_to_issue_of_pr(state: &mut State) {
    let Some(RowKey::Pr(n)) = selected_row(state) else { return };
    match closed_issue_of_pr(state) {
        Some(issue) => jump_to(state, RowKey::Issue(issue)),
        None => toast::info(state, format!("PR {n} closes no issue.")),
    }
}

/// Shows the tab of `key` with `key` selected. That tab's filter is cleared
/// only when it would hide `key`. The filter of the tab the jump came from
/// stays.
pub(super) fn jump_to(state: &mut State, key: RowKey) {
    let tab = match key {
        RowKey::Issue(_) => Tab::Issues,
        RowKey::Pr(_) => Tab::Prs,
        RowKey::Session(_) => Tab::Sessions,
    };
    state.tab = tab;
    state.tabs.select(key);
    if !visible_rows(state).contains(&key) {
        state.tabs.filter_mut(tab).clear();
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
    let query = Query::parse(state.filter(), state.viewer.login());
    let issue_activity = |n: IssueNumber| activity(state, n.into());
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
                    query.matches(&text, &facets(state, RowKey::Issue(i.number)))
                })
                .collect();
            sort::issues(&mut rows, state.tabs.issues.sort, issue_activity);
            rows.into_iter().map(|i| RowKey::Issue(i.number)).collect()
        }
        Tab::Prs => {
            let mut rows: Vec<&PullRequest> = state
                .prs
                .items
                .iter()
                .filter(|p| {
                    query.matches(
                        &[&p.number.to_string(), &p.title, p.head.display()],
                        &facets(state, RowKey::Pr(p.number)),
                    )
                })
                .collect();
            sort::prs(&mut rows, state.tabs.prs.sort);
            rows.into_iter().map(|p| RowKey::Pr(p.number)).collect()
        }
        Tab::Sessions => {
            let keys: BTreeSet<SessionKey> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            let mut rows: Vec<SessionKey> = keys
                .into_iter()
                .filter(|key| {
                    let title = session_title(state, *key);
                    let session = state.sessions.get(key);
                    let branch = session.map_or("", |s| s.branch.as_str());
                    let checked_out = session.map_or("", Session::checked_out);
                    query
                        .matches(&[&key.to_string(), title, branch, checked_out], &facets(state, RowKey::Session(*key)))
                })
                .collect();
            sort::sessions(&mut rows, state.tabs.sessions.sort, |key| activity(state, key));
            rows.into_iter().map(RowKey::Session).collect()
        }
    }
}

/// The title of what a session is for: its issue or its PR.
pub fn session_title(state: &State, key: SessionKey) -> &str {
    match key {
        SessionKey::Issue(n) => state.issues.items.iter().find(|i| i.number == n).map_or("", |i| i.title.as_str()),
        SessionKey::Pr(n) => state.prs.items.iter().find(|p| p.number == n).map_or("", |p| p.title.as_str()),
    }
}

/// What the filter terms read from a row. One function for all tabs, so a
/// term means the same on each. A session row reads its issue or its PR.
fn facets(state: &State, key: RowKey) -> Facets<'_> {
    let group = Some(group_of(state, key));
    match key {
        RowKey::Issue(n) | RowKey::Session(SessionKey::Issue(n)) => {
            let lead = lead_pr(state, n);
            let session = SessionKey::Issue(n);
            let mut has = Vec::new();
            has.extend(lead.map(|_| Has::Pr));
            has.extend(
                (state.sessions.contains_key(&session) || state.starting.contains(&session)).then_some(Has::Session),
            );
            let mut is = Vec::new();
            is.extend(lead.filter(|p| matches!(p.head, PrBranch::Fork(_))).map(|_| Is::Fork));
            is.extend(state.marked.contains(&n).then_some(Is::Marked));
            let base = state.issues.items.iter().find(|i| i.number == n).map(Facets::from).unwrap_or_default();
            Facets { group, checks: lead.map(|p| p.checks), has, is, ..base }
        }
        RowKey::Pr(n) | RowKey::Session(SessionKey::Pr(n)) => {
            let Some(pr) = state.prs.items.iter().find(|p| p.number == n) else {
                return Facets { group, has: vec![Has::Pr, Has::Session], ..Facets::default() };
            };
            let mut has = vec![Has::Pr];
            let own = SessionKey::Pr(n);
            let has_session = matches!(key, RowKey::Session(_))
                || state.starting.contains(&own)
                || session_of_pr(state, pr).is_some();
            has.extend(has_session.then_some(Has::Session));
            let is = if matches!(pr.head, PrBranch::Fork(_)) { vec![Is::Fork] } else { Vec::new() };
            Facets { group, checks: Some(pr.checks), has, is, ..Facets::from(pr) }
        }
    }
}

/// The state of the session `key` for sorting. A start in flight counts as
/// running, so the row does not jump when the window appears.
pub(super) fn activity(state: &State, key: SessionKey) -> Option<SessionState> {
    match state.sessions.get(&key) {
        Some(s) => Some(s.state),
        None => state.starting.contains(&key).then_some(SessionState::Running),
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
        RowKey::Issue(n) => state.sessions.get(&n.into()),
        RowKey::Session(key) => state.sessions.get(&key),
        RowKey::Pr(n) => session_of_pr(state, state.prs.items.iter().find(|p| p.number == n)?),
    }
}

/// The session of a PR: one of the issues it closes, its own PR session, or
/// one on its head branch.
fn session_of_pr<'a>(state: &'a State, pr: &PullRequest) -> Option<&'a Session> {
    pr.closes
        .iter()
        .find_map(|c| state.sessions.get(&(*c).into()))
        .or_else(|| state.sessions.get(&pr.number.into()))
        .or_else(|| state.sessions.values().find(|s| pr.head.branch() == Some(&s.branch)))
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

/// The session `key` has a branch with a merged PR. Its work is done,
/// whatever its window still does.
pub fn session_merged(state: &State, key: SessionKey) -> bool {
    state.sessions.get(&key).and_then(|s| pr_for_session(state, s)).is_some_and(|pr| pr.state == PrState::Merged)
}

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    state.prs.items.iter().find(|p| p.head.branch() == Some(&session.branch))
}
