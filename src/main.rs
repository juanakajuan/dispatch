//! The shell: terminal setup, the event loop, and the job runner. All
//! decisions are in `app::update`; this file only moves messages.
//!
//! Concurrency model:
//! - The main thread owns `State`, the terminal, and stdin. It is the only
//!   reader of crossterm events. There is no input thread, because a thread
//!   blocked in `event::read` would steal keys from `tmux attach` while the
//!   TUI is suspended.
//! - Each background `Cmd` runs on its own short-lived `std::thread`, with an
//!   owned clone of the immutable `Workspace`, and sends exactly one `Msg` back
//!   on an mpsc channel. `Inflight` caps it at one job per kind, so at most
//!   about six threads exist at once. No `Mutex`, no `Arc<State>`.
//! - The loop: drain the channel, poll input for up to 100 ms, emit a `Tick`,
//!   update, draw. gh latency never blocks a frame.

mod app;
mod config;
mod domain;
mod github;
mod proc;
mod rules;
mod session;
mod ui;
#[cfg(test)]
mod testutil;

use app::{Cmd, Job, Msg};
use session::Workspace;
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, Sender};

fn main() -> ExitCode {
    // TODO:
    // let config = config::load()           -> on Err print and exit 2 (before the TUI).
    // let ws = Workspace::discover(cwd, config.detect_waiting) -> same.
    // let mut terminal = ratatui::init();   (installs a panic hook that restores the tty)
    // crossterm::execute!(stdout, EnableMouseCapture)
    // let (tx, rx) = mpsc::channel();
    // let (mut state, mut cmds) = app::init(Instant::now());
    // loop {
    //     for cmd in cmds.drain(..) {
    //         match cmd {
    //             Cmd::Quit => break 'outer,
    //             Cmd::Attach(id) => { let r = attach(&mut terminal, &ws, &id); pending.push(Msg::Attached(r)) }
    //             Cmd::Job(job) => spawn_job(ws.clone(), job, tx.clone()),
    //         }
    //     }
    //     terminal.draw(|f| state.hits = ui::draw(f, &state, &config))?;
    //     let msgs = drain(&rx) ++ pending ++ poll_input(Duration::from_millis(100)) ++ [Msg::Tick(now)];
    //     for m in msgs { cmds.extend(app::update(&mut state, m, &config)) }
    // }
    // DisableMouseCapture; ratatui::restore();
    todo!()
}

/// Runs one background job and sends its single result.
fn spawn_job(ws: Workspace, job: Job, tx: Sender<Msg>) {
    // TODO: std::thread::spawn(move || {
    //   let msg = match job {
    //     FetchIssues      => Msg::Issues(github::fetch_issues(ws.repo_root()).map_err(..)),
    //     FetchPrs         => Msg::Prs(..),
    //     ObserveSessions  => Msg::Sessions(ws.observe().map_err(..)),
    //     Tail(id)         => Msg::Tail { window: id.clone(), result: ws.tail(&id, app::TAIL_LINES) },
    //     Start(req)       => Msg::Started { issue: req.issue, result: ws.start(&req) },
    //     Stop(s)          => Msg::Stopped { issue: s.issue, result: ws.stop(&s) },
    //     Resume(s)        => Msg::Resumed { .. },
    //   };
    //   let _ = tx.send(msg); // receiver gone = app quitting; dropping is correct
    // });
    todo!()
}

/// Switch: run the argv, return. Suspend: DisableMouseCapture, ratatui::restore(),
/// run `tmux ... attach-session` with inherited stdio and wait, ratatui::init(),
/// EnableMouseCapture, terminal.clear().
fn attach(terminal: &mut ratatui::DefaultTerminal, ws: &Workspace, window: &session::WindowId) -> Result<(), String> {
    todo!()
}

fn drain(rx: &Receiver<Msg>) -> Vec<Msg> {
    todo!()
}
