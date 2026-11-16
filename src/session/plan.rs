use super::error::StartConflict;
use super::request::{StartFrom, StartRequest};
use super::start_argv::claude_argv;
use super::worktree::worktree_dir_for;
use super::{Session, Sessions, git};
use crate::config::ClaudeCommand;
use crate::domain::{BranchName, IssueNumber, WorktreePath};
use std::path::PathBuf;

/// Everything `plan_start` needs. Gathered by `observe_for_start`.
pub(super) struct StartObservation {
    pub(super) sessions: Sessions,
    pub(super) worktrees: Vec<git::Worktree>,
    pub(super) markers: git::Markers,
    /// The deterministic dir (`worktree_dir_for`) exists on disk.
    pub(super) target_dir_exists: bool,
    /// `req.branch` exists as a local branch.
    pub(super) branch_exists: bool,
}

pub(super) struct Layout {
    pub(super) repo_root: PathBuf,
    /// `origin/main`.
    pub(super) base: String,
    pub(super) hooks: Option<&'static str>,
    pub(super) claude: ClaudeCommand,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StartPlan {
    AlreadyRunning(Session),
    Steps(Vec<Step>),
}

/// One subprocess-level action. Each is idempotent on its own or guarded by
/// the plan, so a crash between any two steps leaves a state that the next
/// `plan_start` recognizes and finishes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// `git fetch origin <branch>` for a remote ref such as `origin/main`, so
    /// the new branch starts from fresh commits.
    Fetch(String),
    /// `git worktree add <dir> -b <branch> <from>` when the branch is new,
    /// `git worktree add <dir> <branch>` when it exists (`from` is `None`).
    AddWorktree { dir: PathBuf, branch: BranchName, from: Option<String> },
    /// `git config branch.<b>.dispatch-issue N`.
    /// Overwrites, so running it twice is harmless.
    Mark { branch: BranchName, issue: IssueNumber },
    /// Two tmux calls: new-window (or new-session when the detached session
    /// is missing) with argv `claude ...`, then set-option for
    /// `@dispatch-worktree` and `remain-on-exit on` on that window id.
    OpenWindow { dir: WorktreePath, name: String, argv: Vec<String> },
}

/// Pure. Decision table, tested row by row:
///
/// | observed                                              | plan                              |
/// |-------------------------------------------------------|-----------------------------------|
/// | session for issue, window alive                       | `AlreadyRunning`                    |
/// | session for issue, no window or dead window           | [`OpenWindow`]                      |
/// | a branch marked for issue, no worktree (crash)        | [`AddWorktree` create=false, Mark, `OpenWindow`] using that branch |
/// | worktree at target dir, branch unmarked (crash)       | [Mark, `OpenWindow`]                |
/// | target dir exists, not a worktree                     | Conflict `PathTaken`                |
/// | req.branch marked for another issue                   | Conflict `BranchBelongsTo`          |
/// | req.branch checked out elsewhere                      | Conflict `BranchCheckedOut`         |
/// | req.branch exists, unmarked, free                     | [`AddWorktree` create=false, Mark, `OpenWindow`] |
/// | nothing exists                                        | [Fetch, `AddWorktree` create=true, Mark, `OpenWindow`] |
///
/// A created branch starts from the default branch, or from `origin/<branch>`
/// when `req.from` is `StartFrom::Origin`.
///
/// A marked branch for this issue wins over `req.branch`, so a retry with an
/// edited branch name can not make a second branch for one issue.
pub(super) fn plan_start(
    obs: &StartObservation,
    req: &StartRequest,
    layout: &Layout,
) -> Result<StartPlan, StartConflict> {
    let target = worktree_dir_for(&layout.repo_root, req.issue);
    let open = |dir: WorktreePath| Step::OpenWindow {
        dir,
        name: format!("#{}", req.issue.0),
        argv: claude_argv(&layout.claude, layout.hooks, req.mode, &req.prompt),
    };
    let mark = |branch: BranchName| Step::Mark { branch, issue: req.issue };
    let add = |branch: BranchName, from: Option<String>| Step::AddWorktree { dir: target.clone(), branch, from };
    let planned = WorktreePath::planned(target.clone());

    if let Some(s) = obs.sessions.get(&req.issue) {
        return Ok(match &s.window {
            Some(w) if w.alive => StartPlan::AlreadyRunning(s.clone()),
            _ => StartPlan::Steps(vec![open(s.worktree.clone())]),
        });
    }
    if let Some(w) = obs.worktrees.iter().find(|w| w.path.as_path() == target) {
        let Some(branch) = &w.branch else { return Err(StartConflict::PathTaken(target)) };
        if let Some(&issue) = obs.markers.get(branch) {
            return Err(StartConflict::BranchBelongsTo { branch: branch.clone(), issue });
        }
        return Ok(StartPlan::Steps(vec![mark(branch.clone()), open(w.path.clone())]));
    }
    if obs.target_dir_exists {
        return Err(StartConflict::PathTaken(target));
    }
    if let Some((branch, _)) = obs.markers.iter().find(|(_, n)| **n == req.issue) {
        return Ok(StartPlan::Steps(vec![add(branch.clone(), None), mark(branch.clone()), open(planned)]));
    }
    if let Some(&issue) = obs.markers.get(&req.branch) {
        return Err(StartConflict::BranchBelongsTo { branch: req.branch.clone(), issue });
    }
    if let Some(w) = obs.worktrees.iter().find(|w| w.branch.as_ref() == Some(&req.branch)) {
        return Err(StartConflict::BranchCheckedOut {
            branch: req.branch.clone(),
            path: w.path.as_path().to_path_buf(),
        });
    }
    let mut steps = Vec::new();
    let from = (!obs.branch_exists).then(|| match req.from {
        StartFrom::Base => layout.base.clone(),
        StartFrom::Origin => format!("origin/{}", req.branch.as_str()),
    });
    if let Some(remote_ref) = &from {
        steps.push(Step::Fetch(remote_ref.clone()));
    }
    steps.extend([add(req.branch.clone(), from), mark(req.branch.clone()), open(planned)]);
    Ok(StartPlan::Steps(steps))
}
