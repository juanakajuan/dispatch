use super::{GitWorld, br, launch, req, workspace};
use crate::proc::fake;
use crate::session::mux::Backend;
use crate::session::{AttachPlan, AttachTo, SessionState, tmux};
use crate::testutil::{issue_key, temp_dir};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[expect(
    clippy::struct_excessive_bools,
    reason = "a test sim of external state: each flag is one independent fact the sim flips as the code under test runs commands"
)]
#[derive(Default)]
struct Sim {
    git: GitWorld,
    window: bool,
    tagged: bool,
    fetch_fails: bool,
}

/// A tiny git and tmux: answers the observation commands from `Sim`
/// and changes `Sim` on the mutating ones.
fn install_world(base: PathBuf, sim: Rc<RefCell<Sim>>) {
    fake::install(move |call| {
        let mut s = sim.borrow_mut();
        let dir = base.join("orbit/.claude/worktrees/142");
        if let Some(answer) = s.git.answer(call, &base) {
            answer
        } else if call.starts_with("tmux list-windows") {
            Ok(if s.window && s.tagged { format!("@4\t0\t{}\t\n", dir.display()) } else { String::new() })
        } else if call.starts_with("git fetch") {
            if s.fetch_fails { Err(fake::fail("git", 128, "no network")) } else { Ok(String::new()) }
        } else if call.starts_with("tmux new-window") {
            s.window = true;
            Ok("@4\n".to_string())
        } else if call.starts_with("tmux set-option") {
            s.tagged = true;
            Ok(String::new())
        } else if call.starts_with("tmux kill-window") {
            s.window = false;
            s.tagged = false;
            Ok(String::new())
        } else {
            Ok(String::new())
        }
    });
}

fn mutating(calls: &[String]) -> Vec<&str> {
    calls
        .iter()
        .map(String::as_str)
        .filter(|c| {
            ["git fetch", "git worktree add", "git config branch", "tmux new-", "tmux set-option"]
                .iter()
                .any(|p| c.starts_with(p))
        })
        .collect()
}

#[test]
fn start_runs_the_steps_in_order_and_a_second_start_runs_none() {
    let base = temp_dir("start");
    let ws = workspace(&base);
    let r = req();
    install_world(base.clone(), Rc::new(RefCell::new(Sim::default())));
    let out = ws.start(&r, &launch()).expect("start");
    assert_eq!(out.session.state, SessionState::Running);
    assert!(out.warnings.is_empty());
    let dir = base.join("orbit/.claude/worktrees/142");
    let calls = fake::calls();
    let ran = mutating(&calls);
    assert_eq!(ran.len(), 6, "{ran:?}");
    assert_eq!(ran[0], "git fetch origin main");
    assert_eq!(ran[1], format!("git worktree add {} -b fix/142-x origin/main", dir.display()));
    assert_eq!(ran[2], "git config branch.fix/142-x.dispatch-issue 142");
    assert_eq!(ran[3], "git config branch.fix/142-x.dispatch-harness claude");
    assert!(ran[4].starts_with(&format!(
        "tmux new-window -d -t $1: -P -F #{{window_id}} -c {} -n #142 -- claude p",
        dir.display()
    )));
    assert!(ran[5].starts_with("tmux set-option -w -t @4 @dispatch-worktree"));
    let exclude = base.join("orbit/.git/info/exclude");
    assert_eq!(std::fs::read_to_string(&exclude).expect("exclude written"), ".claude/worktrees/\n");

    let before = fake::calls().len();
    let again = ws.start(&r, &launch()).expect("second start");
    assert_eq!(again.session, out.session);
    assert!(mutating(&fake::calls()[before..]).is_empty(), "second start must not mutate");
}

#[test]
fn start_after_a_crash_between_steps_runs_only_the_rest() {
    let base = temp_dir("crash");
    let ws = workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = Sim { git: GitWorld { added: true, ..GitWorld::default() }, ..Sim::default() };
    install_world(base.clone(), Rc::new(RefCell::new(sim)));
    let out = ws.start(&req(), &launch()).expect("start");
    assert_eq!(out.session.branch, br("fix/142-x"));
    let calls = fake::calls();
    let ran = mutating(&calls);
    assert_eq!(ran.len(), 4, "{ran:?}");
    assert!(ran[0].starts_with("git config branch.fix/142-x.dispatch-issue"));
    assert!(ran.iter().all(|c| !c.starts_with("git worktree add") && !c.starts_with("git fetch")));
}

