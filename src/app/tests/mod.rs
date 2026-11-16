use super::dialog::*;
use super::feed::*;
use super::keys::*;
use super::list::*;
use super::menus::*;
use super::merge::*;
use super::mouse::*;
use super::msg::*;
use super::toast::*;
use super::*;
use crate::config::{Config, SETTINGS, Setting};
use crate::domain::{BranchName, GithubRef, IssueNumber, MergeMethod, Mode, PrNumber, PrState, PullRequest};
use crate::domain::{Mergeability, WorktreePath};
use crate::glyphs::IconSet;
use crate::merge::{MergeOutcome, MergeRun};
use crate::rules::{self, MergeProblem};
use crate::session::Window;
use crate::session::request::{Launch, StartFrom, StartRequest};
use crate::session::{Session, StartOutcome, WindowId};
use crate::sort::{Cycle, IssueSort};
use crate::testutil::{issue, pr};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
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

pub(super) fn session(n: u32, window: Option<(bool, bool)>) -> Session {
    Session {
        issue: IssueNumber(n),
        branch: BranchName::parse(&format!("fix/{n}-x")).expect("branch"),
        worktree: WorktreePath::planned(PathBuf::from(format!("/w/orbit-{n}"))),
        window: window.map(|(alive, waiting)| Window {
            id: WindowId::parse(&format!("@{n}")).expect("id"),
            alive,
            waiting,
        }),
    }
}

/// Three issues (#3 bug, #2 docs, #1 feature) and two open PRs (#10
/// newer, #11), idle jobs.
pub(super) fn app() -> (State, Instant) {
    let now = Instant::now();
    let (mut state, _) = init(now, Config::default(), Context::default());
    state.issues = Feed {
        items: vec![
            issue(3, "Retry queue drops jobs", &["bug"]),
            issue(2, "Docs for plugins", &["docs"]),
            issue(1, "Add json output", &["feature"]),
        ],
        status: FeedStatus::Fresh { at: now },
    };
    let newer = jiff::Timestamp::from_second(60).expect("in range");
    let items = vec![PullRequest { created_at: newer, ..pr(10, "fix/3-x", &[3]) }, pr(11, "other", &[])];
    state.prs = Feed { items, status: FeedStatus::Fresh { at: now } };
    state.inflight = Inflight::default();
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
            Cmd::Job(Job::Start(r, _)) => Some(r),
            _ => None,
        })
        .collect()
}

pub(super) fn sends(cmds: &[Cmd]) -> Vec<(IssueNumber, &str)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Send { issue, text }) => Some((*issue, text.as_str())),
            _ => None,
        })
        .collect()
}

pub(super) fn toast(state: &State) -> String {
    state.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}
