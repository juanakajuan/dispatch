//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::Config;
use crate::domain::{Issue, IssueNumber, MergeMethod, PrNumber, PullRequest, SessionKey};
use crate::session::{Diff, Sessions};
use crossterm::event::KeyEventKind;
use ratatui::layout::Position;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use batch::{BatchStart, batch_key};
use dialog::{StartDialog, dialog_key};
use diff::{diff_overlay_key, read_diff};
use feed::{Due, Feed, Poll, observed, tick};
use filter::{Viewer, filter_key};
use keys::{act, action_for};
use list::{Tab, Tabs};
use menus::{SettingsMenu, keybinds_key, settings_key};
use merge::{Armed, handed_off_merged, merged, removed_after_merge};
use message::{message_key, sent};
use mouse::{HitMap, HitTarget, on_mouse};
use msg::{Cmd, Msg, ReadJob, SESSIONS_EVERY};
use open::{copied, opened, page_url};
use remove::{cleaned, removed};
use sessions::{attached, resumed, started, stopped};
use summary::summary_overlay_key;
use toast::Toast;

pub mod batch;
pub mod complete;
pub mod dialog;
pub mod diff;
pub mod feed;
pub mod filter;
pub mod keys;
pub mod list;
pub mod menus;
pub mod merge;
pub mod message;
pub mod mouse;
pub mod msg;
pub mod open;
pub mod remove;
pub mod sessions;
pub mod summary;
#[cfg(test)]
mod tests;
pub mod toast;

/// Where keys go. One enum, so "dialog open while typing a filter" can not
/// be represented.
pub enum Focus {
    List,
    Filter,
    Dialog(Box<StartDialog>),
    /// The start dialog for every marked issue.
    Batch(Box<BatchStart>),
    Settings(Box<SettingsMenu>),
    /// The one-line message field for a running session, from `t`.
    Message {
        key: SessionKey,
        text: String,
    },
    /// The keybinds overlay, scrolled down by `scroll` lines.
    Keybinds {
        scroll: u16,
    },
    /// All of what the session's Claude last said, scrolled down by `scroll` lines.
    Summary {
        key: SessionKey,
        scroll: u16,
    },
    /// The session's worktree diff, scrolled down by `scroll` lines.
    Diff {
        key: SessionKey,
        scroll: u16,
        body: DiffBody,
    },
}

/// What the diff overlay holds: the read is in flight, or it landed.
pub enum DiffBody {
    Reading,
    Read(Diff),
}

/// Facts about the repo that the view shows, resolved once at startup.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct Context {
    pub repo_name: String,
    pub open_with: OpenWith,
    pub start_preview: crate::session::StartPreview,
}

/// What `o` does with a page. Over SSH, `gh --web` opens a browser on the
/// remote machine, so dispatch copies the URL to the local clipboard instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenWith {
    #[default]
    Browser,
    Clipboard,
}

pub struct State {
    pub context: Context,
    /// The live config. The settings menu edits a copy. `enter` puts the copy
    /// here, and `WriteJob::SaveConfig` writes the changes to the file.
    pub config: Config,
    pub tab: Tab,
    pub issues: Feed<Issue>,
    pub prs: Feed<PullRequest>,
    /// Last observation from `Workspace::observe`. Replaced whole on each
    /// observation. A finished start adds its own session at once, so the row
    /// does not blink out until the next observation.
    pub sessions: Sessions,
    pub observe: Poll,
    /// Sessions with a `WriteJob::Start` in flight. The only session-ish state the
    /// app owns; it covers the gap before the next observation shows the window.
    pub starting: BTreeSet<SessionKey>,
    /// Issues marked with Space for a batch start.
    pub marked: BTreeSet<IssueNumber>,
    pub tabs: Tabs,
    /// Tab completion under way in the filter.
    pub completion: Option<complete::Completion>,
    pub viewer: Viewer,
    pub focus: Focus,
    pub toast: Option<Toast>,
    pub due: Due,
    pub hits: HitMap,
    pub last_click: Option<(HitTarget, Instant)>,
    /// The cell under the mouse pointer, from the last mouse event. `None`
    /// until the mouse moves, and after the terminal loses focus. The view
    /// tints the target under it.
    pub pointer: Option<Position>,
    /// Set by an `m` or `D` that waits for one more press of the same key on
    /// the same PR or issue. Any other key clears it.
    pub armed: Option<Armed>,
    /// PRs that `m` gave to an agent to fix and merge, with the method it was
    /// told to use. The agent merges, so the PR refresh that shows one merged
    /// does what a direct merge does.
    pub handed_off: BTreeMap<PrNumber, MergeMethod>,
    pub transcripts: std::collections::BTreeMap<SessionKey, crate::transcript::Transcript>,
    /// What each session committed, left uncommitted, and pushed.
    pub work: crate::session::Works,
}

