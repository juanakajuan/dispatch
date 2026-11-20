//! The shell: terminal setup, the event loop, and suspend-attach. All
//! decisions are in `app::update`; this file only moves messages.
//!
//! Concurrency model:
//! - The main thread owns `State`, the terminal, and stdin. It is the only
//!   reader of crossterm events. There is no input thread, because a thread
//!   blocked in `event::read` would steal keys from `tmux attach` while the
//!   TUI is suspended.
//! - `jobs` runs each read on a short thread and each write on one serial
//!   writer thread; results come back as `Msg`s on an mpsc channel. Each feed's
//!   `Poll` caps its reads at one job in flight. No `Mutex`, no `Arc<State>`.
//! - The loop: drain the channel, poll input for up to 100 ms, emit a `Tick`,
//!   update, draw. gh latency never blocks a frame.

mod app;
mod clipboard;
mod config;
mod domain;
mod github;
mod glyphs;
mod jobs;
mod merge;
mod proc;
mod session;
mod sort;
#[cfg(test)]
mod testutil;
mod transcript;
mod ui;

use app::msg::{Cmd, Msg};
use app::{Context, OpenWith};
use crossterm::event::{self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event};
use session::error::{AttachError, SetupError};
use session::{AttachPlan, Workspace};
use std::fmt;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dispatch: {message}");
            ExitCode::from(2)
        }
    }
}

/// Why dispatch stopped with an error line instead of the TUI.
#[derive(Debug)]
enum RunError {
    Config(config::ConfigError),
    Cwd(std::io::Error),
    Setup(SetupError),
    Terminal(std::io::Error),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Config(e) => e.fmt(f),
            RunError::Cwd(e) => write!(f, "cannot read the current directory: {e}"),
            RunError::Setup(e) => e.fmt(f),
            RunError::Terminal(e) => e.fmt(f),
        }
    }
}

fn run() -> Result<(), RunError> {
    let config = config::load().map_err(RunError::Config)?;
    let cwd = std::env::current_dir().map_err(RunError::Cwd)?;
    let ws = Workspace::discover(&cwd, config.multiplexer).map_err(RunError::Setup)?;

    let mut terminal = ratatui::init();
    // A crashed earlier run can leave the alternate screen dirty, and the
    // first draw only writes non-blank cells.
    terminal.clear().map_err(RunError::Terminal)?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
        previous_hook(info);
    }));
    let result = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange)
        .and_then(|()| event_loop(&mut terminal, &ws, config))
        .map_err(RunError::Terminal);
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, ws: &Workspace, config: config::Config) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let jobs = jobs::Jobs::new(ws, tx);
    let repo_name = ws.repo_root().file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    #[expect(
        clippy::disallowed_methods,
        reason = "boundary: over SSH, a URL opens through the local clipboard, not a remote browser"
    )]
    let over_ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    let open_with = if over_ssh { OpenWith::Clipboard } else { OpenWith::Browser };
    let context = Context { repo_name, open_with, start_preview: ws.start_preview() };
    let (mut state, mut cmds) = app::init(Instant::now(), config, context);
    let mut pending: Vec<Msg> = Vec::new();
    loop {
        for cmd in std::mem::take(&mut cmds) {
            match cmd {
                Cmd::Quit => return Ok(()),
                Cmd::Attach(key) => pending.push(Msg::Attached(attach(terminal, ws, key))),
                Cmd::Copy { target, text } => {
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "boundary: inside tmux, the copy goes through tmux so it reaches the client terminal"
                    )]
                    let via = if std::env::var_os("TMUX").is_some() {
                        clipboard::Via::Tmux { cwd: ws.repo_root() }
                    } else {
                        clipboard::Via::Terminal
                    };
                    let result = clipboard::copy(&text, via).map(|()| text);
                    pending.push(Msg::Copied { target, result });
                }
                Cmd::Job(job) => jobs.run(job),
            }
        }
        let mut hits = app::mouse::HitMap::default();
        terminal.draw(|frame| hits = ui::draw(frame, &state))?;
        state.hits = hits;

        let mut msgs: Vec<Msg> = drain(&rx);
        msgs.append(&mut pending);
        msgs.extend(poll_input(POLL)?);
        msgs.push(Msg::Tick(Instant::now()));
        for msg in msgs {
            cmds.extend(app::update(&mut state, msg));
        }
    }
}

/// Switch: run the argv, return. Suspend: leave the TUI, run the argv with
/// inherited stdio and wait, then restore the TUI.
fn attach(terminal: &mut ratatui::DefaultTerminal, ws: &Workspace, key: domain::SessionKey) -> Result<(), AttachError> {
    match ws.attach_plan(key).map_err(AttachError::Session)? {
        AttachPlan::Switch(command) => {
            let (program, args) = command.split_first().ok_or(AttachError::EmptyCommand)?;
            proc::run(program, args, ws.repo_root()).map(|_| ()).map_err(AttachError::Run)
        }
        AttachPlan::Suspend(command) => {
            let (program, args) = command.split_first().ok_or(AttachError::EmptyCommand)?;
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
            ratatui::restore();
            #[expect(
                clippy::disallowed_methods,
                reason = "the attach hands the terminal to the child with inherited stdio, which `proc::run` cannot do"
            )]
            let status = std::process::Command::new(program).args(args).status();
            *terminal = ratatui::init();
            let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange);
            terminal.clear().map_err(AttachError::Terminal)?;
            match status {
                Ok(s) if s.success() => Ok(()),
                Ok(status) => Err(AttachError::Exited { program: program.clone(), status }),
                Err(e) => Err(AttachError::Run(proc::ProcError::spawn(program, &e))),
            }
        }
    }
}

fn drain(rx: &Receiver<Msg>) -> Vec<Msg> {
    rx.try_iter().collect()
}

/// Waits up to `timeout` for input, then takes everything that is queued.
fn poll_input(timeout: Duration) -> std::io::Result<Vec<Msg>> {
    let mut msgs = Vec::new();
    let mut wait = timeout;
    while event::poll(wait)? {
        match event::read()? {
            Event::Key(key) => msgs.push(Msg::Key(key)),
            Event::Mouse(mouse) => msgs.push(Msg::Mouse(mouse, Instant::now())),
            Event::FocusGained => msgs.push(Msg::FocusGained),
            Event::FocusLost => msgs.push(Msg::FocusLost),
            _ => {}
        }
        wait = Duration::ZERO;
    }
    Ok(msgs)
}

#[cfg(test)]
mod tests {
    use super::RunError;

    #[test]
    fn a_missing_current_directory_keeps_its_sentence() {
        let gone = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory (os error 2)");
        assert_eq!(
            RunError::Cwd(gone).to_string(),
            "cannot read the current directory: No such file or directory (os error 2)"
        );
    }
}
