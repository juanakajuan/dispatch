use crate::clipboard::CopyError;
use crate::config::settings::Setting;
use crate::config::{Config, ConfigError};
use crate::domain::{GithubRef, Issue, IssueNumber, PullRequest};
use crate::github::GhError;
use crate::merge::MergeAsk;
use crate::merge::job::MergeRun;
use crate::session::error::{AttachError, SessionError};
use crate::session::request::{Launch, StartRequest};
use crate::session::{CleanReport, RemoveOutcome, RemoveTarget, Sessions, StartOutcome, Works};
use crate::transcript::Transcript;
use crossterm::event::{KeyEvent, MouseEvent};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent, Instant),
    Tick(Instant),
    /// `at` is stamped by the worker when `gh` returns.
    Issues {
        result: Result<Vec<Issue>, GhError>,
        at: Instant,
    },
    Prs {
        result: Result<Vec<PullRequest>, GhError>,
        at: Instant,
    },
    /// The login `gh` is signed in as.
    Viewer(Result<String, GhError>),
    Observed(Result<Observation, SessionError>),
    Started {
        issue: IssueNumber,
        result: Result<StartOutcome, SessionError>,
    },
    Stopped {
        issue: IssueNumber,
        result: Result<(), SessionError>,
    },
    Resumed {
        issue: IssueNumber,
        result: Result<(), SessionError>,
    },
    Removed {
        issue: IssueNumber,
        result: Result<RemoveOutcome, SessionError>,
    },
    Cleaned(CleanReport),
    Attached(Result<(), AttachError>),
    Opened {
        target: GithubRef,
        result: Result<(), GhError>,
    },
    PageUrl {
        target: GithubRef,
        result: Result<String, GhError>,
    },
    Copied {
        target: GithubRef,
        result: Result<String, CopyError>,
    },
    Sent {
        issue: IssueNumber,
        kind: SendKind,
        result: Result<(), SessionError>,
    },
    Merged {
        ask: MergeAsk,
        run: MergeRun,
    },
    /// A setting reached the config file.
    Saved(Result<(), ConfigError>),
    /// The terminal got focus back, often after a merge or comment in the browser.
    FocusGained,
    /// The terminal lost focus, so the pointer is no longer over dispatch.
    FocusLost,
}

/// One look at the sessions, with what each one's Claude last said.
pub struct Observation {
    pub sessions: Sessions,
    /// Only sessions that have a conversation.
    pub transcripts: BTreeMap<IssueNumber, Transcript>,
    /// Only sessions whose git reads worked.
    pub work: Works,
}

/// Work for the shell.
pub enum Cmd {
    Job(Job),
    /// Runs on the main thread: `AttachPlan::Suspend` must own the terminal.
    Attach(IssueNumber),
    /// Runs on the main thread: the copy writes to the terminal.
    Copy {
        target: GithubRef,
        text: String,
    },
    Quit,
}

/// Runs on a worker thread and comes back as exactly one `Msg`. The shell
/// runs a read on its own short thread and a write in order on one writer
/// thread.
#[derive(Debug, PartialEq, Eq)]
pub enum Job {
    Read(ReadJob),
    Write(WriteJob),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadJob {
    FetchIssues,
    FetchPrs,
    ObserveSessions,
    /// Once at start, for `assignee:@me`.
    FetchViewer,
    /// Open the page in the browser. No inflight flag: two presses may open
    /// two tabs, which is what the user asked for.
    Open(GithubRef),
    /// Look up the page URL, for `OpenWith::Clipboard`.
    FetchPageUrl(GithubRef),
}

/// Writes share one thread, because two concurrent `git config` writes can
/// fail on the `.git/config` lock, two quick saves could interleave their
/// read and write, and a second merge must see the first one done. `Stop`
/// and `Resume` carry the issue, not the session, because the worker
/// observes again before it acts, so a stale row can not aim at a reused
/// window id.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteJob {
    Start(StartRequest, Launch),
    Stop(IssueNumber),
    Resume(IssueNumber, Launch),
    /// Close the window, then remove the worktree and branch, unless they hold
    /// work that is not saved.
    Remove(RemoveTarget),
    Clean(Vec<RemoveTarget>),
    /// Save one setting to the config file.
    SaveConfig {
        setting: Setting,
        config: Config,
    },
    /// Type a line into the session's Claude prompt and submit it.
    Send {
        issue: IssueNumber,
        kind: SendKind,
        text: String,
    },
    /// Check the PR again, then merge it or report what the agent must fix.
    Merge(MergeAsk),
}

/// Who wrote the line a `WriteJob::Send` types, so its result toast can say so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendKind {
    /// The merge check asks the agent to fix its PR.
    MergeFix,
    /// The user typed it with `t`.
    Message,
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
pub(super) const TOAST_FOR: Duration = Duration::from_secs(5);
pub(super) const GITHUB_COOLDOWN: Duration = Duration::from_secs(5);
