//! Each `Workspace` operation, written once for every backend.

use super::error::{SessionError, StartWarning};
use super::join::{Joined, Row, join, sessions};
use super::markers;
use super::mux::{Mux, Observed, hooks};
use super::plan::{Layout, LocalBranch, StartObservation, StartPlan, Step, TargetDir, plan_start};
use super::request::{Launch, StartRequest};
use super::worktree::{WORKTREES_EXCLUDE, worktree_dir_for};
use super::{
    AttachPlan, Checkout, CleanReport, RemoveOutcome, RemoveRefusal, RemoveTarget, Sessions, StartOutcome, Workspace,
    git, start_argv,
};
use crate::domain::IssueNumber;
use std::num::NonZeroU32;

struct World<W> {
    root: std::path::PathBuf,
    worktrees: Vec<git::Worktree>,
    markers: markers::Markers,
    harnesses: markers::Harnesses,
    windows: Vec<Observed<W>>,
}

impl<W: Clone> World<W> {
    fn join(&self) -> Joined<W> {
        join(&self.root, &self.worktrees, &self.markers, &self.harnesses, &self.windows)
    }
}

fn read_world<M: Mux>(ws: &Workspace, mux: &M) -> Result<World<M::Window>, SessionError> {
    let windows = mux.list(ws.repo())?;
    let worktrees = git::list_worktrees(&ws.repo_root).map_err(SessionError::Git)?;
    let marked = markers::list_markers(&ws.repo_root).map_err(SessionError::Git)?;
    Ok(World { root: ws.repo_root.clone(), worktrees, markers: marked.issues, harnesses: marked.harnesses, windows })
}

/// A fresh observation of one issue, so a stale row can not aim at a window
/// id that the multiplexer reused.
fn row<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber) -> Result<Option<Row<M::Window>>, SessionError> {
    Ok(read_world(ws, mux)?.join().remove(&issue))
}

/// Three reads (the backend's windows, worktree list, config regexp), then
/// the pure `join`.
pub(super) fn observe<M: Mux>(ws: &Workspace, mux: &M) -> Result<Sessions, SessionError> {
    Ok(sessions(&read_world(ws, mux)?.join()))
}

fn observe_for_start<M: Mux>(ws: &Workspace, mux: &M, req: &StartRequest) -> Result<StartObservation, SessionError> {
    let world = read_world(ws, mux)?;
    Ok(StartObservation {
        sessions: sessions(&world.join()),
        worktrees: world.worktrees,
        markers: world.markers,
        target_dir: if worktree_dir_for(&ws.repo_root, req.issue).exists() {
            TargetDir::Taken
        } else {
            TargetDir::Free
        },
        branch: if git::branch_exists(&ws.repo_root, &req.branch).map_err(SessionError::Git)? {
            LocalBranch::Exists
        } else {
            LocalBranch::Missing
        },
    })
}

pub(super) fn start<M: Mux>(
    ws: &Workspace,
    mux: &M,
    req: &StartRequest,
    launch: &Launch,
) -> Result<StartOutcome, SessionError> {
    let obs = observe_for_start(ws, mux, req)?;
    let layout = Layout {
        repo_root: ws.repo_root.clone(),
        base: ws.base.clone(),
        hooks: hooks(mux, launch),
        harness: launch.harness,
        command: launch.command.clone(),
    };
    match plan_start(&obs, req, &layout).map_err(SessionError::Conflict)? {
        StartPlan::AlreadyRunning(session) => Ok(StartOutcome { session, warnings: Vec::new() }),
        StartPlan::Steps(steps) => {
            let warnings = run_steps(ws, mux, steps, req.issue)?;
            let session = observe(ws, mux)?.remove(&req.issue).ok_or(SessionError::NotVisibleAfterStart(req.issue))?;
            Ok(StartOutcome { session, warnings })
        }
    }
}

/// The step runner. A failed `Fetch` is a warning: the start continues
/// from the local base. A failed exclude before `AddWorktree` is a warning
/// too: the worktree only shows in `git status`. Any other failure stops
/// the run, and the next `start` plans from whatever state exists.
fn run_steps<M: Mux>(
    ws: &Workspace,
    mux: &M,
    steps: Vec<Step>,
    issue: IssueNumber,
) -> Result<Vec<StartWarning>, SessionError> {
    let mut warnings = Vec::new();
    for step in steps {
        match step {
            Step::Fetch(remote_ref) => {
                if let Err(error) = git::fetch(&ws.repo_root, &remote_ref) {
                    warnings.push(StartWarning::FetchFailed { base: remote_ref, error });
                }
            }
            Step::FetchFork { repo, head, branch } => git::fetch_fork(&ws.repo_root, &repo, &head, &branch)?,
            Step::AddWorktree { dir, branch, from } => {
                if let Err(e) = git::ensure_excluded(&ws.git_common_dir, WORKTREES_EXCLUDE) {
                    warnings.push(StartWarning::NotExcluded(e));
                }
                mux.add_worktree(ws.repo(), &dir, &branch, from.as_ref(), issue)?;
            }
            Step::Mark { branch, issue, harness } => {
                markers::set_marker(&ws.repo_root, &branch, issue).map_err(SessionError::Git)?;
                markers::set_harness(&ws.repo_root, &branch, harness).map_err(SessionError::Git)?;
            }
            Step::OpenWindow { dir, argv } => mux.open(ws.repo(), &dir, issue, &argv)?,
        }
    }
    Ok(warnings)
}

