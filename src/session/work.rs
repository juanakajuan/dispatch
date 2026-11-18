//! What each session has done that a row can show without an attach: commits
//! on its branch, files not committed in its worktree, and how its branch
//! stands against its remote copy, on `origin` or in its fork. The poll reads it every 2 s, so it costs one
//! `for-each-ref` for all sessions, one `status` per worktree, and one
//! `rev-list` only for a branch whose `origin` copy is at another commit.

use super::{Sessions, Workspace, git};
use crate::domain::{BranchName, IssueNumber};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Work {
    /// Commits on the session branch that the base does not have.
    pub commits: u32,
    /// Paths that `git status` lists in the worktree, untracked ones included.
    pub uncommitted: u32,
    pub push: Push,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Push {
    /// `origin` has no branch of this name.
    NotPushed,
    /// `origin/<branch>` is at the branch's commit.
    Pushed,
    /// Commits only on the local branch, and commits only on
    /// `origin/<branch>`. At least one is not zero.
    Differs { to_push: u32, to_pull: u32 },
}

/// Only sessions whose git reads all worked. A worktree removed during the
/// poll has no entry, and the next poll reads it again.
pub type Works = BTreeMap<IssueNumber, Work>;

pub(super) fn read(ws: &Workspace, sessions: &Sessions) -> Works {
    let branches: Vec<&BranchName> = sessions.values().map(|s| &s.branch).collect();
    let Ok(tips) = git::tips(&ws.repo_root, &ws.base, &branches) else {
        return Works::new();
    };
    sessions.values().filter_map(|s| Some((s.issue, one(ws, &tips, &s.branch, s.worktree.as_path())?))).collect()
}

fn one(ws: &Workspace, tips: &git::Tips, branch: &BranchName, worktree: &std::path::Path) -> Option<Work> {
    let local = git::local_ref(branch);
    let tip = tips.get(&local)?;
    let push = match git::remote_copy(tips, branch) {
        None => Push::NotPushed,
        Some((_, remote)) if remote.commit == tip.commit => Push::Pushed,
        Some((remote, _)) => {
            let (to_push, to_pull) = git::left_right(&ws.repo_root, &local, remote).ok()?;
            Push::Differs { to_push, to_pull }
        }
    };
    let uncommitted = git::uncommitted_count(worktree).ok()?;
    Some(Work { commits: tip.ahead_of_base, uncommitted, push })
}
