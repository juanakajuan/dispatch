use super::tmux::WindowId;
use super::worktree::WORKTREES_DIR;
use crate::domain::{BranchName, IssueNumber, OriginRef};
use crate::proc::ProcError;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The request can not converge without the user deciding something.
    Conflict(StartConflict),
    Mux(MuxError),
    Git(ProcError),
    NoSession(IssueNumber),
    NotRunning(IssueNumber),
    NotVisibleAfterStart(IssueNumber),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Conflict(c) => c.fmt(f),
            SessionError::Mux(e) => e.fmt(f),
            SessionError::Git(e) => e.fmt(f),
            SessionError::NoSession(issue) => write!(f, "no session for {issue}"),
            SessionError::NotRunning(issue) => write!(f, "{issue} has no running session"),
            SessionError::NotVisibleAfterStart(issue) => write!(f, "session for {issue} is not visible after start"),
        }
    }
}

impl From<MuxError> for SessionError {
    fn from(e: MuxError) -> SessionError {
        SessionError::Mux(e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartConflict {
    /// `branch` is already checked out in a worktree that is not this issue's.
    BranchCheckedOut { branch: BranchName, path: PathBuf },
    /// `branch` carries a marker for a different issue.
    BranchBelongsTo { branch: BranchName, issue: IssueNumber },
    /// The deterministic worktree dir exists and is not a git worktree.
    PathTaken(PathBuf),
}

impl fmt::Display for StartConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartConflict::BranchCheckedOut { branch, path } => {
                write!(f, "branch {} is already checked out in {}", branch.as_str(), path.display())
            }
            StartConflict::BranchBelongsTo { branch, issue } => {
                write!(f, "branch {} already belongs to issue {issue}", branch.as_str())
            }
            StartConflict::PathTaken(path) => {
                write!(f, "{} exists and is not a git worktree. Move it away and start again.", path.display())
            }
        }
    }
}

/// A non-fatal problem in a start that went on anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartWarning {
    /// The start continued from the local copy of `base`.
    FetchFailed { base: OriginRef, error: ProcError },
    /// The worktree shows in the main checkout's `git status`.
    NotExcluded(FsError),
}

impl fmt::Display for StartWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartWarning::FetchFailed { base, error } => write!(f, "fetch failed, using the local {base}: {error}"),
            StartWarning::NotExcluded(e) => write!(f, "{WORKTREES_DIR} still shows in git status: {e}"),
        }
    }
}

/// A failed multiplexer call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxError {
    Run(ProcError),
    /// The program answered with JSON that does not match its schema.
    Reply {
        program: &'static str,
        detail: String,
    },
    NotAWindowId(String),
    /// tmux made the window, and it closed before dispatch could tag it.
    Untagged {
        window: WindowId,
        error: ProcError,
    },
    ServerTimeout {
        program: &'static str,
        waited: Duration,
    },
    Fs(FsError),
    UnsafeScriptPath(String),
    NoHerdrWorkspace,
}

impl fmt::Display for MuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MuxError::Run(e) => e.fmt(f),
            MuxError::Reply { program, detail } => write!(f, "{program} printed unexpected JSON: {detail}"),
            MuxError::NotAWindowId(raw) => write!(f, "tmux returned {raw:?} instead of a window id"),
            MuxError::Untagged { window, error } => {
                write!(f, "window {} died before it was tagged: {error}", window.as_str())
            }
            MuxError::ServerTimeout { program, waited } => write!(f, "{program} did not start within {waited:?}"),
            MuxError::Fs(e) => e.fmt(f),
            MuxError::UnsafeScriptPath(path) => {
                write!(f, "cannot launch from {path}: the path has a quote, a backslash, or a newline")
            }
            MuxError::NoHerdrWorkspace => f.write_str("HERDR_ENV is set but HERDR_WORKSPACE_ID is not"),
        }
    }
}

impl From<ProcError> for MuxError {
    fn from(e: ProcError) -> MuxError {
        MuxError::Run(e)
    }
}

/// A failed file system call on `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
    pub action: FsAction,
    pub path: PathBuf,
    pub kind: std::io::ErrorKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsAction {
    Resolve,
    Update,
    Write,
}

impl FsError {
    pub fn new(action: FsAction, path: &std::path::Path, source: &std::io::Error) -> FsError {
        FsError { action, path: path.to_path_buf(), kind: source.kind(), message: source.to_string() }
    }
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verb = match self.action {
            FsAction::Resolve => "resolve",
            FsAction::Update => "update",
            FsAction::Write => "write",
        };
        write!(f, "cannot {verb} {}: {}", self.path.display(), self.message)
    }
}

/// Why `git` could not name the repository's directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    Run(ProcError),
    Fs(FsError),
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::Run(e) => e.fmt(f),
            GitError::Fs(e) => e.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    NotARepo(GitError),
    NoOriginHead(ProcError),
    BadOriginHead(String),
    ProgramMissing(&'static str),
    Mux(MuxError),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotARepo(e) => write!(f, "dispatch must run inside a git repository: {e}"),
            SetupError::NoOriginHead(e) => write!(
                f,
                "cannot find the default branch of origin: {e}\nrun `git remote set-head origin --auto` first"
            ),
            SetupError::BadOriginHead(raw) => {
                write!(f, "origin/HEAD is {raw}, not origin/<branch>. Run `git remote set-head origin --auto`.")
            }
            SetupError::ProgramMissing(program) => write!(f, "{program} is not installed or not on PATH"),
            SetupError::Mux(e) => e.fmt(f),
        }
    }
}