#[test]
fn start_after_the_worktree_was_deleted_by_hand_prunes_it_and_replaces_the_old_window() {
    let base = temp_dir("deleted");
    let ws = workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees")).expect("mkdir");
    let git = GitWorld { marked: true, stale: true, ..GitWorld::default() };
    install_world(base.clone(), Rc::new(RefCell::new(Sim { git, window: true, tagged: true, ..Sim::default() })));
    assert!(ws.observe().expect("observe").is_empty(), "a deleted worktree is no session");
    let out = ws.start(&req(), &launch()).expect("start");
    assert_eq!(out.session.state, SessionState::Running);
    let dir = base.join("orbit/.claude/worktrees/142");
    let calls = fake::calls();
    let ran: Vec<&str> = calls
        .iter()
        .map(String::as_str)
        .filter(|c| ["git worktree", "tmux kill-window", "tmux new-window"].iter().any(|p| c.starts_with(p)))
        .filter(|c| !c.starts_with("git worktree list"))
        .collect();
    assert_eq!(ran.len(), 4, "{ran:?}");
    assert_eq!(ran[0], "git worktree prune");
    assert_eq!(ran[1], "tmux kill-window -t @4");
    assert_eq!(ran[2], format!("git worktree add {} fix/142-x", dir.display()));
    assert!(ran[3].starts_with("tmux new-window"), "{}", ran[3]);
}

#[test]
fn failed_fetch_is_a_warning_and_the_start_continues() {
    let base = temp_dir("fetch");
    let ws = workspace(&base);
    install_world(base.clone(), Rc::new(RefCell::new(Sim { fetch_fails: true, ..Sim::default() })));
    let out = ws.start(&req(), &launch()).expect("start continues");
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(
        out.warnings[0].to_string(),
        "fetch failed, using the local origin/main: git failed with code 128: no network"
    );
}

#[test]
fn failed_worktree_add_stops_the_run_with_the_git_message() {
    let base = temp_dir("addfail");
    let ws = workspace(&base);
    fake::install(|call| {
        if call.starts_with("git worktree add") {
            Err(fake::fail("git", 128, "fatal: boom"))
        } else if call.starts_with("git worktree list") || call.starts_with("tmux list-windows") {
            Ok(String::new())
        } else if call.starts_with("git config --get-regexp") || call.starts_with("git show-ref") {
            Err(fake::fail("git", 1, ""))
        } else {
            Ok(String::new())
        }
    });
    let err = ws.start(&req(), &launch()).expect_err("fails");
    assert!(err.to_string().contains("fatal: boom"), "{err}");
    assert!(!fake::calls().iter().any(|c| c.starts_with("tmux new-")));
}

#[test]
fn stop_kills_the_window_by_a_fresh_observation_and_keeps_the_worktree() {
    let base = temp_dir("stop");
    let ws = workspace(&base);
    install_world(
        base.clone(),
        Rc::new(RefCell::new(Sim { git: GitWorld::started(), window: true, tagged: true, ..Sim::default() })),
    );
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    ws.stop(issue_key(142)).expect("stop");
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c == "tmux kill-window -t @4"));
    assert!(!calls.iter().any(|c| c.contains("worktree remove") || c.contains("branch -")));
    assert!(ws.stop(issue_key(999)).is_ok(), "no session is already stopped");
}

