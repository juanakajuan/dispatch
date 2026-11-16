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

mod git;
mod hc;
mod herdr;
mod start_argv;
mod tmux;

pub use start_argv::StartPreview;
use start_argv::claude_argv;

use crate::config::{ClaudeCommand, Config, Multiplexer};
use crate::domain::{BranchName, IssueNumber, Mode, WorktreePath};
use crate::rules;
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mux {
    Tmux(tmux::Place),
    Herdr(herdr::Place),
    /// The herdr app. hc finds its core itself, so the only fact is whether
    /// dispatch runs inside the app.
    Hc(hc::Place),
}

impl Mux {
    fn hooks(&self, launch: &Launch) -> Option<&'static str> {
        (launch.detect_waiting && matches!(self, Mux::Tmux(_))).then_some(WAITING_HOOKS)
    }
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

/// What the start dialog hands over after validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub issue: IssueNumber,
    pub branch: BranchName,
    pub mode: Mode,
    pub prompt: String,
    pub from: StartFrom,
}

/// Where a new branch starts. Used only when `branch` does not exist locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartFrom {
    /// The default branch, for new work on an issue.
    Base,
    /// The branch of the same name on origin, for work on an open PR.
    Origin,
}

/// A converged start. `warnings` hold non-fatal problems, such as a failed
/// `git fetch` that fell back to the local base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartOutcome {
    pub session: Session,
    pub warnings: Vec<String>,
}

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
                write!(f, "branch {} already belongs to issue #{}", branch.as_str(), issue.0)
            }
            StartConflict::PathTaken(path) => {
                write!(f, "{} exists and is not a git worktree. Move it away and start again.", path.display())
            }
        }
    }
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

/// The config that shapes a start or resume. It is a value the caller passes
/// per call, so a settings change reaches the next session with no shared
/// state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub detect_waiting: bool,
    pub claude: ClaudeCommand,
}

impl Launch {
    pub fn from_config(config: &Config) -> Launch {
        Launch { detect_waiting: config.detect_waiting, claude: config.claude_command.clone() }
    }
}

#[derive(Debug)]
pub enum SetupError {
    NotARepo(String),
    NoOriginHead(String),
    ProgramMissing(&'static str),
    Command(String),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotARepo(m) => write!(f, "dispatch must run inside a git repository: {m}"),
            SetupError::NoOriginHead(m) => write!(f, "cannot find the default branch of origin: {m}"),
            SetupError::ProgramMissing(program) => write!(f, "{program} is not installed or not on PATH"),
            SetupError::Command(m) => f.write_str(m),
        }
    }
}

fn require(program: &'static str, version_flag: &str, cwd: &std::path::Path) -> Result<(), SetupError> {
    crate::proc::run(program, [version_flag], cwd).map(|_| ()).map_err(|e| match e {
        crate::proc::ProcError::Spawn { .. } => SetupError::ProgramMissing(program),
        other => SetupError::Command(other.to_string()),
    })
}

#[derive(Clone, Debug, Default)]
struct MuxEnv {
    herdr_env: Option<String>,
    herdr_workspace: Option<String>,
    tmux: Option<String>,
    hc_socket: Option<String>,
}

impl MuxEnv {
    fn from_process() -> MuxEnv {
        let var = |name| std::env::var(name).ok();
        MuxEnv {
            herdr_env: var("HERDR_ENV"),
            herdr_workspace: var("HERDR_WORKSPACE_ID"),
            tmux: var("TMUX"),
            hc_socket: var("HC_TERMINAL_SOCKET"),
        }
    }
}

