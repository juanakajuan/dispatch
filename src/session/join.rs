use super::mux::{Liveness, Observed};
use super::{Session, SessionState, Sessions, git};
use crate::domain::{BranchName, IssueNumber, WorktreePath};
use std::collections::BTreeMap;

/// One session with its backend window. `Session` is this without the handle.
pub(super) struct Row<W> {
    pub(super) branch: BranchName,
    pub(super) worktree: WorktreePath,
    pub(super) window: Option<(W, Liveness)>,
}

impl<W> Row<W> {
    /// The only place state is decided.
    pub(super) fn state(&self) -> SessionState {
        match self.window {
            Some((_, Liveness::Waiting)) => SessionState::NeedsYou,
            Some((_, Liveness::Working)) => SessionState::Running,
            Some((_, Liveness::Dead)) | None => SessionState::Exited,
        }
    }
}

/// At most one row per issue, by construction of the map.
pub(super) type Joined<W> = BTreeMap<IssueNumber, Row<W>>;

pub(super) fn sessions<W>(joined: &Joined<W>) -> Sessions {
    let session = |(&issue, row): (&IssueNumber, &Row<W>)| Session {
        issue,
        branch: row.branch.clone(),
        worktree: row.worktree.clone(),
        state: row.state(),
    };
    joined.iter().map(|entry| (*entry.0, session(entry))).collect()
}

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree is a session only if its branch has a valid `dispatch-issue`.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one issue: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
/// - Two windows tagged for one worktree: the first live one wins.
pub(super) fn join<W: Clone>(
    worktrees: &[git::Worktree],
    markers: &git::Markers,
    windows: &[Observed<W>],
) -> Joined<W> {
    let mut by_path: Vec<&git::Worktree> = worktrees.iter().collect();
    by_path.sort_by(|a, b| a.path.cmp(&b.path));
    let mut joined = Joined::new();
    for w in by_path {
        let Some(branch) = &w.branch else { continue };
        let Some(&issue) = markers.get(branch) else { continue };
        let window = windows
            .iter()
            .filter(|o| o.worktree == w.path)
            .min_by_key(|o| !o.liveness.is_live())
            .map(|o| (o.window.clone(), o.liveness));
        joined.entry(issue).or_insert_with(|| Row { branch: branch.clone(), worktree: w.path.clone(), window });
    }
    joined
}