#[test]
fn resume_respawns_a_dead_pane_in_an_old_sibling_worktree() {
    let base = temp_dir("resume");
    let ws = workspace(&base);
    std::fs::create_dir_all(base.join("orbit-142")).expect("mkdir");
    let dir = base.join("orbit-142");
    let listed = format!("@4\t1\t{}\t\n", dir.display());
    let wt_text = format!("worktree {}\nbranch refs/heads/fix/142-x\n\n", dir.display());
    fake::install(move |call| {
        if call.starts_with("git worktree list") {
            Ok(wt_text.clone())
        } else if call.starts_with("git config --get-regexp") {
            Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
        } else if call.starts_with("tmux list-windows") {
            Ok(listed.clone())
        } else {
            Ok(String::new())
        }
    });
    ws.resume(issue_key(142), &launch()).expect("resume");
    let last = fake::calls().pop().expect("a call");
    assert_eq!(last, format!("tmux respawn-window -k -t @4 -c {} -- claude --continue", dir.display()));
}

#[test]
fn attach_inside_switches_and_outside_suspends() {
    let base = temp_dir("attach");
    let mut ws = workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = Rc::new(RefCell::new(Sim { git: GitWorld::started(), window: true, tagged: true, ..Sim::default() }));
    install_world(base.clone(), Rc::clone(&sim));
    let id = issue_key(142);
    assert_eq!(
        ws.attach_plan(id, AttachTo::Claude).expect("plan"),
        AttachPlan::Switch(
            ["tmux", "select-window", "-t", "@4", ";", "switch-client", "-t", "$1"].map(String::from).to_vec()
        )
    );
    ws.mux = Backend::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
    assert_eq!(
        ws.attach_plan(id, AttachTo::Claude).expect("plan"),
        AttachPlan::Suspend(
            ["tmux", "select-window", "-t", "@4", ";", "attach-session", "-t", "=dispatch-orbit"]
                .map(String::from)
                .to_vec()
        )
    );
    sim.borrow_mut().window = false;
    let gone = ws.attach_plan(id, AttachTo::Claude).expect_err("the window is gone");
    assert_eq!(gone.to_string(), "#142 has no running session");
}

#[test]
fn shell_opens_an_untagged_window_and_attaches_to_it() {
    let base = temp_dir("tmux-shell");
    let mut ws = workspace(&base);
    let dir = base.join("orbit/.claude/worktrees/142");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let listed = dir.clone();
    fake::install(move |call| {
        let mut git = GitWorld::started();
        if let Some(answer) = git.answer(call, &base) {
            answer
        } else if call.starts_with("tmux list-windows") {
            Ok(format!("@4\t1\t{}\t\n", listed.display()))
        } else if call.starts_with("tmux new-window") {
            Ok("@9\n".to_string())
        } else {
            Ok(String::new())
        }
    });
    let id = issue_key(142);
    let plan = ws.attach_plan(id, AttachTo::Shell).expect("plan");
    assert_eq!(
        plan,
        AttachPlan::Switch(
            ["tmux", "select-window", "-t", "@9", ";", "switch-client", "-t", "$1"].map(String::from).to_vec()
        ),
        "the new window, not Claude's dead one, though Claude exited"
    );
    let calls = fake::calls();
    let new = calls.iter().find(|c| c.starts_with("tmux new-window")).expect("a new window");
    assert_eq!(*new, format!("tmux new-window -d -t $1: -P -F #{{window_id}} -c {} -n #142 shell", dir.display()));
    let sets: Vec<&String> = calls.iter().filter(|c| c.starts_with("tmux set-option")).collect();
    assert_eq!(
        sets,
        ["tmux set-option -w -t @9 remain-on-exit off"],
        "no tag, so observe does not list it, and it closes on exit even when remain-on-exit is on globally"
    );

    ws.mux = Backend::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
    let plan = ws.attach_plan(id, AttachTo::Shell).expect("plan");
    assert!(matches!(plan, AttachPlan::Suspend(_)));
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c.starts_with("tmux new-window -d -t =dispatch-orbit: -P")), "{calls:?}");
}

#[test]
fn shell_for_an_issue_with_no_session_is_refused_before_any_window() {
    let base = temp_dir("tmux-shell-none");
    let ws = workspace(&base);
    install_world(base.clone(), Rc::new(RefCell::new(Sim::default())));
    let err = ws.attach_plan(issue_key(142), AttachTo::Shell).expect_err("no session");
    assert_eq!(err.to_string(), "no session for #142");
    assert!(!fake::calls().iter().any(|c| c.starts_with("tmux new-")));
}
