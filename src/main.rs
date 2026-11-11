//! The shell: terminal setup, the event loop, and the job runner. All
//! decisions are in `app::update`; this file only moves messages.
//!
//! Concurrency model:
//! - The main thread owns `State`, the terminal, and stdin. It is the only
//!   reader of crossterm events. There is no input thread, because a thread
//!   blocked in `event::read` would steal keys from `tmux attach` while the
//!   TUI is suspended.
//! - Each read job (`gh`, observe, tail) runs on its own short-lived
//!   `std::thread`, with an owned clone of the immutable `Workspace`, and sends
//!   exactly one `Msg` back on an mpsc channel. `Inflight` caps it at one job
//!   per kind. Start, stop, resume, send, and merge go through one serial
//!   writer thread, because two concurrent `git config` writes can fail on the
//!   `.git/config` lock, and because a second merge must see the first one
//!   done. No `Mutex`, no `Arc<State>`.
//! - The loop: drain the channel, poll input for up to 100 ms, emit a `Tick`,
//!   update, draw. gh latency never blocks a frame.

mod app;
mod config;
mod domain;
mod github;
mod glyphs;
mod markdown;
mod merge;
mod proc;
mod rules;
mod session;
mod sort;
#[cfg(test)]
mod testutil;
mod ui;

use app::{Cmd, Context, Job, Msg};
use crossterm::event::{self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event};
use session::{AttachPlan, Workspace};
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

fn run() -> Result<(), String> {
    let config = config::load().map_err(|e| e.to_string())?;
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current directory: {e}"))?;
    let ws = Workspace::discover(&cwd, &config).map_err(|e| e.to_string())?;

    let mut terminal = ratatui::init();
    // A crashed earlier run can leave the alternate screen dirty, and the
    // first draw only writes non-blank cells.
    terminal.clear().map_err(|e| e.to_string())?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
        previous_hook(info);
    }));
    let result = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange)
        .map_err(|e| e.to_string())
        .and_then(|()| event_loop(&mut terminal, &ws, &config));
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, ws: &Workspace, config: &config::Config) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Msg>();
    let writer = spawn_writer(ws.clone(), tx.clone());
    let repo_name = ws.repo_root().file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let context = Context { repo_name, base: ws.base().to_string() };
    let (mut state, mut cmds) = app::init(Instant::now(), config, context);
    let mut pending: Vec<Msg> = Vec::new();
    loop {
        for cmd in std::mem::take(&mut cmds) {
            match cmd {
                Cmd::Quit => return Ok(()),
                Cmd::Attach(id) => pending.push(Msg::Attached(attach(terminal, ws, &id))),
                Cmd::Job(job) => dispatch_job(ws, job, &tx, &writer),
            }
        }
        let mut hits = app::HitMap::default();
        terminal.draw(|frame| hits = ui::draw(frame, &state, config)).map_err(|e| e.to_string())?;
        state.hits = hits;

        let mut msgs: Vec<Msg> = drain(&rx);
        msgs.append(&mut pending);
        msgs.extend(poll_input(POLL)?);
        msgs.push(Msg::Tick(Instant::now()));
        for msg in msgs {
            cmds.extend(app::update(&mut state, msg, config));
        }
    }
}

/// A write job, run in order on the writer thread.
enum WriteJob {
    Start(session::StartRequest),
    Stop(domain::IssueNumber),
    Resume(domain::IssueNumber),
    Send { issue: domain::IssueNumber, text: String },
    Merge(app::MergeAsk),
}

