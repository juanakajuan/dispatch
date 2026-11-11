//! Sessions. The only module that knows git and tmux exist.
//!
//! Session truth lives in git and tmux, never in a dispatch file:
//!
//! - git worktree list            -> which worktrees exist, and their branch
//! - git config branch.<b>.dispatch-issue / dispatch-skill
//!                                -> which branches dispatch made, for which issue
//! - tmux window option @dispatch-worktree (and @dispatch-waiting)
//!                                -> which window runs Claude in which worktree
//!
//! `observe()` reads all three and joins them on the worktree path. Nothing is
//! cached here; `Workspace` is immutable and `Clone`, so each worker thread
//! gets its own copy and no lock exists anywhere.
//!
//! The git markers live in `.git/config`, so they survive a dispatch crash, a
//! tmux server restart, and a reboot. `git branch -D` deletes the `branch.<b>`
//! section, so a deleted branch leaves no stale marker.

mod git;
mod tmux;

use crate::domain::{BranchName, IssueNumber, Mode, SkillChoice, WorktreePath};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub use tmux::WindowId;

/// One issue being worked in one worktree. Derived, never stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub issue: IssueNumber,
    pub branch: BranchName,
    pub worktree: WorktreePath,
    /// From the `dispatch-skill` marker. Shown in the detail pane.
    pub skill: SkillChoice,
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
        // TODO: match &self.window {
        //   Some(w) if w.alive && w.waiting => NeedsYou,
        //   Some(w) if w.alive => Running,
        //   _ => Exited }
        todo!()
    }
}

/// At most one session per issue, by construction of the map.
pub type Sessions = BTreeMap<IssueNumber, Session>;

/// What the start dialog hands over after validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub issue: IssueNumber,
    pub branch: BranchName,
    pub skill: SkillChoice,
    pub mode: Mode,
    pub prompt: String,
}

#[derive(Debug)]
pub enum SessionError {
    /// The request can not converge without the user deciding something.
    Conflict(StartConflict),
    Command(String),
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

/// How the shell attaches. Pure data, so the choice is unit tested.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachPlan {
    /// Inside tmux: select the window; dispatch keeps running in its own window.
    Switch(Vec<String>),
    /// Outside tmux: the shell leaves the alternate screen, runs this argv in
    /// the foreground, and restores the TUI when it returns.
    Suspend(Vec<String>),
}

/// Startup facts, resolved once. Immutable after `discover`.
#[derive(Clone, Debug)]
pub struct Workspace {
    repo_root: PathBuf,
    /// `origin/main` from `git symbolic-ref --short refs/remotes/origin/HEAD`.
    base: String,
    place: tmux::Place,
    detect_waiting: bool,
}

#[derive(Debug)]
pub enum SetupError {
    NotARepo,
    NoOriginHead,
    TmuxMissing,
    Command(String),
}

