use crate::config::{ClaudeCommand, Config};
use crate::domain::{BranchName, IssueNumber, Mode};

/// What the start dialog hands over after validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequest {
    pub issue: IssueNumber,
    pub branch: BranchName,
    pub mode: Mode,
    pub prompt: String,
    pub from: StartFrom,
}

/// Where a new branch starts. Used only when `branch` does not exist locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartFrom {
    /// The default branch, for new work on an issue.
    Base,
    /// The branch of the same name on origin, for work on an open PR.
    Origin,
}

/// The config that shapes a start or resume. It is a value the caller passes
/// per call, so a settings change reaches the next session with no shared
/// state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub detect_waiting: bool,
    pub claude: ClaudeCommand,
}

impl Launch {
    pub fn from_config(config: &Config) -> Launch {
        Launch { detect_waiting: config.detect_waiting, claude: config.claude_command.clone() }
    }
}
