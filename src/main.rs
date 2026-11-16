//! The shell: terminal setup, the event loop, and the job runner. All
//! decisions are in `app::update`; this file only moves messages.
//!
//! Concurrency model:
//! - The main thread owns `State`, the terminal, and stdin. It is the only
//!   reader of crossterm events. There is no input thread, because a thread
//!   blocked in `event::read` would steal keys from `tmux attach` while the
//!   TUI is suspended.
//! - Each read job (`gh`, observe) runs on its own short-lived
//!   `std::thread`, with an owned clone of the immutable `Workspace`, and sends
//!   exactly one `Msg` back on an mpsc channel. `Inflight` caps it at one job
//!   per kind. Start, stop, resume, send, merge, and config saves go through one serial
//!   writer thread, because two concurrent `git config` writes can fail on the
//!   `.git/config` lock, and because a second merge must see the first one
//!   done. No `Mutex`, no `Arc<State>`.
//! - The loop: drain the channel, poll input for up to 100 ms, emit a `Tick`,
//!   update, draw. gh latency never blocks a frame.

mod app;
mod clipboard;
mod config;
mod domain;
mod github;
mod glyphs;
mod merge;
mod proc;
mod session;
mod sort;
#[cfg(test)]
mod testutil;
mod transcript;
mod ui;

use app::msg::{Cmd, Job, Msg};
use app::{Context, OpenWith};
use crossterm::event::{self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event};
use session::error::{AttachError, SetupError};
use session::{AttachPlan, Workspace};
use std::fmt;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender};
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
    let writer = spawn_writer(ws.clone(), tx.clone());
    let repo_name = ws.repo_root().file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let over_ssh = std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some();
    let open_with = if over_ssh { OpenWith::Clipboard } else { OpenWith::Browser };
    let context = Context { repo_name, open_with, start_preview: ws.start_preview() };
    let (mut state, mut cmds) = app::init(Instant::now(), config, context);
    let mut pending: Vec<Msg> = Vec::new();
    loop {
        for cmd in std::mem::take(&mut cmds) {
            match cmd {
                Cmd::Quit => return Ok(()),
                Cmd::Attach(issue) => pending.push(Msg::Attached(attach(terminal, ws, issue))),
                Cmd::Copy { target, text } => {
                    let inside_tmux = std::env::var_os("TMUX").is_some();
                    let result = clipboard::copy(&text, inside_tmux, ws.repo_root()).map(|()| text);
                    pending.push(Msg::Copied { target, result });
                }
                Cmd::Job(job) => dispatch_job(ws, job, &tx, &writer),
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

/// A write job, run in order on the writer thread.
enum WriteJob {
    Start(session::request::StartRequest, session::request::Launch),
    Stop(domain::IssueNumber),
    Resume(domain::IssueNumber, session::request::Launch),
    SaveConfig { setting: config::settings::Setting, config: config::Config },
    Send { issue: domain::IssueNumber, text: String },
    Merge(merge::MergeAsk),
}

fn spawn_writer(ws: Workspace, tx: Sender<Msg>) -> Sender<WriteJob> {
    let (writer, jobs) = mpsc::channel::<WriteJob>();
    std::thread::spawn(move || {
        for job in jobs {
            let msg = match job {
                WriteJob::Start(req, launch) => Msg::Started { issue: req.issue, result: ws.start(&req, &launch) },
                WriteJob::Stop(issue) => Msg::Stopped { issue, result: ws.stop(issue) },
                WriteJob::Resume(issue, launch) => Msg::Resumed { issue, result: ws.resume(issue, &launch) },
                WriteJob::SaveConfig { setting, config } => Msg::Saved(config::settings::save(setting, &config)),
                WriteJob::Send { issue, text } => Msg::Sent { issue, result: ws.send(issue, &text) },
                WriteJob::Merge(ask) => Msg::Merged { ask, run: merge::job::merge(ws.repo_root(), ask) },
            };
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
    writer
}

fn dispatch_job(ws: &Workspace, job: Job, tx: &Sender<Msg>, writer: &Sender<WriteJob>) {
    match job {
        Job::Start(req, launch) => {
            let _ = writer.send(WriteJob::Start(req, launch));
        }
        Job::Stop(issue) => {
            let _ = writer.send(WriteJob::Stop(issue));
        }
        Job::Resume(issue, launch) => {
            let _ = writer.send(WriteJob::Resume(issue, launch));
        }
        Job::SaveConfig { setting, config } => {
            let _ = writer.send(WriteJob::SaveConfig { setting, config });
        }
        Job::Send { issue, text } => {
            let _ = writer.send(WriteJob::Send { issue, text });
        }
        Job::Merge(ask) => {
            let _ = writer.send(WriteJob::Merge(ask));
        }
        Job::FetchIssues => spawn_read(ws, tx, |ws| {
            let result = github::fetch_issues(ws.repo_root());
            Msg::Issues { result, at: Instant::now() }
        }),
        Job::FetchPrs => spawn_read(ws, tx, |ws| {
            let result = github::fetch_prs(ws.repo_root());
            Msg::Prs { result, at: Instant::now() }
        }),
        Job::ObserveSessions => {
            let (ws, tx) = (ws.clone(), tx.clone());
            std::thread::spawn(move || {
                let result = ws.observe();
                let summaries = result.as_ref().ok().map(|sessions| {
                    sessions
                        .values()
                        .filter_map(|s| Some((s.issue, transcript::latest(s.worktree.as_path())?)))
                        .collect()
                });
                let _ = tx.send(Msg::Sessions(result));
                if let Some(summaries) = summaries {
                    let _ = tx.send(Msg::Summaries(summaries));
                }
            });
        }
        Job::Open(target) => spawn_read(ws, tx, move |ws| Msg::Opened {
            target,
            result: github::open_in_browser(ws.repo_root(), target),
        }),
        Job::FetchPageUrl(target) => {
            spawn_read(ws, tx, move |ws| Msg::PageUrl { target, result: github::page_url(ws.repo_root(), target) });
        }
    }
}

/// One short thread per read job. A closed channel means the app is quitting,
/// so a dropped result is correct.
fn spawn_read(ws: &Workspace, tx: &Sender<Msg>, job: impl FnOnce(&Workspace) -> Msg + Send + 'static) {
    let (ws, tx) = (ws.clone(), tx.clone());
    std::thread::spawn(move || {
        let _ = tx.send(job(&ws));
    });
}

/// Switch: run the argv, return. Suspend: leave the TUI, run the argv with
/// inherited stdio and wait, then restore the TUI.
fn attach(
    terminal: &mut ratatui::DefaultTerminal,
    ws: &Workspace,
    issue: domain::IssueNumber,
) -> Result<(), AttachError> {
    match ws.attach_plan(issue).map_err(AttachError::Session)? {
        AttachPlan::Switch(command) => {
            let (program, args) = command.split_first().ok_or(AttachError::EmptyCommand)?;
            proc::run(program, args, ws.repo_root()).map(|_| ()).map_err(AttachError::Run)
        }
        AttachPlan::Suspend(command) => {
            let (program, args) = command.split_first().ok_or(AttachError::EmptyCommand)?;
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
            ratatui::restore();
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
