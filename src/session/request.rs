use crate::config::{ClaudeCommand, Config};
use crate::domain::{BranchName, Issue, IssueNumber, Mode};

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

/// The editable defaults the start dialog opens with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartDefaults {
    pub branch: BranchName,
    pub mode: Mode,
    pub prompt: String,
}

pub fn start_defaults(issue: &Issue) -> StartDefaults {
    StartDefaults { branch: branch_for(issue), mode: Mode::Implement, prompt: prompt_for(issue) }
}

/// `fix/142-retry-queue-drops-jobs`. Prefix from the first matching label
/// (bug -> fix, docs/documentation -> docs, chore -> chore, else feat).
/// Slug: lowercase ASCII alnum words, first 4, joined by `-`.
/// Always yields a valid `BranchName` (the slug can be empty: `feat/142`).
pub(super) fn branch_for(issue: &Issue) -> BranchName {
    let prefix = issue
        .labels
        .iter()
        .find_map(|l| match l.to_ascii_lowercase().as_str() {
            "bug" => Some("fix"),
            "docs" | "documentation" => Some("docs"),
            "chore" => Some("chore"),
            _ => None,
        })
        .unwrap_or("feat");
    let slug = issue
        .title
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let name = if slug.is_empty() {
        format!("{prefix}/{}", issue.number.0)
    } else {
        format!("{prefix}/{}-{slug}", issue.number.0)
    };
    BranchName::parse(&name).unwrap_or_else(|_| BranchName::numbered(issue.number))
}

pub(super) fn prompt_for(issue: &Issue) -> String {
    let n = issue.number.0;
    format!(
        "Implement GitHub issue #{n}: {}\n\nRead the issue and its comments with `gh issue view {n} --comments`.\nAdd or update tests. Open a ready-for-review PR (not a draft) that closes #{n} when the tests pass.",
        issue.title
    )
}