pub fn init(now: Instant, config: Config, context: Context) -> (State, Vec<Cmd>) {
    let mut state = State {
        context,
        tab: Tab::Issues,
        issues: Feed::new(),
        prs: Feed::new(),
        sessions: Sessions::new(),
        observe: Poll::default(),
        starting: BTreeSet::new(),
        marked: BTreeSet::new(),
        tabs: Tabs::default(),
        completion: None,
        viewer: Viewer::Unknown,
        focus: Focus::List,
        toast: None,
        due: Due {
            github: now + config.github_refresh,
            github_soon: false,
            github_sent: now,
            sessions: now + SESSIONS_EVERY,
        },
        hits: HitMap::default(),
        last_click: None,
        pointer: None,
        armed: None,
        handed_off: BTreeMap::new(),
        transcripts: std::collections::BTreeMap::new(),
        work: crate::session::Works::new(),
        config,
    };
    let cmds = [
        state.issues.poll.request(ReadJob::FetchIssues),
        state.prs.poll.request(ReadJob::FetchPrs),
        state.observe.request(ReadJob::ObserveSessions),
        Some(Cmd::Job(msg::Job::Read(ReadJob::FetchViewer))),
    ];
    (state, cmds.into_iter().flatten().collect())
}

impl State {
    /// The `/` filter of the shown tab.
    pub fn filter(&self) -> &str {
        self.tabs.filter(self.tab)
    }

    fn filter_mut(&mut self) -> &mut String {
        self.tabs.filter_mut(self.tab)
    }
}

/// The reducer.
pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(ev) if ev.kind == KeyEventKind::Release => Vec::new(),
        Msg::Key(ev) => match &state.focus {
            Focus::Dialog(_) => dialog_key(state, &ev),
            Focus::Batch(_) => batch_key(state, &ev),
            Focus::Filter => filter_key(state, &ev),
            Focus::Message { .. } => message_key(state, &ev),
            Focus::Settings(_) => settings_key(state, &ev),
            Focus::Keybinds { .. } => keybinds_key(state, &ev),
            Focus::Summary { .. } => summary_overlay_key(state, &ev),
            Focus::Diff { .. } => diff_overlay_key(state, &ev),
            Focus::List => {
                let armed = state.armed.take();
                match action_for(&ev) {
                    Some(action) => act(state, action, armed.as_ref()),
                    None => Vec::new(),
                }
            }
        },
        Msg::Mouse(ev, now) => on_mouse(state, ev, now),
        Msg::Tick(now) => tick(state, now),
        Msg::Issues { result, at } => {
            state.issues.landed(result, at);
            Vec::new()
        }
        Msg::Prs { result, at } => {
            state.prs.landed(result, at);
            handed_off_merged(state)
        }
        Msg::Viewer(result) => {
            state.viewer = result.map_or(Viewer::Failed, Viewer::Login);
            Vec::new()
        }
        Msg::Observed(result) => observed(state, result),
        Msg::Started { key, result } => started(state, key, result),
        Msg::Stopped { key, result } => stopped(state, key, result),
        Msg::Resumed { key, result } => resumed(state, key, result),
        Msg::Removed { key, result } => removed(state, key, result),
        Msg::RemovedAfterMerge { pr, method, key, session, result } => {
            removed_after_merge(state, pr, method, key, session, result)
        }
        Msg::Cleaned(report) => cleaned(state, &report),
        Msg::Opened { target, result } => opened(state, target, result),
        Msg::PageUrl { target, result } => page_url(state, target, result),
        Msg::Copied { target, result } => copied(state, target, result),
        Msg::Sent { key, kind, result } => sent(state, key, kind, result),
        Msg::Merged { ask, run } => merged(state, ask, run),
        Msg::Attached { key, to, result } => attached(state, key, to, result),
        Msg::Diff { key, result } => read_diff(state, key, result),
        Msg::Saved(result) => {
            if let Err(e) = result {
                toast::error(state, toast::failed("Saving settings", e));
            }
            Vec::new()
        }
        Msg::FocusGained => {
            state.due.github_soon = true;
            Vec::new()
        }
        Msg::FocusLost => {
            state.pointer = None;
            Vec::new()
        }
    }
}
