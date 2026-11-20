use super::{GitWorld, launch, req, workspace};
use crate::domain::IssueNumber;
use crate::proc::fake;
use crate::session::mux::Backend;
use crate::session::{AttachPlan, AttachTo, SessionState, Workspace, herdr};
use crate::testutil::temp_dir;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

#[expect(
    clippy::struct_excessive_bools,
    reason = "a test sim of external state: each flag is one independent fact the sim flips as the code under test runs commands"
)]
#[derive(Default)]
struct HerdrSim {
    git: GitWorld,
    workspace_open: bool,
    agent: bool,
}

fn install_herdr_world(base: PathBuf, sim: Rc<RefCell<HerdrSim>>) {
    fake::install(move |call| {
        let mut s = sim.borrow_mut();
        let dir = base.join("orbit/.claude/worktrees/142");
        if let Some(answer) = s.git.answer(call, &base) {
            answer
        } else if call.starts_with("herdr workspace list") {
            let rows = if s.workspace_open {
                serde_json::json!([{"workspace_id": "w5", "worktree": {"checkout_path": dir}}])
            } else {
                serde_json::json!([])
            };
            Ok(serde_json::json!({"result": {"workspaces": rows}}).to_string())
        } else if call.starts_with("herdr pane list") {
            let agent = s.agent.then_some("claude");
            let rows = if s.workspace_open {
                serde_json::json!([{"pane_id": "w5:p1", "workspace_id": "w5", "agent": agent, "agent_status": "idle"}])
            } else {
                serde_json::json!([])
            };
            Ok(serde_json::json!({"result": {"panes": rows}}).to_string())
        } else if call.starts_with("herdr worktree create") {
            std::fs::create_dir_all(&dir).expect("mkdir");
            s.git.added = true;
            s.workspace_open = true;
            Ok("{}".to_string())
        } else if call.starts_with("herdr worktree open") {
            s.workspace_open = true;
            let opened = serde_json::json!({"workspace": {"workspace_id": "w5"}, "root_pane": {"pane_id": "w5:p1"}});
            Ok(serde_json::json!({"result": opened}).to_string())
        } else if call.starts_with("herdr pane run") {
            s.agent = true;
            Ok(String::new())
        } else if call.starts_with("herdr pane get") {
            Ok(serde_json::json!({"result": {"pane": {"agent": s.agent.then_some("claude")}}}).to_string())
        } else {
            Ok(String::new())
        }
    });
}

fn herdr_workspace(base: &std::path::Path) -> Workspace {
    let mut ws = workspace(base);
    let place = herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside herdr");
    ws.mux = Backend::Herdr(place);
    ws
}

fn herdr_calls() -> Vec<String> {
    fake::calls()
        .into_iter()
        .filter(|c| {
            ["git fetch", "git worktree add", "git config branch", "herdr worktree", "herdr pane run"]
                .iter()
                .any(|p| c.starts_with(p))
        })
        .collect()
}

#[test]
fn herdr_start_of_a_new_branch_creates_the_worktree_through_herdr_and_types_claude_into_its_pane() {
    let base = temp_dir("herdr-start");
    let ws = herdr_workspace(&base);
    install_herdr_world(base.clone(), Rc::new(RefCell::new(HerdrSim::default())));
    let out = ws.start(&req(), &launch()).expect("start");
    assert_eq!(out.session.state, SessionState::NeedsYou, "herdr reports an idle agent as waiting");
    let dir = base.join("orbit/.claude/worktrees/142");
    assert_eq!(
        herdr_calls(),
        [
            "git fetch origin main".to_string(),
            format!(
                "herdr worktree create --workspace w1 --branch fix/142-x --base origin/main --path {} --label #142 --no-focus",
                dir.display()
            ),
            "git config branch.fix/142-x.dispatch-issue 142".to_string(),
            format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display()),
        ],
        "the workspace that create opened is reused, and git does not add the worktree"
    );
    let exclude = base.join("orbit/.git/info/exclude");
    assert_eq!(std::fs::read_to_string(&exclude).expect("exclude written"), ".claude/worktrees/\n");
    let before = fake::calls().len();
    ws.start(&req(), &launch()).expect("second start");
    assert!(
        !fake::calls()[before..].iter().any(|c| c.starts_with("herdr pane run")),
        "a second start must not type again"
    );
}

#[test]
fn herdr_start_of_an_existing_branch_leaves_out_the_base() {
    let base = temp_dir("herdr-existing");
    let ws = herdr_workspace(&base);
    let sim = HerdrSim { git: GitWorld { branch_exists: true, ..GitWorld::default() }, ..HerdrSim::default() };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.start(&req(), &launch()).expect("start");
    let dir = base.join("orbit/.claude/worktrees/142");
    assert_eq!(
        herdr_calls()[0],
        format!(
            "herdr worktree create --workspace w1 --branch fix/142-x --path {} --label #142 --no-focus",
            dir.display()
        )
    );
}

#[test]
fn herdr_start_writes_a_multi_line_prompt_into_the_launch_script() {
    let base = temp_dir("herdr-prompt");
    let ws = herdr_workspace(&base);
    install_herdr_world(base.clone(), Rc::new(RefCell::new(HerdrSim::default())));
    let mut r = req();
    r.prompt = "line one\nit's line two".to_string();
    ws.start(&r, &launch()).expect("start");
    let file = base.join("orbit/.git/dispatch/launch/142.sh");
    let script = std::fs::read_to_string(&file).expect("script");
    assert!(script.ends_with("'line one\nit'\\''s line two'\n"), "{script}");
    let run = herdr_calls().pop().expect("a call");
    assert_eq!(run, format!("herdr pane run w5:p1 '{}'", file.display()));
}

