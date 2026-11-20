use super::{GithubRef, IssueNumber, PrNumber};

/// What a session is for. An issue session keeps the legacy markers and
/// dirs; a PR session has its own, so PR #7 and issue #7 never collide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionKey {
    Issue(IssueNumber),
    Pr(PrNumber),
}

impl SessionKey {
    /// The worktree directory name: `<n>` for an issue, `pr-<n>` for a PR.
    pub fn dir_name(self) -> String {
        match self {
            SessionKey::Issue(n) => n.0.to_string(),
            SessionKey::Pr(n) => format!("pr-{}", n.0),
        }
    }
}

impl std::fmt::Display for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionKey::Issue(n) => write!(f, "{n}"),
            SessionKey::Pr(n) => write!(f, "PR {n}"),
        }
    }
}

impl From<IssueNumber> for SessionKey {
    fn from(n: IssueNumber) -> SessionKey {
        SessionKey::Issue(n)
    }
}

impl From<PrNumber> for SessionKey {
    fn from(n: PrNumber) -> SessionKey {
        SessionKey::Pr(n)
    }
}

impl From<SessionKey> for GithubRef {
    fn from(key: SessionKey) -> GithubRef {
        match key {
            SessionKey::Issue(n) => GithubRef::Issue(n),
            SessionKey::Pr(n) => GithubRef::Pr(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_keys_display_order_and_name_their_dir() {
        let issue = SessionKey::Issue(IssueNumber(7));
        let pr = SessionKey::Pr(PrNumber(7));
        assert_eq!((issue.to_string(), pr.to_string()), ("#7".to_string(), "PR #7".to_string()));
        assert_eq!((issue.dir_name(), pr.dir_name()), ("7".to_string(), "pr-7".to_string()));
        assert!(issue < pr && pr < SessionKey::Pr(PrNumber(8)));
        assert_ne!(issue, pr);
        assert_eq!(GithubRef::from(issue), GithubRef::Issue(IssueNumber(7)));
        assert_eq!(GithubRef::from(pr), GithubRef::Pr(PrNumber(7)));
    }
}
