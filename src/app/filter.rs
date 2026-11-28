//! The `/` filter. Free text matches as a substring of the row's text. A term
//! (`key:value`) matches one fact about the row, ignoring case. A `-` before a
//! term key keeps the rows that the term does not match. A row shows when the
//! text and every term match.

use super::complete::{self, Step};
use super::keys::{Key, key_of};
use super::list::{Group, IssueGroup, PrGroup, SessionGroup};
use super::msg::Cmd;
use super::{Focus, State};
use crate::domain::{Checks, Issue, PullRequest};
use crossterm::event::{KeyCode, KeyEvent};
use std::ops::Range;

pub(super) fn filter_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let key = key_of(ev);
    match key {
        Some(Key::Code(KeyCode::Tab)) => return tab(state, Step::Next),
        Some(Key::Code(KeyCode::BackTab)) => return tab(state, Step::Previous),
        _ => state.completion = None,
    }
    match key {
        Some(Key::Code(KeyCode::Enter)) => state.focus = Focus::List,
        Some(Key::Code(KeyCode::Esc)) => {
            state.filter_mut().clear();
            state.focus = Focus::List;
        }
        Some(Key::Code(KeyCode::Down)) => cycle_saved(state, Step::Next),
        Some(Key::Code(KeyCode::Up)) => cycle_saved(state, Step::Previous),
        Some(Key::Ctrl('c')) => return vec![Cmd::Quit],
        _ => {
            state.filter_mut().edit(ev);
        }
    }
    Vec::new()
}

fn tab(state: &mut State, step: Step) -> Vec<Cmd> {
    complete::complete(state, step);
    Vec::new()
}

/// `Down` puts the saved filter after the one in the prompt, and `Up` the one
/// before it, wrapping at the ends. Text that is not a saved filter goes to
/// the first one (`Down`) or the last one (`Up`).
fn cycle_saved(state: &mut State, step: Step) {
    let saved = &state.config.saved_filters;
    let n = saved.len();
    if n == 0 {
        return;
    }
    let at = saved.iter().position(|f| f == state.filter());
    let next = saved[step.from(at, n)].clone();
    state.filter_mut().set(&next);
}

/// One word of the typed filter, with the bytes it came from, so the prompt
/// can show which words are terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub at: Range<usize>,
    pub kind: TokenKind,
}

/// A `-` before a term key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Include,
    /// Keeps the rows that the term does not match.
    Exclude,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Text,
    Term(Sign, Term),
    /// A term key with no value yet, as in `label:` while it is typed. It
    /// matches every row.
    Incomplete,
    /// A term key with a value outside its fixed set, as in `state:bogus`. It
    /// matches no row, whatever its sign.
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Label(String),
    Assignee(Who),
    Author(Who),
    Milestone(String),
    State(RowState),
    Checks(Checks),
    Has(Has),
    Is(Is),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    /// `@me`: the login `gh` is signed in as.
    Me,
    Login(String),
}

/// A word from a fixed set, as `state:` and `has:` take.
pub trait Word: Sized + Copy + 'static {
    const ALL: &'static [Self];
    fn word(self) -> &'static str;

    fn parse(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|w| w.word().eq_ignore_ascii_case(value))
    }
}

/// What `state:` names. Each tab has its own groups, and `RowState::of` is
/// the one place that maps them to words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Open,
    Draft,
    Merged,
    Closed,
    NeedsYou,
    Working,
    Exited,
    InReview,
}

impl RowState {
    pub fn of(group: Group) -> RowState {
        match group {
            Group::Issue(IssueGroup::NeedsYou) | Group::Session(SessionGroup::NeedsYou) => RowState::NeedsYou,
            Group::Issue(IssueGroup::Working) | Group::Session(SessionGroup::Working) => RowState::Working,
            Group::Issue(IssueGroup::InReview) => RowState::InReview,
            Group::Issue(IssueGroup::Merged) | Group::Pr(PrGroup::Merged) | Group::Session(SessionGroup::Merged) => {
                RowState::Merged
            }
            Group::Issue(IssueGroup::Open) | Group::Pr(PrGroup::ReadyForReview) => RowState::Open,
            Group::Pr(PrGroup::Draft) => RowState::Draft,
            Group::Pr(PrGroup::Closed) => RowState::Closed,
            Group::Session(SessionGroup::Exited) => RowState::Exited,
        }
    }
}

