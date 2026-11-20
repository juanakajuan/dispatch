//! Session truth lives in git and the multiplexer, never in a dispatch file:
//!
//! - `git worktree list`: which worktrees exist, and their branch
//! - `git config branch.<b>.dispatch-issue`: which
//!   branches dispatch made, for which issue
//! - tmux window option `@dispatch-worktree` (and `@dispatch-waiting`): which
//!   window runs Claude in which worktree
//!
//! `observe()` reads all three and joins them on the worktree path. A
//! worktree at `.claude/worktrees/<issue>` stays that issue's session when it
//! has another branch checked out. Nothing is cached here; `Workspace` is
//! immutable and `Clone`, so each worker thread gets its own copy and no lock
//! exists anywhere.
//!
//! The git markers live in `.git/config`, so they survive a dispatch crash, a
//! tmux server restart, and a reboot. `git branch -D` deletes the `branch.<b>`
//! section, so a deleted branch leaves no stale marker.

mod diff;
pub mod error;
mod git;
mod hc;
mod herdr;
mod join;
mod mux;
mod ops;
mod plan;
pub mod request;
mod start_argv;
mod tmux;
mod work;
mod worktree;

pub use diff::Diff;
pub use start_argv::StartPreview;
pub use work::{Push, Work, Works};

use crate::config::Multiplexer;
use crate::domain::{BranchName, CommitOid, IssueNumber, OriginRef, WorktreePath};
use error::{SessionError, SetupError, StartWarning};
use mux::{Backend, MuxEnv, Repo, detect_mux, each_backend};
use request::{Launch, StartRequest};
use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::PathBuf;

/// One issue being worked in one worktree. Derived, never stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub issue: IssueNumber,
    /// The branch marked for the issue. PRs and `D` go by this branch.
    pub branch: BranchName,
    pub checkout: Checkout,
    pub worktree: WorktreePath,
    pub state: SessionState,
}

impl Session {
    /// What the worktree has checked out, as the list shows it.
    pub fn checked_out(&self) -> &str {
        match &self.checkout {
            Checkout::SessionBranch => self.branch.as_str(),
            Checkout::Other(branch) => branch.as_str(),
            Checkout::Detached => "detached HEAD",
        }
    }
}

/// What the session's worktree has checked out. Someone can switch it by
/// hand; the session stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checkout {
    SessionBranch,
    Other(BranchName),
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    /// Claude waits for the user.
    NeedsYou,
    /// Claude exited, or its window is gone. The worktree and branch stay.
    Exited,
}

impl SessionState {
    pub fn is_live(self) -> bool {
        matches!(self, SessionState::Running | SessionState::NeedsYou)
    }
}

/// At most one session per issue, by construction of the map.
pub type Sessions = BTreeMap<IssueNumber, Session>;

/// A converged start. `warnings` hold non-fatal problems, such as a failed
/// `git fetch` that fell back to the local base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartOutcome {
    pub session: Session,
    pub warnings: Vec<StartWarning>,
}

/// What `Workspace::remove` did. A refusal is an answer, not an error: it
/// changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveOutcome {
    Removed,
    Refused(RemoveRefusal),
}

/// Why the worktree stays. Removing it would lose work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveRefusal {
    Uncommitted,
    /// This many commits of the branch are on no remote.
    Unpushed(NonZeroU32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveTarget {
    pub issue: IssueNumber,
    pub merged_head: Option<CommitOid>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanReport {
    pub removed: Vec<IssueNumber>,
    pub skipped: Vec<(IssueNumber, RemoveRefusal)>,
    pub failed: Vec<(IssueNumber, SessionError)>,
}

/// What an attach shows: the session's Claude window, or a plain shell in
/// its worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachTo {
    Claude,
    Shell,
}

/// How the shell attaches. Pure data, so the choice is unit tested.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachPlan {
    Switch(Vec<String>),
    Suspend(Vec<String>),
}

/// Startup facts, resolved once. Immutable after `discover`.
#[derive(Clone, Debug)]
pub struct Workspace {
    repo_root: PathBuf,
    /// The git dir that all worktrees share, for `info/exclude`.
    git_common_dir: PathBuf,
    /// From `git symbolic-ref --short refs/remotes/origin/HEAD`.
    base: OriginRef,
    mux: Backend,
}

