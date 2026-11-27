use super::{br, markers, req, win, wp, wt};
use crate::config::{AgentCommand, Harness};
use crate::domain::{Mode, OriginRef, RepoSlug, WorktreePath};
use crate::session::error::StartConflict;
use crate::session::join::{join, sessions};
use crate::session::mux::{Liveness, Observed};
use crate::session::plan::{Layout, LocalBranch, StartObservation, StartPlan, Step, TargetDir, plan_start};
use crate::session::request::{StartFrom, StartRequest};
use crate::session::{git, tmux};
use crate::testutil::{issue_key, pr_key};
use std::path::PathBuf;

/// `worktree_dir_for` of `layout().repo_root` and issue 142.
const TARGET: &str = "/a/orbit/.claude/worktrees/142";

fn layout() -> Layout {
    Layout {
        repo_root: PathBuf::from("/a/orbit"),
        base: origin("origin/main"),
        hooks: None,
        harness: Harness::Claude,
        command: AgentCommand::default_for(Harness::Claude),
    }
}

struct World {
    worktrees: Vec<git::Worktree>,
    markers: markers::Markers,
    windows: Vec<Observed<tmux::WindowId>>,
    target_dir: TargetDir,
    branch: LocalBranch,
    from: StartFrom,
}

impl World {
    fn empty() -> World {
        World {
            worktrees: vec![wt("/a/orbit", Some("main"))],
            markers: markers::Markers::new(),
            windows: Vec::new(),
            target_dir: TargetDir::Free,
            branch: LocalBranch::Missing,
            from: StartFrom::Base,
        }
    }

    fn obs(&self) -> StartObservation {
        StartObservation {
            sessions: sessions(&join(
                &layout().repo_root,
                &self.worktrees,
                &self.markers,
                &markers::Harnesses::new(),
                &markers::Modes::new(),
                &self.windows,
            )),
            worktrees: self.worktrees.clone(),
            markers: self.markers.clone(),
            target_dir: self.target_dir,
            branch: self.branch,
        }
    }

    fn plan(&self) -> Result<StartPlan, StartConflict> {
        plan_start(&self.obs(), &StartRequest { from: self.from.clone(), ..req() }, &layout())
    }

    /// Applies one step to the observable world, the way the real
    /// commands would.
    fn apply(&mut self, step: &Step) {
        match step {
            Step::Fetch(_) => {}
            Step::FetchFork { .. } => self.branch = LocalBranch::Exists,
            Step::AddWorktree { dir, branch, .. } => {
                self.worktrees
                    .push(git::Worktree { path: WorktreePath::planned(dir.clone()), branch: Some(branch.clone()) });
                self.branch = LocalBranch::Exists;
                self.target_dir = TargetDir::Taken;
            }
            Step::Mark { branch, key, .. } => {
                self.markers.insert(branch.clone(), *key);
            }
            Step::OpenWindow { dir, .. } => {
                self.windows.push(Observed {
                    window: tmux::WindowId::parse("@9").expect("id"),
                    worktree: dir.clone(),
                    liveness: Liveness::Working,
                });
            }
        }
    }
}

fn open_step() -> Step {
    let argv = ["claude".to_string(), "p".to_string()].to_vec();
    Step::OpenWindow { dir: wp(TARGET), argv }
}

fn add(branch: &str, from: Option<&str>) -> Step {
    Step::AddWorktree { dir: PathBuf::from(TARGET), branch: br(branch), from: from.map(origin) }
}

fn fetch(remote_ref: &str) -> Step {
    Step::Fetch(origin(remote_ref))
}

fn origin(remote_ref: &str) -> OriginRef {
    OriginRef::parse(remote_ref).expect("origin ref")
}

fn mark(branch: &str) -> Step {
    Step::Mark { branch: br(branch), key: issue_key(142), harness: Harness::Claude, mode: Mode::Implement }
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
    w.windows.push(win("@3", TARGET, Liveness::Working));
    assert!(matches!(w.plan(), Ok(StartPlan::AlreadyRunning(s)) if s.key == issue_key(142)));
}

#[test]
fn plan_session_without_live_window_opens_one() {
    let mut w = World::empty();
    w.worktrees.push(wt(TARGET, Some("fix/142-x")));
    w.markers = markers(&[("fix/142-x", 142)]);
    assert_eq!(steps(w.plan()), [open_step()]);
    w.windows.push(win("@3", TARGET, Liveness::Dead));
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
    w.branch = LocalBranch::Missing;
    assert_eq!(steps(w.plan()), [add("fix/142-earlier-name", None), mark("fix/142-earlier-name"), open_step()]);
}