fn spawn_writer(ws: Workspace, tx: Sender<Msg>) -> Sender<WriteJob> {
    let (writer, jobs) = mpsc::channel::<WriteJob>();
    std::thread::spawn(move || {
        for job in jobs {
            let msg = match job {
                WriteJob::Start(req) => {
                    Msg::Started { issue: req.issue, result: ws.start(&req).map_err(|e| e.to_string()) }
                }
                WriteJob::Stop(issue) => Msg::Stopped { issue, result: ws.stop(issue).map_err(|e| e.to_string()) },
                WriteJob::Resume(issue) => Msg::Resumed { issue, result: ws.resume(issue).map_err(|e| e.to_string()) },
                WriteJob::Send { issue, text } => {
                    Msg::Sent { issue, result: ws.send(issue, &text).map_err(|e| e.to_string()) }
                }
                WriteJob::Merge(ask) => Msg::Merged { ask, result: merge::merge(ws.repo_root(), ask.pr) },
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
        Job::Start(req) => {
            let _ = writer.send(WriteJob::Start(req));
        }
        Job::Stop(issue) => {
            let _ = writer.send(WriteJob::Stop(issue));
        }
        Job::Resume(issue) => {
            let _ = writer.send(WriteJob::Resume(issue));
        }
        Job::Send { issue, text } => {
            let _ = writer.send(WriteJob::Send { issue, text });
        }
        Job::Merge(ask) => {
            let _ = writer.send(WriteJob::Merge(ask));
        }
        Job::FetchIssues => spawn_read(ws, tx, |ws| {
            let result = github::fetch_issues(ws.repo_root()).map_err(|e| e.to_string());
            Msg::Issues { result, at: Instant::now() }
        }),
        Job::FetchPrs => spawn_read(ws, tx, |ws| {
            let result = github::fetch_prs(ws.repo_root()).map_err(|e| e.to_string());
            Msg::Prs { result, at: Instant::now() }
        }),
        Job::ObserveSessions => spawn_read(ws, tx, |ws| Msg::Sessions(ws.observe().map_err(|e| e.to_string()))),
        Job::Open(target) => spawn_read(ws, tx, move |ws| Msg::Opened {
            target,
            result: github::open_in_browser(ws.repo_root(), target).map_err(|e| e.to_string()),
        }),
        Job::Tail(id) => spawn_read(ws, tx, move |ws| {
            let result = ws.tail(&id, app::TAIL_LINES).map_err(|e| e.to_string());
            Msg::Tail { window: id, result }
        }),
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
fn attach(terminal: &mut ratatui::DefaultTerminal, ws: &Workspace, window: &session::WindowId) -> Result<(), String> {
    match ws.attach_plan(window) {
        AttachPlan::Switch(command) => {
            let (program, args) = command.split_first().ok_or("empty attach command")?;
            proc::run(program, args, ws.repo_root()).map(|_| ()).map_err(|e| e.to_string())
        }
        AttachPlan::Suspend(command) => {
            let (program, args) = command.split_first().ok_or("empty attach command")?;
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture, DisableFocusChange);
            ratatui::restore();
            let status = std::process::Command::new(program).args(args).status();
            *terminal = ratatui::init();
            let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableFocusChange);
            terminal.clear().map_err(|e| e.to_string())?;
            match status {
                Ok(s) if s.success() => Ok(()),
                Ok(s) => Err(format!("{program} exited with {s}")),
                Err(e) => Err(format!("cannot run {program}: {e}")),
            }
        }
    }
}

fn drain(rx: &Receiver<Msg>) -> Vec<Msg> {
    rx.try_iter().collect()
}

/// Waits up to `timeout` for input, then takes everything that is queued.
fn poll_input(timeout: Duration) -> Result<Vec<Msg>, String> {
    let mut msgs = Vec::new();
    let mut wait = timeout;
    while event::poll(wait).map_err(|e| e.to_string())? {
        match event::read().map_err(|e| e.to_string())? {
            Event::Key(key) => msgs.push(Msg::Key(key)),
            Event::Mouse(mouse) => msgs.push(Msg::Mouse(mouse, Instant::now())),
            Event::FocusGained => msgs.push(Msg::FocusGained),
            _ => {}
        }
        wait = Duration::ZERO;
    }
    Ok(msgs)
}