fn detect_mux(choice: Multiplexer, env: &MuxEnv, repo_root: &std::path::Path) -> Result<Mux, SetupError> {
    let inside_herdr = env.herdr_env.as_deref() == Some("1");
    let dir_name = repo_root.file_name().map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    let inside_app = env.hc_socket.as_deref().is_some_and(|s| !s.is_empty());
    if choice == Multiplexer::HerdrApp || (choice == Multiplexer::Auto && !inside_herdr && inside_app) {
        require("hc", "--version", repo_root)?;
        return Ok(Mux::Hc(if inside_app { hc::Place::App } else { hc::Place::Outside }));
    }
    if choice == Multiplexer::Herdr || (choice == Multiplexer::Auto && inside_herdr) {
        require("herdr", "--version", repo_root)?;
        let place = herdr::Place::detect(env.herdr_env.as_deref(), env.herdr_workspace.as_deref(), &dir_name)
            .map_err(SetupError::Command)?;
        return Ok(Mux::Herdr(place));
    }
    require("tmux", "-V", repo_root)?;
    let place = tmux::Place::detect(env.tmux.as_deref(), &dir_name).map_err(SetupError::Command)?;
    Ok(Mux::Tmux(place))
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
            target_dir_exists: rules::worktree_dir_for(&self.repo_root, req.issue).exists(),
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
                    if let Err(e) = git::ensure_excluded(&self.git_common_dir, rules::WORKTREES_EXCLUDE) {
                        warnings.push(format!("{} still shows in git status: {e}", rules::WORKTREES_DIR));
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
        let argv = launch.claude.argv(&rules::continue_args(self.mux.hooks(launch)));
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

// ---- pure core -------------------------------------------------------------

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree is a session only if its branch has a valid `dispatch-issue`.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one issue: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
/// - Two windows tagged for one worktree: a live one wins.
fn join(worktrees: &[git::Worktree], markers: &git::Markers, windows: &[WindowObs]) -> Sessions {
    let mut by_path: Vec<&git::Worktree> = worktrees.iter().collect();
    by_path.sort_by(|a, b| a.path.cmp(&b.path));
    let mut sessions = Sessions::new();
    for w in by_path {
        let Some(branch) = &w.branch else { continue };
        let Some(&issue) = markers.get(branch) else { continue };
        let window = windows.iter().filter(|o| o.worktree == w.path).min_by_key(|o| o.dead).map(|o| Window {
            id: o.id.clone(),
            alive: !o.dead,
            waiting: o.waiting,
        });
        sessions.entry(issue).or_insert_with(|| Session {
            issue,
            branch: branch.clone(),
            worktree: w.path.clone(),
            window,
        });
    }
    sessions
}

/// Everything `plan_start` needs. Gathered by `observe_for_start`.
struct StartObservation {
    sessions: Sessions,
    worktrees: Vec<git::Worktree>,
    markers: git::Markers,
    /// The deterministic dir (`rules::worktree_dir_for`) exists on disk.
    target_dir_exists: bool,
    /// `req.branch` exists as a local branch.
    branch_exists: bool,
}

struct Layout {
    repo_root: PathBuf,
    /// `origin/main`.
    base: String,
    hooks: Option<&'static str>,
    claude: ClaudeCommand,
}

#[derive(Debug, PartialEq, Eq)]
enum StartPlan {
    AlreadyRunning(Session),
    Steps(Vec<Step>),
}

/// One subprocess-level action. Each is idempotent on its own or guarded by
/// the plan, so a crash between any two steps leaves a state that the next
/// `plan_start` recognizes and finishes.
#[derive(Debug, PartialEq, Eq)]
enum Step {
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
fn plan_start(obs: &StartObservation, req: &StartRequest, layout: &Layout) -> Result<StartPlan, StartConflict> {
    let target = rules::worktree_dir_for(&layout.repo_root, req.issue);
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

/// Claude hooks passed with `--settings` when `detect_waiting` is on.
/// `$TMUX_PANE` is in every pane's environment, and hooks inherit it.
/// Notification and Stop set the flag; `UserPromptSubmit` and `PreToolUse` clear it.
const WAITING_HOOKS: &str = r#"{"hooks":{
  "Notification":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "Stop":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "UserPromptSubmit":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}],
  "PreToolUse":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}]
}}"#;

#[cfg(test)]
mod tests;
