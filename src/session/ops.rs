//! Each `Workspace` operation, written once for every backend.

use super::error::{SessionError, StartWarning};
use super::join::{Joined, Row, join, sessions};
use super::markers;
use super::mux::{Mux, Observed, hooks};
use super::plan::{Layout, LocalBranch, StartObservation, StartPlan, Step, TargetDir, plan_start};
use super::request::{Launch, StartRequest};
use super::worktree::{WORKTREES_EXCLUDE, worktree_dir_for};
use super::{
    AttachPlan, AttachTo, Checkout, CleanReport, RemoveOutcome, RemoveRefusal, RemoveTarget, Sessions, StartOutcome,
    Workspace, git, start_argv,
};
use crate::domain::SessionKey;
use std::num::NonZeroU32;

struct World<W> {
    root: std::path::PathBuf,
    worktrees: Vec<git::Worktree>,
    markers: markers::Markers,
    harnesses: markers::Harnesses,
    modes: markers::Modes,
    windows: Vec<Observed<W>>,
}

impl<W: Clone> World<W> {
    fn join(&self) -> Joined<W> {
        join(&self.root, &self.worktrees, &self.markers, &self.harnesses, &self.modes, &self.windows)
    }
}

fn read_world<M: Mux>(ws: &Workspace, mux: &M) -> Result<World<M::Window>, SessionError> {
    let windows = mux.list(ws.repo())?;
    let worktrees = git::list_worktrees(&ws.repo_root).map_err(SessionError::Git)?;
    let marked = markers::list_markers(&ws.repo_root).map_err(SessionError::Git)?;
    Ok(World {
        root: ws.repo_root.clone(),
        worktrees,
        markers: marked.sessions,
        harnesses: marked.harnesses,
        modes: marked.modes,
        windows,
    })
}

/// A fresh observation of one key, so a stale row can not aim at a window
/// id that the multiplexer reused.
fn row<M: Mux>(ws: &Workspace, mux: &M, key: SessionKey) -> Result<Option<Row<M::Window>>, SessionError> {
    Ok(read_world(ws, mux)?.join().remove(&key))
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
        target_dir: if worktree_dir_for(&ws.repo_root, req.key).exists() { TargetDir::Taken } else { TargetDir::Free },
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
            let warnings = run_steps(ws, mux, steps, req.key)?;
            let session = observe(ws, mux)?.remove(&req.key).ok_or(SessionError::NotVisibleAfterStart(req.key))?;
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
    key: SessionKey,
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
                clear_stale(ws, mux, &dir)?;
                if let Err(e) = git::ensure_excluded(&ws.git_common_dir, WORKTREES_EXCLUDE) {
                    warnings.push(StartWarning::NotExcluded(e));
                }
                mux.add_worktree(ws.repo(), &dir, &branch, from.as_ref(), key)?;
            }
            Step::Mark { branch, key, harness, mode } => {
                markers::set_marker(&ws.repo_root, &branch, key).map_err(SessionError::Git)?;
                markers::set_harness(&ws.repo_root, &branch, harness).map_err(SessionError::Git)?;
                markers::set_mode(&ws.repo_root, &branch, mode).map_err(SessionError::Git)?;
            }
            Step::OpenWindow { dir, argv } => mux.open(ws.repo(), &dir, key, &argv)?,
        }
    }
    Ok(warnings)
}

/// The plan adds a worktree only where none is listed and the dir is free,
/// so a git registration or a window for `dir` is left from a worktree that
/// the user deleted by hand. Both go first: git refuses to add a worktree
/// over a stale registration, and the old agent must not keep running
/// next to the new one.
fn clear_stale<M: Mux>(ws: &Workspace, mux: &M, dir: &std::path::Path) -> Result<(), SessionError> {
    git::prune_worktrees(&ws.repo_root).map_err(SessionError::Git)?;
    for stale in mux.list(ws.repo())?.iter().filter(|o| o.worktree.as_path() == dir) {
        mux.stop(&stale.window)?;
    }
    Ok(())
}

pub(super) fn resume<M: Mux>(ws: &Workspace, mux: &M, key: SessionKey, launch: &Launch) -> Result<(), SessionError> {
    let Some(row) = row(ws, mux, key)? else {
        return Err(SessionError::NoSession(key));
    };
    let argv = start_argv::resume_argv(&launch.command, launch.harness, hooks(mux, launch), row.mode);
    match &row.window {
        Some((_, liveness)) if liveness.is_live() => Ok(()),
        Some((window, _)) => Ok(mux.relaunch(ws.repo(), window, &row.worktree, key, &argv)?),
        None => Ok(mux.open(ws.repo(), &row.worktree, key, &argv)?),
    }
}

pub(super) fn stop<M: Mux>(ws: &Workspace, mux: &M, key: SessionKey) -> Result<(), SessionError> {
    match row(ws, mux, key)?.and_then(|row| row.window) {
        Some((window, _)) => Ok(mux.stop(&window)?),
        None => Ok(()),
    }
}

fn remove_one<M: Mux>(ws: &Workspace, mux: &M, target: &RemoveTarget) -> Result<RemoveOutcome, SessionError> {
    let Some(row) = row(ws, mux, target.key)? else {
        return Err(SessionError::NoSession(target.key));
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
    stop(ws, mux, target.key)?;
    remove(ws, mux, target)
}

pub(super) fn clean<M: Mux>(ws: &Workspace, mux: &M, targets: &[RemoveTarget]) -> CleanReport {
    let mut report = CleanReport::default();
    for target in targets {
        match remove_one(ws, mux, target) {
            Ok(RemoveOutcome::Removed) => report.removed.push(target.key),
            Ok(RemoveOutcome::Refused(why)) => report.skipped.push((target.key, why)),
            Err(err) => report.failed.push((target.key, err)),
        }
    }
    let _ = git::prune_worktrees(&ws.repo_root);
    report
}

pub(super) fn send<M: Mux>(ws: &Workspace, mux: &M, key: SessionKey, text: &str) -> Result<(), SessionError> {
    match row(ws, mux, key)?.and_then(|row| row.window) {
        Some((window, liveness)) if liveness.is_live() => Ok(mux.send_line(&window, text)?),
        _ => Err(SessionError::NotRunning(key)),
    }
}

pub(super) fn attach_plan<M: Mux>(
    ws: &Workspace,
    mux: &M,
    key: SessionKey,
    to: AttachTo,
) -> Result<AttachPlan, SessionError> {
    match to {
        AttachTo::Claude => match row(ws, mux, key)?.and_then(|row| row.window) {
            Some((window, liveness)) if liveness.is_live() => Ok(mux.attach(&window)),
            _ => Err(SessionError::NotRunning(key)),
        },
        AttachTo::Shell => {
            let Some(row) = row(ws, mux, key)? else {
                return Err(SessionError::NoSession(key));
            };
            Ok(mux.shell(ws.repo(), &row.worktree, key, row.window.as_ref().map(|(w, _)| w))?)
        }
    }
}
