use super::{Session, Sessions, Window, WindowObs, git};

/// Joins the three observations. Pure; the main unit-test target.
/// - A worktree is a session only if its branch has a valid `dispatch-issue`.
/// - A marker with an unparseable value is ignored (boundary rule).
/// - Two worktrees marked with one issue: the first by path wins, so the map
///   invariant holds even after manual meddling.
/// - A tagged window whose worktree is gone is ignored; it is not a session.
/// - Two windows tagged for one worktree: a live one wins.
pub(super) fn join(worktrees: &[git::Worktree], markers: &git::Markers, windows: &[WindowObs]) -> Sessions {
    let mut by_path: Vec<&git::Worktree> = worktrees.iter().collect();
    by_path.sort_by(|a, b| a.path.cmp(&b.path));
    let mut sessions = Sessions::new();
    for w in by_path {
        let Some(branch) = &w.branch else { continue };
        let Some(&issue) = markers.get(branch) else { continue };
        let window = windows.iter().filter(|o| o.worktree == w.path).min_by_key(|o| o.dead).map(|o| Window {
            id: o.id.clone(),
            alive: !o.dead,
            waiting: o.waiting,
        });
        sessions.entry(issue).or_insert_with(|| Session {
            issue,
            branch: branch.clone(),
            worktree: w.path.clone(),
            window,
        });
    }
    sessions
}
