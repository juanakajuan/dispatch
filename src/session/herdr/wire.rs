use super::{PaneId, PaneRef, WorkspaceId};
use crate::domain::WorktreePath;
use crate::session::{WindowId, WindowObs};
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Deserialize)]
struct Reply<T> {
    result: T,
}

#[derive(Default, Deserialize)]
pub(super) struct WorkspaceList {
    pub(super) workspaces: Vec<WorkspaceRow>,
}

#[derive(Deserialize)]
pub(super) struct WorkspaceRow {
    pub(super) workspace_id: WorkspaceId,
    pub(super) label: Option<String>,
    pub(super) worktree: Option<WorktreeRow>,
}

#[derive(Deserialize)]
pub(super) struct WorktreeRow {
    checkout_path: Option<String>,
}

#[derive(Default, Deserialize)]
pub(super) struct PaneList {
    pub(super) panes: Vec<PaneRow>,
}

#[derive(Deserialize)]
pub(super) struct PaneRow {
    pane_id: PaneId,
    workspace_id: WorkspaceId,
    agent: Option<String>,
    #[serde(default)]
    agent_status: AgentStatus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    #[serde(other)]
    Unknown,
}

impl AgentStatus {
    fn waiting(self) -> bool {
        matches!(self, AgentStatus::Blocked | AgentStatus::Done | AgentStatus::Idle)
    }
}

#[derive(Deserialize)]
pub(super) struct Opened {
    pub(super) workspace: OpenedWorkspace,
    pub(super) root_pane: OpenedPane,
}

#[derive(Deserialize)]
pub(super) struct Created {
    pub(super) workspace: OpenedWorkspace,
}

#[derive(Deserialize)]
pub(super) struct OpenedWorkspace {
    pub(super) workspace_id: WorkspaceId,
}

#[derive(Deserialize)]
pub(super) struct OpenedPane {
    pub(super) pane_id: PaneId,
}

#[derive(Deserialize)]
pub(super) struct PaneGet {
    pub(super) pane: PaneAgent,
}

#[derive(Deserialize)]
pub(super) struct PaneAgent {
    pub(super) agent: Option<String>,
}

pub(super) fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str::<Reply<T>>(text).map(|r| r.result).map_err(|e| format!("herdr printed unexpected JSON: {e}"))
}

pub(super) fn window_obs(workspaces: &WorkspaceList, panes: &PaneList) -> Vec<WindowObs> {
    workspaces
        .workspaces
        .iter()
        .filter_map(|ws| {
            let checkout = ws.worktree.as_ref()?.checkout_path.as_deref()?;
            let worktree = WorktreePath::from_observed(checkout)?;
            let own: Vec<&PaneRow> = panes.panes.iter().filter(|p| p.workspace_id == ws.workspace_id).collect();
            let pane = own.iter().find(|p| p.agent.is_some()).or(own.first())?;
            Some(WindowObs {
                id: WindowId::Herdr(PaneRef { workspace: ws.workspace_id.clone(), pane: pane.pane_id.clone() }),
                worktree,
                dead: pane.agent.is_none(),
                waiting: pane.agent_status.waiting(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_dir;
    use std::path::Path;

    const WORKSPACES: &str = r##"{"id":"cli:workspace:list","result":{"type":"workspace_list","workspaces":[
        {"workspace_id":"w1","label":"dispatch","worktree":null},
        {"workspace_id":"w2","label":"#7","worktree":{"checkout_path":"CHECKOUT_A","branch":"fix/7-x"}},
        {"workspace_id":"w3","label":"#8","worktree":{"checkout_path":"CHECKOUT_B"}},
        {"workspace_id":"w4","label":"#9","worktree":{"checkout_path":"/definitely/not/here"}}]}}"##;

    const PANES: &str = r#"{"id":"cli:pane:list","result":{"type":"pane_list","panes":[
        {"pane_id":"w1:p1","workspace_id":"w1","agent":null,"agent_status":"unknown"},
        {"pane_id":"w2:p1","workspace_id":"w2","agent":null,"agent_status":"unknown"},
        {"pane_id":"w2:p2","workspace_id":"w2","agent":"claude","agent_status":"blocked"},
        {"pane_id":"w3:p1","workspace_id":"w3","agent":null,"agent_status":"unknown"},
        {"pane_id":"w4:p1","workspace_id":"w4","agent":"claude","agent_status":"working"}]}}"#;

    fn fixture(dirs: &[&Path; 2]) -> (WorkspaceList, PaneList) {
        let text = WORKSPACES
            .replace("CHECKOUT_A", &dirs[0].to_string_lossy())
            .replace("CHECKOUT_B", &dirs[1].to_string_lossy());
        (parse(&text).expect("workspaces"), parse(PANES).expect("panes"))
    }

    #[test]
    fn workspaces_become_windows_by_their_agent_pane() {
        let base = temp_dir("herdr-obs");
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir(&a).expect("mkdir");
        std::fs::create_dir(&b).expect("mkdir");
        let (workspaces, panes) = fixture(&[&a, &b]);
        let got = window_obs(&workspaces, &panes);
        let summary: Vec<_> = got.iter().map(|o| (o.id.clone(), o.dead, o.waiting)).collect();
        assert_eq!(
            summary,
            [
                (WindowId::Herdr(PaneRef::new("w2", "w2:p2")), false, true),
                (WindowId::Herdr(PaneRef::new("w3", "w3:p1")), true, false),
            ],
            "no worktree and a missing checkout are skipped; the agent pane wins; no agent is dead"
        );
        assert!(got[0].worktree.as_path().ends_with("a"));
    }

    #[test]
    fn only_blocked_done_and_idle_wait() {
        let waits = |status: &str| {
            let row = format!(r#"{{"pane_id":"p","workspace_id":"w","agent":"claude","agent_status":"{status}"}}"#);
            serde_json::from_str::<PaneRow>(&row).expect("row").agent_status.waiting()
        };
        assert_eq!(
            ["idle", "working", "blocked", "done", "unknown", "from-the-future"].map(waits),
            [true, false, true, true, false, false]
        );
    }
}
