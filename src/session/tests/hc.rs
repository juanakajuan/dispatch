use super::{GitWorld, launch, req, workspace};
use crate::domain::IssueNumber;
use crate::proc::fake;
use crate::session::mux::Backend;
use crate::session::{AttachPlan, AttachTo, SessionState, Workspace, hc};
use crate::testutil::temp_dir;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Default)]
struct HcSim {
    git: GitWorld,
    terminals: Vec<(&'static str, &'static str, Option<&'static str>)>,
    gone: bool,
    /// Other terminals that carry the space label, such as one the user opened.
    space_mates: Vec<&'static str>,
    /// Shells the app opened in the space when it restored it after a restart.
    /// They carry dispatch's labels but run no Claude.
    shells: Vec<&'static str>,
}

fn not_found() -> crate::proc::ProcError {
    fake::fail("hc", 3, r#"{"error":{"hint":"see `hc ls --all`","kind":"not_found","message":"no terminal"}}"#)
}

fn install_hc_world(base: PathBuf, sim: Rc<RefCell<HcSim>>) {
    fake::install(move |call| {
        let mut s = sim.borrow_mut();
        let dir = base.join("orbit/.claude/worktrees/142");
        if let Some(answer) = s.git.answer(call, &base) {
            answer
        } else if call.starts_with("hc ls") {
            let mates = if call.contains("--label herdr.space=") { s.space_mates.as_slice() } else { &[] };
            let shells = s
                .shells
                .iter()
                .map(|a| serde_json::json!({"address": a, "agent": "fish", "cwd": dir, "state": "live"}));
            let rows: Vec<_> = shells
                .chain(
                    s.terminals
                        .iter()
                        .map(|(a, state, status)| (*a, *state, *status))
                        .chain(mates.iter().map(|a| (*a, "live", Some("idle"))))
                        .map(|(a, state, status)| serde_json::json!({"address": a, "cwd": dir, "state": state, "status": status})),
                )
                .collect();
            Ok(serde_json::json!({"hidden_finished": 0, "terminals": rows}).to_string())
        } else if call.starts_with("hc run") {
            s.terminals.push(("t9", "live", Some("idle")));
            Ok(r#"{"address":"t9","state":"live"}"#.to_string())
        } else if call.starts_with("hc kill") && s.terminals.iter().all(|(_, state, _)| *state == "finished") {
            Err(fake::fail("hc", 11, r#"{"error":{"kind":"finished"}}"#))
        } else if call.starts_with("hc rm") && s.shells.iter().any(|a| call == format!("hc rm {a}")) {
            Err(fake::fail("hc", 1, r#"{"error":{"hint":"kill it first","kind":"live","message":"still running"}}"#))
        } else if call.starts_with("hc rm") || call.starts_with("hc kill") {
            if s.gone { Err(not_found()) } else { Ok("{}".to_string()) }
        } else {
            Ok(String::new())
        }
    });
}

fn hc_workspace(base: &std::path::Path) -> Workspace {
    let mut ws = workspace(base);
    ws.mux = Backend::Hc(hc::Hc { place: hc::Place::Outside, path: None });
    ws
}

fn hc_calls() -> Vec<String> {
    fake::calls()
        .into_iter()
        .filter(|c| {
            ["git fetch", "git worktree add", "git config branch", "hc run", "hc rm", "hc kill", "hc send"]
                .iter()
                .any(|p| c.starts_with(p))
        })
        .collect()
}

fn hc_run(base: &std::path::Path, argv: &str) -> String {
    let dir = base.join("orbit/.claude/worktrees/142");
    let space = hc::space_id(&dir);
    format!(
        "hc run --bg --json --no-parent --no-notify --cwd {} --label herdr.space={space} --label dispatch.repo={} --label dispatch.issue=142 -- {argv}",
        dir.display(),
        base.join("orbit").display()
    )
}

#[test]
fn hc_start_adds_the_worktree_with_git_and_runs_claude_with_the_labels() {
    let base = temp_dir("hc-start");
    let ws = hc_workspace(&base);
    install_hc_world(base.clone(), Rc::new(RefCell::new(HcSim::default())));
    let mut r = req();
    r.prompt = "line one\nit's \"line\" two;".to_string();
    let out = ws.start(&r, &launch()).expect("start");
    assert_eq!(out.session.state, SessionState::NeedsYou, "an idle status is waiting");
    let dir = base.join("orbit/.claude/worktrees/142");
    assert_eq!(
        hc_calls(),
        [
            "git fetch origin main".to_string(),
            format!("git worktree add {} -b fix/142-x origin/main", dir.display()),
            "git config branch.fix/142-x.dispatch-issue 142".to_string(),
            hc_run(&base, "claude line one\nit's \"line\" two;"),
        ]
    );
    let before = fake::calls().len();
    ws.start(&r, &launch()).expect("second start");
    assert!(!fake::calls()[before..].iter().any(|c| c.starts_with("hc run")), "a live session is not started twice");
}

#[test]
fn hc_start_runs_claude_with_the_path_dispatch_sees() {
    let base = temp_dir("hc-path");
    let mut ws = workspace(&base);
    ws.mux = Backend::Hc(hc::Hc { place: hc::Place::Outside, path: Some("/home/u/.local/bin:/usr/bin".to_string()) });
    install_hc_world(base.clone(), Rc::new(RefCell::new(HcSim::default())));
    let mut r = req();
    r.prompt = "go".to_string();
    ws.start(&r, &launch()).expect("start");
    let run = hc_calls().pop().expect("a call");
    assert_eq!(run, hc_run(&base, "/usr/bin/env PATH=/home/u/.local/bin:/usr/bin claude go"));
}

#[test]
fn hc_start_passes_no_tmux_hooks_even_when_waiting_detection_is_on() {
    let base = temp_dir("hc-hooks");
    let ws = hc_workspace(&base);
    install_hc_world(base.clone(), Rc::new(RefCell::new(HcSim::default())));
    let mut l = launch();
    l.detect_waiting = true;
    ws.start(&req(), &l).expect("start");
    let run = hc_calls().pop().expect("a call");
    assert!(!run.contains("--settings"), "{run}");
}

#[test]
fn hc_resume_of_a_dead_terminal_removes_it_then_runs_continue_in_the_same_space() {
    let base = temp_dir("hc-resume");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "finished", Some("done"))],
        gone: false,
        ..HcSim::default()
    };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.resume(IssueNumber(142), &launch()).expect("resume");
    assert_eq!(hc_calls(), ["hc rm t4".to_string(), hc_run(&base, "claude --continue")]);
}

#[test]
fn hc_resume_without_a_terminal_runs_continue_and_removes_nothing() {
    let base = temp_dir("hc-resume-missing");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim { git: GitWorld::started(), ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.resume(IssueNumber(142), &launch()).expect("resume");
    assert_eq!(hc_calls(), [hc_run(&base, "claude --continue")]);
}

#[test]
fn hc_a_restored_shell_is_an_exited_session_and_resume_keeps_it() {
    let base = temp_dir("hc-restored");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim { git: GitWorld::started(), shells: vec!["t4"], ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    let session = ws.observe().expect("observe").remove(&IssueNumber(142)).expect("session");
    assert_eq!(session.state, SessionState::Exited, "a shell is not Claude");
    assert!(ws.attach_plan(IssueNumber(142), AttachTo::Claude).is_err(), "attach resumes instead");
    ws.resume(IssueNumber(142), &launch()).expect("resume");
    assert_eq!(hc_calls(), ["hc rm t4".to_string(), hc_run(&base, "claude --continue")], "the shell stays");
}

#[test]
fn hc_resume_of_a_live_terminal_does_nothing() {
    let base = temp_dir("hc-resume-live");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "live", Some("working"))],
        gone: false,
        ..HcSim::default()
    };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.resume(IssueNumber(142), &launch()).expect("resume");
    assert_eq!(hc_calls(), Vec::<String>::new());
}

#[test]
fn hc_stop_kills_then_removes_and_keeps_the_worktree() {
    let base = temp_dir("hc-stop");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "live", Some("working"))],
        gone: false,
        ..HcSim::default()
    };
    let sim = Rc::new(RefCell::new(sim));
    install_hc_world(base.clone(), Rc::clone(&sim));
    ws.stop(IssueNumber(142)).expect("stop");
    assert_eq!(hc_calls(), ["hc kill t4", "hc rm t4"]);
    assert!(!fake::calls().iter().any(|c| c.contains("worktree remove")));
    sim.borrow_mut().gone = true;
    ws.stop(IssueNumber(142)).expect("a terminal that vanished is already stopped");
    assert_eq!(hc_calls()[2..], ["hc kill t4"], "not found ends the stop");
    assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
}

