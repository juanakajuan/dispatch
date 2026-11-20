use super::{
    PaneRef, Place, close_workspace, create_worktree, launch, list_windows, open_window, script_file, send_line,
};
use crate::domain::{BranchName, OriginRef, SessionKey, WorktreePath};
use crate::session::AttachPlan;
use crate::session::error::MuxError;
use crate::session::mux::{Mux, Observed, Repo};
use std::path::Path;

impl Mux for Place {
    type Window = PaneRef;

    fn list(&self, _repo: Repo<'_>) -> Result<Vec<Observed<PaneRef>>, MuxError> {
        list_windows()
    }

    /// herdr makes the worktree itself, so its workspace is nested under the
    /// source workspace.
    fn add_worktree(
        &self,
        repo: Repo<'_>,
        dir: &Path,
        branch: &BranchName,
        from: Option<&OriginRef>,
        key: SessionKey,
    ) -> Result<(), MuxError> {
        create_worktree(self, repo.root, dir, branch, from, key)
    }

    fn open(&self, repo: Repo<'_>, dir: &WorktreePath, key: SessionKey, argv: &[String]) -> Result<(), MuxError> {
        open_window(self, repo.root, dir, key, argv, &script_file(repo.git_common_dir, key))
    }

    fn relaunch(
        &self,
        repo: Repo<'_>,
        window: &PaneRef,
        _dir: &WorktreePath,
        key: SessionKey,
        argv: &[String],
    ) -> Result<(), MuxError> {
        launch(window, argv, &script_file(repo.git_common_dir, key))
    }

    fn stop(&self, window: &PaneRef) -> Result<(), MuxError> {
        close_workspace(&window.workspace)
    }

    fn send_line(&self, window: &PaneRef, text: &str) -> Result<(), MuxError> {
        send_line(window, text)
    }

    fn attach(&self, window: &PaneRef) -> AttachPlan {
        let workspace = window.workspace.as_str();
        match self {
            Place::Inside { .. } => {
                AttachPlan::Switch(["herdr", "workspace", "focus", workspace].map(String::from).to_vec())
            }
            Place::Detached { .. } => AttachPlan::Suspend(
                ["sh", "-c", "herdr workspace focus \"$1\" >/dev/null && exec herdr", "sh", workspace]
                    .map(String::from)
                    .to_vec(),
            ),
        }
    }
}