impl Workspace {
    /// Fails before the TUI starts, so the user sees a plain error line.
    pub fn discover(cwd: &std::path::Path, multiplexer: Multiplexer) -> Result<Workspace, SetupError> {
        let git::RepoDirs { root: repo_root, common: git_common_dir } =
            git::repo_dirs(cwd).map_err(SetupError::NotARepo)?;
        let head = git::origin_head(&repo_root).map_err(SetupError::NoOriginHead)?;
        let base = OriginRef::parse(&head).ok_or(SetupError::BadOriginHead(head))?;
        let mux = detect_mux(multiplexer, &MuxEnv::from_process(), &repo_root)?;
        Ok(Workspace { repo_root, git_common_dir, base, mux })
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    pub fn start_preview(&self) -> StartPreview {
        StartPreview { mux: self.mux.clone(), base: self.base.clone() }
    }

    fn repo(&self) -> Repo<'_> {
        Repo { root: &self.repo_root, git_common_dir: &self.git_common_dir }
    }

    /// A missing multiplexer server is an empty window list.
    pub fn observe(&self) -> Result<Sessions, SessionError> {
        each_backend!(&self.mux, mux => ops::observe(self, mux))
    }

    /// Commits, uncommitted files, and push state of each session. Read
    /// with git only; see `work`.
    pub fn work(&self, sessions: &Sessions) -> Works {
        work::read(self, sessions)
    }

    /// Converges to "a live window runs Claude in this issue's worktree".
    /// Safe to call twice, and safe to call again after a crash at any step:
    /// it re-observes, plans only the missing steps, and runs them.
    pub fn start(&self, req: &StartRequest, launch: &Launch) -> Result<StartOutcome, SessionError> {
        each_backend!(&self.mux, mux => ops::start(self, mux, req, launch))
    }

    /// Runs `claude --continue` for an exited session: in the dead window
    /// when it is still there, else in a new window as in `start`. No-op if
    /// the window is alive.
    pub fn resume(&self, issue: IssueNumber, launch: &Launch) -> Result<(), SessionError> {
        each_backend!(&self.mux, mux => ops::resume(self, mux, issue, launch))
    }

    /// Kills the window. The worktree and branch stay. A window that is
    /// already gone counts as success. Under hc this ends every terminal in
    /// the worktree's space, as herdr closes its whole workspace.
    pub fn stop(&self, issue: IssueNumber) -> Result<(), SessionError> {
        each_backend!(&self.mux, mux => ops::stop(self, mux, issue))
    }

    pub fn remove(&self, target: &RemoveTarget) -> Result<RemoveOutcome, SessionError> {
        each_backend!(&self.mux, mux => ops::remove(self, mux, target))
    }

    pub fn stop_and_remove(&self, target: &RemoveTarget) -> Result<RemoveOutcome, SessionError> {
        each_backend!(&self.mux, mux => ops::stop_and_remove(self, mux, target))
    }

    pub fn clean(&self, targets: &[RemoveTarget]) -> CleanReport {
        each_backend!(&self.mux, mux => ops::clean(self, mux, targets))
    }

    /// Types `text` into the session's Claude prompt and submits it. Refuses
    /// a dead window, where the keys would go nowhere.
    pub fn send(&self, issue: IssueNumber, text: &str) -> Result<(), SessionError> {
        each_backend!(&self.mux, mux => ops::send(self, mux, issue, text))
    }

    /// How to show the issue's live Claude window, or, for `AttachTo::Shell`,
    /// a new plain shell in its worktree (also for an exited session). Claude
    /// fails when the window died since the last poll; the next poll shows
    /// the session as exited.
    pub fn attach_plan(&self, issue: IssueNumber, to: AttachTo) -> Result<AttachPlan, SessionError> {
        each_backend!(&self.mux, mux => ops::attach_plan(self, mux, issue, to))
    }

    /// The worktree's changes against the base: commits ahead, uncommitted
    /// tracked changes, and untracked files. Read with git only.
    pub fn diff(&self, issue: IssueNumber) -> Result<Diff, SessionError> {
        let dir = worktree::worktree_dir_for(&self.repo_root, issue);
        if !dir.exists() {
            return Err(SessionError::NoSession(issue));
        }
        diff::read(&dir, &self.base)
    }
}

#[cfg(test)]
mod tests;
