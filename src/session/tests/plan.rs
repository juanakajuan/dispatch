use super::{br, markers, req, win, wp, wt};
use crate::config::ClaudeCommand;
use crate::domain::{IssueNumber, Mode, WorktreePath};
use crate::session::error::StartConflict;
use crate::session::join::{join, sessions};
use crate::session::mux::{Liveness, Observed};
use crate::session::plan::{Layout, StartObservation, StartPlan, Step, plan_start};
use crate::session::request::StartFrom;
use crate::session::{git, tmux};
use std::path::PathBuf;

/// `worktree_dir_for` of `layout().repo_root` and issue 142.
const TARGET: &str = "/a/orbit/.claude/worktrees/142";

fn layout() -> Layout {
    Layout {
        repo_root: PathBuf::from("/a/orbit"),
        base: "origin/main".to_string(),
        hooks: None,
        claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() },
    }
}

struct World {
    worktrees: Vec<git::Worktree>,
    markers: git::Markers,
    windows: Vec<Observed<tmux::WindowId>>,
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
            sessions: sessions(&join(&self.worktrees, &self.markers, &self.windows)),
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
            Step::Fetch(_) => {}
            Step::AddWorktree { dir, branch, .. } => {
                self.worktrees
                    .push(git::Worktree { path: WorktreePath::planned(dir.clone()), branch: Some(branch.clone()) });
                self.branch_exists = true;
                self.target_dir_exists = true;
            }
            Step::Mark { branch, issue } => {
                self.markers.insert(branch.clone(), *issue);
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
    Step::AddWorktree { dir: PathBuf::from(TARGET), branch: br(branch), from: from.map(String::from) }
}

fn fetch(remote_ref: &str) -> Step {
    Step::Fetch(remote_ref.to_string())
}

fn mark(branch: &str) -> Step {
    Step::Mark { branch: br(branch), issue: IssueNumber(142) }
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
    assert!(matches!(w.plan(), Ok(StartPlan::AlreadyRunning(s)) if s.issue == IssueNumber(142)));
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
    w.branch_exists = false;
    assert_eq!(steps(w.plan()), [add("fix/142-earlier-name", None), mark("fix/142-earlier-name"), open_step()]);
}

#[test]
fn plan_worktree_at_target_without_marker_marks_its_branch() {
    let mut w = World::empty();
    w.worktrees.push(wt(TARGET, Some("fix/142-x")));
    w.target_dir_exists = true;
    assert_eq!(steps(w.plan()), [mark("fix/142-x"), open_step()]);
}

#[test]
fn plan_target_dir_that_is_no_worktree_conflicts() {
    let mut w = World::empty();
    w.target_dir_exists = true;
    assert_eq!(w.plan(), Err(StartConflict::PathTaken(PathBuf::from(TARGET))));
}

#[test]
fn plan_branch_marked_for_another_issue_conflicts() {
    let mut w = World::empty();
    w.markers = markers(&[("fix/142-x", 7)]);
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
    w.branch_exists = true;
    let got = steps(plan_start(&w.obs(), &r, &layout()));
    assert_eq!(got, [add("fix/142-x", None), mark("fix/142-x"), open_step()], "an existing local branch is used as is");
}

#[test]
fn plan_uses_config_claude_command_hooks_and_plan_mode() {
    let mut l = layout();
    l.claude =
        ClaudeCommand { program: "sh".to_string(), args: vec!["-c".to_string(), "echo".to_string(), "_".to_string()] };
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
        fresh.branch_exists = false;
        let mut existing = World::empty();
        existing.branch_exists = true;
        let mut marked = World::empty();
        marked.markers = markers(&[("fix/142-old", 142)]);
        let mut half = World::empty();
        half.worktrees.push(wt(TARGET, Some("fix/142-x")));
        half.target_dir_exists = true;
        vec![fresh, existing, marked, half]
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
