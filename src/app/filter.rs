//! The `/` filter. Free text matches as a substring of the row's text. A
//! `label:`, `assignee:`, or `milestone:` term matches one field exactly,
//! ignoring case. A row shows when the text and every term match.

use super::keys::{Key, key_of};
use super::msg::Cmd;
use super::{Focus, State};
use crate::domain::{Issue, PullRequest};
use crossterm::event::{KeyCode, KeyEvent};
use std::ops::Range;

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

/// One word of the typed filter, with the bytes it came from, so the prompt
/// can show which words are terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub at: Range<usize>,
    pub kind: TokenKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Text,
    Term(Term),
    /// A term key with no value yet, as in `label:` while it is typed. It
    /// matches every row.
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Label(String),
    Assignee(Assignee),
    Milestone(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assignee {
    /// `@me`: the login `gh` is signed in as.
    Me,
    Login(String),
}

/// The login `gh` is signed in as, which `assignee:@me` names. One fetch
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

/// The fields that terms read from one row.
#[derive(Clone, Copy, Debug, Default)]
pub struct Facets<'a> {
    pub labels: &'a [String],
    pub assignees: &'a [String],
    pub milestone: Option<&'a str>,
}

impl<'a> From<&'a Issue> for Facets<'a> {
    fn from(i: &'a Issue) -> Self {
        Facets { labels: &i.labels, assignees: &i.assignees, milestone: i.milestone.as_deref() }
    }
}

impl<'a> From<&'a PullRequest> for Facets<'a> {
    fn from(p: &'a PullRequest) -> Self {
        Facets { labels: &p.labels, assignees: &p.assignees, milestone: p.milestone.as_deref() }
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

fn token(raw: &str, at: Range<usize>) -> Token {
    let word = &raw[at.clone()];
    let kind = match word.split_once(':') {
        Some((key, value)) => {
            let value = value.trim_matches('"');
            match (key.to_lowercase().as_str(), value.is_empty()) {
                ("label" | "assignee" | "milestone", true) => TokenKind::Incomplete,
                ("label", false) => TokenKind::Term(Term::Label(value.to_string())),
                ("milestone", false) => TokenKind::Term(Term::Milestone(value.to_string())),
                ("assignee", false) if value.eq_ignore_ascii_case("@me") => {
                    TokenKind::Term(Term::Assignee(Assignee::Me))
                }
                ("assignee", false) => {
                    TokenKind::Term(Term::Assignee(Assignee::Login(value.trim_start_matches('@').to_string())))
                }
                _ => TokenKind::Text,
            }
        }
        None => TokenKind::Text,
    };
    Token { at, kind }
}

/// The parsed filter. `viewer` is the login that `@me` names.
pub struct Query<'a> {
    text: String,
    terms: Vec<Term>,
    viewer: Option<&'a str>,
}

impl<'a> Query<'a> {
    pub fn parse(raw: &str, viewer: Option<&'a str>) -> Query<'a> {
        let mut text = Vec::new();
        let mut terms = Vec::new();
        for t in tokens(raw) {
            match t.kind {
                TokenKind::Text => text.push(&raw[t.at]),
                TokenKind::Term(term) => terms.push(term),
                TokenKind::Incomplete => {}
            }
        }
        Query { text: text.join(" ").to_lowercase(), terms, viewer }
    }

    /// `text` is the row's searchable words: number, title, and the like.
    pub fn matches(&self, text: &[&str], facets: Facets<'_>) -> bool {
        let text_ok = self.text.is_empty() || text.join(" ").to_lowercase().contains(&self.text);
        text_ok && self.terms.iter().all(|t| self.term_matches(t, facets))
    }

    fn term_matches(&self, term: &Term, facets: Facets<'_>) -> bool {
        let any = |items: &[String], want: &str| items.iter().any(|i| same(i, want));
        match term {
            Term::Label(want) => any(facets.labels, want),
            Term::Milestone(want) => facets.milestone.is_some_and(|m| same(m, want)),
            Term::Assignee(Assignee::Login(want)) => any(facets.assignees, want),
            Term::Assignee(Assignee::Me) => self.viewer.is_some_and(|me| any(facets.assignees, me)),
        }
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