#[test]
fn herdr_start_reopens_a_closed_workspace_for_an_existing_worktree() {
    let base = temp_dir("herdr-reopen");
    let ws = herdr_workspace(&base);
    let dir = base.join("orbit/.claude/worktrees/142");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), ..HerdrSim::default() };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.start(&req(), &launch()).expect("start");
    assert_eq!(
        herdr_calls(),
        [
            format!("herdr worktree open --workspace w1 --path {} --label #142 --no-focus", dir.display()),
            format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display()),
        ]
    );
}

#[test]
fn herdr_resume_runs_claude_continue_in_the_dead_pane_without_the_tmux_hooks() {
    let base = temp_dir("herdr-resume");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, ..HerdrSim::default() };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    let mut l = launch();
    l.detect_waiting = true;
    ws.resume(IssueNumber(142), &l).expect("resume");
    assert_eq!(
        herdr_calls(),
        [format!("herdr pane run w5:p1 '{}'", base.join("orbit/.git/dispatch/launch/142.sh").display())]
    );
    let script = std::fs::read_to_string(base.join("orbit/.git/dispatch/launch/142.sh")).expect("script");
    assert_eq!(script, "#!/bin/sh\nexec 'claude' '--continue'\n");
}

#[test]
fn herdr_stop_closes_the_workspace_and_keeps_the_checkout() {
    let base = temp_dir("herdr-stop");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, agent: true };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.stop(IssueNumber(142)).expect("stop");
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c == "herdr workspace close w5"));
    assert!(!calls.iter().any(|c| c.contains("worktree remove") || c.contains("--group")));
    assert!(ws.stop(IssueNumber(999)).is_ok(), "no session is already stopped");
}

#[test]
fn herdr_send_goes_to_the_agent_pane() {
    let base = temp_dir("herdr-send");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, agent: true };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    ws.send(IssueNumber(142), "merge it").expect("send");
    let calls = fake::calls();
    assert_eq!(calls[calls.len() - 2..], ["herdr pane send-text w5:p1 merge it", "herdr pane send-keys w5:p1 enter"]);
}

#[test]
fn herdr_send_refuses_a_pane_without_an_agent() {
    let base = temp_dir("herdr-send-dead");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, ..HerdrSim::default() };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    assert!(ws.send(IssueNumber(142), "hi").is_err());
}

#[test]
fn herdr_attach_focuses_the_workspace() {
    let base = temp_dir("herdr-attach");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = Rc::new(RefCell::new(HerdrSim { git: GitWorld::started(), workspace_open: true, agent: true }));
    install_herdr_world(base.clone(), Rc::clone(&sim));
    assert_eq!(
        ws.attach_plan(IssueNumber(142), AttachTo::Claude).expect("plan"),
        AttachPlan::Switch(["herdr", "workspace", "focus", "w5"].map(String::from).to_vec())
    );
    sim.borrow_mut().agent = false;
    assert!(
        ws.attach_plan(IssueNumber(142), AttachTo::Claude).is_err(),
        "a pane without an agent has nothing to attach to"
    );
}

#[test]
fn a_detached_herdr_attaches_by_focusing_the_workspace_then_opening_herdr() {
    let base = temp_dir("herdr-attach-detached");
    let mut ws = herdr_workspace(&base);
    ws.mux = Backend::Herdr(herdr::Place::detect(None, None, "orbit").expect("detached"));
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, agent: true };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    let script = r#"herdr workspace focus "$1" >/dev/null && exec herdr"#;
    assert_eq!(
        ws.attach_plan(IssueNumber(142), AttachTo::Claude).expect("plan"),
        AttachPlan::Suspend(["sh", "-c", script, "sh", "w5"].map(String::from).to_vec())
    );
    assert!(herdr_calls().is_empty(), "planning only reads");
}

#[test]
fn herdr_shell_opens_a_tab_in_the_sessions_workspace() {
    let base = temp_dir("herdr-shell-tab");
    let ws = herdr_workspace(&base);
    let dir = base.join("orbit/.claude/worktrees/142");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: true, agent: false };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    let plan = ws.attach_plan(IssueNumber(142), AttachTo::Shell).expect("plan");
    assert_eq!(plan, AttachPlan::Switch(["herdr", "workspace", "focus", "w5"].map(String::from).to_vec()));
    let calls = fake::calls();
    assert_eq!(
        calls.last().map(String::as_str),
        Some(format!("herdr tab create --workspace w5 --cwd {} --label #142 shell --focus", dir.display()).as_str())
    );
}

#[test]
fn herdr_shell_without_a_workspace_opens_the_worktree_and_uses_its_root_pane() {
    let base = temp_dir("herdr-shell-open");
    let ws = herdr_workspace(&base);
    std::fs::create_dir_all(base.join("orbit/.claude/worktrees/142")).expect("mkdir");
    let sim = HerdrSim { git: GitWorld::started(), workspace_open: false, agent: false };
    install_herdr_world(base.clone(), Rc::new(RefCell::new(sim)));
    let plan = ws.attach_plan(IssueNumber(142), AttachTo::Shell).expect("plan");
    assert_eq!(plan, AttachPlan::Switch(["herdr", "workspace", "focus", "w5"].map(String::from).to_vec()));
    let calls = fake::calls();
    assert!(calls.iter().any(|c| c.starts_with("herdr worktree open")), "{calls:?}");
    assert!(!calls.iter().any(|c| c.starts_with("herdr tab create") || c.starts_with("herdr pane run")), "{calls:?}");
}
