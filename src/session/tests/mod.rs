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

use super::mux::{Backend, Liveness, Observed};
use super::request::{Launch, StartFrom, StartRequest};
use super::{Workspace, git, tmux as tmux_backend};
use crate::config::ClaudeCommand;
use crate::domain::{BranchName, IssueNumber, Mode, OriginRef, WorktreePath};
use crate::proc::{ProcError, fake};
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

fn markers(rows: &[(&str, u32)]) -> git::Markers {
    rows.iter().map(|(b, n)| (br(b), IssueNumber(*n))).collect()
}

fn win(id: &str, path: &str, liveness: Liveness) -> Observed<tmux_backend::WindowId> {
    Observed { window: tmux_backend::WindowId::parse(id).expect("window id"), worktree: wp(path), liveness }
}

fn req() -> StartRequest {
    StartRequest {
        issue: IssueNumber(142),
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
    Launch { detect_waiting: false, claude: ClaudeCommand { program: "claude".to_string(), args: Vec::new() } }
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
}

impl GitWorld {
    /// A worktree for issue 142 that carries its marker.
    fn started() -> GitWorld {
        GitWorld { added: true, marked: true, branch_exists: false }
    }

    fn answer(&mut self, call: &str, base: &Path) -> Option<Result<String, ProcError>> {
        let dir = base.join("orbit/.claude/worktrees/142");
        let answer = if call.starts_with("git worktree list") {
            let mut t = format!("worktree {}\nHEAD x\nbranch refs/heads/main\n\n", base.join("orbit").display());
            if self.added {
                t = format!("{t}worktree {}\nHEAD x\nbranch refs/heads/fix/142-x\n\n", dir.display());
            }
            Ok(t)
        } else if call.starts_with("git config --get-regexp") {
            if self.marked {
                Ok("branch.fix/142-x.dispatch-issue 142\n".to_string())
            } else {
                Err(fake::fail("git", 1, ""))
            }
        } else if call.starts_with("git show-ref") {
            if self.added || self.branch_exists { Ok(String::new()) } else { Err(fake::fail("git", 1, "")) }
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
