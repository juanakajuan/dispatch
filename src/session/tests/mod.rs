//! Backend sims: each test installs a fake `proc` that answers the commands
//! of one multiplexer and one shared fake git world.

mod detect;
mod error;
mod hc;
mod herdr;
mod join;
mod plan;
mod remove;
mod request;
mod start_argv;
mod tmux;
mod work;
mod worktree;

use super::markers;
use super::mux::{Backend, Liveness, Observed};
use super::request::{Launch, StartFrom, StartRequest};
use super::{Workspace, git, tmux as tmux_backend};
use crate::config::{AgentCommand, Harness};
use crate::domain::{BranchName, Mode, OriginRef, WorktreePath};
use crate::proc::{ProcError, fake};
use crate::testutil::issue_key;
use std::path::{Path, PathBuf};

fn wp(s: &str) -> WorktreePath {
    WorktreePath::planned(PathBuf::from(s))
}

fn br(s: &str) -> BranchName {
    BranchName::parse(s).expect("valid branch")
}

fn wt(path: &str, branch: Option<&str>) -> git::Worktree {
    git::Worktree { path: wp(path), branch: branch.map(br) }
}

fn markers(rows: &[(&str, u32)]) -> markers::Markers {
    rows.iter().map(|(b, n)| (br(b), issue_key(*n))).collect()
}

fn win(id: &str, path: &str, liveness: Liveness) -> Observed<tmux_backend::WindowId> {
    Observed { window: tmux_backend::WindowId::parse(id).expect("window id"), worktree: wp(path), liveness }
}

fn req() -> StartRequest {
    StartRequest {
        key: issue_key(142),
        branch: br("fix/142-x"),
        mode: Mode::Implement,
        prompt: "p".to_string(),
        from: StartFrom::Base,
    }
}

fn workspace(base: &Path) -> Workspace {
    std::fs::create_dir_all(base.join("orbit")).expect("mkdir");
    Workspace {
        repo_root: base.join("orbit"),
        git_common_dir: base.join("orbit").join(".git"),
        base: OriginRef::parse("origin/main").expect("ref"),
        mux: Backend::Tmux(tmux_backend::Place::Inside { session_id: "$1".to_string() }),
    }
}

fn launch() -> Launch {
    Launch { detect_waiting: false, harness: Harness::Claude, command: AgentCommand::default_for(Harness::Claude) }
}

/// The git repository every sim shares: `orbit` with its main worktree, and
/// the worktree for issue 142 once `added`. A sim asks `answer` first and
/// handles its own multiplexer commands when it returns `None`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "a test sim of external state: each flag is one independent fact the sim flips as the code under test runs commands"
)]
#[derive(Default)]
struct GitWorld {
    added: bool,
    marked: bool,
    branch_exists: bool,
    /// The worktree dir was deleted by hand: git lists it as prunable and
    /// refuses to add a worktree there until a prune.
    stale: bool,
}

impl GitWorld {
    /// A worktree for issue 142 that carries its marker.
    fn started() -> GitWorld {
        GitWorld { added: true, marked: true, ..GitWorld::default() }
    }

    fn answer(&mut self, call: &str, base: &Path) -> Option<Result<String, ProcError>> {
        let dir = base.join("orbit/.claude/worktrees/142");
        let answer = if call.starts_with("git worktree list") {
            let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
            if self.added {
                t = format!("{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display());
            }
            if self.stale {
                t = format!(
                    "{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\nprunable gitdir file points to non-existent location\n\n",
                    dir.display()
                );
            }
            Ok(t)
        } else if call.starts_with("git config --get-regexp") {
            if self.marked {
                Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
            } else {
                Err(fake::fail("git", 1, ""))
            }
        } else if call.starts_with("git show-ref") {
            if self.added || self.branch_exists || self.stale {
                Ok(String::new())
            } else {
                Err(fake::fail("git", 1, ""))
            }
        } else if call.starts_with("git worktree prune") {
            self.stale = false;
            Ok(String::new())
        } else if call.starts_with("git worktree add") && self.stale {
            Err(fake::fail("git", 128, "fatal: is a missing but already registered worktree"))
        } else if call.starts_with("git worktree add") {
            std::fs::create_dir_all(&dir).expect("mkdir");
            self.added = true;
            Ok(String::new())
        } else if call.contains("dispatch-issue 142") {
            self.marked = true;
            Ok(String::new())
        } else {
            return None;
        };
        Some(answer)
    }
}
