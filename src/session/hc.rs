//! Checked against hc 0.1.21.
//!
//! `hc run` execs argv directly, so the Claude prompt is one argv element and
//! needs no shell, no quoting, and no prompt file. The app shows a terminal
//! labelled `herdr.space=sp<32 hex>` (an id it has never seen) with its cwd in
//! a git worktree as a new space nested under the repo.
//!
//! hc starts its core on the first command, so there is no "core not running"
//! case to map to an empty list.

use super::AttachPlan;
use super::error::MuxError;
use super::mux::{Liveness, Mux, Observed, Repo};
use crate::domain::{SessionKey, WorktreePath};
use crate::proc::{self, ProcError};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
pub struct Address(String);

impl Address {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pane {
    pub address: Address,
    pub space: Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    App,
    Outside,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hc {
    pub place: Place,
    /// dispatch's `PATH`. hc starts a program with its core's environment,
    /// not the caller's, so the core may not find a `claude` that dispatch finds.
    pub path: Option<String>,
}

impl Mux for Hc {
    type Window = Pane;

    fn list(&self, repo: Repo<'_>) -> Result<Vec<Observed<Pane>>, MuxError> {
        list_windows(repo.root)
    }

    fn open(&self, repo: Repo<'_>, dir: &WorktreePath, key: SessionKey, argv: &[String]) -> Result<(), MuxError> {
        open_window(repo.root, dir, key, &self.launch_argv(argv)).map(|_| ())
    }

    /// hc cannot restart a finished terminal, so a new one takes its place.
    /// A restored shell stays, and Claude opens in a new tab of its space.
    fn relaunch(
        &self,
        repo: Repo<'_>,
        window: &Pane,
        dir: &WorktreePath,
        key: SessionKey,
        argv: &[String],
    ) -> Result<(), MuxError> {
        match hc(["rm", window.address.as_str()]) {
            Ok(_) => {}
            Err(e) if matches!(classify(&e), Some(Gone::NotFound | Gone::Live)) => {}
            Err(e) => return Err(e.into()),
        }
        self.open(repo, dir, key, argv)
    }

    /// The terminal has no argv, so hc runs the machine's own shell, and it
    /// carries no `dispatch.*` label, so `list` never sees it. It joins the
    /// worktree's space by its `herdr.space` label.
    fn shell(
        &self,
        _repo: Repo<'_>,
        dir: &WorktreePath,
        _key: SessionKey,
        _window: Option<&Pane>,
    ) -> Result<AttachPlan, MuxError> {
        let space = space_id(dir.as_path());
        let address = run_terminal(dir, &[format!("herdr.space={space}")], &[])?;
        Ok(self.attach(&Pane { address, space }))
    }

    /// The app shows a space for as long as one terminal carries its label,
    /// so ending only Claude's terminal would leave the space open, and any
    /// terminal the user opened there with it.
    fn stop(&self, window: &Pane) -> Result<(), MuxError> {
        let label = format!("herdr.space={}", window.space);
        let text = hc(["ls", "--all", "--json", "--label", label.as_str()])?;
        let list: TerminalList = serde_json::from_str(&text).map_err(|e| unexpected_json(&e))?;
        list.terminals.iter().try_for_each(|row| stop(&row.address))
    }

    /// `--force` types even when Claude waits on a dialog.
    fn send_line(&self, window: &Pane, text: &str) -> Result<(), MuxError> {
        hc(["send", "--force", window.address.as_str(), text])?;
        Ok(())
    }

