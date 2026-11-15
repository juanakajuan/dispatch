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
mod tmux;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Tmux,
    Herdr,
    Hc,
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

fn text_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text.lines().map(str::to_string).collect();
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out
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

    pub fn backend(&self) -> Backend {
        match self.mux {
            Mux::Tmux(_) => Backend::Tmux,
            Mux::Herdr(_) => Backend::Herdr,
            Mux::Hc(_) => Backend::Hc,
        }
    }

    /// `origin/main`, for the dialog preview.
    pub fn base(&self) -> &str {
        &self.base
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

    /// Last `lines` lines of the pane as plain text (`capture-pane -p -J`, no
    /// `-e`, so no escape codes reach the UI). Works on dead panes too.
    #[expect(clippy::unused_self, reason = "keeps every session operation a `Workspace` method")]
    pub fn tail(&self, window: &WindowId, lines: u16) -> Result<Vec<String>, SessionError> {
        match window {
            WindowId::Tmux(id) => Ok(tmux::capture(id, lines)?),
            WindowId::Herdr(pane) => Ok(herdr::read_tail(pane, lines)?),
            WindowId::Hc(pane) => Ok(hc::read_tail(&pane.address, lines)?),
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
        argv: layout.claude.argv(&rules::claude_args(req.mode, &req.prompt, layout.hooks)),
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
mod tests {
    use super::*;
    use crate::proc::fake;
    use crate::testutil::temp_dir;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn wp(s: &str) -> WorktreePath {
        WorktreePath::planned(PathBuf::from(s))
    }

    fn br(s: &str) -> BranchName {
        BranchName::parse(s).expect("valid branch")
    }

    fn wt(path: &str, branch: Option<&str>) -> git::Worktree {
        git::Worktree { path: wp(path), branch: branch.map(br) }
    }

    fn markers(rows: &[(&str, u32)]) -> git::Markers {
        rows.iter().map(|(b, n)| (br(b), IssueNumber(*n))).collect()
    }

    fn win(id: &str, path: &str, dead: bool, waiting: bool) -> WindowObs {
        WindowObs { id: WindowId::parse(id).expect("window id"), worktree: wp(path), dead, waiting }
    }

    // ---- join ----

    #[test]
    fn join_live_window_is_running_and_waiting_flag_needs_you() {
        let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-1", Some("fix/1-x"))];
        let m = markers(&[("fix/1-x", 1)]);
        let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[&IssueNumber(1)].state(), SessionState::Running);
        let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, true)]);
        assert_eq!(s[&IssueNumber(1)].state(), SessionState::NeedsYou);
    }

    #[test]
    fn join_dead_pane_and_missing_window_are_exited() {
        let worktrees = [wt("/a/orbit-1", Some("fix/1-x"))];
        let m = markers(&[("fix/1-x", 1)]);
        let dead = join(&worktrees, &m, &[win("@1", "/a/orbit-1", true, true)]);
        assert_eq!(dead[&IssueNumber(1)].state(), SessionState::Exited);
        assert!(dead[&IssueNumber(1)].window.is_some());
        let none = join(&worktrees, &m, &[]);
        assert_eq!(none[&IssueNumber(1)].state(), SessionState::Exited);
        assert!(none[&IssueNumber(1)].window.is_none());
    }

    #[test]
    fn join_ignores_unmarked_detached_and_orphan_windows() {
        let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-2", None)];
        let s = join(&worktrees, &markers(&[("other", 9)]), &[win("@5", "/a/gone", false, false)]);
        assert!(s.is_empty());
    }

    #[test]
    fn join_one_issue_marked_twice_keeps_first_path_and_prefers_live_window() {
        let worktrees = [wt("/a/z", Some("b2")), wt("/a/a", Some("b1"))];
        let m = markers(&[("b1", 7), ("b2", 7)]);
        let s = join(&worktrees, &m, &[win("@1", "/a/a", true, false), win("@2", "/a/a", false, false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[&IssueNumber(7)].branch, br("b1"));
        assert_eq!(s[&IssueNumber(7)].window.as_ref().map(|w| w.id.as_str()), Some("@2"));
    }

    // ---- plan_start, one test per table row ----

    /// `rules::worktree_dir_for` of `layout().repo_root` and issue 142.
    const TARGET: &str = "/a/orbit/.claude/worktrees/142";

    fn layout() -> Layout {
        Layout {
            repo_root: PathBuf::from("/a/orbit"),
            base: "origin/main".to_string(),
            hooks: None,
            claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() },
        }
    }

    fn req() -> StartRequest {
        StartRequest {
            issue: IssueNumber(142),
            branch: br("fix/142-x"),
            mode: Mode::Implement,
            prompt: "p".to_string(),
            from: StartFrom::Base,
        }
    }

    struct World {
        worktrees: Vec<git::Worktree>,
        markers: git::Markers,
        windows: Vec<WindowObs>,
        target_dir_exists: bool,
        branch_exists: bool,
    }

    impl World {
        fn empty() -> World {
            World {
                worktrees: vec![wt("/a/orbit", Some("main"))],
                markers: git::Markers::new(),
                windows: Vec::new(),
                target_dir_exists: false,
                branch_exists: false,
            }
        }

        fn obs(&self) -> StartObservation {
            StartObservation {
                sessions: join(&self.worktrees, &self.markers, &self.windows),
                worktrees: self.worktrees.clone(),
                markers: self.markers.clone(),
                target_dir_exists: self.target_dir_exists,
                branch_exists: self.branch_exists,
            }
        }

        fn plan(&self) -> Result<StartPlan, StartConflict> {
            plan_start(&self.obs(), &req(), &layout())
        }

        /// Applies one step to the observable world, the way the real
        /// commands would.
        fn apply(&mut self, step: &Step) {
            match step {
                Step::Fetch(_) => {}
                Step::AddWorktree { dir, branch, .. } => {
                    self.worktrees
                        .push(git::Worktree { path: WorktreePath::planned(dir.clone()), branch: Some(branch.clone()) });
                    self.branch_exists = true;
                    self.target_dir_exists = true;
                }
                Step::Mark { branch, issue } => {
                    self.markers.insert(branch.clone(), *issue);
                }
                Step::OpenWindow { dir, .. } => {
                    self.windows.push(WindowObs {
                        id: WindowId::parse("@9").expect("id"),
                        worktree: dir.clone(),
                        dead: false,
                        waiting: false,
                    });
                }
            }
        }
    }

    fn open_step() -> Step {
        let argv = ["claude".to_string(), "p".to_string()].to_vec();
        Step::OpenWindow { dir: wp(TARGET), name: "#142".to_string(), argv }
    }

    fn add(branch: &str, from: Option<&str>) -> Step {
        Step::AddWorktree { dir: PathBuf::from(TARGET), branch: br(branch), from: from.map(String::from) }
    }

    fn fetch(remote_ref: &str) -> Step {
        Step::Fetch(remote_ref.to_string())
    }

    fn mark(branch: &str) -> Step {
        Step::Mark { branch: br(branch), issue: IssueNumber(142) }
    }

    fn steps(plan: Result<StartPlan, StartConflict>) -> Vec<Step> {
        match plan {
            Ok(StartPlan::Steps(s)) => s,
            other => panic!("expected steps, got {other:?}"),
        }
    }

    #[test]
    fn plan_session_with_live_window_is_already_running() {
        let mut w = World::empty();
        w.worktrees.push(wt(TARGET, Some("fix/142-x")));
        w.markers = markers(&[("fix/142-x", 142)]);
        w.windows.push(win("@3", TARGET, false, false));
        assert!(matches!(w.plan(), Ok(StartPlan::AlreadyRunning(s)) if s.issue == IssueNumber(142)));
    }

    #[test]
    fn plan_session_without_live_window_opens_one() {
        let mut w = World::empty();
        w.worktrees.push(wt(TARGET, Some("fix/142-x")));
        w.markers = markers(&[("fix/142-x", 142)]);
        assert_eq!(steps(w.plan()), [open_step()]);
        w.windows.push(win("@3", TARGET, true, false));
        assert_eq!(steps(w.plan()), [open_step()]);
    }

    #[test]
    fn plan_session_in_an_old_sibling_worktree_reopens_there() {
        let mut w = World::empty();
        w.worktrees.push(wt("/a/orbit-142", Some("fix/142-x")));
        w.markers = markers(&[("fix/142-x", 142)]);
        let opened = steps(w.plan());
        assert!(matches!(&opened[..], [Step::OpenWindow { dir, .. }] if dir == &wp("/a/orbit-142")), "{opened:?}");
    }

    #[test]
    fn plan_marked_branch_without_worktree_uses_that_branch() {
        let mut w = World::empty();
        w.markers = markers(&[("fix/142-earlier-name", 142)]);
        w.branch_exists = false;
        assert_eq!(steps(w.plan()), [add("fix/142-earlier-name", None), mark("fix/142-earlier-name"), open_step()]);
    }

    #[test]
    fn plan_worktree_at_target_without_marker_marks_its_branch() {
        let mut w = World::empty();
        w.worktrees.push(wt(TARGET, Some("fix/142-x")));
        w.target_dir_exists = true;
        assert_eq!(steps(w.plan()), [mark("fix/142-x"), open_step()]);
    }

    #[test]
    fn plan_target_dir_that_is_no_worktree_conflicts() {
        let mut w = World::empty();
        w.target_dir_exists = true;
        assert_eq!(w.plan(), Err(StartConflict::PathTaken(PathBuf::from(TARGET))));
    }

    #[test]
    fn plan_branch_marked_for_another_issue_conflicts() {
        let mut w = World::empty();
        w.markers = markers(&[("fix/142-x", 7)]);
        w.branch_exists = true;
        assert_eq!(w.plan(), Err(StartConflict::BranchBelongsTo { branch: br("fix/142-x"), issue: IssueNumber(7) }));
    }

    #[test]
    fn plan_branch_checked_out_elsewhere_conflicts() {
        let mut w = World::empty();
        w.worktrees.push(wt("/elsewhere", Some("fix/142-x")));
        w.branch_exists = true;
        assert_eq!(
            w.plan(),
            Err(StartConflict::BranchCheckedOut { branch: br("fix/142-x"), path: PathBuf::from("/elsewhere") })
        );
    }

    #[test]
    fn plan_existing_free_branch_skips_the_fetch_and_the_create() {
        let mut w = World::empty();
        w.branch_exists = true;
        assert_eq!(steps(w.plan()), [add("fix/142-x", None), mark("fix/142-x"), open_step()]);
    }

    #[test]
    fn plan_from_nothing_runs_all_four_steps() {
        assert_eq!(
            steps(World::empty().plan()),
            [fetch("origin/main"), add("fix/142-x", Some("origin/main")), mark("fix/142-x"), open_step()]
        );
    }

    #[test]
    fn plan_for_a_pr_branch_starts_from_its_origin_branch() {
        let mut r = req();
        r.from = StartFrom::Origin;
        let got = steps(plan_start(&World::empty().obs(), &r, &layout()));
        assert_eq!(
            got,
            [fetch("origin/fix/142-x"), add("fix/142-x", Some("origin/fix/142-x")), mark("fix/142-x"), open_step()]
        );
        let mut w = World::empty();
        w.branch_exists = true;
        let got = steps(plan_start(&w.obs(), &r, &layout()));
        assert_eq!(
            got,
            [add("fix/142-x", None), mark("fix/142-x"), open_step()],
            "an existing local branch is used as is"
        );
    }

    #[test]
    fn plan_uses_config_claude_command_hooks_and_plan_mode() {
        let mut l = layout();
        l.claude = ClaudeCommand {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "echo".to_string(), "_".to_string()],
        };
        l.hooks = Some("{}");
        let mut r = req();
        r.mode = Mode::PlanFirst;
        let got = steps(plan_start(&World::empty().obs(), &r, &l));
        let Some(Step::OpenWindow { argv, .. }) = got.last() else { panic!("no OpenWindow") };
        assert_eq!(argv, &["sh", "-c", "echo", "_", "--permission-mode", "plan", "--settings", "{}", "p"]);
    }

    /// Crash at any step: apply each prefix of a plan, observe, plan again.
    /// The new plan is the original (nothing observable changed yet) or a
    /// suffix of it, so no step runs twice and the run finishes.
    #[test]
    fn plan_converges_after_a_crash_at_any_step() {
        let starts: Vec<World> = {
            let mut fresh = World::empty();
            fresh.branch_exists = false;
            let mut existing = World::empty();
            existing.branch_exists = true;
            let mut marked = World::empty();
            marked.markers = markers(&[("fix/142-old", 142)]);
            let mut half = World::empty();
            half.worktrees.push(wt(TARGET, Some("fix/142-x")));
            half.target_dir_exists = true;
            vec![fresh, existing, marked, half]
        };
        for mut world in starts {
            let original = steps(world.plan());
            let without_fetch: Vec<&Step> = original.iter().filter(|s| !matches!(s, Step::Fetch(_))).collect();
            for (k, step) in original.iter().enumerate() {
                world.apply(step);
                let rest = world.plan();
                if k + 1 == original.len() {
                    assert!(matches!(rest, Ok(StartPlan::AlreadyRunning(_))), "after all steps");
                    continue;
                }
                let rest = steps(rest);
                let rest_refs: Vec<&Step> = rest.iter().collect();
                let is_original = rest == original;
                let is_suffix = without_fetch.ends_with(&rest_refs);
                assert!(is_original || is_suffix, "after {} steps got {rest:?}, from {original:?}", k + 1);
            }
        }
    }

    // ---- the step runner through the proc fake ----

    #[derive(Default)]
    #[expect(clippy::struct_excessive_bools, reason = "one independent flag per fake world fact")]
    struct Sim {
        added: bool,
        marked: bool,
        window: bool,
        tagged: bool,
        fetch_fails: bool,
    }

    /// A tiny git and tmux: answers the observation commands from `Sim`
    /// and changes `Sim` on the mutating ones.
    fn install_world(base: PathBuf, sim: Rc<RefCell<Sim>>) {
        fake::install(move |call| {
            let mut s = sim.borrow_mut();
            let dir = base.join("orbit/.claude/worktrees/142");
            if call.starts_with("git worktree list") {
                let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
                if s.added {
                    t = format!("{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display());
                }
                Ok(t)
            } else if call.starts_with("git config --get-regexp") {
                if s.marked {
                    Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
                } else {
                    Err(fake::fail("git", 1, ""))
                }
            } else if call.starts_with("git show-ref") {
                if s.added { Ok(String::new()) } else { Err(fake::fail("git", 1, "")) }
            } else if call.starts_with("tmux list-windows") {
                Ok(if s.window && s.tagged { format!("@4\t0\t{}\t\n", dir.display()) } else { String::new() })
            } else if call.starts_with("git fetch") {
                if s.fetch_fails { Err(fake::fail("git", 128, "no network")) } else { Ok(String::new()) }
            } else if call.starts_with("git worktree add") {
                std::fs::create_dir_all(&dir).expect("mkdir");
                s.added = true;
                Ok(String::new())
            } else if call.contains("dispatch-issue 142") {
                s.marked = true;
                Ok(String::new())
            } else if call.starts_with("tmux new-window") {
                s.window = true;
                Ok("@4\n".to_string())
            } else if call.starts_with("tmux set-option") {
                s.tagged = true;
                Ok(String::new())
            } else {
                Ok(String::new())
            }
        });
    }

    fn workspace(base: &std::path::Path) -> Workspace {
        std::fs::create_dir_all(base.join("orbit")).expect("mkdir");
        Workspace {
            repo_root: base.join("orbit"),
            git_common_dir: base.join("orbit").join(".git"),
            base: "origin/main".to_string(),
            mux: Mux::Tmux(tmux::Place::Inside { session_id: "$1".to_string() }),
        }
    }

    fn launch() -> Launch {
        Launch { detect_waiting: false, claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() } }
    }

    fn mutating(calls: &[String]) -> Vec<&str> {
        calls
            .iter()
            .map(String::as_str)
            .filter(|c| {
                ["git fetch", "git worktree add", "git config branch", "tmux new-", "tmux set-option"]
                    .iter()
                    .any(|p| c.starts_with(p))
            })
            .collect()
    }

    #[test]
    fn start_runs_the_steps_in_order_and_a_second_start_runs_none() {
        let base = temp_dir("start");
        let ws = workspace(&base);
        let r = req();
        install_world(base.clone(), Rc::new(RefCell::new(Sim::default())));
        let out = ws.start(&r, &launch()).expect("start");
        assert_eq!(out.session.state(), SessionState::Running);
        assert!(out.warnings.is_empty());
        let dir = base.join("orbit/.claude/worktrees/142");
        let calls = fake::calls();
        let ran = mutating(&calls);
        assert_eq!(ran.len(), 5, "{ran:?}");
        assert_eq!(ran[0], "git fetch origin main");
        assert_eq!(ran[1], format!("git worktree add {} -b fix/142-x origin/main", dir.display()));
        assert_eq!(ran[2], "git config branch.fix/142-x.dispatch-issue 142");
        assert!(ran[3].starts_with(&format!(
            "tmux new-window -d -t $1: -P -F #{{window_id}} -c {} -n #142 -- claude p",
            dir.display()
        )));
        assert!(ran[4].starts_with("tmux set-option -w -t @4 @dispatch-worktree"));
        let exclude = base.join("orbit/.git/info/exclude");
        assert_eq!(std::fs::read_to_string(&exclude).expect("exclude written"), ".claude/worktrees/\n");

        let before = fake::calls().len();
        let again = ws.start(&r, &launch()).expect("second start");
        assert_eq!(again.session, out.session);
        assert!(mutating(&fake::calls()[before..]).is_empty(), "second start must not mutate");
    }

    #[test]
    fn start_after_a_crash_between_steps_runs_only_the_rest() {
        let base = temp_dir("crash");
        let ws = workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = Sim { added: true, ..Sim::default() };
        install_world(base.clone(), Rc::new(RefCell::new(sim)));
        let out = ws.start(&req(), &launch()).expect("start");
        assert_eq!(out.session.branch, br("fix/142-x"));
        let calls = fake::calls();
        let ran = mutating(&calls);
        assert_eq!(ran.len(), 3, "{ran:?}");
        assert!(ran[0].starts_with("git config branch.fix/142-x.dispatch-issue"));
        assert!(ran.iter().all(|c| !c.starts_with("git worktree add") && !c.starts_with("git fetch")));
    }

    #[test]
    fn failed_fetch_is_a_warning_and_the_start_continues() {
        let base = temp_dir("fetch");
        let ws = workspace(&base);
        install_world(base.clone(), Rc::new(RefCell::new(Sim { fetch_fails: true, ..Sim::default() })));
        let out = ws.start(&req(), &launch()).expect("start continues");
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("origin/main") && out.warnings[0].contains("no network"));
    }

    #[test]
    fn failed_worktree_add_stops_the_run_with_the_git_message() {
        let base = temp_dir("addfail");
        let ws = workspace(&base);
        fake::install(|call| {
            if call.starts_with("git worktree add") {
                Err(fake::fail("git", 128, "fatal: boom"))
            } else if call.starts_with("git worktree list") || call.starts_with("tmux list-windows") {
                Ok(String::new())
            } else if call.starts_with("git config --get-regexp") || call.starts_with("git show-ref") {
                Err(fake::fail("git", 1, ""))
            } else {
                Ok(String::new())
            }
        });
        let err = ws.start(&req(), &launch()).expect_err("fails");
        assert!(err.to_string().contains("fatal: boom"), "{err}");
        assert!(!fake::calls().iter().any(|c| c.starts_with("tmux new-")));
    }

    #[test]
    fn stop_kills_the_window_by_a_fresh_observation_and_keeps_the_worktree() {
        let base = temp_dir("stop");
        let ws = workspace(&base);
        install_world(
            base.clone(),
            Rc::new(RefCell::new(Sim { added: true, marked: true, window: true, tagged: true, ..Sim::default() })),
        );
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        ws.stop(IssueNumber(142)).expect("stop");
        let calls = fake::calls();
        assert!(calls.iter().any(|c| c == "tmux kill-window -t @4"));
        assert!(!calls.iter().any(|c| c.contains("worktree remove") || c.contains("branch -")));
        assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
    }

    #[test]
    fn resume_respawns_a_dead_pane_in_an_old_sibling_worktree() {
        let base = temp_dir("resume");
        let ws = workspace(&base);
        std::fs::create_dir_all(base.join("orbit-142")).expect("mkdir");
        let dir = base.join("orbit-142");
        let listed = format!("@4\t1\t{}\t\n", dir.display());
        let wt_text = format!("worktree {}\nbranch refs/heads/fix/142-x\n\n", dir.display());
        fake::install(move |call| {
            if call.starts_with("git worktree list") {
                Ok(wt_text.clone())
            } else if call.starts_with("git config --get-regexp") {
                Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
            } else if call.starts_with("tmux list-windows") {
                Ok(listed.clone())
            } else {
                Ok(String::new())
            }
        });
        ws.resume(IssueNumber(142), &launch()).expect("resume");
        let last = fake::calls().pop().expect("a call");
        assert_eq!(last, format!("tmux respawn-window -k -t @4 -c {} -- claude --continue", dir.display()));
    }

    #[test]
    fn attach_inside_switches_and_outside_suspends() {
        let base = temp_dir("attach");
        let mut ws = workspace(&base);
        let id = WindowId::parse("@4").expect("id");
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Switch(
                ["tmux", "select-window", "-t", "@4", ";", "switch-client", "-t", "$1"].map(String::from).to_vec()
            )
        );
        ws.mux = Mux::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Suspend(
                ["tmux", "select-window", "-t", "@4", ";", "attach-session", "-t", "=dispatch-orbit"]
                    .map(String::from)
                    .to_vec()
            )
        );
    }

    #[derive(Default)]
    #[expect(clippy::struct_excessive_bools, reason = "one independent flag per fake world fact")]
    struct HerdrSim {
        added: bool,
        marked: bool,
        branch_exists: bool,
        workspace_open: bool,
        agent: bool,
    }

    fn install_herdr_world(base: PathBuf, sim: Rc<RefCell<HerdrSim>>) {
        fake::install(move |call| {
            let mut s = sim.borrow_mut();
            let dir = base.join("orbit/.claude/worktrees/142");
            if call.starts_with("git worktree list") {
                let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
                if s.added {
                    t = format!("{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display());
                }
                Ok(t)
            } else if call.starts_with("git config --get-regexp") {
                if s.marked {
                    Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
                } else {
                    Err(fake::fail("git", 1, ""))
                }
            } else if call.starts_with("git show-ref") {
                if s.added || s.branch_exists { Ok(String::new()) } else { Err(fake::fail("git", 1, "")) }
            } else if call.starts_with("herdr workspace list") {
                let rows = if s.workspace_open {
                    serde_json::json!([{"workspace_id": "w5", "worktree": {"checkout_path": dir}}])
                } else {
                    serde_json::json!([])
                };
                Ok(serde_json::json!({"result": {"workspaces": rows}}).to_string())
            } else if call.starts_with("herdr pane list") {
                let agent = s.agent.then_some("claude");
                let rows = if s.workspace_open {
                    serde_json::json!([{"pane_id": "w5:p1", "workspace_id": "w5", "agent": agent, "agent_status": "idle"}])
                } else {
                    serde_json::json!([])
                };
                Ok(serde_json::json!({"result": {"panes": rows}}).to_string())
            } else if call.starts_with("herdr worktree create") {
                std::fs::create_dir_all(&dir).expect("mkdir");
                s.added = true;
                s.workspace_open = true;
                Ok("{}".to_string())
            } else if call.starts_with("herdr worktree open") {
                s.workspace_open = true;
                let opened =
                    serde_json::json!({"workspace": {"workspace_id": "w5"}, "root_pane": {"pane_id": "w5:p1"}});
                Ok(serde_json::json!({"result": opened}).to_string())
            } else if call.starts_with("herdr pane run") {
                s.agent = true;
                Ok(String::new())
            } else if call.starts_with("herdr pane get") {
                Ok(serde_json::json!({"result": {"pane": {"agent": s.agent.then_some("claude")}}}).to_string())
            } else if call.contains("dispatch-issue 142") {
                s.marked = true;
                Ok(String::new())
            } else {
                Ok(String::new())
            }
        });
    }

    fn herdr_workspace(base: &std::path::Path) -> Workspace {
        let mut ws = workspace(base);
        let place = herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside herdr");
        ws.mux = Mux::Herdr(place);
        ws
    }

    fn herdr_calls() -> Vec<String> {
        fake::calls()
            .into_iter()
            .filter(|c| {
                ["git fetch", "git worktree add", "git config branch", "herdr worktree", "herdr pane run"]
                    .iter()
                    .any(|p| c.starts_with(p))
            })
            .collect()
    }

    #[test]
    fn herdr_start_of_a_new_branch_creates_the_worktree_through_herdr_and_types_claude_into_its_pane() {
        let base = temp_dir("herdr-start");
        let ws = herdr_workspace(&base);
        install_herdr_world(base.clone(), Rc::new(RefCell::new(HerdrSim::default())));
        let out = ws.start(&req(), &launch()).expect("start");
        assert_eq!(out.session.state(), SessionState::NeedsYou, "herdr reports an idle agent as waiting");
        let dir = base.join("orbit/.claude/worktrees/142");
        assert_eq!(
            herdr_calls(),
            [
                "git fetch origin main".to_string(),
                format!(
                    "herdr worktree create --workspace w1 --branch fix/142-x --base origin/main --path {} --label #142 --no-focus",
                    dir.display()
                ),
                "git config branch.fix/142-x.dispatch-issue 142".to_string(),
                format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display()),
            ],
            "the workspace that create opened is reused, and git does not add the worktree"
        );
        let exclude = base.join("orbit/.git/info/exclude");
        assert_eq!(std::fs::read_to_string(&exclude).expect("exclude written"), ".claude/worktrees/\n");
        let before = fake::calls().len();
        ws.start(&req(), &launch()).expect("second start");
        assert!(
            !fake::calls()[before..].iter().any(|c| c.starts_with("herdr pane run")),
            "a second start must not type again"
        );
    }

    #[test]
    fn herdr_start_of_an_existing_branch_leaves_out_the_base() {
        let base = temp_dir("herdr-existing");
        let ws = herdr_workspace(&base);
        let sim = HerdrSim { branch_exists: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.start(&req(), &launch()).expect("start");
        let dir = base.join("orbit/.claude/worktrees/142");
        assert_eq!(
            herdr_calls()[0],
            format!(
                "herdr worktree create --workspace w1 --branch fix/142-x --path {} --label #142 --no-focus",
                dir.display()
            )
        );
    }

    #[test]
    fn herdr_start_writes_a_multi_line_prompt_into_the_launch_script() {
        let base = temp_dir("herdr-prompt");
        let ws = herdr_workspace(&base);
        install_herdr_world(base.clone(), Rc::new(RefCell::new(HerdrSim::default())));
        let mut r = req();
        r.prompt = "line one\nit's line two".to_string();
        ws.start(&r, &launch()).expect("start");
        let file = base.join("orbit/.git/dispatch/launch/142.sh");
        let script = std::fs::read_to_string(&file).expect("script");
        assert!(script.ends_with("'line one\nit'\\''s line two'\n"), "{script}");
        let run = herdr_calls().pop().expect("a call");
        assert_eq!(run, format!("herdr pane run w5:p1 '{}'", file.display()));
    }

    #[test]
    fn herdr_start_reopens_a_closed_workspace_for_an_existing_worktree() {
        let base = temp_dir("herdr-reopen");
        let ws = herdr_workspace(&base);
        let dir = base.join("orbit/.claude/worktrees/142");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let sim = HerdrSim { added: true, marked: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.start(&req(), &launch()).expect("start");
        assert_eq!(
            herdr_calls(),
            [
                format!("herdr worktree open --workspace w1 --path {} --label #142 --no-focus", dir.display()),
                format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display()),
            ]
        );
    }

    #[test]
    fn herdr_resume_runs_claude_continue_in_the_dead_pane_without_the_tmux_hooks() {
        let base = temp_dir("herdr-resume");
        let ws = herdr_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HerdrSim { added: true, marked: true, workspace_open: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        let mut l = launch();
        l.detect_waiting = true;
        ws.resume(IssueNumber(142), &l).expect("resume");
        assert_eq!(
            herdr_calls(),
            [format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display())]
        );
        let script = std::fs::read_to_string(base.join("orbit/.git/dispatch/launch/142.sh")).expect("script");
        assert_eq!(script, "#!/bin/sh\nexec 'claude' '--continue'\n");
    }

    #[test]
    fn herdr_stop_closes_the_workspace_and_keeps_the_checkout() {
        let base = temp_dir("herdr-stop");
        let ws = herdr_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HerdrSim { added: true, marked: true, workspace_open: true, agent: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.stop(IssueNumber(142)).expect("stop");
        let calls = fake::calls();
        assert!(calls.iter().any(|c| c == "herdr workspace close w5"));
        assert!(!calls.iter().any(|c| c.contains("worktree remove") || c.contains("--group")));
        assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
    }

    #[test]
    fn herdr_send_and_tail_go_to_the_agent_pane() {
        let base = temp_dir("herdr-send");
        let ws = herdr_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HerdrSim { added: true, marked: true, workspace_open: true, agent: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.send(IssueNumber(142), "merge it").expect("send");
        let calls = fake::calls();
        assert_eq!(
            calls[calls.len() - 2..],
            ["herdr pane send-text w5:p1 merge it", "herdr pane send-keys w5:p1 enter"]
        );
        let session = ws.observe().expect("observe").remove(&IssueNumber(142)).expect("session");
        let window = session.window.expect("window");
        ws.tail(&window.id, 20).expect("tail");
        assert_eq!(fake::calls().pop().expect("a call"), "herdr pane read w5:p1 --source recent-unwrapped --lines 20");
    }

    #[test]
    fn herdr_send_refuses_a_pane_without_an_agent() {
        let base = temp_dir("herdr-send-dead");
        let ws = herdr_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HerdrSim { added: true, marked: true, workspace_open: true, ..HerdrSim::default() };
        install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
        assert!(ws.send(IssueNumber(142), "hi").is_err());
    }

    #[test]
    fn herdr_attach_focuses_the_workspace() {
        let base = temp_dir("herdr-attach");
        let ws = herdr_workspace(&base);
        let id = WindowId::Herdr(herdr::PaneRef::new("w5", "w5:p1"));
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Switch(["herdr", "workspace", "focus", "w5"].map(String::from).to_vec())
        );
        assert!(ws.attach_plan(&WindowId::parse("@4").expect("id")).is_err(), "a tmux window is not ours");
    }

    #[test]
    fn a_missing_multiplexer_is_named_in_the_setup_error() {
        assert_eq!(SetupError::ProgramMissing("herdr").to_string(), "herdr is not installed or not on PATH");
        assert!(herdr::Place::detect(Some("1"), None, "orbit").is_err());
    }

    #[test]
    fn a_detached_herdr_attaches_by_focusing_the_workspace_then_opening_herdr() {
        let base = temp_dir("herdr-attach-detached");
        let mut ws = herdr_workspace(&base);
        ws.mux = Mux::Herdr(herdr::Place::detect(None, None, "orbit").expect("detached"));
        fake::install(|_| Ok(String::new()));
        let id = WindowId::Herdr(herdr::PaneRef::new("w5", "w5:p1"));
        let script = r#"herdr workspace focus "$1" >/dev/null && exec herdr"#;
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Suspend(["sh", "-c", script, "sh", "w5"].map(String::from).to_vec())
        );
        assert!(fake::calls().is_empty(), "planning runs nothing");
    }

    fn env(herdr: bool, tmux: bool) -> MuxEnv {
        MuxEnv {
            herdr_env: herdr.then(|| "1".to_string()),
            herdr_workspace: herdr.then(|| "w1".to_string()),
            tmux: tmux.then(|| "/tmp/tmux-1000/default,1,0".to_string()),
            hc_socket: None,
        }
    }

    fn in_app(env: MuxEnv) -> MuxEnv {
        MuxEnv { hc_socket: Some("/run/user/1000/herdr-core/default.sock".to_string()), ..env }
    }

    #[test]
    fn the_multiplexer_choice_picks_the_backend_from_the_environment() {
        use Multiplexer::{Auto, Herdr, Tmux};
        let inside_herdr = Mux::Herdr(herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside"));
        let detached_herdr = Mux::Herdr(herdr::Place::detect(None, None, "orbit").expect("detached"));
        let inside_tmux = Mux::Tmux(tmux::Place::Inside { session_id: "$1".to_string() });
        let detached_tmux = Mux::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
        let table = [
            (Auto, false, false, &detached_tmux),
            (Auto, false, true, &inside_tmux),
            (Auto, true, false, &inside_herdr),
            (Auto, true, true, &inside_herdr),
            (Tmux, false, false, &detached_tmux),
            (Tmux, false, true, &inside_tmux),
            (Tmux, true, false, &detached_tmux),
            (Tmux, true, true, &inside_tmux),
            (Herdr, false, false, &detached_herdr),
            (Herdr, false, true, &detached_herdr),
            (Herdr, true, false, &inside_herdr),
            (Herdr, true, true, &inside_herdr),
        ];
        for (choice, in_herdr, in_tmux, want) in table {
            fake::install(|call| {
                Ok(if call.starts_with("tmux display-message") { "$1\n".to_string() } else { String::new() })
            });
            let got = detect_mux(choice, &env(in_herdr, in_tmux), std::path::Path::new("/w/orbit")).expect("detects");
            assert_eq!(&got, want, "{choice:?} herdr={in_herdr} tmux={in_tmux}");
        }
    }

    #[test]
    fn the_herdr_app_is_chosen_by_setting_or_by_its_socket() {
        use Multiplexer::{Auto, Herdr, HerdrApp, Tmux};
        let tmux_detached = Mux::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
        let herdr_inside = Mux::Herdr(herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside"));
        let (app, outside) = (Mux::Hc(hc::Place::App), Mux::Hc(hc::Place::Outside));
        let table = [
            (Auto, in_app(env(false, false)), &app),
            (Auto, in_app(env(false, true)), &app),
            (Auto, in_app(env(true, false)), &herdr_inside),
            (Auto, env(false, false), &tmux_detached),
            (Tmux, in_app(env(false, false)), &tmux_detached),
            (Herdr, in_app(env(true, false)), &herdr_inside),
            (HerdrApp, env(false, false), &outside),
            (HerdrApp, env(true, true), &outside),
            (HerdrApp, in_app(env(false, false)), &app),
            (HerdrApp, in_app(env(true, true)), &app),
        ];
        for (choice, e, want) in table {
            fake::install(|_| Ok(String::new()));
            let got = detect_mux(choice, &e, std::path::Path::new("/w/orbit")).expect("detects");
            assert_eq!(&got, want, "{choice:?} {e:?}");
        }
        fake::install(|call| {
            if call.starts_with("hc") {
                Err(crate::proc::ProcError::Spawn {
                    program: "hc".to_string(),
                    source: std::io::Error::from(std::io::ErrorKind::NotFound),
                })
            } else {
                Ok(String::new())
            }
        });
        assert!(matches!(
            detect_mux(HerdrApp, &env(false, false), std::path::Path::new("/w/orbit")),
            Err(SetupError::ProgramMissing("hc"))
        ));
    }

    #[test]
    fn the_chosen_multiplexer_must_be_installed() {
        fake::install(|_| Err(fake::fail("x", 1, "")));
        let spawn = || crate::proc::ProcError::Spawn {
            program: "x".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        fake::install(move |call| if call.starts_with("herdr") { Err(spawn()) } else { Ok(String::new()) });
        let err = detect_mux(Multiplexer::Herdr, &env(false, true), std::path::Path::new("/w/orbit"));
        assert!(matches!(err, Err(SetupError::ProgramMissing("herdr"))));
        assert!(detect_mux(Multiplexer::Tmux, &env(false, false), std::path::Path::new("/w/orbit")).is_ok());
        assert!(matches!(
            detect_mux(Multiplexer::Auto, &env(true, false), std::path::Path::new("/w/orbit")),
            Err(SetupError::ProgramMissing("herdr"))
        ));
    }

    #[derive(Default)]
    struct HcSim {
        added: bool,
        marked: bool,
        terminals: Vec<(&'static str, &'static str, Option<&'static str>)>,
        gone: bool,
    }

    fn not_found() -> crate::proc::ProcError {
        fake::fail("hc", 3, r#"{"error":{"hint":"see `hc ls --all`","kind":"not_found","message":"no terminal"}}"#)
    }

    fn install_hc_world(base: PathBuf, sim: Rc<RefCell<HcSim>>) {
        fake::install(move |call| {
            let mut s = sim.borrow_mut();
            let dir = base.join("orbit/.claude/worktrees/142");
            if call.starts_with("git worktree list") {
                let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
                if s.added {
                    t = format!("{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display());
                }
                Ok(t)
            } else if call.starts_with("git config --get-regexp") {
                if s.marked {
                    Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
                } else {
                    Err(fake::fail("git", 1, ""))
                }
            } else if call.starts_with("git show-ref") {
                if s.added { Ok(String::new()) } else { Err(fake::fail("git", 1, "")) }
            } else if call.starts_with("git worktree add") {
                std::fs::create_dir_all(&dir).expect("mkdir");
                s.added = true;
                Ok(String::new())
            } else if call.starts_with("hc ls") {
                let rows: Vec<_> = s
                    .terminals
                    .iter()
                    .map(|(a, state, status)| serde_json::json!({"address": a, "cwd": dir, "state": state, "status": status}))
                    .collect();
                Ok(serde_json::json!({"hidden_finished": 0, "terminals": rows}).to_string())
            } else if call.starts_with("hc run") {
                s.terminals.push(("t9", "live", Some("idle")));
                Ok(r#"{"address":"t9","state":"live"}"#.to_string())
            } else if call.starts_with("hc kill") && s.terminals.iter().all(|(_, state, _)| *state == "finished") {
                Err(fake::fail("hc", 11, r#"{"error":{"kind":"finished"}}"#))
            } else if call.starts_with("hc rm") || call.starts_with("hc kill") {
                if s.gone { Err(not_found()) } else { Ok("{}".to_string()) }
            } else if call.starts_with("hc read") {
                Ok("one\ntwo\n\n\n".to_string())
            } else if call.contains("dispatch-issue 142") {
                s.marked = true;
                Ok(String::new())
            } else {
                Ok(String::new())
            }
        });
    }

    fn hc_workspace(base: &std::path::Path) -> Workspace {
        let mut ws = workspace(base);
        ws.mux = Mux::Hc(hc::Place::Outside);
        ws
    }

    fn hc_calls() -> Vec<String> {
        fake::calls()
            .into_iter()
            .filter(|c| {
                [
                    "git fetch",
                    "git worktree add",
                    "git config branch",
                    "hc run",
                    "hc rm",
                    "hc kill",
                    "hc send",
                    "hc read",
                ]
                .iter()
                .any(|p| c.starts_with(p))
            })
            .collect()
    }

    fn hc_run(base: &std::path::Path, argv: &str) -> String {
        let dir = base.join("orbit/.claude/worktrees/142");
        let space = hc::space_id(&dir);
        format!(
            "hc run --bg --json --no-parent --no-notify --cwd {} --label herdr.space={space} --label dispatch.repo={} --label dispatch.issue=142 -- {argv}",
            dir.display(),
            base.join("orbit").display()
        )
    }

    #[test]
    fn hc_start_adds_the_worktree_with_git_and_runs_claude_with_the_labels() {
        let base = temp_dir("hc-start");
        let ws = hc_workspace(&base);
        install_hc_world(base.clone(), Rc::new(RefCell::new(HcSim::default())));
        let mut r = req();
        r.prompt = "line one\nit's \"line\" two;".to_string();
        let out = ws.start(&r, &launch()).expect("start");
        assert_eq!(out.session.state(), SessionState::NeedsYou, "an idle status is waiting");
        let dir = base.join("orbit/.claude/worktrees/142");
        assert_eq!(
            hc_calls(),
            [
                "git fetch origin main".to_string(),
                format!("git worktree add {} -b fix/142-x origin/main", dir.display()),
                "git config branch.fix/142-x.dispatch-issue 142".to_string(),
                hc_run(&base, "claude line one\nit's \"line\" two;"),
            ]
        );
        let before = fake::calls().len();
        ws.start(&r, &launch()).expect("second start");
        assert!(
            !fake::calls()[before..].iter().any(|c| c.starts_with("hc run")),
            "a live session is not started twice"
        );
    }

    #[test]
    fn hc_start_passes_no_tmux_hooks_even_when_waiting_detection_is_on() {
        let base = temp_dir("hc-hooks");
        let ws = hc_workspace(&base);
        install_hc_world(base.clone(), Rc::new(RefCell::new(HcSim::default())));
        let mut l = launch();
        l.detect_waiting = true;
        ws.start(&req(), &l).expect("start");
        let run = hc_calls().pop().expect("a call");
        assert!(!run.contains("--settings"), "{run}");
    }

    #[test]
    fn hc_resume_of_a_dead_terminal_removes_it_then_runs_continue_in_the_same_space() {
        let base = temp_dir("hc-resume");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "finished", Some("done"))], gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.resume(IssueNumber(142), &launch()).expect("resume");
        assert_eq!(hc_calls(), ["hc rm t4".to_string(), hc_run(&base, "claude --continue")]);
    }

    #[test]
    fn hc_resume_without_a_terminal_runs_continue_and_removes_nothing() {
        let base = temp_dir("hc-resume-missing");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, ..HcSim::default() };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.resume(IssueNumber(142), &launch()).expect("resume");
        assert_eq!(hc_calls(), [hc_run(&base, "claude --continue")]);
    }

    #[test]
    fn hc_resume_of_a_live_terminal_does_nothing() {
        let base = temp_dir("hc-resume-live");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "live", Some("working"))], gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.resume(IssueNumber(142), &launch()).expect("resume");
        assert_eq!(hc_calls(), Vec::<String>::new());
    }

    #[test]
    fn hc_stop_kills_then_removes_and_keeps_the_worktree() {
        let base = temp_dir("hc-stop");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "live", Some("working"))], gone: false };
        let sim = Rc::new(RefCell::new(sim));
        install_hc_world(base.clone(), Rc::clone(&sim));
        ws.stop(IssueNumber(142)).expect("stop");
        assert_eq!(hc_calls(), ["hc kill t4", "hc rm t4"]);
        assert!(!fake::calls().iter().any(|c| c.contains("worktree remove")));
        sim.borrow_mut().gone = true;
        ws.stop(IssueNumber(142)).expect("a terminal that vanished is already stopped");
        assert_eq!(hc_calls()[2..], ["hc kill t4"], "not found ends the stop");
        assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
    }

    #[test]
    fn hc_stop_of_a_finished_terminal_still_removes_it() {
        let base = temp_dir("hc-stop-finished");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "finished", None)], gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.stop(IssueNumber(142)).expect("stop");
        assert_eq!(hc_calls(), ["hc kill t4", "hc rm t4"]);
    }

    #[test]
    fn hc_observe_prefers_a_live_terminal_over_a_newer_finished_one() {
        let base = temp_dir("hc-observe");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let terminals = vec![("t2", "live", Some("working")), ("t3", "finished", Some("done"))];
        let sim = HcSim { added: true, marked: true, terminals, gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        let session = ws.observe().expect("observe").remove(&IssueNumber(142)).expect("session");
        let space = hc::space_id(&base.join("orbit/.claude/worktrees/142"));
        assert_eq!(
            session.window.as_ref().expect("window").id,
            WindowId::Hc(hc::Pane::new("t2", space.as_str())),
            "the pane carries the space id derived from the worktree"
        );
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(
            fake::calls().iter().find(|c| c.starts_with("hc ls")).map(String::as_str),
            Some(format!("hc ls --all --json --label dispatch.repo={}", base.join("orbit").display()).as_str())
        );
    }

    #[test]
    fn hc_send_and_tail_go_to_the_terminal() {
        let base = temp_dir("hc-send");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "live", Some("idle"))], gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        ws.send(IssueNumber(142), "merge it").expect("send");
        assert_eq!(hc_calls(), ["hc send --force t4 merge it"]);
        let tail = ws.tail(&WindowId::Hc(hc::Pane::new("t4", "sp4")), 20).expect("tail");
        assert_eq!(tail, ["one", "two"], "trailing blank lines are gone");
        assert_eq!(hc_calls()[1], "hc read t4 --lines 20");
    }

    #[test]
    fn hc_send_refuses_a_finished_terminal() {
        let base = temp_dir("hc-send-dead");
        let ws = hc_workspace(&base);
        std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
        let sim = HcSim { added: true, marked: true, terminals: vec![("t4", "finished", None)], gone: false };
        install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
        assert!(ws.send(IssueNumber(142), "hi").is_err());
    }

    #[test]
    fn hc_attach_suspends_outside_the_app_and_switches_space_inside_it() {
        let base = temp_dir("hc-attach");
        let mut ws = hc_workspace(&base);
        let id = WindowId::Hc(hc::Pane::new("t4", "sp4"));
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Suspend(["hc", "attach", "t4"].map(String::from).to_vec())
        );
        ws.mux = Mux::Hc(hc::Place::App);
        assert_eq!(
            ws.attach_plan(&id).expect("plan"),
            AttachPlan::Switch(["herdr-app", "do", "space.select", "--id", "sp4"].map(String::from).to_vec())
        );
        assert!(ws.attach_plan(&WindowId::parse("@4").expect("id")).is_err(), "a tmux window is not ours");
    }
}
