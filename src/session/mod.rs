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

use crate::config::{ClaudeCommand, Config};
use crate::domain::{BranchName, IssueNumber, Mode, SkillChoice, WorktreePath};
use crate::rules;
use std::collections::BTreeMap;
use std::fmt;
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
    pub skill: SkillChoice,
    pub mode: Mode,
    pub prompt: String,
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
    claude: ClaudeCommand,
}

#[derive(Debug)]
pub enum SetupError {
    NotARepo(String),
    NoOriginHead(String),
    TmuxMissing,
    Command(String),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetupError::NotARepo(m) => write!(f, "dispatch must run inside a git repository: {m}"),
            SetupError::NoOriginHead(m) => write!(f, "cannot find the default branch of origin: {m}"),
            SetupError::TmuxMissing => f.write_str("tmux is not installed or not on PATH"),
            SetupError::Command(m) => f.write_str(m),
        }
    }
}

impl Workspace {
    /// Fails before the TUI starts, so the user sees a plain error line.
    pub fn discover(cwd: &std::path::Path, config: &Config) -> Result<Workspace, SetupError> {
        let repo_root = git::repo_root(cwd).map_err(SetupError::NotARepo)?;
        let base = git::origin_head(&repo_root).map_err(SetupError::NoOriginHead)?;
        crate::proc::run("tmux", ["-V"], &repo_root).map_err(|e| match e {
            crate::proc::ProcError::Spawn { .. } => SetupError::TmuxMissing,
            other => SetupError::Command(other.to_string()),
        })?;
        let dir_name = repo_root.file_name().map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
        let place = tmux::Place::detect(std::env::var("TMUX").ok().as_deref(), &dir_name).map_err(SetupError::Command)?;
        Ok(Workspace {
            repo_root,
            base,
            place,
            detect_waiting: config.detect_waiting,
            claude: config.claude_command.clone(),
        })
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    /// Three subprocesses (worktree list, config regexp, tmux list-windows),
    /// then the pure `join`. A missing tmux server is an empty window list.
    pub fn observe(&self) -> Result<Sessions, SessionError> {
        let (worktrees, markers, windows) = self.read_world()?;
        Ok(join(&worktrees, &markers, &windows))
    }

    fn read_world(&self) -> Result<(Vec<git::Worktree>, git::Markers, Vec<tmux::WindowObs>), String> {
        Ok((git::list_worktrees(&self.repo_root)?, git::list_markers(&self.repo_root)?, tmux::list_windows()?))
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

    fn layout(&self) -> Layout {
        Layout {
            repo_root: self.repo_root.clone(),
            base: self.base.clone(),
            hooks: self.detect_waiting.then_some(WAITING_HOOKS),
            claude: self.claude.clone(),
        }
    }

    /// Converges to "a live window runs Claude in this issue's worktree".
    /// Safe to call twice, and safe to call again after a crash at any step:
    /// it re-observes, plans only the missing steps, and runs them.
    pub fn start(&self, req: &StartRequest) -> Result<StartOutcome, SessionError> {
        let obs = self.observe_for_start(req)?;
        match plan_start(&obs, req, &self.layout()).map_err(SessionError::Conflict)? {
            StartPlan::AlreadyRunning(session) => Ok(StartOutcome { session, warnings: Vec::new() }),
            StartPlan::Steps(steps) => {
                let warnings = self.run_steps(steps)?;
                let session = self
                    .observe()?
                    .remove(&req.issue)
                    .ok_or_else(|| SessionError::Command(format!("session for #{} is not visible after start", req.issue.0)))?;
                Ok(StartOutcome { session, warnings })
            }
        }
    }

    /// The step runner. A failed `Fetch` is a warning: the start continues
    /// from the local base. Any other failure stops the run, and the next
    /// `start` plans from whatever state exists.
    fn run_steps(&self, steps: Vec<Step>) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        for step in steps {
            match step {
                Step::Fetch => {
                    if let Err(e) = git::fetch(&self.repo_root, &self.base) {
                        warnings.push(format!("fetch failed, using the local {}: {e}", self.base));
                    }
                }
                Step::AddWorktree { dir, branch, create } => {
                    git::add_worktree(&self.repo_root, &dir, &branch, create.then_some(self.base.as_str()))?;
                }
                Step::Mark { branch, issue, skill } => git::set_marker(&self.repo_root, &branch, issue, &skill)?,
                Step::OpenWindow { dir, name, argv } => {
                    tmux::open_window(&self.place, &dir, &name, &argv)?;
                }
            }
        }
        Ok(warnings)
    }

    /// Runs `claude --continue` for an exited session: `respawn-window -k` when
    /// the dead pane is still there (same window id), else a new tagged window
    /// as in `start`. No-op if the window is alive. It observes again first, so
    /// a stale row can not aim at a window id that tmux reused.
    pub fn resume(&self, issue: IssueNumber) -> Result<(), SessionError> {
        let Some(session) = self.observe()?.remove(&issue) else {
            return Err(SessionError::Command(format!("no session for #{}", issue.0)));
        };
        let hooks = self.detect_waiting.then_some(WAITING_HOOKS);
        let argv = self.claude.argv(&rules::continue_args(hooks));
        match &session.window {
            Some(w) if w.alive => Ok(()),
            Some(w) => Ok(tmux::respawn(&w.id, &session.worktree, &argv)?),
            None => {
                tmux::open_window(&self.place, &session.worktree, &format!("#{}", issue.0), &argv)?;
                Ok(())
            }
        }
    }

    /// Kills the window. The worktree and branch stay. A window that is
    /// already gone counts as success.
    pub fn stop(&self, issue: IssueNumber) -> Result<(), SessionError> {
        match self.observe()?.remove(&issue).and_then(|s| s.window) {
            Some(w) => Ok(tmux::kill_window(&w.id)?),
            None => Ok(()),
        }
    }

    /// Last `lines` lines of the pane as plain text (`capture-pane -p -J`, no
    /// `-e`, so no escape codes reach the UI). Works on dead panes too.
    pub fn tail(&self, window: &WindowId, lines: u16) -> Result<Vec<String>, SessionError> {
        Ok(tmux::capture(window, lines)?)
    }

    pub fn attach_plan(&self, window: &WindowId) -> AttachPlan {
        let id = window.as_str().to_string();
        let session = self.place.session_target();
        let select = ["tmux", "select-window", "-t", id.as_str()].map(String::from);
        match &self.place {
            tmux::Place::Inside { .. } => AttachPlan::Switch(
                select.into_iter().chain([";", "switch-client", "-t", session.as_str()].map(String::from)).collect(),
            ),
            tmux::Place::Detached { .. } => AttachPlan::Suspend(
                select.into_iter().chain([";", "attach-session", "-t", session.as_str()].map(String::from)).collect(),
            ),
        }
    }
}

// ---- pure core -------------------------------------------------------------

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree is a session only if its branch has a valid `dispatch-issue`.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one issue: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
/// - Two windows tagged for one worktree: a live one wins.
fn join(worktrees: &[git::Worktree], markers: &git::Markers, windows: &[tmux::WindowObs]) -> Sessions {
    let mut by_path: Vec<&git::Worktree> = worktrees.iter().collect();
    by_path.sort_by(|a, b| a.path.cmp(&b.path));
    let mut sessions = Sessions::new();
    for w in by_path {
        let Some(branch) = &w.branch else { continue };
        let Some(marker) = markers.get(branch) else { continue };
        let window = windows
            .iter()
            .filter(|o| o.worktree == w.path)
            .min_by_key(|o| o.dead)
            .map(|o| Window { id: o.id.clone(), alive: !o.dead, waiting: o.waiting });
        sessions.entry(marker.issue).or_insert_with(|| Session {
            issue: marker.issue,
            branch: branch.clone(),
            worktree: w.path.clone(),
            skill: marker.skill.clone(),
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
    /// `git fetch origin <default>` so the new branch starts from a fresh base.
    Fetch,
    /// `git worktree add <dir> -b <branch> <base>` when the branch is new,
    /// `git worktree add <dir> <branch>` when it exists (crash recovery).
    AddWorktree { dir: PathBuf, branch: BranchName, create: bool },
    /// `git config branch.<b>.dispatch-issue N` and `dispatch-skill S`.
    /// Overwrites, so running it twice is harmless.
    Mark { branch: BranchName, issue: IssueNumber, skill: SkillChoice },
    /// Two tmux calls: new-window (or new-session when the detached session
    /// is missing) with argv `claude ...`, then set-option for
    /// `@dispatch-worktree` and `remain-on-exit on` on that window id.
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
    let target = rules::worktree_dir_for(&layout.repo_root, req.issue);
    let open = |dir: WorktreePath| Step::OpenWindow {
        dir,
        name: format!("#{}", req.issue.0),
        argv: layout.claude.argv(&rules::claude_args(req.mode, &req.skill, &req.prompt, layout.hooks)),
    };
    let mark = |branch: BranchName| Step::Mark { branch, issue: req.issue, skill: req.skill.clone() };
    let add = |branch: BranchName, create: bool| Step::AddWorktree { dir: target.clone(), branch, create };
    let planned = WorktreePath::planned(target.clone());

    if let Some(s) = obs.sessions.get(&req.issue) {
        return Ok(match &s.window {
            Some(w) if w.alive => StartPlan::AlreadyRunning(s.clone()),
            _ => StartPlan::Steps(vec![open(s.worktree.clone())]),
        });
    }
    if let Some(w) = obs.worktrees.iter().find(|w| w.path.as_path() == target) {
        let Some(branch) = &w.branch else { return Err(StartConflict::PathTaken(target)) };
        if let Some(m) = obs.markers.get(branch) {
            return Err(StartConflict::BranchBelongsTo { branch: branch.clone(), issue: m.issue });
        }
        return Ok(StartPlan::Steps(vec![mark(branch.clone()), open(w.path.clone())]));
    }
    if obs.target_dir_exists {
        return Err(StartConflict::PathTaken(target));
    }
    if let Some((branch, _)) = obs.markers.iter().find(|(_, m)| m.issue == req.issue) {
        return Ok(StartPlan::Steps(vec![add(branch.clone(), false), mark(branch.clone()), open(planned)]));
    }
    if let Some(m) = obs.markers.get(&req.branch) {
        return Err(StartConflict::BranchBelongsTo { branch: req.branch.clone(), issue: m.issue });
    }
    if let Some(w) = obs.worktrees.iter().find(|w| w.branch.as_ref() == Some(&req.branch)) {
        return Err(StartConflict::BranchCheckedOut { branch: req.branch.clone(), path: w.path.as_path().to_path_buf() });
    }
    let create = !obs.branch_exists;
    let mut steps = Vec::new();
    if create {
        steps.push(Step::Fetch);
    }
    steps.extend([add(req.branch.clone(), create), mark(req.branch.clone()), open(planned)]);
    Ok(StartPlan::Steps(steps))
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
    use super::*;
    use crate::domain::SkillName;
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

    fn skill(s: &str) -> SkillName {
        SkillName::parse(s).expect("valid skill")
    }

    fn wt(path: &str, branch: Option<&str>) -> git::Worktree {
        git::Worktree { path: wp(path), branch: branch.map(br) }
    }

    fn markers(rows: &[(&str, u32, Option<&str>)]) -> git::Markers {
        rows.iter()
            .map(|(b, n, s)| (br(b), git::Marker { issue: IssueNumber(*n), skill: s.map(skill) }))
            .collect()
    }

    fn win(id: &str, path: &str, dead: bool, waiting: bool) -> tmux::WindowObs {
        tmux::WindowObs { id: WindowId::parse(id).expect("window id"), worktree: wp(path), dead, waiting }
    }

    // ---- join ----

    #[test]
    fn join_live_window_is_running_and_waiting_flag_needs_you() {
        let worktrees = [wt("/a/orbit", Some("main")), wt("/a/orbit-1", Some("fix/1-x"))];
        let m = markers(&[("fix/1-x", 1, Some("pstack:tdd"))]);
        let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[&IssueNumber(1)].state(), SessionState::Running);
        assert_eq!(s[&IssueNumber(1)].skill, Some(skill("pstack:tdd")));
        let s = join(&worktrees, &m, &[win("@1", "/a/orbit-1", false, true)]);
        assert_eq!(s[&IssueNumber(1)].state(), SessionState::NeedsYou);
    }

    #[test]
    fn join_dead_pane_and_missing_window_are_exited() {
        let worktrees = [wt("/a/orbit-1", Some("fix/1-x"))];
        let m = markers(&[("fix/1-x", 1, None)]);
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
        let s = join(&worktrees, &markers(&[("other", 9, None)]), &[win("@5", "/a/gone", false, false)]);
        assert!(s.is_empty());
    }

    #[test]
    fn join_one_issue_marked_twice_keeps_first_path_and_prefers_live_window() {
        let worktrees = [wt("/a/z", Some("b2")), wt("/a/a", Some("b1"))];
        let m = markers(&[("b1", 7, None), ("b2", 7, None)]);
        let s = join(&worktrees, &m, &[win("@1", "/a/a", true, false), win("@2", "/a/a", false, false)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[&IssueNumber(7)].branch, br("b1"));
        assert_eq!(s[&IssueNumber(7)].window.as_ref().map(|w| w.id.as_str()), Some("@2"));
    }

    // ---- plan_start, one test per table row ----

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
            skill: Some(skill("pstack:tdd")),
            mode: Mode::Implement,
            prompt: "p".to_string(),
        }
    }

    struct World {
        worktrees: Vec<git::Worktree>,
        markers: git::Markers,
        windows: Vec<tmux::WindowObs>,
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
                Step::Fetch => {}
                Step::AddWorktree { dir, branch, .. } => {
                    self.worktrees.push(git::Worktree { path: WorktreePath::planned(dir.clone()), branch: Some(branch.clone()) });
                    self.branch_exists = true;
                    self.target_dir_exists = true;
                }
                Step::Mark { branch, issue, skill } => {
                    self.markers.insert(branch.clone(), git::Marker { issue: *issue, skill: skill.clone() });
                }
                Step::OpenWindow { dir, .. } => {
                    self.windows.push(tmux::WindowObs {
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
        let argv = ["claude".to_string(), "/pstack:tdd p".to_string()].to_vec();
        Step::OpenWindow { dir: wp("/a/orbit-142"), name: "#142".to_string(), argv }
    }

    fn add(branch: &str, create: bool) -> Step {
        Step::AddWorktree { dir: PathBuf::from("/a/orbit-142"), branch: br(branch), create }
    }

    fn mark(branch: &str) -> Step {
        Step::Mark { branch: br(branch), issue: IssueNumber(142), skill: Some(skill("pstack:tdd")) }
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
        w.worktrees.push(wt("/a/orbit-142", Some("fix/142-x")));
        w.markers = markers(&[("fix/142-x", 142, None)]);
        w.windows.push(win("@3", "/a/orbit-142", false, false));
        assert!(matches!(w.plan(), Ok(StartPlan::AlreadyRunning(s)) if s.issue == IssueNumber(142)));
    }

    #[test]
    fn plan_session_without_live_window_opens_one() {
        let mut w = World::empty();
        w.worktrees.push(wt("/a/orbit-142", Some("fix/142-x")));
        w.markers = markers(&[("fix/142-x", 142, None)]);
        assert_eq!(steps(w.plan()), [open_step()]);
        w.windows.push(win("@3", "/a/orbit-142", true, false));
        assert_eq!(steps(w.plan()), [open_step()]);
    }

    #[test]
    fn plan_marked_branch_without_worktree_uses_that_branch() {
        let mut w = World::empty();
        w.markers = markers(&[("fix/142-earlier-name", 142, None)]);
        w.branch_exists = false;
        assert_eq!(steps(w.plan()), [add("fix/142-earlier-name", false), mark("fix/142-earlier-name"), open_step()]);
    }

    #[test]
    fn plan_worktree_at_target_without_marker_marks_its_branch() {
        let mut w = World::empty();
        w.worktrees.push(wt("/a/orbit-142", Some("fix/142-x")));
        w.target_dir_exists = true;
        assert_eq!(steps(w.plan()), [mark("fix/142-x"), open_step()]);
    }

    #[test]
    fn plan_target_dir_that_is_no_worktree_conflicts() {
        let mut w = World::empty();
        w.target_dir_exists = true;
        assert_eq!(w.plan(), Err(StartConflict::PathTaken(PathBuf::from("/a/orbit-142"))));
    }

    #[test]
    fn plan_branch_marked_for_another_issue_conflicts() {
        let mut w = World::empty();
        w.markers = markers(&[("fix/142-x", 7, None)]);
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
        assert_eq!(steps(w.plan()), [add("fix/142-x", false), mark("fix/142-x"), open_step()]);
    }

    #[test]
    fn plan_from_nothing_runs_all_four_steps() {
        assert_eq!(
            steps(World::empty().plan()),
            [Step::Fetch, add("fix/142-x", true), mark("fix/142-x"), open_step()]
        );
    }

    #[test]
    fn plan_uses_config_claude_command_hooks_and_plan_mode() {
        let mut l = layout();
        l.claude = ClaudeCommand { program: "sh".to_string(), args: vec!["-c".to_string(), "echo".to_string(), "_".to_string()] };
        l.hooks = Some("{}");
        let mut r = req();
        r.mode = Mode::PlanFirst;
        r.skill = None;
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
            marked.markers = markers(&[("fix/142-old", 142, None)]);
            let mut half = World::empty();
            half.worktrees.push(wt("/a/orbit-142", Some("fix/142-x")));
            half.target_dir_exists = true;
            vec![fresh, existing, marked, half]
        };
        for mut world in starts {
            let original = steps(world.plan());
            let without_fetch: Vec<&Step> = original.iter().filter(|s| **s != Step::Fetch).collect();
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
            let dir = base.join("orbit-142");
            if call.starts_with("git worktree list") {
                let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
                if s.added {
                    t.push_str(&format!("worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display()));
                }
                Ok(t)
            } else if call.starts_with("git config --get-regexp") {
                if s.marked { Ok("branch.fix/142-x.dispatch-issue 142\n".to_string()) } else { Err(fake::fail("git", 1, "")) }
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
            base: "origin/main".to_string(),
            place: tmux::Place::Inside { session_id: "$1".to_string() },
            detect_waiting: false,
            claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() },
        }
    }

    fn mutating(calls: &[String]) -> Vec<&str> {
        calls
            .iter()
            .map(String::as_str)
            .filter(|c| {
                ["git fetch", "git worktree add", "git config branch", "git config --unset", "tmux new-", "tmux set-option"]
                    .iter()
                    .any(|p| c.starts_with(p))
            })
            .collect()
    }

    #[test]
    fn start_runs_the_steps_in_order_and_a_second_start_runs_none() {
        let base = temp_dir("start");
        let ws = workspace(&base);
        let mut r = req();
        r.skill = None;
        install_world(base.clone(), Rc::new(RefCell::new(Sim::default())));
        let out = ws.start(&r).expect("start");
        assert_eq!(out.session.state(), SessionState::Running);
        assert!(out.warnings.is_empty());
        let dir = base.join("orbit-142");
        let calls = fake::calls();
        let ran = mutating(&calls);
        assert_eq!(ran.len(), 6, "{ran:?}");
        assert_eq!(ran[0], "git fetch origin main");
        assert_eq!(ran[1], format!("git worktree add {} -b fix/142-x origin/main", dir.display()));
        assert_eq!(ran[2], "git config branch.fix/142-x.dispatch-issue 142");
        assert_eq!(ran[3], "git config --unset branch.fix/142-x.dispatch-skill");
        assert!(ran[4].starts_with(&format!("tmux new-window -d -t $1: -P -F #{{window_id}} -c {} -n #142 -- claude p", dir.display())));
        assert!(ran[5].starts_with("tmux set-option -w -t @4 @dispatch-worktree"));

        let before = fake::calls().len();
        let again = ws.start(&r).expect("second start");
        assert_eq!(again.session, out.session);
        assert!(mutating(&fake::calls()[before..]).is_empty(), "second start must not mutate");
    }

    #[test]
    fn start_after_a_crash_between_steps_runs_only_the_rest() {
        let base = temp_dir("crash");
        let ws = workspace(&base);
        std::fs::create_dir_all(base.join("orbit-142")).expect("mkdir");
        let sim = Sim { added: true, ..Sim::default() };
        install_world(base.clone(), Rc::new(RefCell::new(sim)));
        let out = ws.start(&req()).expect("start");
        assert_eq!(out.session.branch, br("fix/142-x"));
        let calls = fake::calls();
        let ran = mutating(&calls);
        assert_eq!(ran.len(), 4, "{ran:?}");
        assert!(ran[0].starts_with("git config branch.fix/142-x.dispatch-issue"));
        assert!(ran.iter().all(|c| !c.starts_with("git worktree add") && !c.starts_with("git fetch")));
    }

    #[test]
    fn failed_fetch_is_a_warning_and_the_start_continues() {
        let base = temp_dir("fetch");
        let ws = workspace(&base);
        install_world(base.clone(), Rc::new(RefCell::new(Sim { fetch_fails: true, ..Sim::default() })));
        let out = ws.start(&req()).expect("start continues");
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
        let err = ws.start(&req()).expect_err("fails");
        assert!(err.to_string().contains("fatal: boom"), "{err}");
        assert!(!fake::calls().iter().any(|c| c.starts_with("tmux new-")));
    }

    #[test]
    fn stop_kills_the_window_by_a_fresh_observation_and_keeps_the_worktree() {
        let base = temp_dir("stop");
        let ws = workspace(&base);
        install_world(base.clone(), Rc::new(RefCell::new(Sim { added: true, marked: true, window: true, tagged: true, ..Sim::default() })));
        std::fs::create_dir_all(base.join("orbit-142")).expect("mkdir");
        ws.stop(IssueNumber(142)).expect("stop");
        let calls = fake::calls();
        assert!(calls.iter().any(|c| c == "tmux kill-window -t @4"));
        assert!(!calls.iter().any(|c| c.contains("worktree remove") || c.contains("branch -")));
        assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
    }

    #[test]
    fn resume_respawns_a_dead_pane_with_continue() {
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
        ws.resume(IssueNumber(142)).expect("resume");
        let last = fake::calls().pop().expect("a call");
        assert_eq!(last, format!("tmux respawn-window -k -t @4 -c {} -- claude --continue", dir.display()));
    }

    #[test]
    fn attach_inside_switches_and_outside_suspends() {
        let base = temp_dir("attach");
        let mut ws = workspace(&base);
        let id = WindowId::parse("@4").expect("id");
        assert_eq!(
            ws.attach_plan(&id),
            AttachPlan::Switch(["tmux", "select-window", "-t", "@4", ";", "switch-client", "-t", "$1"].map(String::from).to_vec())
        );
        ws.place = tmux::Place::Detached { session_name: "dispatch-orbit".to_string() };
        assert_eq!(
            ws.attach_plan(&id),
            AttachPlan::Suspend(
                ["tmux", "select-window", "-t", "@4", ";", "attach-session", "-t", "=dispatch-orbit"].map(String::from).to_vec()
            )
        );
    }
}
