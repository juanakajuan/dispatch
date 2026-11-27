//! tmux adapter. Private to `session`. Checked against tmux 3.8.
//! Multi-arg commands are exec'd directly by tmux (no `sh -c`), so the
//! Claude prompt is one argv element and needs no shell quoting. tmux's own
//! command parser still treats an argument that ends in `;` as a command
//! separator, which `escape_argv` handles.

use super::AttachPlan;
use super::error::MuxError;
use super::mux::{Liveness, Mux, Observed, Repo, WAITING_HOOKS};
use crate::domain::{SessionKey, WorktreePath};
use crate::proc::{self, ProcError};
use std::path::Path;

/// tmux `#{window_id}`, for example `@12`. Stable for the life of the server,
/// unlike the window index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(String);

impl WindowId {
    pub fn parse(raw: &str) -> Result<WindowId, MuxError> {
        let id = raw.trim();
        if id.starts_with('@') && id.len() > 1 && id[1..].chars().all(|c| c.is_ascii_digit()) {
            Ok(WindowId(id.to_string()))
        } else {
            Err(MuxError::NotAWindowId(id.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Place {
    /// `$TMUX` is set: new windows go into the session dispatch runs in.
    Inside { session_id: String },
    /// Not in tmux: windows go into a detached session `dispatch-<repo dir>`,
    /// created on the first start.
    Detached { session_name: String },
}

impl Place {
    pub(super) fn detect(tmux_env: Option<&str>, repo_dir_name: &str) -> Result<Place, MuxError> {
        match tmux_env.filter(|s| !s.is_empty()) {
            Some(_) => {
                let id = tmux(["display-message", "-p", "#{session_id}"])?;
                Ok(Place::Inside { session_id: id.trim().to_string() })
            }
            None => Ok(Place::Detached { session_name: session_name_for(repo_dir_name) }),
        }
    }

    /// The `-t` target that means "the session, as a window parent".
    fn window_parent(&self) -> String {
        match self {
            Place::Inside { session_id } => format!("{session_id}:"),
            Place::Detached { session_name } => format!("={session_name}:"),
        }
    }

    /// The session that `attach-session` and `switch-client` aim at.
    fn session_target(&self) -> String {
        match self {
            Place::Inside { session_id } => session_id.clone(),
            Place::Detached { session_name } => format!("={session_name}"),
        }
    }
}

impl Mux for Place {
    type Window = WindowId;

    fn list(&self, _repo: Repo<'_>) -> Result<Vec<Observed<WindowId>>, MuxError> {
        list_windows()
    }

    fn open(&self, _repo: Repo<'_>, dir: &WorktreePath, key: SessionKey, argv: &[String]) -> Result<(), MuxError> {
        open_window(self, dir, &key.to_string(), argv).map(|_| ())
    }

    fn relaunch(
        &self,
        _repo: Repo<'_>,
        window: &WindowId,
        dir: &WorktreePath,
        _key: SessionKey,
        argv: &[String],
    ) -> Result<(), MuxError> {
        respawn(window, dir, argv)
    }

    fn stop(&self, window: &WindowId) -> Result<(), MuxError> {
        kill_window(window)
    }

    fn send_line(&self, window: &WindowId, text: &str) -> Result<(), MuxError> {
        send_line(window, text)
    }

    fn attach(&self, window: &WindowId) -> AttachPlan {
        let session = self.session_target();
        let select = ["tmux", "select-window", "-t", window.as_str()].map(String::from);
        let (verb, plan): (&str, fn(Vec<String>) -> AttachPlan) = match self {
            Place::Inside { .. } => ("switch-client", AttachPlan::Switch),
            Place::Detached { .. } => ("attach-session", AttachPlan::Suspend),
        };
        plan(select.into_iter().chain([";", verb, "-t", session.as_str()].map(String::from)).collect())
    }

    /// The window carries no tag, so observe skips it. It turns
    /// `remain-on-exit` off, which a global setting can turn on, so it closes
    /// when the shell exits.
    fn shell(
        &self,
        _repo: Repo<'_>,
        dir: &WorktreePath,
        key: SessionKey,
        _window: Option<&WindowId>,
    ) -> Result<AttachPlan, MuxError> {
        let id = new_window(self, dir, &format!("{key} shell"), &[])?;
        tmux(["set-option", "-w", "-t", id.as_str(), "remain-on-exit", "off"])?;
        Ok(self.attach(&id))
    }

    fn waiting_hooks(&self) -> Option<&'static str> {
        Some(WAITING_HOOKS)
    }
}

/// tmux forbids `.` and `:` in session names.
fn session_name_for(repo_dir_name: &str) -> String {
    let clean: String = repo_dir_name.chars().map(|c| if c == '.' || c == ':' { '-' } else { c }).collect();
    format!("dispatch-{clean}")
}

fn tmux<I, S>(args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("tmux", args, Path::new("."))
}

/// tab-separated format; tags are set by dispatch only.
const LIST_FORMAT: &str = "#{window_id}\t#{pane_dead}\t#{@dispatch-worktree}\t#{@dispatch-waiting}";

fn list_windows() -> Result<Vec<Observed<WindowId>>, MuxError> {
    match tmux(["list-windows", "-a", "-F", LIST_FORMAT]) {
        Ok(text) => Ok(parse_windows(&text)),
        Err(e) if classify(&e) == Some(Gone::Server) => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// What a failed call says is missing, read once from tmux's stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    Server,
    Window,
}

fn classify(e: &ProcError) -> Option<Gone> {
    let stderr = e.stderr();
    if stderr.contains("no server running") || stderr.contains("error connecting") {
        Some(Gone::Server)
    } else if stderr.contains("can't find window") {
        Some(Gone::Window)
    } else {
        None
    }
}

/// Rows with an empty `@dispatch-worktree` are not ours and are skipped.
/// Rows with a wrong field count are skipped, not fatal: one odd window must
/// not hide every session. A tag whose directory and its parent are gone is
/// skipped too.
fn parse_windows(text: &str) -> Vec<Observed<WindowId>> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [id, dead, worktree, waiting] = fields.as_slice() else { return None };
            if worktree.is_empty() {
                return None;
            }
            Some(Observed {
                window: WindowId::parse(id).ok()?,
                worktree: WorktreePath::from_tag(worktree)?,
                liveness: match (*dead, *waiting) {
                    ("1", _) => Liveness::Dead,
                    (_, "1") => Liveness::Waiting,
                    _ => Liveness::Working,
                },
            })
        })
        .collect()
}

/// tmux reads a trailing `;` on an argument as a command separator. A
/// backslash in front makes it literal, and tmux strips that backslash. This
/// applies to each argument, so a prompt that ends in `;` survives.
fn escape_argv(argv: &[String]) -> Vec<String> {
    argv.iter()
        .map(|a| match a.strip_suffix(';') {
            Some(head) => format!("{head}\\;"),
            None => a.clone(),
        })
        .collect()
}

/// The window half of `open_window`: no tags. An empty `command` runs the
/// default shell, so no `--` goes in.
fn new_window(place: &Place, dir: &WorktreePath, name: &str, command: &[String]) -> Result<WindowId, MuxError> {
    let dir_text = dir.as_path().to_string_lossy().into_owned();
    let mut args: Vec<String> = Vec::new();
    let new_session = match place {
        Place::Detached { session_name } => tmux(["has-session", "-t", &format!("={session_name}")]).is_err(),
        Place::Inside { .. } => false,
    };
    match place {
        Place::Detached { session_name } if new_session => {
            args.extend(["new-session", "-d", "-s", session_name.as_str()].map(String::from));
        }
        _ => args.extend(["new-window", "-d", "-t"].map(String::from).into_iter().chain([place.window_parent()])),
    }
    args.extend(["-P", "-F", "#{window_id}", "-c", &dir_text, "-n", name].map(String::from));
    if !command.is_empty() {
        args.push("--".to_string());
        args.extend(escape_argv(command));
    }
    WindowId::parse(&tmux(&args)?)
}

/// Two calls for a tagged window, because a chained `set-option -w` after `new-window -d` tags the
/// current window, not the new one (measured on tmux 3.8):
///
/// Inside:   tmux new-window -d -P -F '#{`window_id`}' -t <`session_id>`: -c <dir> -n <name> -- <argv..>
/// Detached: the same with `-t =<name>:`, or `new-session -d -s <name>` when
///           `has-session -t =<name>` fails.
/// Then:     tmux set-option -w -t <id> @dispatch-worktree <dir> ; set-option -w -t <id> remain-on-exit on
///           and `; set_waiting` when the argv has the waiting hooks.
///
/// A crash between the calls leaves an untagged window. That is accepted.
/// Returns the new window id from `-P -F`.
fn open_window(place: &Place, dir: &WorktreePath, name: &str, command: &[String]) -> Result<WindowId, MuxError> {
    let dir_text = dir.as_path().to_string_lossy().into_owned();
    let id = new_window(place, dir, name, command)?;
    let tag = |args: [&str; 5]| args.map(String::from);
    let mut tags: Vec<String> = tag(["set-option", "-w", "-t", id.as_str(), "@dispatch-worktree"]).to_vec();
    tags.extend([dir_text, ";".to_string()]);
    tags.extend(tag(["set-option", "-w", "-t", id.as_str(), "remain-on-exit"]));
    tags.push("on".to_string());
    if runs_waiting_hooks(command) {
        tags.push(";".to_string());
        tags.extend(set_waiting(&id));
    }
    if let Err(error) = tmux(&tags) {
        return Err(MuxError::Untagged { window: id, error });
    }
    Ok(id)
}

/// True when `argv` passes the waiting hooks, so a hook clears the flag
/// that `set_waiting` sets. Without them, nothing would clear it.
fn runs_waiting_hooks(argv: &[String]) -> bool {
    argv.iter().any(|a| a == WAITING_HOOKS)
}

/// The window waits for you until Claude starts: Claude can ask to trust
/// the folder first, and no hook runs during that question.
fn set_waiting(id: &WindowId) -> [String; 6] {
    ["set-option", "-w", "-t", id.as_str(), "@dispatch-waiting", "1"].map(String::from)
}

/// For `Workspace::resume` on a dead pane: tmux respawn-window -k -t <id> -c <dir> -- <argv..>,
/// after `set_waiting` when the argv has the waiting hooks.
/// Keeps the window id and its tags.
fn respawn(id: &WindowId, dir: &WorktreePath, command: &[String]) -> Result<(), MuxError> {
    let mut args: Vec<String> = Vec::new();
    if runs_waiting_hooks(command) {
        args.extend(set_waiting(id));
        args.push(";".to_string());
    }
    args.extend(["respawn-window", "-k", "-t", id.as_str(), "-c"].map(String::from));
    args.push(dir.as_path().to_string_lossy().into_owned());
    args.push("--".to_string());
    args.extend(escape_argv(command));
    tmux(&args)?;
    Ok(())
}

fn kill_window(id: &WindowId) -> Result<(), MuxError> {
    match tmux(["kill-window", "-t", id.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if classify(&e).is_some() => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Types `text` into the pane and submits it, as if the user did:
/// `tmux send-keys -t <id> -l -- <text>` then `tmux send-keys -t <id> Enter`.
/// Two calls, so Claude reads the text as typing and Enter as a submit rather
/// than one paste with a newline in it.
fn send_line(id: &WindowId, text: &str) -> Result<(), MuxError> {
    let mut args: Vec<String> = ["send-keys", "-t", id.as_str(), "-l", "--"].map(String::from).to_vec();
    args.extend(escape_argv(&[text.to_string()]));
    tmux(&args)?;
    tmux(["send-keys", "-t", id.as_str(), "Enter"])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;
    use crate::testutil::temp_dir;

    #[test]
    fn windows_untagged_skipped_flags_and_spaces() {
        let base = temp_dir("tmux-win");
        let spaced = base.join("my repo-1");
        let plain = base.join("repo-2");
        std::fs::create_dir(&spaced).expect("mkdir");
        std::fs::create_dir(&plain).expect("mkdir");
        let text = format!(
            "@1\t0\t\t\n@2\t0\t{}\t\n@3\t1\t{}\t1\n@4\t0\n@5\t0\t{}\t\n@6\t0\t{}\t\n@x\t0\t{}\t\n",
            spaced.display(),
            plain.display(),
            base.join("gone").display(),
            base.join("gone/too").display(),
            plain.display(),
        );
        let got = parse_windows(&text);
        assert_eq!(got.len(), 3);
        assert_eq!((got[0].window.as_str(), got[0].liveness), ("@2", Liveness::Working));
        assert!(got[0].worktree.as_path().ends_with("my repo-1"));
        assert_eq!((got[1].window.as_str(), got[1].liveness), ("@3", Liveness::Dead));
        assert_eq!(got[2].window.as_str(), "@5", "a deleted dir keeps its window, so a start can close it");
        assert!(got[2].worktree.as_path().ends_with("gone"));
    }

    #[test]
    fn session_names_drop_dots_and_colons() {
        assert_eq!(session_name_for("my.repo"), "dispatch-my-repo");
        assert_eq!(session_name_for("a:b"), "dispatch-a-b");
    }

    #[test]
    fn trailing_semicolon_is_escaped_once() {
        let argv =
            ["claude".to_string(), "fix it;".to_string(), "a;b".to_string(), "x\\;".to_string(), ";".to_string()];
        assert_eq!(escape_argv(&argv), ["claude", "fix it\\;", "a;b", "x\\\\;", "\\;"]);
    }

    #[test]
    fn no_server_means_no_windows() {
        fake::install(|_| Err(fake::fail("tmux", 1, "no server running on /tmp/tmux-1000/default")));
        assert!(list_windows().expect("ok").is_empty());
        fake::install(|_| Err(fake::fail("tmux", 1, "boom")));
        assert!(list_windows().is_err());
    }

    #[test]
    fn open_window_tags_the_new_window_by_id() {
        let dir = WorktreePath::from_observed("/tmp").expect("tmp exists");
        fake::install(|call| Ok(if call.contains("new-window") { "@7\n".to_string() } else { String::new() }));
        let place = Place::Inside { session_id: "$3".to_string() };
        let id = open_window(&place, &dir, "#142", &["claude".to_string(), "go;".to_string()]).expect("ok");
        assert_eq!(id.as_str(), "@7");
        let calls = fake::calls();
        let tmp = dir.as_path().display();
        assert_eq!(
            calls[0],
            format!("tmux new-window -d -t $3: -P -F #{{window_id}} -c {tmp} -n #142 -- claude go\\;")
        );
        assert_eq!(
            calls[1],
            format!("tmux set-option -w -t @7 @dispatch-worktree {tmp} ; set-option -w -t @7 remain-on-exit on")
        );
    }

    fn hooked_argv() -> Vec<String> {
        ["claude", "--settings", WAITING_HOOKS, "go"].map(String::from).to_vec()
    }

    #[test]
    fn a_window_with_the_waiting_hooks_waits_until_claude_starts() {
        let dir = WorktreePath::from_observed("/tmp").expect("tmp exists");
        fake::install(|call| Ok(if call.contains("new-window") { "@7\n".to_string() } else { String::new() }));
        let place = Place::Inside { session_id: "$3".to_string() };
        open_window(&place, &dir, "#142", &hooked_argv()).expect("ok");
        assert!(
            fake::calls()[1].ends_with("remain-on-exit on ; set-option -w -t @7 @dispatch-waiting 1"),
            "{:?}",
            fake::calls()
        );
        let id = WindowId::parse("@7").expect("id");
        fake::install(|_| Ok(String::new()));
        respawn(&id, &dir, &hooked_argv()).expect("ok");
        assert!(
            fake::calls()[0].starts_with("tmux set-option -w -t @7 @dispatch-waiting 1 ; respawn-window -k -t @7"),
            "the flag is set before the new Claude can clear it: {:?}",
            fake::calls()
        );
    }

    #[test]
    fn a_new_claude_clears_the_flag_and_a_resumed_one_does_not() {
        let hooks: serde_json::Value = serde_json::from_str(WAITING_HOOKS).expect("valid JSON");
        let start = &hooks["hooks"]["SessionStart"][0];
        assert_eq!(start["matcher"], "startup", "--continue is a resume start; it waits for you");
        let command = start["hooks"][0]["command"].as_str().expect("a command");
        assert!(command.ends_with("-u @dispatch-waiting"), "{command}");
    }

    #[test]
    fn detached_creates_the_session_once() {
        let dir = WorktreePath::from_observed("/tmp").expect("tmp exists");
        let place = Place::Detached { session_name: "dispatch-r".to_string() };
        fake::install(|call| match call {
            c if c.starts_with("tmux has-session") => Err(fake::fail("tmux", 1, "can't find session")),
            c if c.contains("new-session") => Ok("@1\n".to_string()),
            _ => Ok(String::new()),
        });
        open_window(&place, &dir, "#1", &["claude".to_string()]).expect("ok");
        assert!(fake::calls()[1].starts_with("tmux new-session -d -s dispatch-r -P"));
        fake::install(|call| Ok(if call.contains("new-window") { "@2\n".to_string() } else { String::new() }));
        open_window(&place, &dir, "#1", &["claude".to_string()]).expect("ok");
        assert!(fake::calls()[1].starts_with("tmux new-window -d -t =dispatch-r: -P"));
    }

    #[test]
    fn kill_of_a_missing_window_is_fine() {
        fake::install(|_| Err(fake::fail("tmux", 1, "can't find window: @9")));
        let id = WindowId::parse("@9").expect("id");
        assert!(kill_window(&id).is_ok());
        fake::install(|_| Err(fake::fail("tmux", 1, "other")));
        assert!(kill_window(&id).is_err());
    }

    #[test]
    fn send_line_types_the_text_literally_then_presses_enter() {
        fake::install(|_| Ok(String::new()));
        let id = WindowId::parse("@9").expect("id");
        send_line(&id, "merge -it;").expect("ok");
        assert_eq!(fake::calls(), ["tmux send-keys -t @9 -l -- merge -it\\;", "tmux send-keys -t @9 Enter"]);
    }

    #[test]
    fn window_ids_are_validated() {
        assert!(WindowId::parse("@12\n").is_ok());
        for bad in ["", "@", "12", "@a", "no server"] {
            assert!(WindowId::parse(bad).is_err(), "{bad}");
        }
    }
}
