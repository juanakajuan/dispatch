use super::dialog::*;
use super::feed::*;
use super::keys::*;
use super::list::*;
use super::menus::*;
use super::mouse::*;
use super::msg::*;
use super::toast::*;
use super::*;
use crate::config::Config;
use crate::config::settings::{SETTINGS, Setting};
use crate::domain::{BranchName, GithubRef, IssueNumber, MergeMethod, Mode, PrNumber, PrState, PullRequest};
use crate::domain::{Mergeability, WorktreePath};
use crate::github::GhError;
use crate::glyphs::IconSet;
use crate::merge::job::{MergeOutcome, MergeRun};
use crate::merge::plan::{self, MergeProblem};
use crate::session::error::SessionError;
use crate::session::request::{Launch, StartFrom, StartRequest};
use crate::session::{Session, SessionState, StartOutcome};
use crate::sort::{Cycle, IssueSort};
use crate::testutil::{issue, pr};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod dialog;
mod keys;
mod list;
mod menus;
mod merge;
mod open;
mod refresh;
mod sessions;
mod sort;

pub(super) fn key(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

pub(super) fn code(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

pub(super) fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

pub(super) fn click(col: u16, row: u16, at: Instant) -> Msg {
    Msg::Mouse(
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: KeyModifiers::NONE },
        at,
    )
}

pub(super) fn session(n: u32, state: SessionState) -> Session {
    Session {
        issue: IssueNumber(n),
        branch: BranchName::parse(&format!("fix/{n}-x")).expect("branch"),
        worktree: WorktreePath::planned(PathBuf::from(format!("/w/orbit-{n}"))),
        state,
    }
}

/// Three issues (#3 bug, #2 docs, #1 feature) and two open PRs (#10
/// newer, #11), idle jobs.
pub(super) fn app() -> (State, Instant) {
    let now = Instant::now();
    let (mut state, _) = init(now, Config::default(), Context::default());
    state.issues.items = vec![
        issue(3, "Retry queue drops jobs", &["bug"]),
        issue(2, "Docs for plugins", &["docs"]),
        issue(1, "Add json output", &["feature"]),
    ];
    state.issues.last = LastFetch::Fresh { at: now };
    let newer = jiff::Timestamp::from_second(60).expect("in range");
    let items = vec![PullRequest { created_at: newer, ..pr(10, "fix/3-x", &[3]) }, pr(11, "other", &[])];
    state.prs.items = items;
    state.prs.last = LastFetch::Fresh { at: now };
    state.issues.poll.done();
    state.prs.poll.done();
    state.observe.done();
    (state, now)
}

pub(super) fn press(state: &mut State, msg: Msg) -> Vec<Cmd> {
    update(state, msg)
}

pub(super) fn dialog(state: &State) -> &StartDialog {
    match &state.focus {
        Focus::Dialog(d) => d,
        _ => panic!("no dialog is open"),
    }
}

pub(super) fn starts(cmds: &[Cmd]) -> Vec<&StartRequest> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Start(r, _))) => Some(r),
            _ => None,
        })
        .collect()
}

pub(super) fn sends(cmds: &[Cmd]) -> Vec<(IssueNumber, &str)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Send { issue, text })) => Some((*issue, text.as_str())),
            _ => None,
        })
        .collect()
}

pub(super) fn toast(state: &State) -> String {
    state.toast.as_ref().map(|t| t.text().to_string()).unwrap_or_default()
}

fn gh_error(stderr: &str) -> GhError {
    GhError::Run(crate::proc::fake::fail("gh", 1, stderr))
}

fn git_error(stderr: &str) -> SessionError {
    SessionError::Git(crate::proc::fake::fail("git", 128, stderr))
}
