//! A session's worktree diff, read with git only.

use super::error::SessionError;
use crate::domain::OriginRef;
use crate::proc::{self, ProcError};
use std::path::Path;

/// What a worktree changed against its base. `patch` is the text of
/// `git diff --stat --patch`; `untracked` lists files git does not track yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub untracked: Vec<String>,
    pub patch: String,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.untracked.is_empty() && self.patch.trim().is_empty()
    }
}

fn git<I, S>(worktree: &Path, args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("git", args, worktree)
}

/// Commits ahead of the merge base plus uncommitted tracked changes, against
/// the working tree; then the untracked files that git does not ignore.
pub(super) fn read(worktree: &Path, base: &OriginRef) -> Result<Diff, SessionError> {
    let base = base.to_string();
    let patch =
        git(worktree, ["diff", "--merge-base", base.as_str(), "--stat", "--patch"]).map_err(SessionError::Git)?;
    let others = git(worktree, ["ls-files", "--others", "--exclude-standard"]).map_err(SessionError::Git)?;
    Ok(Diff { untracked: others.lines().filter(|l| !l.is_empty()).map(String::from).collect(), patch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;

    #[test]
    fn reads_the_patch_against_the_merge_base_and_the_untracked_files() {
        fake::install(|call| {
            Ok(if call.starts_with("git ls-files") {
                "new.txt\n\nb/c.rs\n".to_string()
            } else {
                "diff --git a b\n".to_string()
            })
        });
        let base = OriginRef::parse("origin/main").expect("ref");
        let diff = read(Path::new("/tmp"), &base).expect("ok");
        assert_eq!(diff, Diff { untracked: vec!["new.txt".into(), "b/c.rs".into()], patch: "diff --git a b\n".into() });
        assert_eq!(
            fake::calls(),
            ["git diff --merge-base origin/main --stat --patch", "git ls-files --others --exclude-standard"]
        );
    }

    #[test]
    fn an_empty_diff_has_no_patch_text_and_no_untracked_files() {
        assert!(Diff::default().is_empty());
        assert!(Diff { untracked: Vec::new(), patch: "\n".into() }.is_empty());
        assert!(!Diff { untracked: vec!["a".into()], patch: String::new() }.is_empty());
        assert!(!Diff { untracked: Vec::new(), patch: "x".into() }.is_empty());
    }

    #[test]
    fn a_git_failure_is_a_git_error() {
        fake::install(|_| Err(fake::fail("git", 128, "fatal: bad")));
        let base = OriginRef::parse("origin/main").expect("ref");
        let err = read(Path::new("/tmp"), &base).expect_err("fails");
        assert!(matches!(err, SessionError::Git(_)));
    }
}
