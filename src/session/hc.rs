//! herdr app adapter (herdr-core, the `hc` CLI). Private to `session`.
//! Checked against hc 0.1.21. This is not the `herdr` server in `herdr.rs`.
//!
//! `hc run` execs argv directly, so the Claude prompt is one argv element and
//! needs no shell, no quoting, and no prompt file. hc has no worktree command,
//! so the worktree is a plain `git worktree add`. The app shows a terminal
//! labelled `herdr.space=sp<32 hex>` (an id it has never seen) with its cwd in
//! a git worktree as a new space nested under the repo.
//!
//! dispatch keeps no state of its own. Every terminal it runs carries labels
//! (`dispatch.repo`, `dispatch.issue`, `herdr.space`), and `hc ls --label`
//! finds them again. The space id is a pure function of the worktree path, so
//! a resume lands in the same space.
//!
//! hc starts its core on the first command, so there is no "core not running"
//! case to map to an empty list.

use super::{AttachPlan, WindowId, WindowObs};
use crate::domain::{IssueNumber, WorktreePath};
use crate::proc::{self, ProcError};
use serde::Deserialize;
use std::path::Path;

/// hc's terminal address, for example `t7`. Opaque: taken from hc's JSON.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct Address(String);

impl Address {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(super) fn new(raw: &str) -> Address {
        Address(raw.to_string())
    }
}

#[cfg(test)]
impl Pane {
    pub(super) fn new(address: &str, space: &str) -> Pane {
        Pane { address: Address::new(address), space: Space(space.to_string()) }
    }
}

/// The app's id for an issue's space, the `herdr.space` label value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Space(String);

impl Space {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Space {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One terminal and the space it lives in. `attach` needs both: the address
/// outside the app, the space inside it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pane {
    pub address: Address,
    pub space: Space,
}

/// Whether dispatch runs in a terminal of the app (`HC_TERMINAL_SOCKET` set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    App,
    Outside,
}

// ---- JSON boundary ---------------------------------------------------------

#[derive(Deserialize)]
struct TerminalList {
    terminals: Vec<TerminalRow>,
}

