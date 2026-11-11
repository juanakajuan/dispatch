//! tmux adapter. Private to `session`. Checked against tmux 3.8.
//! Multi-arg commands are exec'd directly by tmux (no `sh -c`), so the
//! Claude prompt is one argv element and needs no quoting.

use crate::domain::WorktreePath;

/// tmux `#{window_id}`, for example `@12`. Stable for the life of the server,
/// unlike the window index.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(String);

#[derive(Clone, Debug)]
pub(super) enum Place {
    /// `$TMUX` is set: new windows go into the session dispatch runs in.
    Inside { session_id: String },
    /// Not in tmux: windows go into a detached session `dispatch-<repo dir>`,
    /// created on the first start.
    Detached { session_name: String },
}

impl Place {
    pub(super) fn detect(tmux_env: Option<&str>, repo_dir_name: &str) -> Result<Place, String> {
        // TODO: Some(_) -> tmux display-message -p '#{session_id}' -> Inside
        //       None    -> Detached { "dispatch-" + sanitized(repo_dir_name) }
        //       (tmux forbids `.` and `:` in session names; map them to `-`.)
        todo!()
    }
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
    // TODO: tmux list-windows -a -F LIST_FORMAT
    //       "no server running" / "error connecting" -> Ok(vec![]).
    todo!()
}

/// Rows with an empty `@dispatch-worktree` are not ours and are skipped.
/// Rows with a wrong field count are skipped, not fatal: one odd window must
/// not hide every session.
fn parse_windows(text: &str) -> Vec<WindowObs> {
    todo!()
}

/// One tmux client invocation, so the window and its tags appear together:
///
/// Inside:   tmux new-window -d -P -F '#{window_id}' -t <session_id>: -c <dir> -n <name> -- <argv..>
///             ; set-option -w @dispatch-worktree <dir> ; set-option -w remain-on-exit on
/// Detached: same with `new-session -d -s <name>` when `has-session -t =<name>` fails.
///
/// Returns the new window id from `-P -F`.
pub(super) fn open_window(place: &Place, dir: &WorktreePath, name: &str, argv: &[String]) -> Result<WindowId, String> {
    // TODO: build args; run; parse first stdout line as WindowId.
    //       VERIFY before relying on it: that a chained `set-option -w` without `-t`
    //       targets the window `new-window -d` just made. If it does not, run a second
    //       `set-option -w -t <id>` call and accept the small crash window (rationale).
    todo!()
}

/// For `Workspace::resume` on a dead pane: tmux respawn-window -k -t <id> -c <dir> -- <argv..>.
/// Keeps the window id and its tags.
pub(super) fn respawn(id: &WindowId, dir: &WorktreePath, argv: &[String]) -> Result<(), String> {
    todo!()
}

pub(super) fn kill_window(id: &WindowId) -> Result<(), String> {
    // TODO: tmux kill-window -t <id>; "can't find window" -> Ok(()).
    todo!()
}

pub(super) fn capture(id: &WindowId, lines: u16) -> Result<Vec<String>, String> {
    // TODO: tmux capture-pane -p -J -t <id> -S -<lines>; drop trailing empty lines.
    todo!()
}

#[cfg(test)]
mod tests {
    // TODO: parse_windows: untagged rows skipped; dead=1; waiting=1; path with spaces.
    // TODO: Place::detect name sanitizing: "my.repo" -> "dispatch-my-repo".
}
