//! Tab completion in the `/` filter. It completes the word at the end of the
//! filter: a term key, or a value for a key. Values come from the items that
//! the shown tab already holds, so completion makes no `gh` call.

use super::State;
use super::filter::{Facets, Has, Is, KEYS, RowState, Word, tokens};
use super::list::Tab;
use crate::domain::{Checks, SessionKey};
use std::collections::BTreeSet;

/// The choices for the word being completed, and the one that is in the
/// filter now. Any key but Tab and Shift-Tab ends it.
pub struct Completion {
    /// The byte where the word starts.
    start: usize,
    choices: Vec<String>,
    at: usize,
}

#[derive(Clone, Copy)]
pub(super) enum Step {
    Next,
    Previous,
}

/// Puts the next or previous choice in place of the word at the end of the
/// filter. With no choice, the filter stays as it is.
pub(super) fn complete(state: &mut State, step: Step) {
    // A sole choice is not kept, so the next Tab goes on from it: `state:`
    // then offers the state words.
    let live = state
        .completion
        .take()
        .filter(|c| c.choices.len() > 1 && state.filter().get(c.start..) == c.choices.get(c.at).map(String::as_str));
    let c = if let Some(mut c) = live {
        c.at = step.from(Some(c.at), c.choices.len());
        c
    } else {
        let start = word_start(state.filter());
        let choices = choices(state, &state.filter()[start..]);
        if choices.is_empty() {
            return;
        }
        let at = step.from(None, choices.len());
        Completion { start, choices, at }
    };
    let filter = state.filter_mut();
    filter.truncate(c.start);
    filter.push_str(&c.choices[c.at]);
    state.completion = Some(c);
}

impl Step {
    /// The index after `at`, wrapping. With no `at`, the first or last.
    pub(super) fn from(self, at: Option<usize>, len: usize) -> usize {
        match (self, at) {
            (Step::Next, Some(at)) => (at + 1) % len,
            (Step::Next, None) => 0,
            (Step::Previous, Some(at)) => (at + len - 1) % len,
            (Step::Previous, None) => len - 1,
        }
    }
}

/// Where the last word of the filter starts, or the end when the filter is
/// empty or ends in a space.
fn word_start(filter: &str) -> usize {
    match tokens(filter).last() {
        Some(t) if t.at.end == filter.len() => t.at.start,
        _ => filter.len(),
    }
}

/// Every word that `word` can grow into, sorted by its key or value, with no repeat.
fn choices(state: &State, word: &str) -> Vec<String> {
    let (dash, body) = word.strip_prefix('-').map_or(("", word), |body| ("-", body));
    match body.split_once(':') {
        None => {
            let keys: BTreeSet<&str> = KEYS.iter().copied().filter(|k| starts(k, body)).collect();
            keys.into_iter().map(|k| format!("{dash}{k}:")).collect()
        }
        Some((key, typed)) => {
            let key = key.to_lowercase();
            values(state, &key, typed.trim_matches('"'))
                .into_iter()
                .map(|v| format!("{dash}{key}:{}", quote(&v)))
                .collect()
        }
    }
}

fn starts(whole: &str, prefix: &str) -> bool {
    whole.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// A value with whitespace needs quotes to stay one word.
fn quote(value: &str) -> String {
    if value.contains(char::is_whitespace) { format!("\"{value}\"") } else { value.to_string() }
}

fn values(state: &State, key: &str, typed: &str) -> BTreeSet<String> {
    let facets = tab_facets(state);
    let owned =
        |items: &mut dyn Iterator<Item = &str>| items.filter(|v| starts(v, typed)).map(str::to_string).collect();
    let logins = |items: &mut dyn Iterator<Item = &str>| -> BTreeSet<String> {
        let bare = typed.trim_start_matches('@');
        let mut found: BTreeSet<String> = items.filter(|v| starts(v, bare)).map(str::to_string).collect();
        if starts("@me", typed) {
            found.insert("@me".to_string());
        }
        found
    };
    match key {
        "label" => owned(&mut facets.iter().flat_map(|f| f.labels).map(String::as_str)),
        "milestone" => owned(&mut facets.iter().filter_map(|f| f.milestone)),
        "assignee" => logins(&mut facets.iter().flat_map(|f| f.assignees).map(String::as_str)),
        "author" => logins(&mut facets.iter().filter_map(|f| f.author)),
        "state" => words(states(state.tab), typed),
        "checks" => words(Checks::ALL, typed),
        "has" => words(Has::ALL, typed),
        "is" => words(Is::ALL, typed),
        _ => BTreeSet::new(),
    }
}

fn words<T: Word>(all: &[T], typed: &str) -> BTreeSet<String> {
    all.iter().map(|w| w.word()).filter(|w| starts(w, typed)).map(str::to_string).collect()
}

/// The `state:` words that a row of the tab can have.
fn states(tab: Tab) -> &'static [RowState] {
    match tab {
        Tab::Issues => &[RowState::Open, RowState::InReview, RowState::Working, RowState::NeedsYou, RowState::Merged],
        Tab::Prs => &[RowState::Open, RowState::Draft, RowState::Merged, RowState::Closed],
        Tab::Sessions => &[RowState::NeedsYou, RowState::Working, RowState::Exited, RowState::Merged],
    }
}

/// The facts of every item that the shown tab holds. The sessions tab holds
/// the issues and PRs of its sessions.
fn tab_facets(state: &State) -> Vec<Facets<'_>> {
    match state.tab {
        Tab::Issues => state.issues.items.iter().map(Facets::from).collect(),
        Tab::Prs => state.prs.items.iter().map(Facets::from).collect(),
        Tab::Sessions => {
            let keys: BTreeSet<SessionKey> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            let issues = state.issues.items.iter().filter(|i| keys.contains(&i.number.into())).map(Facets::from);
            let prs = state.prs.items.iter().filter(|p| keys.contains(&p.number.into())).map(Facets::from);
            issues.chain(prs).collect()
        }
    }
}
