//! The job runner. Each `ReadJob` runs on its own short-lived thread with an
//! owned clone of the immutable `Workspace`; each `WriteJob` runs in order on
//! the one writer thread. Every job sends exactly one `Msg` back.

use crate::app::msg::{Job, Msg, Observation, ReadJob, WriteJob};
use crate::session::Workspace;
use crate::{config, github, merge, transcript};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

pub struct Jobs {
    ws: Workspace,
    tx: Sender<Msg>,
    writer: Sender<WriteJob>,
    /// One observe runs at a time (`Poll`), so the lock never waits.
    transcripts: Arc<Mutex<transcript::Reader>>,
}

impl Jobs {
    pub fn new(ws: &Workspace, tx: Sender<Msg>) -> Self {
        let writer = spawn_writer(ws.clone(), tx.clone());
        Jobs { ws: ws.clone(), tx, writer, transcripts: Arc::default() }
    }

    pub fn run(&self, job: Job) {
        match job {
            Job::Write(job) => {
                let _ = self.writer.send(job);
            }
            Job::Read(job) => {
                let (ws, tx, transcripts) = (self.ws.clone(), self.tx.clone(), Arc::clone(&self.transcripts));
                std::thread::spawn(move || read(&ws, &tx, &transcripts, job));
            }
        }
    }
}

/// A closed channel means the app is quitting, so a dropped result is correct.
fn read(ws: &Workspace, tx: &Sender<Msg>, transcripts: &Mutex<transcript::Reader>, job: ReadJob) {
    let msg = match job {
        ReadJob::FetchIssues => {
            let result = github::fetch_issues(ws.repo_root());
            Msg::Issues { result, at: Instant::now() }
        }
        ReadJob::FetchPrs => {
            let result = github::fetch_prs(ws.repo_root());
            Msg::Prs { result, at: Instant::now() }
        }
        ReadJob::FetchViewer => Msg::Viewer(github::viewer_login(ws.repo_root())),
        ReadJob::ObserveSessions => Msg::Observed(ws.observe().map(|sessions| {
            let mut reader = transcripts.lock().unwrap_or_else(PoisonError::into_inner);
            let transcripts = reader.read(
                sessions.values().filter(|s| s.harness.has_transcript()).map(|s| (s.issue, s.worktree.as_path())),
            );
            let work = ws.work(&sessions);
            Observation { sessions, transcripts, work }
        })),
        ReadJob::Open(target) => Msg::Opened { target, result: github::open_in_browser(ws.repo_root(), target) },
        ReadJob::Diff(issue) => Msg::Diff { issue, result: ws.diff(issue) },
        ReadJob::FetchPageUrl(target) => Msg::PageUrl { target, result: github::page_url(ws.repo_root(), target) },
    };
    let _ = tx.send(msg);
}

fn spawn_writer(ws: Workspace, tx: Sender<Msg>) -> Sender<WriteJob> {
    let (writer, jobs) = mpsc::channel::<WriteJob>();
    std::thread::spawn(move || {
        for job in jobs {
            let msg = match job {
                WriteJob::Start(req, launch) => Msg::Started { issue: req.issue, result: ws.start(&req, &launch) },
                WriteJob::Stop(issue) => Msg::Stopped { issue, result: ws.stop(issue) },
                WriteJob::Remove(target) => Msg::Removed { issue: target.issue, result: ws.remove(&target) },
                WriteJob::RemoveAfterMerge { pr, method, session, target } => Msg::RemovedAfterMerge {
                    pr,
                    method,
                    issue: target.issue,
                    session,
                    result: ws.stop_and_remove(&target),
                },
                WriteJob::Clean(targets) => Msg::Cleaned(ws.clean(&targets)),
                WriteJob::Resume(issue, launch) => Msg::Resumed { issue, result: ws.resume(issue, &launch) },
                WriteJob::SaveConfig { setting, config } => Msg::Saved(config::settings::save(setting, &config)),
                WriteJob::Send { issue, kind, text } => Msg::Sent { issue, kind, result: ws.send(issue, &text) },
                WriteJob::Merge(ask) => Msg::Merged { ask, run: merge::job::merge(ws.repo_root(), ask) },
            };
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
    writer
}