impl Workspace {
    /// Fails before the TUI starts, so the user sees a plain error line.
    pub fn discover(cwd: &std::path::Path, detect_waiting: bool) -> Result<Workspace, SetupError> {
        // TODO: git::repo_root(cwd); git::origin_head(root);
        //       tmux::Place::detect(env TMUX, repo dir name) -> Inside{session_id} | Detached{name}.
        todo!()
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    /// Three subprocesses (worktree list, config regexp, tmux list-windows),
    /// then the pure `join`. A missing tmux server is an empty window list.
    pub fn observe(&self) -> Result<Sessions, SessionError> {
        todo!()
    }

    /// Converges to "a live window runs Claude in this issue's worktree".
    /// Safe to call twice, and safe to call again after a crash at any step:
    /// it re-observes, plans only the missing steps, and runs them.
    pub fn start(&self, req: &StartRequest) -> Result<Session, SessionError> {
        // TODO: let obs = self.observe_for_start(req)?;
        //       match plan_start(&obs, req, &self.layout())? {
        //         StartPlan::AlreadyRunning(s) => Ok(s),
        //         StartPlan::Steps(steps) => { for step in steps { self.run_step(step)? }
        //                                      re-observe and return sessions[req.issue] } }
        todo!()
    }

    /// Runs `claude --continue` for an exited session: `respawn-window -k` when
    /// the dead pane is still there (same window id), else a new tagged window
    /// as in `start`. No-op if the window is alive.
    pub fn resume(&self, session: &Session) -> Result<(), SessionError> {
        todo!()
    }

    /// Kills the window. The worktree and branch stay. A window that is
    /// already gone counts as success.
    pub fn stop(&self, session: &Session) -> Result<(), SessionError> {
        todo!()
    }

    /// Last `lines` lines of the pane as plain text (`capture-pane -p -J`, no
    /// `-e`, so no escape codes reach the UI). Works on dead panes too.
    pub fn tail(&self, window: &WindowId, lines: u16) -> Result<Vec<String>, SessionError> {
        todo!()
    }

    pub fn attach_plan(&self, window: &WindowId) -> AttachPlan {
        // TODO: Inside  -> Switch(["tmux","select-window","-t",id,";","switch-client","-t",id])
        //       Detached-> Suspend(["tmux","select-window","-t",id,";","attach-session","-t",session])
        todo!()
    }
}

// ---- pure core -------------------------------------------------------------

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree is a session only if its branch has a valid `dispatch-issue`.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one issue: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
fn join(worktrees: &[git::Worktree], markers: &git::Markers, windows: &[tmux::WindowObs]) -> Sessions {
    todo!()
}

/// Everything `plan_start` needs. Gathered by `observe_for_start`.
struct StartObservation {
    sessions: Sessions,
    worktrees: Vec<git::Worktree>,
    markers: git::Markers,
    /// The deterministic dir (`rules::worktree_dir_for`) exists on disk.
    target_dir_exists: bool,
    branch_exists: bool,
}

struct Layout {
    repo_root: PathBuf,
    base: String,
    place: tmux::Place,
    hooks: Option<&'static str>,
}

enum StartPlan {
    AlreadyRunning(Session),
    Steps(Vec<Step>),
}

/// One subprocess-level action. Each is idempotent on its own or guarded by
/// the plan, so a crash between any two steps leaves a state that the next
/// `plan_start` recognizes and finishes.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// `git fetch origin <default>` so the new branch starts from a fresh base.
    Fetch,
    /// `git worktree add <dir> -b <branch> <base>` when the branch is new,
    /// `git worktree add <dir> <branch>` when it exists (crash recovery).
    AddWorktree { dir: PathBuf, branch: BranchName, create: bool },
    /// `git config branch.<b>.dispatch-issue N` and `dispatch-skill S`.
    /// Overwrites, so running it twice is harmless.
    Mark { branch: BranchName, issue: IssueNumber, skill: SkillChoice },
    /// One tmux invocation: new-window (or new-session when the detached
    /// session is missing) with argv `claude ...`, then set-option for
    /// `@dispatch-worktree` and `remain-on-exit on` in the same command chain.
    OpenWindow { dir: WorktreePath, name: String, argv: Vec<String> },
}

/// Pure. Decision table, tested row by row:
///
/// | observed                                              | plan                              |
/// |-------------------------------------------------------|-----------------------------------|
/// | session for issue, window alive                       | AlreadyRunning                    |
/// | session for issue, no window or dead window           | [OpenWindow]                      |
/// | a branch marked for issue, no worktree (crash)        | [AddWorktree create=false, Mark, OpenWindow] using that branch |
/// | worktree at target dir, branch unmarked (crash)       | [Mark, OpenWindow]                |
/// | target dir exists, not a worktree                     | Conflict PathTaken                |
/// | req.branch marked for another issue                   | Conflict BranchBelongsTo          |
/// | req.branch checked out elsewhere                      | Conflict BranchCheckedOut         |
/// | req.branch exists, unmarked, free                     | [AddWorktree create=false, Mark, OpenWindow] |
/// | nothing exists                                        | [Fetch, AddWorktree create=true, Mark, OpenWindow] |
///
/// A marked branch for this issue wins over `req.branch`, so a retry with an
/// edited branch name can not make a second branch for one issue.
fn plan_start(obs: &StartObservation, req: &StartRequest, layout: &Layout) -> Result<StartPlan, StartConflict> {
    todo!()
}

/// Claude hooks passed with `--settings` when `detect_waiting` is on.
/// `$TMUX_PANE` is in every pane's environment, and hooks inherit it.
/// Notification and Stop set the flag; UserPromptSubmit and PreToolUse clear it.
const WAITING_HOOKS: &str = r#"{"hooks":{
  "Notification":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "Stop":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "UserPromptSubmit":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}],
  "PreToolUse":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}]
}}"#;

#[cfg(test)]
mod tests {
    // TODO: join: marked worktree + tagged live window -> Running; tagged dead pane -> Exited
    //       with window Some; no window -> Exited; unmarked worktree -> not a session;
    //       window tagged with a removed worktree -> ignored; @dispatch-waiting=1 -> NeedsYou.
    // TODO: plan_start: one test per decision-table row. Plus: running plan_start on the
    //       observation that results from applying each prefix of a plan yields the
    //       remaining suffix (crash-at-any-step convergence).
}