pub(super) fn resume<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber, launch: &Launch) -> Result<(), SessionError> {
    let Some(row) = row(ws, mux, issue)? else {
        return Err(SessionError::NoSession(issue));
    };
    let argv = start_argv::resume_argv(&launch.command, launch.harness, hooks(mux, launch));
    match &row.window {
        Some((_, liveness)) if liveness.is_live() => Ok(()),
        Some((window, _)) => Ok(mux.relaunch(ws.repo(), window, &row.worktree, issue, &argv)?),
        None => Ok(mux.open(ws.repo(), &row.worktree, issue, &argv)?),
    }
}

pub(super) fn stop<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber) -> Result<(), SessionError> {
    match row(ws, mux, issue)?.and_then(|row| row.window) {
        Some((window, _)) => Ok(mux.stop(&window)?),
        None => Ok(()),
    }
}

fn remove_one<M: Mux>(ws: &Workspace, mux: &M, target: &RemoveTarget) -> Result<RemoveOutcome, SessionError> {
    let Some(row) = row(ws, mux, target.issue)? else {
        return Err(SessionError::NoSession(target.issue));
    };
    let git_err = SessionError::Git;
    if git::has_uncommitted(row.worktree.as_path()).map_err(git_err)? {
        return Ok(RemoveOutcome::Refused(RemoveRefusal::Uncommitted));
    }
    let merged = (row.checkout == Checkout::SessionBranch).then_some(target.merged_head.as_ref()).flatten();
    let unpushed = git::unpushed_count(row.worktree.as_path(), &row.branch, merged)?;
    if let Some(count) = NonZeroU32::new(unpushed) {
        return Ok(RemoveOutcome::Refused(RemoveRefusal::Unpushed(count)));
    }
    if let Some((window, _)) = &row.window {
        mux.stop(window)?;
    }
    git::remove_worktree(&ws.repo_root, row.worktree.as_path()).map_err(git_err)?;
    git::delete_branch(&ws.repo_root, &row.branch).map_err(git_err)?;
    git::remove_fork_remote(&ws.repo_root, &row.branch).map_err(git_err)?;
    Ok(RemoveOutcome::Removed)
}

pub(super) fn remove<M: Mux>(ws: &Workspace, mux: &M, target: &RemoveTarget) -> Result<RemoveOutcome, SessionError> {
    let outcome = remove_one(ws, mux, target)?;
    if outcome == RemoveOutcome::Removed {
        let _ = git::prune_worktrees(&ws.repo_root);
    }
    Ok(outcome)
}

pub(super) fn stop_and_remove<M: Mux>(
    ws: &Workspace,
    mux: &M,
    target: &RemoveTarget,
) -> Result<RemoveOutcome, SessionError> {
    stop(ws, mux, target.issue)?;
    remove(ws, mux, target)
}

pub(super) fn clean<M: Mux>(ws: &Workspace, mux: &M, targets: &[RemoveTarget]) -> CleanReport {
    let mut report = CleanReport::default();
    for target in targets {
        match remove_one(ws, mux, target) {
            Ok(RemoveOutcome::Removed) => report.removed.push(target.issue),
            Ok(RemoveOutcome::Refused(why)) => report.skipped.push((target.issue, why)),
            Err(err) => report.failed.push((target.issue, err)),
        }
    }
    let _ = git::prune_worktrees(&ws.repo_root);
    report
}

pub(super) fn send<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber, text: &str) -> Result<(), SessionError> {
    match row(ws, mux, issue)?.and_then(|row| row.window) {
        Some((window, liveness)) if liveness.is_live() => Ok(mux.send_line(&window, text)?),
        _ => Err(SessionError::NotRunning(issue)),
    }
}

pub(super) fn attach_plan<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber) -> Result<AttachPlan, SessionError> {
    match row(ws, mux, issue)?.and_then(|row| row.window) {
        Some((window, liveness)) if liveness.is_live() => Ok(mux.attach(&window)),
        _ => Err(SessionError::NotRunning(issue)),
    }
}
