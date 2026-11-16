//! dispatch runs Claude by typing a launcher script's path into the
//! workspace's pane, because `herdr agent start` refuses a multi-line prompt.

mod mux;
mod wire;

use super::mux::Observed;
use super::start_argv::herdr_worktree_create_args;
use crate::domain::{BranchName, IssueNumber, WorktreePath};
use crate::proc::{self, ProcError};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wire::{Created, Opened, PaneGet, PaneList, WorkspaceList, parse, window_obs};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct WorkspaceId(String);

impl WorkspaceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct PaneId(String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    Inside { source: WorkspaceId },
    Detached { label: String },
}

impl Place {
    pub(super) fn detect(
        herdr_env: Option<&str>,
        workspace_id: Option<&str>,
        repo_dir_name: &str,
    ) -> Result<Place, String> {
        if herdr_env != Some("1") {
            return Ok(Place::Detached { label: format!("dispatch-{repo_dir_name}") });
        }
        match workspace_id.filter(|id| !id.is_empty()) {
            Some(id) => Ok(Place::Inside { source: WorkspaceId(id.to_string()) }),
            None => Err("HERDR_ENV is set but HERDR_WORKSPACE_ID is not".to_string()),
        }
    }
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

fn call<T: DeserializeOwned>(args: &[&str]) -> Result<T, String> {
    parse(&herdr(args)?)
}

fn is_no_server(stderr: &str) -> bool {
    stderr.contains("server_not_running")
}

fn query<T: DeserializeOwned + Default>(args: &[&str]) -> Result<T, String> {
    match herdr_raw(args) {
        Ok(text) => parse(&text),
        Err(e) if is_no_server(e.stderr()) => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn list_windows() -> Result<Vec<Observed<PaneRef>>, String> {
    let workspaces: WorkspaceList = query(&["workspace", "list"])?;
    let panes: PaneList = query(&["pane", "list"])?;
    Ok(window_obs(&workspaces, &panes))
}

pub(super) fn label(issue: IssueNumber) -> String {
    format!("#{}", issue.0)
}

const SERVER_WAIT: Duration = Duration::from_secs(5);
const SERVER_POLL: Duration = Duration::from_millis(100);

fn workspaces_with_server(budget: Duration) -> Result<WorkspaceList, String> {
    let list = || herdr_raw(["workspace", "list"]);
    match list() {
        Ok(text) => return parse(&text),
        Err(e) if is_no_server(e.stderr()) => {}
        Err(e) => return Err(e.to_string()),
    }
    proc::spawn_detached("herdr", ["server"], Path::new(".")).map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        std::thread::sleep(SERVER_POLL);
        match list() {
            Ok(text) => return parse(&text),
            Err(e) if is_no_server(e.stderr()) => {}
            Err(e) => return Err(e.to_string()),
        }
        if start.elapsed() >= budget {
            return Err(format!("herdr did not start within {budget:?}"));
        }
    }
}

fn source(place: &Place, repo_root: &Path) -> Result<WorkspaceId, String> {
    source_within(place, repo_root, SERVER_WAIT)
}

fn source_within(place: &Place, repo_root: &Path, budget: Duration) -> Result<WorkspaceId, String> {
    let label = match place {
        Place::Inside { source } => return Ok(source.clone()),
        Place::Detached { label } => label,
    };
    let known = workspaces_with_server(budget)?.workspaces;
    if let Some(row) = known.into_iter().find(|ws| ws.label.as_deref() == Some(label.as_str())) {
        return Ok(row.workspace_id);
    }
    let root = repo_root.to_string_lossy();
    let created: Created =
        call(&["workspace", "create", "--cwd", root.as_ref(), "--label", label.as_str(), "--no-focus"])?;
    Ok(created.workspace.workspace_id)
}

fn create_worktree(
    place: &Place,
    repo_root: &Path,
    dir: &Path,
    branch: &BranchName,
    from: Option<&str>,
    issue: IssueNumber,
) -> Result<(), String> {
    let source = source(place, repo_root)?;
    let args = herdr_worktree_create_args(source.as_str(), &dir.to_string_lossy(), branch.as_str(), from, issue);
    herdr(args).map(|_| ())
}

/// `herdr worktree create` already opens a workspace on the new checkout.
fn open_window(
    place: &Place,
    repo_root: &Path,
    dir: &WorktreePath,
    issue: IssueNumber,
    argv: &[String],
    script: &Path,
) -> Result<(), String> {
    let pane = match list_windows()?.into_iter().find(|o| o.worktree == *dir) {
        Some(open) => open.window,
        None => open_worktree(&source(place, repo_root)?, dir, issue)?,
    };
    launch(&pane, argv, script)
}

fn open_worktree(source: &WorkspaceId, dir: &WorktreePath, issue: IssueNumber) -> Result<PaneRef, String> {
    let (dir, label) = (dir.as_path().to_string_lossy(), label(issue));
    let args = [
        "worktree",
        "open",
        "--workspace",
        source.as_str(),
        "--path",
        dir.as_ref(),
        "--label",
        label.as_str(),
        "--no-focus",
    ];
    let opened: Opened = call(&args)?;
    Ok(PaneRef { workspace: opened.workspace.workspace_id, pane: opened.root_pane.pane_id })
}

const AGENT_WAIT: Duration = Duration::from_secs(10);
const AGENT_POLL: Duration = Duration::from_millis(250);

fn launch(pane: &PaneRef, argv: &[String], script: &Path) -> Result<(), String> {
    write_script(argv, script)?;
    herdr(["pane", "run", pane.pane.as_str(), typed_path(script)?.as_str()])?;
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

fn script_file(git_common_dir: &Path, issue: IssueNumber) -> PathBuf {
    git_common_dir.join("dispatch").join("launch").join(format!("{}.sh", issue.0))
}

fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}

/// /bin/sh parses the script, so POSIX quoting is exact. Typing the same
/// quotes into the pane's shell is not: fish reads `\` inside single quotes
/// as an escape.
fn script_text(argv: &[String]) -> String {
    let words: Vec<String> = argv.iter().map(|a| quote(a)).collect();
    format!("#!/bin/sh\nexec {}\n", words.join(" "))
}

fn write_script(argv: &[String], file: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let err = |e: std::io::Error| format!("cannot write {}: {e}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    std::fs::write(file, script_text(argv)).map_err(err)?;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).map_err(err)
}

/// Every shell reads a single-quoted word the same way unless it holds `'`,
/// `\`, or a newline.
fn typed_path(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    if text.contains(['\'', '\\', '\n']) {
        return Err(format!("cannot launch from {text}: the path has a quote, a backslash, or a newline"));
    }
    Ok(format!("'{text}'"))
}

/// The checkout and the branch stay.
fn close_workspace(workspace: &WorkspaceId) -> Result<(), String> {
    match herdr_raw(["workspace", "close", workspace.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if e.stderr().contains("workspace_not_found") || is_no_server(e.stderr()) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Types `text` into the pane, then presses Enter: two calls, like tmux, so
/// Claude reads the text as typing and Enter as a submit.
fn send_line(pane: &PaneRef, text: &str) -> Result<(), String> {
    herdr(["pane", "send-text", pane.pane.as_str(), text])?;
    herdr(["pane", "send-keys", pane.pane.as_str(), "enter"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;
    use crate::testutil::temp_dir;

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
    fn place_is_inside_with_herdr_env_and_detached_without() {
        let inside = Place::Inside { source: WorkspaceId("w1".to_string()) };
        let detached = Place::Detached { label: "dispatch-orbit".to_string() };
        assert_eq!(Place::detect(None, Some("w1"), "orbit"), Ok(detached.clone()));
        assert_eq!(Place::detect(Some("0"), Some("w1"), "orbit"), Ok(detached));
        assert_eq!(Place::detect(Some("1"), Some("w1"), "orbit"), Ok(inside));
        assert!(Place::detect(Some("1"), None, "orbit").is_err());
        assert!(Place::detect(Some("1"), Some(""), "orbit").is_err());
    }

    fn pane() -> PaneRef {
        PaneRef::new("w3", "w3:p1")
    }

    const ROOT: &str = "/w/orbit";

    fn detached() -> Place {
        Place::Detached { label: "dispatch-orbit".to_string() }
    }

    fn list_reply(rows: &str) -> String {
        format!(r#"{{"result":{{"workspaces":{rows}}}}}"#)
    }

    const NO_SERVER: &str = r#"{"error":{"code":"server_not_running","message":"no herdr server"}}"#;

    #[test]
    fn an_inside_source_is_used_with_no_call() {
        fake::install(|_| Ok(String::new()));
        let place = Place::Inside { source: WorkspaceId("w1".to_string()) };
        assert_eq!(source(&place, Path::new(ROOT)).expect("ok"), WorkspaceId("w1".to_string()));
        assert!(fake::calls().is_empty());
    }

    #[test]
    fn a_detached_source_is_found_by_its_label() {
        fake::install(|_| {
            Ok(list_reply(
                r#"[{"workspace_id":"w1","label":"other"},{"workspace_id":"w2","label":"dispatch-orbit"},{"workspace_id":"w3","label":null}]"#,
            ))
        });
        assert_eq!(source(&detached(), Path::new(ROOT)).expect("ok"), WorkspaceId("w2".to_string()));
        assert_eq!(fake::calls(), ["herdr workspace list"]);
    }

    #[test]
    fn a_missing_detached_source_is_created_on_the_repo_root_without_focus() {
        fake::install(|call| {
            Ok(if call.starts_with("herdr workspace create") {
                r#"{"result":{"workspace":{"workspace_id":"w9"}}}"#.to_string()
            } else {
                list_reply(r#"[{"workspace_id":"w1","label":"other"}]"#)
            })
        });
        assert_eq!(source(&detached(), Path::new(ROOT)).expect("ok"), WorkspaceId("w9".to_string()));
        assert_eq!(
            fake::calls(),
            ["herdr workspace list", "herdr workspace create --cwd /w/orbit --label dispatch-orbit --no-focus"]
        );
    }

    #[test]
    fn a_start_with_no_server_spawns_one_detached_then_polls_until_it_answers() {
        let polls = std::cell::Cell::new(0);
        fake::install(move |call| {
            if call.starts_with("herdr workspace create") {
                return Ok(r#"{"result":{"workspace":{"workspace_id":"w1"}}}"#.to_string());
            }
            if call.starts_with("spawn") {
                return Ok(String::new());
            }
            polls.set(polls.get() + 1);
            if polls.get() < 3 { Err(fake::fail("herdr", 1, NO_SERVER)) } else { Ok(list_reply("[]")) }
        });
        assert_eq!(source(&detached(), Path::new(ROOT)).expect("ok"), WorkspaceId("w1".to_string()));
        assert_eq!(
            fake::calls(),
            [
                "herdr workspace list",
                "spawn herdr server",
                "herdr workspace list",
                "herdr workspace list",
                "herdr workspace create --cwd /w/orbit --label dispatch-orbit --no-focus",
            ]
        );
    }

    #[test]
    fn a_server_that_never_answers_is_an_error_that_says_herdr_did_not_start() {
        fake::install(|call| {
            if call.starts_with("spawn") { Ok(String::new()) } else { Err(fake::fail("herdr", 1, NO_SERVER)) }
        });
        let err = source_within(&detached(), Path::new(ROOT), Duration::ZERO).expect_err("times out");
        assert!(err.contains("herdr did not start"), "{err}");
        fake::install(|_| Err(fake::fail("herdr", 1, r#"{"error":{"code":"internal"}}"#)));
        assert!(source_within(&detached(), Path::new(ROOT), Duration::ZERO).is_err());
        assert_eq!(fake::calls(), ["herdr workspace list"], "another error does not start a server");
    }

    #[test]
    fn a_detached_worktree_create_goes_through_the_resolved_source() {
        fake::install(|call| {
            Ok(if call.starts_with("herdr workspace list") {
                list_reply(r#"[{"workspace_id":"w2","label":"dispatch-orbit"}]"#)
            } else {
                String::new()
            })
        });
        let branch = BranchName::parse("fix/7-x").expect("branch");
        create_worktree(
            &detached(),
            Path::new(ROOT),
            Path::new("/w/orbit/.claude/worktrees/7"),
            &branch,
            None,
            IssueNumber(7),
        )
        .expect("ok");
        assert_eq!(
            fake::calls().pop().expect("a call"),
            "herdr worktree create --workspace w2 --branch fix/7-x --path /w/orbit/.claude/worktrees/7 --label #7 --no-focus"
        );
    }

    fn strings<const N: usize>(items: [&str; N]) -> Vec<String> {
        items.map(String::from).to_vec()
    }

    #[test]
    fn the_script_quotes_each_argument_for_sh() {
        assert_eq!(
            script_text(&strings(["claude", "it's $HOME `x`;", "", "a\\b", "end\\", "l1\nl2"])),
            "#!/bin/sh\nexec 'claude' 'it'\\''s $HOME `x`;' '' 'a\\b' 'end\\' 'l1\nl2'\n"
        );
    }

    #[test]
    fn a_typed_path_is_single_quoted_and_a_risky_one_is_refused() {
        assert_eq!(typed_path(Path::new("/g/dispatch/launch/7.sh")).as_deref(), Ok("'/g/dispatch/launch/7.sh'"));
        assert_eq!(typed_path(Path::new("/a b/$x;y/7.sh")).as_deref(), Ok("'/a b/$x;y/7.sh'"));
        for bad in ["/it's/7.sh", "/a\\b/7.sh", "/a\nb/7.sh", "/end\\"] {
            assert!(typed_path(Path::new(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_script_is_written_executable_and_replaced_on_the_next_launch() {
        use std::os::unix::fs::PermissionsExt;
        let file = script_file(&temp_dir("herdr-script"), IssueNumber(7));
        write_script(&strings(["claude", "one\ntwo"]), &file).expect("written");
        write_script(&strings(["claude", "new"]), &file).expect("overwritten");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "#!/bin/sh\nexec 'claude' 'new'\n");
        assert_eq!(std::fs::metadata(&file).expect("meta").permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn launch_types_the_script_path_then_waits_for_the_agent() {
        let file = script_file(&temp_dir("herdr-launch"), IssueNumber(7));
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
            [
                format!("herdr pane run w3:p1 '{}'", file.display()),
                "herdr pane get w3:p1".into(),
                "herdr pane get w3:p1".into()
            ]
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
}
