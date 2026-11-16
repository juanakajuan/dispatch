use crate::clipboard::CopyError;
use crate::config::settings::Setting;
use crate::config::{Config, ConfigError};
use crate::domain::{GithubRef, Issue, IssueNumber, PullRequest};
use crate::github::GhError;
use crate::merge::MergeAsk;
use crate::merge::job::MergeRun;
use crate::session::error::{AttachError, SessionError};
use crate::session::request::{Launch, StartRequest};
use crate::session::{Sessions, StartOutcome};
use crossterm::event::{KeyEvent, MouseEvent};
use std::time::{Duration, Instant};

pub enum Msg {
    /// PROTOTYPE: what each session's Claude last said.
    Summaries(std::collections::BTreeMap<IssueNumber, crate::transcript::Summary>),
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
    Sessions(Result<Sessions, SessionError>),
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

/// Runs on a worker thread and comes back as exactly one `Msg`. `Stop` and
/// `Resume` carry the issue, not the session, because the worker observes
/// again before it acts, so a stale row can not aim at a reused window id.
#[derive(Debug, PartialEq, Eq)]
pub enum Job {
    FetchIssues,
    FetchPrs,
    ObserveSessions,
    Start(StartRequest, Launch),
    Stop(IssueNumber),
    Resume(IssueNumber, Launch),
    /// Open the page in the browser. No inflight flag: two presses may open
    /// two tabs, which is what the user asked for.
    Open(GithubRef),
    /// Look up the page URL, for `OpenWith::Clipboard`.
    FetchPageUrl(GithubRef),
    /// Save one setting to the config file. It runs on the writer thread, so
    /// two quick saves can not interleave their read and write.
    SaveConfig {
        setting: Setting,
        config: Config,
    },
    /// Type a line into the session's Claude prompt and submit it.
    Send {
        issue: IssueNumber,
        text: String,
    },
    /// Check the PR again, then merge it or report what the agent must fix.
    Merge(MergeAsk),
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
pub(super) const TOAST_FOR: Duration = Duration::from_secs(5);
pub(super) const GITHUB_COOLDOWN: Duration = Duration::from_secs(5);
