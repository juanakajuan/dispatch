//! tmux adapter. Private to `session`. Checked against tmux 3.8.
//! Multi-arg commands are exec'd directly by tmux (no `sh -c`), so the
//! Claude prompt is one argv element and needs no shell quoting. tmux's own
//! command parser still treats an argument that ends in `;` as a command
//! separator, which `escape_argv` handles.

use crate::domain::WorktreePath;
use crate::proc::{self, ProcError};
use std::path::Path;

/// tmux `#{window_id}`, for example `@12`. Stable for the life of the server,
/// unlike the window index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(String);

impl WindowId {
    pub fn parse(raw: &str) -> Result<WindowId, String> {
        let id = raw.trim();
        if id.starts_with('@') && id.len() > 1 && id[1..].chars().all(|c| c.is_ascii_digit()) {
            Ok(WindowId(id.to_string()))
        } else {
            Err(format!("tmux returned {id:?} instead of a window id"))
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
    pub(super) fn detect(tmux_env: Option<&str>, repo_dir_name: &str) -> Result<Place, String> {
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
    pub(super) fn session_target(&self) -> String {
        match self {
            Place::Inside { session_id } => session_id.clone(),
            Place::Detached { session_name } => format!("={session_name}"),
        }
    }
}

/// tmux forbids `.` and `:` in session names.
fn session_name_for(repo_dir_name: &str) -> String {
    let clean: String = repo_dir_name.chars().map(|c| if c == '.' || c == ':' { '-' } else { c }).collect();
    format!("dispatch-{clean}")
}

fn tmux<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    tmux_raw(args).map_err(|e| e.to_string())
}

fn tmux_raw<I, S>(args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("tmux", args, Path::new("."))
}

pub(super) struct WindowObs {
    pub id: WindowId,
    pub worktree: WorktreePath,
    pub dead: bool,
    pub waiting: bool,
}

/// tab-separated format; tags are set by dispatch only.
const LIST_FORMAT: &str = "#{window_id}\t#{pane_dead}\t#{@dispatch-worktree}\t#{@dispatch-waiting}";

pub(super) fn list_windows() -> Result<Vec<WindowObs>, String> {
    match tmux_raw(["list-windows", "-a", "-F", LIST_FORMAT]) {
        Ok(text) => Ok(parse_windows(&text)),
        Err(e) if is_no_server(e.stderr()) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn is_no_server(stderr: &str) -> bool {
    stderr.contains("no server running") || stderr.contains("error connecting")
}

/// Rows with an empty `@dispatch-worktree` are not ours and are skipped.
/// Rows with a wrong field count are skipped, not fatal: one odd window must
/// not hide every session. A tag whose directory is gone is skipped too.
fn parse_windows(text: &str) -> Vec<WindowObs> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [id, dead, worktree, waiting] = fields.as_slice() else { return None };
            if worktree.is_empty() {
                return None;
            }
            Some(WindowObs {
                id: WindowId::parse(id).ok()?,
                worktree: WorktreePath::from_observed(worktree)?,
                dead: *dead == "1",
                waiting: *waiting == "1",
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

/// Two calls, because a chained `set-option -w` after `new-window -d` tags the
/// current window, not the new one (measured on tmux 3.8):
///
/// Inside:   tmux new-window -d -P -F '#{window_id}' -t <session_id>: -c <dir> -n <name> -- <argv..>
/// Detached: the same with `-t =<name>:`, or `new-session -d -s <name>` when
///           `has-session -t =<name>` fails.
/// Then:     tmux set-option -w -t <id> @dispatch-worktree <dir> ; set-option -w -t <id> remain-on-exit on
///
/// A crash between the calls leaves an untagged window. That is accepted.
/// Returns the new window id from `-P -F`.
pub(super) fn open_window(place: &Place, dir: &WorktreePath, name: &str, argv: &[String]) -> Result<WindowId, String> {
    let dir_text = dir.as_path().to_string_lossy().into_owned();
    let mut args: Vec<String> = Vec::new();
    let new_session = match place {
        Place::Detached { session_name } => tmux_raw(["has-session", "-t", &format!("={session_name}")]).is_err(),
        Place::Inside { .. } => false,
    };
    match place {
        Place::Detached { session_name } if new_session => {
            args.extend(["new-session", "-d", "-s", session_name.as_str()].map(String::from));
        }
        _ => args.extend(["new-window", "-d", "-t"].map(String::from).into_iter().chain([place.window_parent()])),
    }
    args.extend(["-P", "-F", "#{window_id}", "-c", &dir_text, "-n", name, "--"].map(String::from));
    args.extend(escape_argv(argv));
    let id = WindowId::parse(&tmux(&args)?)?;
    let tag = |args: [&str; 5]| args.map(String::from);
    let mut tags: Vec<String> = tag(["set-option", "-w", "-t", id.as_str(), "@dispatch-worktree"]).to_vec();
    tags.extend([dir_text, ";".to_string()]);
    tags.extend(tag(["set-option", "-w", "-t", id.as_str(), "remain-on-exit"]));
    tags.push("on".to_string());
    tmux(&tags).map_err(|e| format!("window {} died before it was tagged: {e}", id.as_str()))?;
    Ok(id)
}

/// For `Workspace::resume` on a dead pane: tmux respawn-window -k -t <id> -c <dir> -- <argv..>.
/// Keeps the window id and its tags.
pub(super) fn respawn(id: &WindowId, dir: &WorktreePath, argv: &[String]) -> Result<(), String> {
    let mut args: Vec<String> = ["respawn-window", "-k", "-t", id.as_str(), "-c"].map(String::from).to_vec();
    args.push(dir.as_path().to_string_lossy().into_owned());
    args.push("--".to_string());
    args.extend(escape_argv(argv));
    tmux(&args).map(|_| ())
}

pub(super) fn kill_window(id: &WindowId) -> Result<(), String> {
    match tmux_raw(["kill-window", "-t", id.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if e.stderr().contains("can't find window") || is_no_server(e.stderr()) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Types `text` into the pane and submits it, as if the user did:
/// `tmux send-keys -t <id> -l -- <text>` then `tmux send-keys -t <id> Enter`.
/// Two calls, so Claude reads the text as typing and Enter as a submit rather
/// than one paste with a newline in it.
pub(super) fn send_line(id: &WindowId, text: &str) -> Result<(), String> {
    let mut args: Vec<String> = ["send-keys", "-t", id.as_str(), "-l", "--"].map(String::from).to_vec();
    args.extend(escape_argv(&[text.to_string()]));
    tmux(&args)?;
    tmux(["send-keys", "-t", id.as_str(), "Enter"]).map(|_| ())
}

pub(super) fn capture(id: &WindowId, lines: u16) -> Result<Vec<String>, String> {
    let start = format!("-{lines}");
    let text = tmux(["capture-pane", "-p", "-J", "-t", id.as_str(), "-S", start.as_str()])?;
    let mut out: Vec<String> = text.lines().map(str::to_string).collect();
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    Ok(out)
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
            "@1\t0\t\t\n@2\t0\t{}\t\n@3\t1\t{}\t1\n@4\t0\n@5\t0\t{}\t\n@x\t0\t{}\t\n",
            spaced.display(),
            plain.display(),
            base.join("gone").display(),
            plain.display(),
        );
        let got = parse_windows(&text);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].id.as_str(), got[0].dead, got[0].waiting), ("@2", false, false));
        assert!(got[0].worktree.as_path().ends_with("my repo-1"));
        assert_eq!((got[1].id.as_str(), got[1].dead, got[1].waiting), ("@3", true, true));
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
    fn capture_drops_trailing_blank_lines() {
        fake::install(|_| Ok("one\n\ntwo\n\n  \n".to_string()));
        let id = WindowId::parse("@9").expect("id");
        assert_eq!(capture(&id, 10).expect("ok"), ["one", "", "two"]);
        assert_eq!(fake::calls()[0], "tmux capture-pane -p -J -t @9 -S -10");
    }

    #[test]
    fn window_ids_are_validated() {
        assert!(WindowId::parse("@12\n").is_ok());
        for bad in ["", "@", "12", "@a", "no server"] {
            assert!(WindowId::parse(bad).is_err(), "{bad}");
        }
    }
}