impl Word for RowState {
    const ALL: &'static [RowState] = &[
        RowState::Open,
        RowState::Draft,
        RowState::Merged,
        RowState::Closed,
        RowState::NeedsYou,
        RowState::Working,
        RowState::Exited,
        RowState::InReview,
    ];

    fn word(self) -> &'static str {
        match self {
            RowState::Open => "open",
            RowState::Draft => "draft",
            RowState::Merged => "merged",
            RowState::Closed => "closed",
            RowState::NeedsYou => "needs-you",
            RowState::Working => "working",
            RowState::Exited => "exited",
            RowState::InReview => "in-review",
        }
    }
}

impl Word for Checks {
    const ALL: &'static [Checks] = &[Checks::Pass, Checks::Fail, Checks::Pending, Checks::None];

    fn word(self) -> &'static str {
        match self {
            Checks::Pass => "pass",
            Checks::Fail => "fail",
            Checks::Pending => "pending",
            Checks::None => "none",
        }
    }
}

/// `has:`: the row has a pull request, or a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Has {
    Pr,
    Session,
}

impl Word for Has {
    const ALL: &'static [Has] = &[Has::Pr, Has::Session];

    fn word(self) -> &'static str {
        match self {
            Has::Pr => "pr",
            Has::Session => "session",
        }
    }
}

/// `is:`: the row is from a fork, or marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Is {
    Fork,
    Marked,
}

impl Word for Is {
    const ALL: &'static [Is] = &[Is::Fork, Is::Marked];

    fn word(self) -> &'static str {
        match self {
            Is::Fork => "fork",
            Is::Marked => "marked",
        }
    }
}

/// The login `gh` is signed in as, which `@me` names. One fetch
/// at start sets it. While it is not `Login`, `@me` matches no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    Unknown,
    Login(String),
    /// The fetch failed. The prompt shows `@me` as an error.
    Failed,
}

impl Viewer {
    pub fn login(&self) -> Option<&str> {
        match self {
            Viewer::Login(login) => Some(login),
            Viewer::Unknown | Viewer::Failed => None,
        }
    }
}

/// The facts that terms read from one row.
#[derive(Clone, Debug, Default)]
pub struct Facets<'a> {
    pub labels: &'a [String],
    pub assignees: &'a [String],
    pub milestone: Option<&'a str>,
    pub author: Option<&'a str>,
    pub group: Option<Group>,
    /// `None` when the row has no pull request.
    pub checks: Option<Checks>,
    pub has: Vec<Has>,
    pub is: Vec<Is>,
}

impl<'a> From<&'a Issue> for Facets<'a> {
    fn from(i: &'a Issue) -> Self {
        Facets {
            labels: &i.labels,
            assignees: &i.assignees,
            milestone: i.milestone.as_deref(),
            author: Some(&i.author),
            ..Facets::default()
        }
    }
}

impl<'a> From<&'a PullRequest> for Facets<'a> {
    fn from(p: &'a PullRequest) -> Self {
        Facets {
            labels: &p.labels,
            assignees: &p.assignees,
            milestone: p.milestone.as_deref(),
            author: Some(&p.author),
            ..Facets::default()
        }
    }
}

/// Splits on whitespace outside double quotes, so `label:"good first issue"`
/// is one word.
pub fn tokens(raw: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut quoted = false;
    for (i, c) in raw.char_indices() {
        match (start, c.is_whitespace() && !quoted) {
            (None, true) => {}
            (None, false) => {
                start = Some(i);
                quoted = c == '"';
            }
            (Some(from), true) => {
                out.push(token(raw, from..i));
                start = None;
            }
            (Some(_), false) => quoted ^= c == '"',
        }
    }
    if let Some(from) = start {
        out.push(token(raw, from..raw.len()));
    }
    out
}