    fn attach(&self, window: &Pane) -> AttachPlan {
        match self.place {
            Place::App => AttachPlan::Switch(
                ["herdr-app", "do", "space.select", "--id", window.space.as_str()].map(String::from).to_vec(),
            ),
            Place::Outside => AttachPlan::Suspend(["hc", "attach", window.address.as_str()].map(String::from).to_vec()),
        }
    }
}

impl Hc {
    fn launch_argv(&self, argv: &[String]) -> Vec<String> {
        match &self.path {
            Some(path) => {
                let env = ["/usr/bin/env".to_string(), format!("PATH={path}")];
                env.into_iter().chain(argv.iter().cloned()).collect()
            }
            None => argv.to_vec(),
        }
    }
}

#[derive(Deserialize)]
struct TerminalList {
    terminals: Vec<TerminalRow>,
}

#[derive(Deserialize)]
struct TerminalRow {
    address: Address,
    /// The folder the terminal launched in. It stays after a `cd`.
    cwd: String,
    /// The program hc sees in the terminal, such as `claude-code` or `fish`.
    agent: Option<String>,
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
    fn waiting(self) -> bool {
        matches!(self, Status::Blocked | Status::Done | Status::Idle)
    }
}

/// After a restart, the app restores a space with a new shell that keeps
/// the labels of the terminal it had. That shell runs no Claude.
const SHELLS: [&str; 9] = ["bash", "dash", "elvish", "fish", "ksh", "nu", "sh", "xonsh", "zsh"];

impl TerminalRow {
    fn runs_a_shell(&self) -> bool {
        self.agent.as_deref().is_some_and(|agent| SHELLS.contains(&agent))
    }
}

#[derive(Deserialize)]
struct Started {
    address: Address,
}

fn hc<I, S>(args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("hc", args, Path::new("."))
}

/// What a failed call says about its terminal, read once from hc's exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gone {
    /// hc prints `{"error":{"kind":"not_found",..}}` on stderr and exits 3.
    NotFound,
    Finished,
    /// `hc rm` of a running terminal exits 1 with `"kind":"live"`.
    Live,
}

fn classify(e: &ProcError) -> Option<Gone> {
    match e.exit_code() {
        Some(3) if e.stderr().contains("\"not_found\"") => Some(Gone::NotFound),
        Some(11) => Some(Gone::Finished),
        Some(1) if e.stderr().contains("\"live\"") => Some(Gone::Live),
        _ => None,
    }
}

fn unexpected_json(e: &serde_json::Error) -> MuxError {
    MuxError::Reply { program: "hc", detail: e.to_string() }
}

fn list_windows(repo_root: &Path) -> Result<Vec<Observed<Pane>>, MuxError> {
    let repo = format!("dispatch.repo={}", repo_root.display());
    let text = hc(["ls", "--all", "--json", "--label", repo.as_str()])?;
    let list: TerminalList = serde_json::from_str(&text).map_err(|e| unexpected_json(&e))?;
    Ok(window_obs(&list))
}

/// hc lists in creation order.
fn window_obs(list: &TerminalList) -> Vec<Observed<Pane>> {
    list.terminals
        .iter()
        .rev()
        .filter_map(|row| {
            let worktree = WorktreePath::from_tag(&row.cwd)?;
            let liveness = match row.state {
                TerminalState::Other => Liveness::Dead,
                TerminalState::Live if row.runs_a_shell() => Liveness::Dead,
                TerminalState::Live if row.status.is_some_and(Status::waiting) => Liveness::Waiting,
                TerminalState::Live => Liveness::Working,
            };
            let window = Pane { address: row.address.clone(), space: space_id(worktree.as_path()) };
            Some(Observed { window, worktree, liveness })
        })
        .collect()
}

const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_OFFSET_2: u64 = 0x9e37_79b9_7f4a_7c15;

fn fnv1a(offset: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(offset, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// The app's space id, `sp` and 32 lowercase hex digits.
pub(super) fn space_id(worktree: &Path) -> Space {
    let bytes = worktree.as_os_str().as_encoded_bytes();
    Space(format!("sp{:016x}{:016x}", fnv1a(FNV_OFFSET, bytes), fnv1a(FNV_OFFSET_2, bytes)))
}

/// `--no-parent` keeps the terminal alive when dispatch exits. An empty
/// `argv` runs the machine's own shell, so no `--` goes in.
fn run_terminal(dir: &WorktreePath, labels: &[String], argv: &[String]) -> Result<Address, MuxError> {
    let cwd = dir.as_path().to_string_lossy().into_owned();
    let mut run = vec!["run", "--bg", "--json", "--no-parent", "--no-notify", "--cwd", cwd.as_str()];
    for label in labels {
        run.extend(["--label", label.as_str()]);
    }
    if !argv.is_empty() {
        run.push("--");
        run.extend(argv.iter().map(String::as_str));
    }
    let text = hc(run)?;
    serde_json::from_str::<Started>(&text).map(|s| s.address).map_err(|e| unexpected_json(&e))
}

fn open_window(repo_root: &Path, dir: &WorktreePath, key: SessionKey, argv: &[String]) -> Result<Address, MuxError> {
    let labels = [
        format!("herdr.space={}", space_id(dir.as_path())),
        format!("dispatch.repo={}", repo_root.display()),
        match key {
            SessionKey::Issue(n) => format!("dispatch.issue={}", n.0),
            SessionKey::Pr(n) => format!("dispatch.pr={}", n.0),
        },
    ];
    run_terminal(dir, &labels, argv)
}

fn remove(address: &Address) -> Result<(), MuxError> {
    match hc(["rm", address.as_str()]) {
        Ok(_) => Ok(()),
        Err(e) if classify(&e) == Some(Gone::NotFound) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn stop(address: &Address) -> Result<(), MuxError> {
    match hc(["kill", address.as_str()]) {
        Ok(_) => {}
        Err(e) => match classify(&e) {
            Some(Gone::NotFound) => return Ok(()),
            Some(Gone::Finished) => {}
            Some(Gone::Live) | None => return Err(e.into()),
        },
    }
    remove(address)
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
        let got: Vec<(&str, Liveness)> = obs.iter().map(|o| (o.window.address.as_str(), o.liveness)).collect();
        assert_eq!(
            got,
            [("t3", Liveness::Working), ("t2", Liveness::Dead), ("t1", Liveness::Working)],
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
