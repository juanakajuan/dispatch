use super::error::{MuxError, SetupError};
use super::request::Launch;
use super::{AttachPlan, git, hc, herdr, tmux};
use crate::config::Multiplexer;
use crate::domain::{BranchName, IssueNumber, OriginRef, WorktreePath};
use std::path::Path;

/// What a multiplexer does for dispatch. One impl per backend, on its
/// `Place`. `Window` is the backend's own handle; it never leaves `session`.
pub(super) trait Mux {
    type Window: Clone + Eq + std::fmt::Debug;

    /// Every window dispatch opened, dead ones too. No server is an empty list.
    fn list(&self, repo: Repo<'_>) -> Result<Vec<Observed<Self::Window>>, MuxError>;

    fn add_worktree(
        &self,
        repo: Repo<'_>,
        dir: &Path,
        branch: &BranchName,
        from: Option<&OriginRef>,
        _issue: IssueNumber,
    ) -> Result<(), MuxError> {
        git::add_worktree(repo.root, dir, branch, from).map_err(MuxError::Run)
    }

    /// Opens a window on `dir` and runs `argv` in it.
    fn open(&self, repo: Repo<'_>, dir: &WorktreePath, issue: IssueNumber, argv: &[String]) -> Result<(), MuxError>;

    /// Runs `argv` for a session whose window is dead.
    fn relaunch(
        &self,
        repo: Repo<'_>,
        window: &Self::Window,
        dir: &WorktreePath,
        issue: IssueNumber,
        argv: &[String],
    ) -> Result<(), MuxError>;

    /// A window that is already gone counts as success.
    fn stop(&self, window: &Self::Window) -> Result<(), MuxError>;

    fn send_line(&self, window: &Self::Window, text: &str) -> Result<(), MuxError>;

    fn attach(&self, window: &Self::Window) -> AttachPlan;

    /// Opens a plain shell in `dir` that observe never lists as the session,
    /// and returns how to show it. `window` is the session's own window, when
    /// it has one.
    fn shell(
        &self,
        repo: Repo<'_>,
        dir: &WorktreePath,
        issue: IssueNumber,
        window: Option<&Self::Window>,
    ) -> Result<AttachPlan, MuxError>;

    /// The Claude hooks that set the waiting flag, for a backend that cannot
    /// read the agent status itself.
    fn waiting_hooks(&self) -> Option<&'static str> {
        None
    }
}

#[derive(Clone, Copy)]
pub(super) struct Repo<'a> {
    pub(super) root: &'a Path,
    /// The git dir that all worktrees share.
    pub(super) git_common_dir: &'a Path,
}

pub(super) struct Observed<W> {
    pub(super) window: W,
    pub(super) worktree: WorktreePath,
    pub(super) liveness: Liveness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Liveness {
    /// Claude runs and waits for the user. Never reported by tmux when
    /// `Config::detect_waiting` is off.
    Waiting,
    Working,
    /// Claude exited; the window (tmux `remain-on-exit`) is still there, so
    /// the last screen stays readable.
    Dead,
}

impl Liveness {
    pub(super) fn is_live(self) -> bool {
        self != Liveness::Dead
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Backend {
    Tmux(tmux::Place),
    Herdr(herdr::Place),
    /// The herdr app. hc finds its core itself.
    Hc(hc::Hc),
}

/// Runs `$body` with `$mux` bound to the backend's `Mux` impl.
macro_rules! each_backend {
    ($backend:expr, $mux:ident => $body:expr) => {
        match $backend {
            $crate::session::mux::Backend::Tmux($mux) => $body,
            $crate::session::mux::Backend::Herdr($mux) => $body,
            $crate::session::mux::Backend::Hc($mux) => $body,
        }
    };
}
pub(super) use each_backend;

impl Backend {
    pub(super) fn hooks(&self, launch: &Launch) -> Option<&'static str> {
        each_backend!(self, mux => hooks(mux, launch))
    }
}

pub(super) fn hooks<M: Mux>(mux: &M, launch: &Launch) -> Option<&'static str> {
    (launch.detect_waiting && launch.harness.takes_waiting_hooks()).then(|| mux.waiting_hooks()).flatten()
}

/// Claude hooks passed with `--settings` when `detect_waiting` is on.
/// `$TMUX_PANE` is in every pane's environment, and hooks inherit it.
/// Notification and Stop set the flag; `UserPromptSubmit` and `PreToolUse` clear it.
/// tmux sets the flag when it opens a window with these hooks, and
/// `SessionStart` on a new start clears it. No hook runs while Claude asks
/// to trust the folder, so the session shows `needs you` until Claude starts.
/// `--continue` is a `resume` start: the flag stays until you type.
pub(super) const WAITING_HOOKS: &str = r#"{"hooks":{
  "SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}],
  "Notification":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "Stop":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" @dispatch-waiting 1"}]}],
  "UserPromptSubmit":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}],
  "PreToolUse":[{"hooks":[{"type":"command","command":"tmux set-option -w -t \"$TMUX_PANE\" -u @dispatch-waiting"}]}]
}}"#;