/// The term keys, in the order that completion offers them.
pub const KEYS: [&str; 8] = ["label", "author", "assignee", "milestone", "state", "checks", "has", "is"];

fn token(raw: &str, at: Range<usize>) -> Token {
    let word = &raw[at.clone()];
    let (sign, body) = match word.strip_prefix('-') {
        Some(body) => (Sign::Exclude, body),
        None => (Sign::Include, word),
    };
    let kind = match body.split_once(':') {
        Some((key, value)) => term_kind(&key.to_lowercase(), value.trim_matches('"'), sign),
        None => TokenKind::Text,
    };
    Token { at, kind }
}

fn term_kind(key: &str, value: &str, sign: Sign) -> TokenKind {
    if !KEYS.contains(&key) {
        return TokenKind::Text;
    }
    if value.is_empty() {
        return TokenKind::Incomplete;
    }
    let who = || {
        if value.eq_ignore_ascii_case("@me") { Who::Me } else { Who::Login(value.trim_start_matches('@').to_string()) }
    };
    let term = match key {
        "label" => Some(Term::Label(value.to_string())),
        "milestone" => Some(Term::Milestone(value.to_string())),
        "assignee" => Some(Term::Assignee(who())),
        "author" => Some(Term::Author(who())),
        "state" => RowState::parse(value).map(Term::State),
        "checks" => Checks::parse(value).map(Term::Checks),
        "has" => Has::parse(value).map(Term::Has),
        _ => Is::parse(value).map(Term::Is),
    };
    term.map_or(TokenKind::Invalid, |t| TokenKind::Term(sign, t))
}

/// The parsed filter. `viewer` is the login that `@me` names.
pub struct Query<'a> {
    text: String,
    terms: Vec<(Sign, Term)>,
    /// A term had a value outside its set, so no row matches.
    invalid: bool,
    viewer: Option<&'a str>,
}

impl<'a> Query<'a> {
    pub fn parse(raw: &str, viewer: Option<&'a str>) -> Query<'a> {
        let mut text = Vec::new();
        let mut terms = Vec::new();
        let mut invalid = false;
        for t in tokens(raw) {
            match t.kind {
                TokenKind::Text => text.push(&raw[t.at]),
                TokenKind::Term(sign, term) => terms.push((sign, term)),
                TokenKind::Incomplete => {}
                TokenKind::Invalid => invalid = true,
            }
        }
        Query { text: text.join(" ").to_lowercase(), terms, invalid, viewer }
    }

    /// `text` is the row's searchable words: number, title, and the like.
    pub fn matches(&self, text: &[&str], facets: &Facets<'_>) -> bool {
        let text_ok = self.text.is_empty() || text.join(" ").to_lowercase().contains(&self.text);
        text_ok
            && !self.invalid
            && self.terms.iter().all(|(sign, t)| self.term_matches(t, facets) == (*sign == Sign::Include))
    }

    fn term_matches(&self, term: &'a Term, facets: &Facets<'_>) -> bool {
        let any = |items: &[String], want: &str| items.iter().any(|i| same(i, want));
        let who = |who: &'a Who| match who {
            Who::Login(want) => Some(want.as_str()),
            Who::Me => self.viewer,
        };
        match term {
            Term::Label(want) => any(facets.labels, want),
            Term::Milestone(want) => facets.milestone.is_some_and(|m| same(m, want)),
            Term::Assignee(w) => who(w).is_some_and(|want| any(facets.assignees, want)),
            Term::Author(w) => who(w).zip(facets.author).is_some_and(|(want, author)| same(author, want)),
            Term::State(want) => facets.group.is_some_and(|g| RowState::of(g) == *want),
            Term::Checks(want) => facets.checks == Some(*want),
            Term::Has(want) => facets.has.contains(want),
            Term::Is(want) => facets.is.contains(want),
        }
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
