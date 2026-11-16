//! Each `Workspace` operation, written once for every backend.

use super::error::SessionError;
use super::join::{Joined, Row, join, sessions};
use super::mux::{Mux, Observed, hooks};
use super::plan::{Layout, StartObservation, StartPlan, Step, plan_start};
use super::request::{Launch, StartRequest};
use super::worktree::{WORKTREES_DIR, WORKTREES_EXCLUDE, worktree_dir_for};
use super::{AttachPlan, Sessions, StartOutcome, Workspace, git, start_argv};
use crate::domain::IssueNumber;

struct World<W> {
    worktrees: Vec<git::Worktree>,
    markers: git::Markers,
    windows: Vec<Observed<W>>,
}

impl<W: Clone> World<W> {
    fn join(&self) -> Joined<W> {
        join(&self.worktrees, &self.markers, &self.windows)
    }
}

fn read_world<M: Mux>(ws: &Workspace, mux: &M) -> Result<World<M::Window>, String> {
    let windows = mux.list(ws.repo())?;
    Ok(World { worktrees: git::list_worktrees(&ws.repo_root)?, markers: git::list_markers(&ws.repo_root)?, windows })
}

/// A fresh observation of one issue, so a stale row can not aim at a window
/// id that the multiplexer reused.
fn row<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber) -> Result<Option<Row<M::Window>>, String> {
    Ok(read_world(ws, mux)?.join().remove(&issue))
}

fn no_running_session(issue: IssueNumber) -> SessionError {
    SessionError::Command(format!("#{} has no running session", issue.0))
}

/// Three reads (the backend's windows, worktree list, config regexp), then
/// the pure `join`.
pub(super) fn observe<M: Mux>(ws: &Workspace, mux: &M) -> Result<Sessions, SessionError> {
    Ok(sessions(&read_world(ws, mux)?.join()))
}

fn observe_for_start<M: Mux>(ws: &Workspace, mux: &M, req: &StartRequest) -> Result<StartObservation, String> {
    let world = read_world(ws, mux)?;
    Ok(StartObservation {
        sessions: sessions(&world.join()),
        worktrees: world.worktrees,
        markers: world.markers,
        target_dir_exists: worktree_dir_for(&ws.repo_root, req.issue).exists(),
        branch_exists: git::branch_exists(&ws.repo_root, &req.branch)?,
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
        claude: launch.claude.clone(),
    };
    match plan_start(&obs, req, &layout).map_err(SessionError::Conflict)? {
        StartPlan::AlreadyRunning(session) => Ok(StartOutcome { session, warnings: Vec::new() }),
        StartPlan::Steps(steps) => {
            let warnings = run_steps(ws, mux, steps, req.issue)?;
            let session = observe(ws, mux)?.remove(&req.issue).ok_or_else(|| {
                SessionError::Command(format!("session for #{} is not visible after start", req.issue.0))
            })?;
            Ok(StartOutcome { session, warnings })
        }
    }
}

/// The step runner. A failed `Fetch` is a warning: the start continues
/// from the local base. A failed exclude before `AddWorktree` is a warning
/// too: the worktree only shows in `git status`. Any other failure stops
/// the run, and the next `start` plans from whatever state exists.
fn run_steps<M: Mux>(ws: &Workspace, mux: &M, steps: Vec<Step>, issue: IssueNumber) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    for step in steps {
        match step {
            Step::Fetch(remote_ref) => {
                if let Err(e) = git::fetch(&ws.repo_root, &remote_ref) {
                    warnings.push(format!("fetch failed, using the local {remote_ref}: {e}"));
                }
            }
            Step::AddWorktree { dir, branch, from } => {
                if let Err(e) = git::ensure_excluded(&ws.git_common_dir, WORKTREES_EXCLUDE) {
                    warnings.push(format!("{WORKTREES_DIR} still shows in git status: {e}"));
                }
                mux.add_worktree(ws.repo(), &dir, &branch, from.as_ref(), issue)?;
            }
            Step::Mark { branch, issue } => git::set_marker(&ws.repo_root, &branch, issue)?,
            Step::OpenWindow { dir, argv } => mux.open(ws.repo(), &dir, issue, &argv)?,
        }
    }
    Ok(warnings)
}

pub(super) fn resume<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber, launch: &Launch) -> Result<(), SessionError> {
    let Some(row) = row(ws, mux, issue)? else {
        return Err(SessionError::Command(format!("no session for #{}", issue.0)));
    };
    let argv = launch.claude.argv(&start_argv::continue_args(hooks(mux, launch)));
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

pub(super) fn send<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber, text: &str) -> Result<(), SessionError> {
    match row(ws, mux, issue)?.and_then(|row| row.window) {
        Some((window, liveness)) if liveness.is_live() => Ok(mux.send_line(&window, text)?),
        _ => Err(no_running_session(issue)),
    }
}

pub(super) fn attach_plan<M: Mux>(ws: &Workspace, mux: &M, issue: IssueNumber) -> Result<AttachPlan, SessionError> {
    match row(ws, mux, issue)?.and_then(|row| row.window) {
        Some((window, liveness)) if liveness.is_live() => Ok(mux.attach(&window)),
        _ => Err(no_running_session(issue)),
    }
}
