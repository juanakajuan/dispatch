//! What `src/jobs.rs` and the outside world do, in memory. Write jobs wait
//! in one queue and run in order, as on the writer thread. Read jobs wait in
//! a set and may finish in any order, as each runs on its own thread.

use super::super::*;
use crate::merge::MergeAsk;
use crate::merge::job::DraftStep;
use crate::merge::plan::MergeRefusal;
use crate::session::{RemoveOutcome, Sessions};
use std::collections::VecDeque;

/// GitHub and the multiplexer as the jobs see them, and the jobs not yet run.
pub(super) struct World {
    pub(super) sessions: Sessions,
    pub(super) prs: Vec<PullRequest>,
    pub(super) writes: VecDeque<WriteJob>,
    pub(super) reads: Vec<ReadJob>,
}

impl World {
    pub(super) fn new(prs: Vec<PullRequest>) -> Self {
        World { sessions: Sessions::new(), prs, writes: VecDeque::new(), reads: Vec::new() }
    }

    /// Queues what the reducer asked for. Shell commands do no job.
    pub(super) fn take(&mut self, cmds: Vec<Cmd>) {
        for cmd in cmds {
            match cmd {
                Cmd::Job(Job::Write(job)) => self.writes.push_back(job),
                Cmd::Job(Job::Read(job)) => self.reads.push(job),
                Cmd::Attach { .. } | Cmd::Copy { .. } | Cmd::Quit => {}
            }
        }
    }

    fn pr(&self, n: PrNumber) -> &PullRequest {
        self.prs.iter().find(|p| p.number == n).expect("the world lists every PR the app lists")
    }

    fn pr_mut(&mut self, n: PrNumber) -> &mut PullRequest {
        self.prs.iter_mut().find(|p| p.number == n).expect("the world lists every PR the app lists")
    }

    /// Runs the writer's next job, as `ws.start`, `ws.stop`, and the merge
    /// job do, and returns the `Msg` it sends back.
    pub(super) fn run_write(&mut self) -> Msg {
        let job = self.writes.pop_front().expect("a write job is queued");
        match job {
            WriteJob::Start(req, _) => {
                let session = self.sessions.entry(req.key).or_insert_with(|| started(&req)).clone();
                Msg::Started { key: req.key, result: Ok(StartOutcome { session, warnings: Vec::new() }) }
            }
            WriteJob::Stop(key) => {
                if let Some(s) = self.sessions.get_mut(&key) {
                    s.state = SessionState::Exited;
                }
                Msg::Stopped { key, result: Ok(()) }
            }
            WriteJob::Send { key, kind, .. } => Msg::Sent { key, kind, result: Ok(()) },
            WriteJob::Merge(ask) => Msg::Merged { ask, run: self.merge(ask) },
            WriteJob::RemoveAfterMerge { pr, method, session, target } => {
                self.sessions.remove(&target.key);
                Msg::RemovedAfterMerge { pr, method, key: target.key, session, result: Ok(RemoveOutcome::Removed) }
            }
            WriteJob::Remove(target) => {
                self.sessions.remove(&target.key);
                Msg::Removed { key: target.key, result: Ok(RemoveOutcome::Removed) }
            }
            WriteJob::Resume(..) | WriteJob::Clean(_) | WriteJob::SaveConfig { .. } => {
                panic!("the explorer presses no key that sends {job:?}; model it before adding that key")
            }
        }
    }

    /// `merge::job::merge` against this world's PR.
    fn merge(&mut self, ask: MergeAsk) -> MergeRun {
        let fresh = self.pr(ask.pr).clone();
        let result = match (fresh.state, fresh.mergeability) {
            (PrState::Merged, _) => Ok(MergeOutcome::Refused(MergeRefusal::AlreadyMerged(ask.pr))),
            (_, Mergeability::Conflict) => Ok(MergeOutcome::Handoff {
                problem: MergeProblem::Conflict,
                method: MergeMethod::Rebase,
                pr: Box::new(fresh),
            }),
            _ => {
                self.merge_on_github(ask.pr);
                Ok(MergeOutcome::Merged { method: MergeMethod::Rebase, head_oid: fresh.head_oid })
            }
        };
        MergeRun { draft: DraftStep::WasReady, result }
    }

    /// The PR is merged on GitHub, by the merge job or by an agent.
    pub(super) fn merge_on_github(&mut self, n: PrNumber) {
        self.pr_mut(n).state = PrState::Merged;
    }

    /// The PR's base moves on, and the PR now conflicts with it.
    pub(super) fn conflict_on_github(&mut self, n: PrNumber) {
        self.pr_mut(n).mergeability = Mergeability::Conflict;
    }

    /// Finishes the read job at `i` with this world's data, and returns its `Msg`.
    pub(super) fn run_read(&mut self, i: usize, at: Instant) -> Msg {
        match self.reads.remove(i) {
            ReadJob::FetchPrs => Msg::Prs { result: Ok(self.prs.clone()), at },
            ReadJob::FetchIssues => Msg::Issues { result: Ok(issues()), at },
            ReadJob::ObserveSessions => Msg::Observed(Ok(Observation {
                sessions: self.sessions.clone(),
                transcripts: BTreeMap::new(),
                work: crate::session::Works::new(),
            })),
            ReadJob::FetchViewer => Msg::Viewer(Ok("viewer".to_string())),
            job @ (ReadJob::Open(_) | ReadJob::FetchPageUrl(_) | ReadJob::Diff(_)) => {
                panic!("the explorer presses no key that sends {job:?}; model it before adding that key")
            }
        }
    }

    /// The jobs and the outside state, for telling explored states apart.
    pub(super) fn fingerprint(&self) -> String {
        let writes: Vec<String> = self.writes.iter().map(write_summary).collect();
        let reads: Vec<String> = self.reads.iter().map(|r| format!("{r:?}")).collect();
        let sessions: Vec<_> = self.sessions.values().map(|s| (s.key, s.branch.as_str(), s.state)).collect();
        let prs: Vec<_> = self.prs.iter().map(|p| (p.number, p.state, p.mergeability)).collect();
        format!("{writes:?}|{reads:?}|{sessions:?}|{prs:?}")
    }
}

/// A write job without its `Launch`, which only says how to run the agent.
pub(super) fn write_summary(job: &WriteJob) -> String {
    match job {
        WriteJob::Start(req, _) => format!("Start({}, {})", req.key, req.branch.as_str()),
        other => format!("{other:?}"),
    }
}

/// The issues of the `app()` fixture, as `gh` lists them.
fn issues() -> Vec<crate::domain::Issue> {
    let (state, _) = app();
    state.issues.items
}

fn started(req: &StartRequest) -> Session {
    Session {
        key: req.key,
        branch: req.branch.clone(),
        checkout: crate::session::Checkout::SessionBranch,
        worktree: WorktreePath::planned(PathBuf::from(format!("/w/{}", req.key))),
        harness: crate::config::Harness::Claude,
        state: SessionState::Running,
        mode: req.mode,
    }
}
