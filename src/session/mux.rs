use super::error::SetupError;
use super::request::Launch;
use super::{hc, herdr, tmux};
use crate::config::Multiplexer;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Mux {
    Tmux(tmux::Place),
    Herdr(herdr::Place),
    /// The herdr app. hc finds its core itself, so the only fact is whether
    /// dispatch runs inside the app.
    Hc(hc::Place),
}

impl Mux {
    pub(super) fn hooks(&self, launch: &Launch) -> Option<&'static str> {
        (launch.detect_waiting && matches!(self, Mux::Tmux(_))).then_some(WAITING_HOOKS)
    }
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

fn require(program: &'static str, version_flag: &str, cwd: &std::path::Path) -> Result<(), SetupError> {
    crate::proc::run(program, [version_flag], cwd).map(|_| ()).map_err(|e| match e {
        crate::proc::ProcError::Spawn { .. } => SetupError::ProgramMissing(program),
        other => SetupError::Command(other.to_string()),
    })
}

#[derive(Clone, Debug, Default)]
pub(super) struct MuxEnv {
    pub(super) herdr_env: Option<String>,
    pub(super) herdr_workspace: Option<String>,
    pub(super) tmux: Option<String>,
    pub(super) hc_socket: Option<String>,
}

impl MuxEnv {
    pub(super) fn from_process() -> MuxEnv {
        let var = |name| std::env::var(name).ok();
        MuxEnv {
            herdr_env: var("HERDR_ENV"),
            herdr_workspace: var("HERDR_WORKSPACE_ID"),
            tmux: var("TMUX"),
            hc_socket: var("HC_TERMINAL_SOCKET"),
        }
    }
}

pub(super) fn detect_mux(choice: Multiplexer, env: &MuxEnv, repo_root: &std::path::Path) -> Result<Mux, SetupError> {
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
