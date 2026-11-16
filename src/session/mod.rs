//! Session truth lives in git and the multiplexer, never in a dispatch file:
//!
//! - `git worktree list`: which worktrees exist, and their branch
//! - `git config branch.<b>.dispatch-issue`: which
//!   branches dispatch made, for which issue
//! - tmux window option `@dispatch-worktree` (and `@dispatch-waiting`): which
//!   window runs Claude in which worktree
//!
//! `observe()` reads all three and joins them on the worktree path. Nothing is
//! cached here; `Workspace` is immutable and `Clone`, so each worker thread
//! gets its own copy and no lock exists anywhere.
//!
//! The git markers live in `.git/config`, so they survive a dispatch crash, a
//! tmux server restart, and a reboot. `git branch -D` deletes the `branch.<b>`
//! section, so a deleted branch leaves no stale marker.

mod error;
mod git;
mod hc;
mod herdr;
mod join;
mod mux;
mod plan;
pub mod request;
mod start_argv;
mod tmux;
mod worktree;

pub use start_argv::StartPreview;

use crate::config::Multiplexer;
use crate::domain::{BranchName, IssueNumber, WorktreePath};
use error::{SessionError, SetupError};
use join::join;
use mux::{Mux, MuxEnv, detect_mux};
use plan::{Layout, StartObservation, StartPlan, Step, plan_start};
use request::{Launch, StartRequest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use worktree::{WORKTREES_DIR, WORKTREES_EXCLUDE, worktree_dir_for};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WindowId {
    Tmux(tmux::WindowId),
    Herdr(herdr::PaneRef),
    Hc(hc::Pane),
}

#[cfg(test)]
impl WindowId {
    pub fn parse(raw: &str) -> Result<WindowId, String> {
        tmux::WindowId::parse(raw).map(WindowId::Tmux)
    }

    pub fn as_str(&self) -> &str {
        match self {
            WindowId::Tmux(id) => id.as_str(),
            WindowId::Herdr(p) => p.pane.as_str(),
            WindowId::Hc(p) => p.address.as_str(),
        }
    }
}

struct WindowObs {
    id: WindowId,
    worktree: WorktreePath,
    dead: bool,
    waiting: bool,
}

/// One issue being worked in one worktree. Derived, never stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub issue: IssueNumber,
    pub branch: BranchName,
    pub worktree: WorktreePath,
    pub window: Option<Window>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    /// False when Claude exited and `remain-on-exit` keeps the dead pane, so
    /// the last screen stays readable.
    pub alive: bool,
    /// Set by the Claude hooks in `WAITING_HOOKS`. Always false when
    /// `Config::detect_waiting` is off.
    pub waiting: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Running,
    NeedsYou,
    Exited,
}

impl Session {
    /// The only place state is decided. No stored state can disagree with it.
    pub fn state(&self) -> SessionState {
        match &self.window {
            Some(w) if w.alive && w.waiting => SessionState::NeedsYou,
            Some(w) if w.alive => SessionState::Running,
            _ => SessionState::Exited,
        }
    }
}

/// At most one session per issue, by construction of the map.
pub type Sessions = BTreeMap<IssueNumber, Session>;

/// A converged start. `warnings` hold non-fatal problems, such as a failed
/// `git fetch` that fell back to the local base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartOutcome {
    pub session: Session,
    pub warnings: Vec<String>,
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
    /// `origin/main` from `git symbolic-ref --short refs/remotes/origin/HEAD`.
    base: String,
    mux: Mux,
}