fn require(program: &'static str, version_flag: &str, cwd: &std::path::Path) -> Result<(), SetupError> {
    crate::proc::run(program, [version_flag], cwd).map(|_| ()).map_err(|e| match e {
        crate::proc::ProcError::Spawn { .. } => SetupError::ProgramMissing(program),
        other => SetupError::Mux(MuxError::Run(other)),
    })
}

#[derive(Clone, Debug, Default)]
pub(super) struct MuxEnv {
    pub(super) herdr_env: Option<String>,
    pub(super) herdr_workspace: Option<String>,
    pub(super) tmux: Option<String>,
    pub(super) hc_socket: Option<String>,
    pub(super) path: Option<String>,
}

impl MuxEnv {
    #[expect(
        clippy::disallowed_methods,
        reason = "boundary: the multiplexer variables are read once here; `detect_mux` takes them as a `MuxEnv`"
    )]
    pub(super) fn from_process() -> MuxEnv {
        let var = |name| std::env::var(name).ok();
        MuxEnv {
            herdr_env: var("HERDR_ENV"),
            herdr_workspace: var("HERDR_WORKSPACE_ID"),
            tmux: var("TMUX"),
            hc_socket: var("HC_TERMINAL_SOCKET"),
            path: var("PATH"),
        }
    }
}

pub(super) fn detect_mux(
    choice: Multiplexer,
    env: &MuxEnv,
    repo_root: &std::path::Path,
) -> Result<Backend, SetupError> {
    let inside_herdr = env.herdr_env.as_deref() == Some("1");
    let dir_name = repo_root.file_name().map_or_else(|| "repo".to_string(), |n| n.to_string_lossy().into_owned());
    let inside_app = env.hc_socket.as_deref().is_some_and(|s| !s.is_empty());
    if choice == Multiplexer::HerdrApp || (choice == Multiplexer::Auto && !inside_herdr && inside_app) {
        require("hc", "--version", repo_root)?;
        let place = if inside_app { hc::Place::App } else { hc::Place::Outside };
        return Ok(Backend::Hc(hc::Hc { place, path: env.path.clone() }));
    }
    if choice == Multiplexer::Herdr || (choice == Multiplexer::Auto && inside_herdr) {
        require("herdr", "--version", repo_root)?;
        let place = herdr::Place::detect(env.herdr_env.as_deref(), env.herdr_workspace.as_deref(), &dir_name)
            .map_err(SetupError::Mux)?;
        return Ok(Backend::Herdr(place));
    }
    require("tmux", "-V", repo_root)?;
    let place = tmux::Place::detect(env.tmux.as_deref(), &dir_name).map_err(SetupError::Mux)?;
    Ok(Backend::Tmux(place))
}
