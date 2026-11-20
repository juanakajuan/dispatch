use super::markers;
use super::mux::{Liveness, Observed};
use super::worktree::worktree_dir_for;
use super::{Checkout, Session, SessionState, Sessions, git};
use crate::domain::{BranchName, SessionKey, WorktreePath};
use std::collections::BTreeMap;
use std::path::Path;

/// One session with its backend window. `Session` is this without the handle.
pub(super) struct Row<W> {
    pub(super) branch: BranchName,
    pub(super) checkout: Checkout,
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

/// At most one row per key, by construction of the map.
pub(super) type Joined<W> = BTreeMap<SessionKey, Row<W>>;

pub(super) fn sessions<W>(joined: &Joined<W>) -> Sessions {
    let session = |(&key, row): (&SessionKey, &Row<W>)| Session {
        key,
        branch: row.branch.clone(),
        checkout: row.checkout.clone(),
        worktree: row.worktree.clone(),
        state: row.state(),
    };
    joined.iter().map(|entry| (*entry.0, session(entry))).collect()
}

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree whose branch has a valid `dispatch-key` is that key's
///   session.
/// - A worktree at `worktree_dir_for(root, N)` is key N's session when some
///   branch is marked N, whatever it has checked out. So a session stays
///   when its worktree changes branch. A match by branch wins over a match
///   by path, and one worktree is at most one session.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one key: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
/// - Two windows tagged for one worktree: the first live one wins.
pub(super) fn join<W: Clone>(
    root: &Path,
    worktrees: &[git::Worktree],
    markers: &markers::Markers,
    windows: &[Observed<W>],
) -> Joined<W> {
    let mut by_path: Vec<&git::Worktree> = worktrees.iter().collect();
    by_path.sort_by(|a, b| a.path.cmp(&b.path));
    let window_of = |w: &git::Worktree| {
        windows
            .iter()
            .filter(|o| o.worktree == w.path)
            .min_by_key(|o| !o.liveness.is_live())
            .map(|o| (o.window.clone(), o.liveness))
    };
    let mut joined = Joined::new();
    let mut unmatched = Vec::new();
    for w in by_path {
        match w.branch.as_ref().and_then(|b| Some((b, *markers.get(b)?))) {
            Some((branch, key)) => {
                joined.entry(key).or_insert_with(|| Row {
                    branch: branch.clone(),
                    checkout: Checkout::SessionBranch,
                    worktree: w.path.clone(),
                    window: window_of(w),
                });
            }
            None => unmatched.push(w),
        }
    }
    for w in unmatched {
        let at_dir = markers.iter().find(|&(_, &key)| worktree_dir_for(root, key) == w.path.as_path());
        let Some((branch, &key)) = at_dir else { continue };
        let checkout = w.branch.clone().map_or(Checkout::Detached, Checkout::Other);
        joined.entry(key).or_insert_with(|| Row {
            branch: branch.clone(),
            checkout,
            worktree: w.path.clone(),
            window: window_of(w),
        });
    }
    joined
}