#[test]
fn hc_stop_ends_every_terminal_in_the_space() {
    let base = temp_dir("hc-stop-space");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "finished", None)],
        space_mates: vec!["t7"],
        ..HcSim::default()
    };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.stop(IssueNumber(142)).expect("stop");
    assert_eq!(hc_calls(), ["hc kill t4", "hc rm t4", "hc kill t7", "hc rm t7"]);
}

#[test]
fn hc_stop_of_a_finished_terminal_still_removes_it() {
    let base = temp_dir("hc-stop-finished");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim =
        HcSim { git: GitWorld::started(), terminals: vec![("t4", "finished", None)], gone: false, ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.stop(IssueNumber(142)).expect("stop");
    assert_eq!(hc_calls(), ["hc kill t4", "hc rm t4"]);
}

#[test]
fn hc_observe_prefers_a_live_terminal_over_a_newer_finished_one() {
    let base = temp_dir("hc-observe");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let terminals = vec![("t2", "live", Some("working")), ("t3", "finished", Some("done"))];
    let sim = HcSim { git: GitWorld::started(), terminals, gone: false, ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    let session = ws.observe().expect("observe").remove(&IssueNumber(142)).expect("session");
    assert_eq!(
        ws.attach_plan(IssueNumber(142), AttachTo::Claude).expect("plan"),
        AttachPlan::Suspend(["hc", "attach", "t2"].map(String::from).to_vec())
    );
    assert_eq!(session.state, SessionState::Running);
    assert_eq!(
        fake::calls().iter().find(|c| c.starts_with("hc ls")).map(String::as_str),
        Some(format!("hc ls --all --json --label dispatch.repo={}", base.join("orbit").display()).as_str())
    );
}

#[test]
fn hc_send_goes_to_the_terminal() {
    let base = temp_dir("hc-send");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "live", Some("idle"))],
        gone: false,
        ..HcSim::default()
    };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.send(IssueNumber(142), "merge it").expect("send");
    assert_eq!(hc_calls(), ["hc send --force t4 merge it"]);
}