#[derive(Deserialize)]
struct TerminalRow {
    address: Address,
    /// The folder the terminal launched in. It stays after a `cd`.
    cwd: String,
    state: TerminalState,
    status: Option<Status>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TerminalState {
    Live,
    #[serde(other)]
    Other,
}

/// What the program in a terminal reports. A word that a later hc adds counts
/// as `Unknown`, so one new word does not hide every session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

impl Status {
    /// The program stopped and the user is next.
    fn waiting(self) -> bool {
        matches!(self, Status::Blocked | Status::Done | Status::Idle)
    }
}

#[derive(Deserialize)]
struct Started {
    address: Address,
}

// ---- commands --------------------------------------------------------------

fn hc_raw<I, S>(args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("hc", args, Path::new("."))
}

fn hc<I, S>(args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    hc_raw(args).map_err(|e| e.to_string())
}

/// hc prints `{"error":{"kind":"not_found",..}}` on stderr and exits 3.
fn is_not_found(e: &ProcError) -> bool {
    e.exit_code() == Some(3) && e.stderr().contains("\"not_found\"")
}

// ---- observe ---------------------------------------------------------------

pub(super) fn list_windows(repo_root: &Path) -> Result<Vec<WindowObs>, String> {
    let repo = format!("dispatch.repo={}", repo_root.display());
    let text = hc(["ls", "--all", "--json", "--label", repo.as_str()])?;
    let list: TerminalList = serde_json::from_str(&text).map_err(|e| format!("hc printed unexpected JSON: {e}"))?;
    Ok(window_obs(&list))
}

/// One window per terminal whose launch folder still exists. Newest first
/// (hc lists in creation order), so when `join` meets several dead windows for
/// one worktree it shows the latest. A live one always wins in `join`.
fn window_obs(list: &TerminalList) -> Vec<WindowObs> {
    list.terminals
        .iter()
        .rev()
        .filter_map(|row| {
            let worktree = WorktreePath::from_observed(&row.cwd)?;
            Some(WindowObs {
                id: WindowId::Hc(Pane { address: row.address.clone(), space: space_id(worktree.as_path()) }),
                worktree,
                dead: row.state != TerminalState::Live,
                waiting: row.status.is_some_and(Status::waiting),
            })
        })
        .collect()
}

// ---- start -----------------------------------------------------------------

const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_OFFSET_2: u64 = 0x9e37_79b9_7f4a_7c15;

fn fnv1a(offset: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(offset, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// The app's space id, `sp` and 32 lowercase hex digits, derived from the
/// worktree path alone. The same worktree always lands in the same space, and
/// no id is stored anywhere.
pub(super) fn space_id(worktree: &Path) -> Space {
    let bytes = worktree.as_os_str().as_encoded_bytes();
    Space(format!("sp{:016x}{:016x}", fnv1a(FNV_OFFSET, bytes), fnv1a(FNV_OFFSET_2, bytes)))
}

/// `hc run --bg --json --no-parent --no-notify --cwd <dir> --label ... -- <argv>`.
/// `--no-parent` keeps the terminal alive when dispatch exits.
pub(super) fn open_window(
    repo_root: &Path,
    dir: &WorktreePath,
    issue: IssueNumber,
    argv: &[String],
) -> Result<Address, String> {
    let cwd = dir.as_path().to_string_lossy().into_owned();
    let labels = [
        format!("herdr.space={}", space_id(dir.as_path())),
        format!("dispatch.repo={}", repo_root.display()),
        format!("dispatch.issue={}", issue.0),
    ];
    let mut run = vec!["run", "--bg", "--json", "--no-parent", "--no-notify", "--cwd", cwd.as_str()];
    for label in &labels {
        run.extend(["--label", label.as_str()]);
    }
    run.push("--");
    run.extend(argv.iter().map(String::as_str));
    let text = hc(run)?;
    serde_json::from_str::<Started>(&text).map(|s| s.address).map_err(|e| format!("hc printed unexpected JSON: {e}"))
}

/// `hc rm <t>` for a finished terminal, before its replacement starts. A
/// terminal that is already gone counts as removed.
pub(super) fn remove(address: &Address) -> Result<(), String> {
    match hc_raw(["rm", address.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if is_not_found(&e) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ---- stop, send, tail ------------------------------------------------------

/// `hc kill <t>` then `hc rm <t>`. The worktree stays. A terminal that is
/// already gone counts as stopped, and a finished one only needs the `rm`.
pub(super) fn stop(address: &Address) -> Result<(), String> {
    match hc_raw(["kill", address.as_str()]) {
        Ok(_) => {}
        Err(e) if is_not_found(&e) => return Ok(()),
        Err(e) if e.exit_code() == Some(11) => {}
        Err(e) => return Err(e.to_string()),
    }
    remove(address)
}

/// `--force` types even when Claude waits on a dialog, like tmux `send-keys`.
pub(super) fn send_line(address: &Address, text: &str) -> Result<(), String> {
    hc(["send", "--force", address.as_str(), text]).map(|_| ())
}

pub(super) fn read_tail(address: &Address, lines: u16) -> Result<Vec<String>, String> {
    let n = lines.to_string();
    let text = hc(["read", address.as_str(), "--lines", n.as_str()])?;
    Ok(super::text_lines(&text))
}

/// Outside the app, `hc attach` takes over the terminal. Inside it, the
/// app's own CLI shows the issue's space and dispatch keeps its pane.
pub(super) fn attach_plan(place: Place, pane: &Pane) -> AttachPlan {
    match place {
        Place::App => AttachPlan::Switch(
            ["herdr-app", "do", "space.select", "--id", pane.space.as_str()].map(String::from).to_vec(),
        ),
        Place::Outside => AttachPlan::Suspend(["hc", "attach", pane.address.as_str()].map(String::from).to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_id_is_stable_well_formed_and_per_path() {
        let a = space_id(Path::new("/w/orbit/.claude/worktrees/142")).to_string();
        assert_eq!(a, space_id(Path::new("/w/orbit/.claude/worktrees/142")).to_string());
        assert_ne!(a, space_id(Path::new("/w/orbit/.claude/worktrees/143")).to_string());
        assert_eq!(a.len(), 34);
        assert!(a.starts_with("sp"));
        assert!(a[2..].chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "{a}");
        assert_ne!(a[2..18], a[18..], "the two halves use different offsets");
    }

    #[test]
    fn ls_rows_parse_with_live_finished_unknown_status_and_extra_fields() {
        let json = r#"{"hidden_finished":0,"terminals":[
          {"address":"t1","agent":"claude-code","cwd":"/tmp","state":"live","status":"working","turn":3,"where":"a / b"},
          {"address":"t2","agent":"fish","cwd":"/tmp","exit_code":0,"reason":"exited","state":"finished","turn":1},
          {"address":"t3","cwd":"/tmp","state":"live","status":"melting"},
          {"address":"t4","cwd":"/definitely/not/here","state":"live"}]}"#;
        let list: TerminalList = serde_json::from_str(json).expect("parses");
        let obs = window_obs(&list);
        let got: Vec<(&str, bool, bool)> = obs
            .iter()
            .map(|o| match &o.id {
                WindowId::Hc(p) => (p.address.as_str(), o.dead, o.waiting),
                _ => panic!("not an hc window"),
            })
            .collect();
        assert_eq!(
            got,
            [("t3", false, false), ("t2", true, false), ("t1", false, false)],
            "newest first, a gone folder is skipped, an unknown status is not waiting"
        );
    }

    #[test]
    fn idle_done_and_blocked_are_waiting_and_working_is_not() {
        for (word, want) in [("idle", true), ("done", true), ("blocked", true), ("working", false), ("error", false)] {
            let status: Status = serde_json::from_str(&format!("\"{word}\"")).expect("parses");
            assert_eq!(status.waiting(), want, "{word}");
        }
    }
}