#[test]
fn plan_worktree_at_target_without_marker_marks_its_branch() {
    let mut w = World::empty();
    w.worktrees.push(wt(TARGET, Some("fix/142-x")));
    w.target_dir = TargetDir::Taken;
    assert_eq!(steps(w.plan()), [mark("fix/142-x"), open_step()]);
}

#[test]
fn plan_target_dir_that_is_no_worktree_conflicts() {
    let mut w = World::empty();
    w.target_dir = TargetDir::Taken;
    assert_eq!(w.plan(), Err(StartConflict::PathTaken(PathBuf::from(TARGET))));
}

#[test]
fn plan_branch_marked_for_another_issue_conflicts() {
    let mut w = World::empty();
    w.markers = markers(&[("fix/142-x", 7)]);
    w.branch = LocalBranch::Exists;
    assert_eq!(w.plan(), Err(StartConflict::BranchBelongsTo { branch: br("fix/142-x"), key: issue_key(7) }));
}

#[test]
fn plan_branch_checked_out_elsewhere_conflicts() {
    let mut w = World::empty();
    w.worktrees.push(wt("/elsewhere", Some("fix/142-x")));
    w.branch = LocalBranch::Exists;
    assert_eq!(
        w.plan(),
        Err(StartConflict::BranchCheckedOut { branch: br("fix/142-x"), path: PathBuf::from("/elsewhere") })
    );
}

#[test]
fn plan_existing_free_branch_skips_the_fetch_and_the_create() {
    let mut w = World::empty();
    w.branch = LocalBranch::Exists;
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
    w.branch = LocalBranch::Exists;
    let got = steps(plan_start(&w.obs(), &r, &layout()));
    assert_eq!(got, [add("fix/142-x", None), mark("fix/142-x"), open_step()], "an existing local branch is used as is");
}

fn from_fork() -> StartFrom {
    StartFrom::Fork { repo: RepoSlug::parse("maxbeizer/cli").expect("slug"), branch: br("fix-scope") }
}

#[test]
fn plan_for_a_fork_pr_fetches_the_fork_branch_into_a_new_local_branch() {
    let mut w = World::empty();
    w.from = from_fork();
    let fetch_fork = Step::FetchFork {
        repo: RepoSlug::parse("maxbeizer/cli").expect("slug"),
        head: br("fix-scope"),
        branch: br("fix/142-x"),
    };
    assert_eq!(steps(w.plan()), [fetch_fork, add("fix/142-x", None), mark("fix/142-x"), open_step()]);
    w.branch = LocalBranch::Exists;
    assert_eq!(steps(w.plan()), [add("fix/142-x", None), mark("fix/142-x"), open_step()], "no fetch over local work");
}

