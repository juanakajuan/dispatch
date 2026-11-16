use crate::domain::{BranchName, IssueNumber};
use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum SessionError {
    /// The request can not converge without the user deciding something.
    Conflict(StartConflict),
    Command(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Conflict(c) => c.fmt(f),
            SessionError::Command(m) => f.write_str(m),
        }
    }
}

impl From<String> for SessionError {
    fn from(message: String) -> SessionError {
        SessionError::Command(message)
    }
}

#[derive(Debug, PartialEq, Eq)]
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

#[derive(Debug)]
pub enum SetupError {
    NotARepo(String),
    NoOriginHead(String),
    BadOriginHead(String),
    ProgramMissing(&'static str),
    Command(String),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotARepo(m) => write!(f, "dispatch must run inside a git repository: {m}"),
            SetupError::NoOriginHead(m) => write!(f, "cannot find the default branch of origin: {m}"),
            SetupError::BadOriginHead(raw) => {
                write!(f, "origin/HEAD is {raw}, not origin/<branch>. Run `git remote set-head origin --auto`.")
            }
            SetupError::ProgramMissing(program) => write!(f, "{program} is not installed or not on PATH"),
            SetupError::Command(m) => f.write_str(m),
        }
    }
}
