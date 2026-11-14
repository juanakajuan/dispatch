//! herdr adapter. Private to `session`. Checked against herdr 0.9.5.
//! Every command prints `{"id":..,"result":{..}}` on success, and
//! `{"error":{"code":..,"message":..}}` on stderr with exit 1 on failure.
//! Replies are parsed into serde structs at this boundary, so the rest of
//! `session` never sees herdr's JSON.
//!
//! herdr owns the worktree checkout: `worktree create` makes the git worktree
//! and a linked workspace for it in one call. dispatch runs Claude by typing a
//! command line into that workspace's pane, because `herdr agent start`
//! refuses a multi-line prompt.

use super::{WindowId, WindowObs};
use crate::domain::{BranchName, IssueNumber, WorktreePath};
use crate::proc::{self, ProcError};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// herdr's workspace id, for example `w3`. Opaque: taken from herdr's JSON,
/// never derived from a pane id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// herdr's pane id, for example `w3:p1`. Opaque, like `WorkspaceId`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct PaneId(String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The pane that runs Claude, and the workspace that holds it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneRef {
    pub workspace: WorkspaceId,
    pub pane: PaneId,
}

impl PaneRef {
    #[cfg(test)]
    pub(super) fn new(workspace: &str, pane: &str) -> PaneRef {
        PaneRef { workspace: WorkspaceId(workspace.to_string()), pane: PaneId(pane.to_string()) }
    }
}

/// Startup facts for a dispatch that runs inside herdr.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Place {
    /// The workspace dispatch itself runs in. herdr makes each issue's
    /// worktree from this workspace's repository.
    pub source: WorkspaceId,
}

impl Place {
    /// `Ok(None)` when dispatch does not run inside herdr. `HERDR_ENV=1` with
    /// no workspace id is a setup error, not a fall back to tmux.
    pub(super) fn detect(herdr_env: Option<&str>, workspace_id: Option<&str>) -> Result<Option<Place>, String> {
        if herdr_env != Some("1") {
            return Ok(None);
        }
        match workspace_id.filter(|id| !id.is_empty()) {
            Some(id) => Ok(Some(Place { source: WorkspaceId(id.to_string()) })),
            None => Err("HERDR_ENV is set but HERDR_WORKSPACE_ID is not".to_string()),
        }
    }
}

// ---- JSON boundary ---------------------------------------------------------

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

#[derive(Default, Deserialize)]
struct WorkspaceList {
    workspaces: Vec<WorkspaceRow>,
}

#[derive(Deserialize)]
struct WorkspaceRow {
    workspace_id: WorkspaceId,
    worktree: Option<WorktreeRow>,
}

#[derive(Deserialize)]
struct WorktreeRow {
    checkout_path: Option<String>,
}

#[derive(Default, Deserialize)]
struct PaneList {
    panes: Vec<PaneRow>,
}

#[derive(Deserialize)]
struct PaneRow {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
    agent: Option<String>,
    #[serde(default)]
    agent_status: AgentStatus,
}

/// herdr's own read of what the agent in a pane does. A status that a later
/// herdr adds counts as `Unknown`, so one new word does not hide every session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    /// The agent stopped and the user is next.
    fn waiting(self) -> bool {
        matches!(self, AgentStatus::Blocked | AgentStatus::Done | AgentStatus::Idle)
    }
}

#[derive(Deserialize)]
struct Opened {
    workspace: OpenedWorkspace,
    root_pane: OpenedPane,
}

#[derive(Deserialize)]
struct OpenedWorkspace {
    workspace_id: WorkspaceId,
}

#[derive(Deserialize)]
struct OpenedPane {
    pane_id: PaneId,
}

#[derive(Deserialize)]
struct PaneGet {
    pane: PaneAgent,
}

#[derive(Deserialize)]
struct PaneAgent {
    agent: Option<String>,
}

fn herdr_raw<I, S>(args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("herdr", args, Path::new("."))
}