impl Workspace {
    /// Fails before the TUI starts, so the user sees a plain error line.
    pub fn discover(cwd: &std::path::Path, multiplexer: Multiplexer) -> Result<Workspace, SetupError> {
        let git::RepoDirs { root: repo_root, common: git_common_dir } =
            git::repo_dirs(cwd).map_err(SetupError::NotARepo)?;
        let base = git::origin_head(&repo_root).map_err(SetupError::NoOriginHead)?;
        let mux = detect_mux(multiplexer, &MuxEnv::from_process(), &repo_root)?;
        Ok(Workspace { repo_root, git_common_dir, base, mux })
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    pub fn start_preview(&self) -> StartPreview {
        StartPreview { mux: self.mux.clone(), base: self.base.clone() }
    }

    /// Three subprocesses (worktree list, config regexp, tmux list-windows),
    /// then the pure `join`. A missing tmux server is an empty window list.
    pub fn observe(&self) -> Result<Sessions, SessionError> {
        let (worktrees, markers, windows) = self.read_world()?;
        Ok(join(&worktrees, &markers, &windows))
    }

    fn read_world(&self) -> Result<(Vec<git::Worktree>, git::Markers, Vec<WindowObs>), String> {
        let windows = match &self.mux {
            Mux::Tmux(_) => tmux::list_windows()?,
            Mux::Herdr(_) => herdr::list_windows()?,
            Mux::Hc(_) => hc::list_windows(&self.repo_root)?,
        };
        Ok((git::list_worktrees(&self.repo_root)?, git::list_markers(&self.repo_root)?, windows))
    }

    fn observe_for_start(&self, req: &StartRequest) -> Result<StartObservation, String> {
        let (worktrees, markers, windows) = self.read_world()?;
        Ok(StartObservation {
            sessions: join(&worktrees, &markers, &windows),
            worktrees,
            markers,
            target_dir_exists: worktree_dir_for(&self.repo_root, req.issue).exists(),
            branch_exists: git::branch_exists(&self.repo_root, &req.branch)?,
        })
    }

    fn layout(&self, launch: &Launch) -> Layout {
        Layout {
            repo_root: self.repo_root.clone(),
            base: self.base.clone(),
            hooks: self.mux.hooks(launch),
            claude: launch.claude.clone(),
        }
    }

    /// Converges to "a live window runs Claude in this issue's worktree".
    /// Safe to call twice, and safe to call again after a crash at any step:
    /// it re-observes, plans only the missing steps, and runs them.
    pub fn start(&self, req: &StartRequest, launch: &Launch) -> Result<StartOutcome, SessionError> {
        let obs = self.observe_for_start(req)?;
        match plan_start(&obs, req, &self.layout(launch)).map_err(SessionError::Conflict)? {
            StartPlan::AlreadyRunning(session) => Ok(StartOutcome { session, warnings: Vec::new() }),
            StartPlan::Steps(steps) => {
                let warnings = self.run_steps(steps, req.issue)?;
                let session = self.observe()?.remove(&req.issue).ok_or_else(|| {
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
    fn run_steps(&self, steps: Vec<Step>, issue: IssueNumber) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        for step in steps {
            match step {
                Step::Fetch(remote_ref) => {
                    if let Err(e) = git::fetch(&self.repo_root, &remote_ref) {
                        warnings.push(format!("fetch failed, using the local {remote_ref}: {e}"));
                    }
                }
                Step::AddWorktree { dir, branch, from } => {
                    if let Err(e) = git::ensure_excluded(&self.git_common_dir, WORKTREES_EXCLUDE) {
                        warnings.push(format!("{WORKTREES_DIR} still shows in git status: {e}"));
                    }
                    match &self.mux {
                        Mux::Tmux(_) | Mux::Hc(_) => {
                            git::add_worktree(&self.repo_root, &dir, &branch, from.as_deref())?;
                        }
                        Mux::Herdr(place) => {
                            herdr::create_worktree(place, &self.repo_root, &dir, &branch, from.as_deref(), issue)?;
                        }
                    }
                }
                Step::Mark { branch, issue } => git::set_marker(&self.repo_root, &branch, issue)?,
                Step::OpenWindow { dir, name, argv } => self.open_window(&dir, &name, issue, &argv)?,
            }
        }
        Ok(warnings)
    }

    fn open_window(&self, dir: &WorktreePath, name: &str, issue: IssueNumber, argv: &[String]) -> Result<(), String> {
        match &self.mux {
            Mux::Tmux(place) => tmux::open_window(place, dir, name, argv).map(|_| ()),
            Mux::Herdr(place) => herdr::open_window(
                place,
                &self.repo_root,
                dir,
                issue,
                argv,
                &herdr::script_file(&self.git_common_dir, issue),
            ),
            Mux::Hc(_) => hc::open_window(&self.repo_root, dir, issue, argv).map(|_| ()),
        }
    }

    /// Runs `claude --continue` for an exited session: `respawn-window -k` when
    /// the dead pane is still there (same window id), else a new tagged window
    /// as in `start`. No-op if the window is alive. It observes again first, so
    /// a stale row can not aim at a window id that tmux reused.
    pub fn resume(&self, issue: IssueNumber, launch: &Launch) -> Result<(), SessionError> {
        let Some(session) = self.observe()?.remove(&issue) else {
            return Err(SessionError::Command(format!("no session for #{}", issue.0)));
        };
        let argv = launch.claude.argv(&start_argv::continue_args(self.mux.hooks(launch)));
        match &session.window {
            Some(w) if w.alive => Ok(()),
            Some(w) => match &w.id {
                WindowId::Tmux(id) => Ok(tmux::respawn(id, &session.worktree, &argv)?),
                WindowId::Herdr(pane) => {
                    Ok(herdr::launch(pane, &argv, &herdr::script_file(&self.git_common_dir, issue))?)
                }
                WindowId::Hc(pane) => {
                    hc::remove(&pane.address)?;
                    Ok(self.open_window(&session.worktree, &format!("#{}", issue.0), issue, &argv)?)
                }
            },
            None => Ok(self.open_window(&session.worktree, &format!("#{}", issue.0), issue, &argv)?),
        }
    }

    /// Kills the window. The worktree and branch stay. A window that is
    /// already gone counts as success.
    pub fn stop(&self, issue: IssueNumber) -> Result<(), SessionError> {
        match self.observe()?.remove(&issue).and_then(|s| s.window) {
            Some(Window { id: WindowId::Tmux(id), .. }) => Ok(tmux::kill_window(&id)?),
            Some(Window { id: WindowId::Herdr(pane), .. }) => Ok(herdr::close_workspace(&pane.workspace)?),
            Some(Window { id: WindowId::Hc(pane), .. }) => Ok(hc::stop(&pane.address)?),
            None => Ok(()),
        }
    }

    /// Types `text` into the session's Claude prompt and submits it. Observes
    /// again first, like `stop`, and refuses a dead pane, where the keys would
    /// go nowhere.
    pub fn send(&self, issue: IssueNumber, text: &str) -> Result<(), SessionError> {
        match self.observe()?.remove(&issue).and_then(|s| s.window) {
            Some(Window { id: WindowId::Tmux(id), alive: true, .. }) => Ok(tmux::send_line(&id, text)?),
            Some(Window { id: WindowId::Herdr(pane), alive: true, .. }) => Ok(herdr::send_line(&pane, text)?),
            Some(Window { id: WindowId::Hc(pane), alive: true, .. }) => Ok(hc::send_line(&pane.address, text)?),
            _ => Err(SessionError::Command(format!("#{} has no running session", issue.0))),
        }
    }

    pub fn attach_plan(&self, window: &WindowId) -> Result<AttachPlan, SessionError> {
        match (&self.mux, window) {
            (Mux::Tmux(place), WindowId::Tmux(id)) => Ok(tmux_attach_plan(place, id)),
            (Mux::Herdr(herdr::Place::Inside { .. }), WindowId::Herdr(pane)) => Ok(AttachPlan::Switch(
                ["herdr", "workspace", "focus", pane.workspace.as_str()].map(String::from).to_vec(),
            )),
            (Mux::Herdr(herdr::Place::Detached { .. }), WindowId::Herdr(pane)) => Ok(AttachPlan::Suspend(
                ["sh", "-c", "herdr workspace focus \"$1\" >/dev/null && exec herdr", "sh", pane.workspace.as_str()]
                    .map(String::from)
                    .to_vec(),
            )),
            (Mux::Hc(place), WindowId::Hc(pane)) => Ok(hc::attach_plan(*place, pane)),
            _ => Err(SessionError::Command("that window belongs to another multiplexer".to_string())),
        }
    }
}

fn tmux_attach_plan(place: &tmux::Place, window: &tmux::WindowId) -> AttachPlan {
    let session = place.session_target();
    let select = ["tmux", "select-window", "-t", window.as_str()].map(String::from);
    let (verb, plan): (&str, fn(Vec<String>) -> AttachPlan) = match place {
        tmux::Place::Inside { .. } => ("switch-client", AttachPlan::Switch),
        tmux::Place::Detached { .. } => ("attach-session", AttachPlan::Suspend),
    };
    plan(select.into_iter().chain([";", verb, "-t", session.as_str()].map(String::from)).collect())
}

#[cfg(test)]
mod tests;