#[test]
fn hc_send_refuses_a_finished_terminal() {
    let base = temp_dir("hc-send-dead");
    let ws = hc_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim =
        HcSim { git: GitWorld::started(), terminals: vec![("t4", "finished", None)], gone: false, ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    assert!(ws.send(IssueNumber(142), "hi").is_err());
}

#[test]
fn hc_attach_suspends_outside_the_app_and_switches_space_inside_it() {
    let base = temp_dir("hc-attach");
    let mut ws = hc_workspace(&base);
    let dir = base.join("orbit/.claude/worktrees/142");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sim = HcSim {
        git: GitWorld::started(),
        terminals: vec![("t4", "live", Some("working"))],
        gone: false,
        ..HcSim::default()
    };
    let sim = Rc::new(RefCell::new(sim));
    install_hc_world(base.clone(), Rc::clone(&sim));
    let id = IssueNumber(142);
    assert_eq!(
        ws.attach_plan(id, AttachTo::Claude).expect("plan"),
        AttachPlan::Suspend(["hc", "attach", "t4"].map(String::from).to_vec())
    );
    ws.mux = Backend::Hc(hc::Hc { place: hc::Place::App, path: None });
    let space = hc::space_id(&dir);
    assert_eq!(
        ws.attach_plan(id, AttachTo::Claude).expect("plan"),
        AttachPlan::Switch(["herdr-app", "do", "space.select", "--id", space.as_str()].map(String::from).to_vec()),
        "the space id is derived from the worktree"
    );
    sim.borrow_mut().terminals = vec![("t4", "finished", None)];
    assert!(ws.attach_plan(id, AttachTo::Claude).is_err(), "a finished terminal has nothing to attach to");
}

#[test]
fn hc_shell_runs_no_argv_and_no_dispatch_labels_and_attaches_to_the_new_terminal() {
    let base = temp_dir("hc-shell");
    let mut ws = hc_workspace(&base);
    let dir = base.join("orbit/.claude/worktrees/142");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sim = HcSim { git: GitWorld::started(), terminals: vec![("t4", "finished", None)], ..HcSim::default() };
    install_hc_world(base.clone(), Rc::new(RefCell::new(sim)));
    let id = IssueNumber(142);
    let plan = ws.attach_plan(id, AttachTo::Shell).expect("plan");
    assert_eq!(plan, AttachPlan::Suspend(["hc", "attach", "t9"].map(String::from).to_vec()));
    let space = hc::space_id(&dir);
    let calls = hc_calls();
    let run = calls.iter().find(|c| c.starts_with("hc run")).expect("a run");
    assert_eq!(
        *run,
        format!("hc run --bg --json --no-parent --no-notify --cwd {} --label herdr.space={space}", dir.display())
    );
    assert!(!run.contains("dispatch.") && !run.contains(" -- "), "{run}");

    ws.mux = Backend::Hc(hc::Hc { place: hc::Place::App, path: None });
    assert_eq!(
        ws.attach_plan(id, AttachTo::Shell).expect("plan"),
        AttachPlan::Switch(["herdr-app", "do", "space.select", "--id", space.as_str()].map(String::from).to_vec())
    );
}