fn herdr<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    herdr_raw(args).map_err(|e| e.to_string())
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str::<Reply<T>>(text).map(|r| r.result).map_err(|e| format!("herdr printed unexpected JSON: {e}"))
}

fn call<T: DeserializeOwned>(args: &[&str]) -> Result<T, String> {
    parse(&herdr(args)?)
}

fn is_no_server(stderr: &str) -> bool {
    stderr.contains("server_not_running")
}

// ---- observe ---------------------------------------------------------------

/// `Default` stands for a herdr that is not running: no workspaces, no panes.
fn query<T: DeserializeOwned + Default>(args: &[&str]) -> Result<T, String> {
    match herdr_raw(args) {
        Ok(text) => parse(&text),
        Err(e) if is_no_server(e.stderr()) => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}

pub(super) fn list_windows() -> Result<Vec<WindowObs>, String> {
    let workspaces: WorkspaceList = query(&["workspace", "list"])?;
    let panes: PaneList = query(&["pane", "list"])?;
    Ok(window_obs(&workspaces, &panes))
}

/// One window per workspace that has a worktree checkout. The window is the
/// pane that hosts an agent, else the first pane. A checkout that is gone from
/// disk is skipped, like a tmux tag whose directory is gone.
fn window_obs(workspaces: &WorkspaceList, panes: &PaneList) -> Vec<WindowObs> {
    workspaces
        .workspaces
        .iter()
        .filter_map(|ws| {
            let checkout = ws.worktree.as_ref()?.checkout_path.as_deref()?;
            let worktree = WorktreePath::from_observed(checkout)?;
            let own: Vec<&PaneRow> = panes.panes.iter().filter(|p| p.workspace_id == ws.workspace_id).collect();
            let pane = own.iter().find(|p| p.agent.is_some()).or(own.first())?;
            Some(WindowObs {
                id: WindowId::Herdr(PaneRef { workspace: ws.workspace_id.clone(), pane: pane.pane_id.clone() }),
                worktree,
                dead: pane.agent.is_none(),
                waiting: pane.agent_status.waiting(),
            })
        })
        .collect()
}

// ---- start -----------------------------------------------------------------

fn label(issue: IssueNumber) -> String {
    format!("#{}", issue.0)
}

/// `herdr worktree create --workspace <source> --branch <b> [--base <from>]
/// --path <dir> --label #<n> --no-focus`. herdr makes the git worktree and
/// opens a linked workspace for it. `from` is `None` for a branch that exists,
/// so `--base` is left out.
pub(super) fn create_worktree(
    place: &Place,
    dir: &Path,
    branch: &BranchName,
    from: Option<&str>,
    issue: IssueNumber,
) -> Result<(), String> {
    let (dir, label) = (dir.to_string_lossy(), label(issue));
    let mut args = vec!["worktree", "create", "--workspace", place.source.as_str(), "--branch", branch.as_str()];
    args.extend(from.into_iter().flat_map(|base| ["--base", base]));
    args.extend(["--path", dir.as_ref(), "--label", label.as_str(), "--no-focus"]);
    herdr(args).map(|_| ())
}

/// The pane to run Claude in. `worktree create` already opened a workspace for
/// a new start, so look for an open one first and open a second only when the
/// worktree has none (a reopen after the workspace was closed).
pub(super) fn open_window(
    place: &Place,
    dir: &WorktreePath,
    issue: IssueNumber,
    argv: &[String],
    prompt_file: &Path,
) -> Result<(), String> {
    let open = list_windows()?.into_iter().find(|o| o.worktree == *dir);
    let pane = match open {
        Some(WindowObs { id: WindowId::Herdr(pane), .. }) => pane,
        _ => open_worktree(place, dir, issue)?,
    };
    launch(&pane, argv, prompt_file)
}

/// `herdr worktree open --workspace <source> --path <dir> --label #<n> --no-focus`.
fn open_worktree(place: &Place, dir: &WorktreePath, issue: IssueNumber) -> Result<PaneRef, String> {
    let (dir, label) = (dir.as_path().to_string_lossy(), label(issue));
    let args = [
        "worktree",
        "open",
        "--workspace",
        place.source.as_str(),
        "--path",
        dir.as_ref(),
        "--label",
        label.as_str(),
        "--no-focus",
    ];
    let opened: Opened = call(&args)?;
    Ok(PaneRef { workspace: opened.workspace.workspace_id, pane: opened.root_pane.pane_id })
}

/// How long `launch` waits for herdr to see the agent, so the observation
/// right after a start does not show "exited". A slow start is not an error:
/// the next tick catches up.
const AGENT_WAIT: Duration = Duration::from_secs(10);
const AGENT_POLL: Duration = Duration::from_millis(250);

/// Types `argv` as one command line into the pane's shell and presses Enter.
pub(super) fn launch(pane: &PaneRef, argv: &[String], prompt_file: &Path) -> Result<(), String> {
    write_prompt(argv, prompt_file)?;
    herdr(["pane", "run", pane.pane.as_str(), command_line(argv, prompt_file).as_str()])?;
    wait_for_agent(pane, AGENT_WAIT);
    Ok(())
}

fn wait_for_agent(pane: &PaneRef, budget: Duration) {
    let start = Instant::now();
    loop {
        let seen = call::<PaneGet>(&["pane", "get", pane.pane.as_str()]).is_ok_and(|g| g.pane.agent.is_some());
        if seen || start.elapsed() >= budget {
            return;
        }
        std::thread::sleep(AGENT_POLL);
    }
}

/// `<git common dir>/dispatch/prompts/<issue>.txt`, overwritten on each start.
pub(super) fn prompt_file(git_common_dir: &Path, issue: IssueNumber) -> PathBuf {
    git_common_dir.join("dispatch").join("prompts").join(format!("{}.txt", issue.0))
}

/// The shell would break a multi-line prompt that is typed as one line, so the
/// text goes to a file and `command_line` reads it back. One such argument fits
/// one file.
fn write_prompt(argv: &[String], file: &Path) -> Result<(), String> {
    let mut multi_line = argv.iter().filter(|a| a.contains('\n'));
    let Some(text) = multi_line.next() else { return Ok(()) };
    if multi_line.next().is_some() {
        return Err("herdr can launch only one multi-line argument; fold the others into the prompt".to_string());
    }
    let err = |e: std::io::Error| format!("cannot write {}: {e}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    std::fs::write(file, text).map_err(err)
}

/// Quotes each argument POSIX-style, so zsh, bash, and fish read the same
/// words. An argument with a newline becomes `"$(cat '<prompt_file>')"`.
fn command_line(argv: &[String], prompt_file: &Path) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    argv.iter()
        .map(
            |a| {
                if a.contains('\n') {
                    format!("\"$(cat {})\"", quote(&prompt_file.to_string_lossy()))
                } else {
                    quote(a)
                }
            },
        )
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- stop, send, tail ------------------------------------------------------

/// `herdr workspace close <ws>`. The checkout and the branch stay. A workspace
/// that is already gone counts as closed.
pub(super) fn close_workspace(workspace: &WorkspaceId) -> Result<(), String> {
    match herdr_raw(["workspace", "close", workspace.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if e.stderr().contains("workspace_not_found") || is_no_server(e.stderr()) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Types `text` into the pane, then presses Enter: two calls, like tmux, so
/// Claude reads the text as typing and Enter as a submit.
pub(super) fn send_line(pane: &PaneRef, text: &str) -> Result<(), String> {
    herdr(["pane", "send-text", pane.pane.as_str(), text])?;
    herdr(["pane", "send-keys", pane.pane.as_str(), "enter"]).map(|_| ())
}

pub(super) fn read_tail(pane: &PaneRef, lines: u16) -> Result<Vec<String>, String> {
    let n = lines.to_string();
    let args = ["pane", "read", pane.pane.as_str(), "--source", "recent-unwrapped", "--lines", n.as_str()];
    herdr(args).map(|text| super::text_lines(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;
    use crate::testutil::temp_dir;

    const WORKSPACES: &str = r##"{"id":"cli:workspace:list","result":{"type":"workspace_list","workspaces":[
        {"workspace_id":"w1","label":"dispatch","worktree":null},
        {"workspace_id":"w2","label":"#7","worktree":{"checkout_path":"CHECKOUT_A","branch":"fix/7-x"}},
        {"workspace_id":"w3","label":"#8","worktree":{"checkout_path":"CHECKOUT_B"}},
        {"workspace_id":"w4","label":"#9","worktree":{"checkout_path":"/definitely/not/here"}}]}}"##;

    const PANES: &str = r#"{"id":"cli:pane:list","result":{"type":"pane_list","panes":[
        {"pane_id":"w1:p1","workspace_id":"w1","agent":null,"agent_status":"unknown"},
        {"pane_id":"w2:p1","workspace_id":"w2","agent":null,"agent_status":"unknown"},
        {"pane_id":"w2:p2","workspace_id":"w2","agent":"claude","agent_status":"blocked"},
        {"pane_id":"w3:p1","workspace_id":"w3","agent":null,"agent_status":"unknown"},
        {"pane_id":"w4:p1","workspace_id":"w4","agent":"claude","agent_status":"working"}]}}"#;

    fn fixture(dirs: &[&Path; 2]) -> (WorkspaceList, PaneList) {
        let text = WORKSPACES
            .replace("CHECKOUT_A", &dirs[0].to_string_lossy())
            .replace("CHECKOUT_B", &dirs[1].to_string_lossy());
        (parse(&text).expect("workspaces"), parse(PANES).expect("panes"))
    }

    fn pane() -> PaneRef {
        PaneRef::new("w3", "w3:p1")
    }

    #[test]
    fn workspaces_become_windows_by_their_agent_pane() {
        let base = temp_dir("herdr-obs");
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir(&a).expect("mkdir");
        std::fs::create_dir(&b).expect("mkdir");
        let (workspaces, panes) = fixture(&[&a, &b]);
        let got = window_obs(&workspaces, &panes);
        let summary: Vec<_> = got.iter().map(|o| (o.id.clone(), o.dead, o.waiting)).collect();
        assert_eq!(
            summary,
            [
                (WindowId::Herdr(PaneRef::new("w2", "w2:p2")), false, true),
                (WindowId::Herdr(PaneRef::new("w3", "w3:p1")), true, false),
            ],
            "no worktree and a missing checkout are skipped; the agent pane wins; no agent is dead"
        );
        assert!(got[0].worktree.as_path().ends_with("a"));
    }

    #[test]
    fn only_blocked_done_and_idle_wait() {
        let waits = |status: &str| {
            let row = format!(r#"{{"pane_id":"p","workspace_id":"w","agent":"claude","agent_status":"{status}"}}"#);
            serde_json::from_str::<PaneRow>(&row).expect("row").agent_status.waiting()
        };
        assert_eq!(
            ["idle", "working", "blocked", "done", "unknown", "from-the-future"].map(waits),
            [true, false, true, true, false, false]
        );
    }

    #[test]
    fn a_herdr_that_is_not_running_has_no_windows() {
        let error = r#"{"id":"cli:workspace:list","error":{"code":"server_not_running","message":"no herdr server"}}"#;
        fake::install(move |_| Err(fake::fail("herdr", 1, error)));
        assert!(list_windows().expect("ok").is_empty());
        fake::install(|_| Err(fake::fail("herdr", 1, r#"{"error":{"code":"internal"}}"#)));
        assert!(list_windows().is_err());
        fake::install(|_| Ok("not json".to_string()));
        assert!(list_windows().is_err());
    }

    #[test]
    fn place_needs_herdr_env_and_a_workspace() {
        assert_eq!(Place::detect(None, Some("w1")), Ok(None));
        assert_eq!(Place::detect(Some("0"), Some("w1")), Ok(None));
        assert_eq!(Place::detect(Some("1"), Some("w1")), Ok(Some(Place { source: WorkspaceId("w1".to_string()) })));
        assert!(Place::detect(Some("1"), None).is_err());
        assert!(Place::detect(Some("1"), Some("")).is_err());
    }

    #[test]
    fn command_line_quotes_each_argument_and_reads_multi_line_ones_from_the_file() {
        let file = Path::new("/g/dispatch/prompts/7.txt");
        let argv = ["claude", "--permission-mode", "plan", "it's $HOME `x`;", "", "a\nb"].map(String::from);
        assert_eq!(
            command_line(&argv, file),
            r#"'claude' '--permission-mode' 'plan' 'it'\''s $HOME `x`;' '' "$(cat '/g/dispatch/prompts/7.txt')""#
        );
    }

    #[test]
    fn a_multi_line_argument_goes_to_the_prompt_file_and_a_second_one_is_refused() {
        let file = prompt_file(&temp_dir("herdr-prompt"), IssueNumber(7));
        write_prompt(&["claude".to_string(), "one\ntwo".to_string()], &file).expect("written");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "one\ntwo");
        write_prompt(&["claude".to_string(), "new\ntext".to_string()], &file).expect("overwritten");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "new\ntext");
        assert!(write_prompt(&["a\nb".to_string(), "c\nd".to_string()], &file).is_err());
    }

    #[test]
    fn launch_types_the_line_then_waits_for_the_agent() {
        let file = prompt_file(&temp_dir("herdr-launch"), IssueNumber(7));
        let polls = std::cell::Cell::new(0);
        fake::install(move |call| {
            Ok(if call.starts_with("herdr pane get") {
                polls.set(polls.get() + 1);
                let agent = if polls.get() < 2 { "null" } else { "\"claude\"" };
                format!(r#"{{"result":{{"pane":{{"agent":{agent}}}}}}}"#)
            } else {
                String::new()
            })
        });
        launch(&pane(), &["claude".to_string(), "p q".to_string()], &file).expect("ok");
        assert_eq!(
            fake::calls(),
            ["herdr pane run w3:p1 'claude' 'p q'", "herdr pane get w3:p1", "herdr pane get w3:p1"]
        );
    }

    #[test]
    fn waiting_for_an_agent_that_never_shows_gives_up_quietly() {
        fake::install(|_| Ok(r#"{"result":{"pane":{"agent":null}}}"#.to_string()));
        wait_for_agent(&pane(), Duration::ZERO);
        assert_eq!(fake::calls(), ["herdr pane get w3:p1"]);
        fake::install(|_| Err(fake::fail("herdr", 1, "boom")));
        wait_for_agent(&pane(), Duration::ZERO);
    }

    #[test]
    fn closing_a_workspace_that_is_gone_is_fine() {
        let id = WorkspaceId("w3".to_string());
        fake::install(|_| Ok(String::new()));
        close_workspace(&id).expect("ok");
        assert_eq!(fake::calls(), ["herdr workspace close w3"]);
        fake::install(|_| Err(fake::fail("herdr", 1, r#"{"error":{"code":"workspace_not_found"}}"#)));
        assert!(close_workspace(&id).is_ok());
        fake::install(|_| Err(fake::fail("herdr", 1, r#"{"error":{"code":"internal"}}"#)));
        assert!(close_workspace(&id).is_err());
    }

    #[test]
    fn send_types_the_text_then_presses_enter() {
        fake::install(|_| Ok(String::new()));
        send_line(&pane(), "merge it").expect("ok");
        assert_eq!(fake::calls(), ["herdr pane send-text w3:p1 merge it", "herdr pane send-keys w3:p1 enter"]);
    }

    #[test]
    fn tail_reads_unwrapped_text_and_drops_trailing_blank_lines() {
        fake::install(|_| Ok("one\n\ntwo\n\n  \n".to_string()));
        assert_eq!(read_tail(&pane(), 10).expect("ok"), ["one", "", "two"]);
        assert_eq!(fake::calls(), ["herdr pane read w3:p1 --source recent-unwrapped --lines 10"]);
    }
}