#[test]
fn plan_uses_the_command_hooks_and_plan_mode() {
    let mut l = layout();
    l.command =
        AgentCommand { program: "sh".to_string(), args: vec!["-c".to_string(), "echo".to_string(), "_".to_string()] };
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
        fresh.branch = LocalBranch::Missing;
        let mut existing = World::empty();
        existing.branch = LocalBranch::Exists;
        let mut marked = World::empty();
        marked.markers = markers(&[("fix/142-old", 142)]);
        let mut half = World::empty();
        half.worktrees.push(wt(TARGET, Some("fix/142-x")));
        half.target_dir = TargetDir::Taken;
        let mut fork = World::empty();
        fork.from = from_fork();
        vec![fresh, existing, marked, half, fork]
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

#[test]
fn a_session_whose_worktree_changed_branch_reopens_and_marks_nothing() {
    let mut w = World::empty();
    w.worktrees.push(wt(TARGET, Some("try-other")));
    w.markers = markers(&[("fix/142-x", 142)]);
    w.target_dir = TargetDir::Taken;
    assert_eq!(steps(w.plan()), [open_step()]);
}

fn pr_request(from: StartFrom) -> StartRequest {
    StartRequest { key: pr_key(7), branch: br("chore/dep"), from, ..req() }
}

const PR_TARGET: &str = "/a/orbit/.claude/worktrees/pr-7";

fn pr_step_add(from: Option<&str>) -> Step {
    Step::AddWorktree { dir: PathBuf::from(PR_TARGET), branch: br("chore/dep"), from: from.map(origin) }
}

fn pr_mark() -> Step {
    Step::Mark { branch: br("chore/dep"), key: pr_key(7), harness: Harness::Claude, mode: Mode::Implement }
}

fn pr_open() -> Step {
    let Step::OpenWindow { argv, .. } = open_step() else { panic!("open step") };
    Step::OpenWindow { dir: wp(PR_TARGET), argv }
}

#[test]
fn plan_for_a_pr_key_creates_the_branch_from_origin_in_the_pr_dir_and_marks_it_for_the_pr() {
    let got = steps(plan_start(&World::empty().obs(), &pr_request(StartFrom::Origin), &layout()));
    assert_eq!(got, [fetch("origin/chore/dep"), pr_step_add(Some("origin/chore/dep")), pr_mark(), pr_open()]);
}

#[test]
fn plan_for_a_fork_pr_key_fetches_the_fork_branch_and_marks_it_for_the_pr() {
    let r = StartRequest { branch: br("fork/pr-7"), ..pr_request(from_fork()) };
    let got = steps(plan_start(&World::empty().obs(), &r, &layout()));
    let fetch_fork = Step::FetchFork {
        repo: RepoSlug::parse("maxbeizer/cli").expect("slug"),
        head: br("fix-scope"),
        branch: br("fork/pr-7"),
    };
    let add = Step::AddWorktree { dir: PathBuf::from(PR_TARGET), branch: br("fork/pr-7"), from: None };
    let mark = Step::Mark { branch: br("fork/pr-7"), key: pr_key(7), harness: Harness::Claude, mode: Mode::Implement };
    assert_eq!(got, [fetch_fork, add, mark, pr_open()]);
}

#[test]
fn plan_pr_7_and_issue_7_coexist() {
    let mut w = World::empty();
    w.worktrees.push(wt("/a/orbit/.claude/worktrees/7", Some("fix/7-x")));
    w.markers = markers(&[("fix/7-x", 7)]);
    w.windows.push(win("@3", "/a/orbit/.claude/worktrees/7", Liveness::Working));
    let got = steps(plan_start(&w.obs(), &pr_request(StartFrom::Origin), &layout()));
    assert_eq!(got[1..3], [pr_step_add(Some("origin/chore/dep")), pr_mark()], "the issue 7 session does not stop it");

    let mut w = World::empty();
    w.worktrees.push(wt(PR_TARGET, Some("chore/dep")));
    w.markers = [(br("chore/dep"), pr_key(7))].into_iter().collect();
    w.windows.push(win("@4", PR_TARGET, Liveness::Working));
    let plan = plan_start(&w.obs(), &pr_request(StartFrom::Origin), &layout());
    assert!(matches!(plan, Ok(StartPlan::AlreadyRunning(s)) if s.key == pr_key(7)));
}

#[test]
fn plan_branch_marked_for_a_pr_conflicts_with_an_issue_start_on_it() {
    let mut w = World::empty();
    w.markers = [(br("fix/142-x"), pr_key(7))].into_iter().collect();
    w.branch = LocalBranch::Exists;
    assert_eq!(w.plan(), Err(StartConflict::BranchBelongsTo { branch: br("fix/142-x"), key: pr_key(7) }));
    assert_eq!(
        StartConflict::BranchBelongsTo { branch: br("fix/142-x"), key: pr_key(7) }.to_string(),
        "branch fix/142-x already belongs to PR #7"
    );
}

#[test]
fn plan_marks_and_launches_the_chosen_harness() {
    let mut l = layout();
    l.harness = Harness::Codex;
    l.command = AgentCommand::default_for(Harness::Codex);
    l.hooks = None;
    let mut r = req();
    r.mode = Mode::PlanFirst;
    let got = steps(plan_start(&World::empty().obs(), &r, &l));
    let mark =
        Step::Mark { branch: br("fix/142-x"), key: issue_key(142), harness: Harness::Codex, mode: Mode::PlanFirst };
    assert!(got.contains(&mark), "the mode is marked, so a resume keeps it");
    let Some(Step::OpenWindow { argv, .. }) = got.last() else { panic!("no OpenWindow") };
    assert_eq!(argv, &["codex", "--sandbox", "read-only", "p"]);
}
