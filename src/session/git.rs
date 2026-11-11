//! git adapter. Private to `session`. Parsers are pure and tested on text
//! captured from git 2.56.

use crate::domain::{BranchName, IssueNumber, WorktreePath};
use crate::proc::{self, ProcError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Worktree {
    pub path: WorktreePath,
    /// `None` for a detached HEAD or a bare entry.
    pub branch: Option<BranchName>,
}

/// branch -> issue, from `dispatch-issue`. Only values that parsed.
pub(super) type Markers = BTreeMap<BranchName, IssueNumber>;

fn git<I, S>(root: &Path, args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("git", args, root)
}

fn text<I, S>(root: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    git(root, args).map_err(|e| e.to_string())
}

/// The repo's two fixed dirs, both canonical.
pub(super) struct RepoDirs {
    /// The main worktree's root, even when `cwd` is inside a linked worktree,
    /// so session worktrees always go under the main checkout.
    pub root: PathBuf,
    /// The git dir that all worktrees share (`.git` of the main checkout).
    pub common: PathBuf,
}

pub(super) fn repo_dirs(cwd: &Path) -> Result<RepoDirs, String> {
    let top = text(cwd, ["rev-parse", "--show-toplevel"])?;
    let common = text(cwd, ["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let common = PathBuf::from(common.trim());
    let root = match (common.file_name(), common.parent()) {
        (Some(name), Some(parent)) if name == ".git" => parent.to_path_buf(),
        _ => PathBuf::from(top.trim()),
    };
    let canonical = |p: &Path| std::fs::canonicalize(p).map_err(|e| format!("cannot resolve {}: {e}", p.display()));
    Ok(RepoDirs { root: canonical(&root)?, common: canonical(&common)? })
}

/// Adds `pattern` as one line to `<common>/info/exclude`, unless a line equal
/// to it is there. Idempotent. dispatch never edits the user's `.gitignore`.
pub(super) fn ensure_excluded(common: &Path, pattern: &str) -> Result<(), String> {
    let info = common.join("info");
    let file = info.join("exclude");
    let err = |e: std::io::Error| format!("cannot update {}: {e}", file.display());
    let existing = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(err(e)),
    };
    if existing.lines().any(|l| l.trim() == pattern) {
        return Ok(());
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    std::fs::create_dir_all(&info).map_err(err)?;
    std::fs::write(&file, format!("{existing}{separator}{pattern}\n")).map_err(err)
}

pub(super) fn origin_head(root: &Path) -> Result<String, String> {
    text(root, ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("{e}\nrun `git remote set-head origin --auto` first"))
}

pub(super) fn list_worktrees(root: &Path) -> Result<Vec<Worktree>, String> {
    text(root, ["worktree", "list", "--porcelain"]).map(|t| parse_worktrees(&t))
}

/// Porcelain blocks separated by blank lines:
/// `worktree <path>` / `HEAD <sha>` / `branch refs/heads/<name>` | `detached` | `bare`.
/// A `prunable` block (directory deleted by hand) is dropped, and so is a
/// path that no longer resolves.
fn parse_worktrees(text: &str) -> Vec<Worktree> {
    text.split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut branch = None;
            let mut prunable = false;
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    path = Some(p);
                } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                    branch = BranchName::parse(b).ok();
                } else if line == "prunable" || line.starts_with("prunable ") {
                    prunable = true;
                }
            }
            if prunable {
                return None;
            }
            WorktreePath::from_observed(path?).map(|path| Worktree { path, branch })
        })
        .collect()
}

pub(super) fn list_markers(root: &Path) -> Result<Markers, String> {
    match git(root, ["config", "--get-regexp", r"^branch\..*\.dispatch-issue$"]) {
        Ok(t) => Ok(parse_markers(&t)),
        Err(e) if e.exit_code() == Some(1) => Ok(Markers::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Lines like `branch.fix/142-retry-queue.dispatch-issue 142`.
/// The branch is everything between `branch.` and the last `.`; the value is
/// everything after the first space.
fn parse_markers(text: &str) -> Markers {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(' ')?;
            let b = key.strip_prefix("branch.")?.strip_suffix(".dispatch-issue")?;
            Some((BranchName::parse(b).ok()?, IssueNumber(value.trim().parse().ok()?)))
        })
        .collect()
}

pub(super) fn branch_exists(root: &Path, branch: &BranchName) -> Result<bool, String> {
    let r = format!("refs/heads/{}", branch.as_str());
    match git(root, ["show-ref", "--verify", "--quiet", r.as_str()]) {
        Ok(_) => Ok(true),
        Err(e) if e.exit_code() == Some(1) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// `base` is `origin/main`: fetch that one branch from that remote.
pub(super) fn fetch(root: &Path, base: &str) -> Result<(), String> {
    let (remote, branch) = base.split_once('/').unwrap_or(("origin", base));
    text(root, ["fetch", remote, branch]).map(|_| ())
}

pub(super) fn add_worktree(
    root: &Path,
    dir: &Path,
    branch: &BranchName,
    create_from: Option<&str>,
) -> Result<(), String> {
    let dir = dir.to_string_lossy();
    match create_from {
        Some(base) => text(root, ["worktree", "add", dir.as_ref(), "-b", branch.as_str(), base]),
        None => text(root, ["worktree", "add", dir.as_ref(), branch.as_str()]),
    }
    .map(|_| ())
}

pub(super) fn set_marker(root: &Path, branch: &BranchName, issue: IssueNumber) -> Result<(), String> {
    let key = format!("branch.{}.dispatch-issue", branch.as_str());
    text(root, ["config".to_string(), key, issue.0.to_string()]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::temp_dir;

    fn branch(s: &str) -> BranchName {
        BranchName::parse(s).expect("valid branch")
    }

    #[test]
    fn worktrees_main_linked_detached_and_prunable() {
        let base = temp_dir("git-wt");
        for d in ["my repo", "my repo-1", "wt3"] {
            std::fs::create_dir(base.join(d)).expect("mkdir");
        }
        let b = base.display();
        let sha = "07e2ba7143f4de2812171b22d9670532a4857cff";
        let text = format!(
            "worktree {b}/my repo\nHEAD {sha}\nbranch refs/heads/main\n\n\
             worktree {b}/my repo-1\nHEAD {sha}\nbranch refs/heads/fix/1-a.b-c\n\n\
             worktree {b}/wt3\nHEAD {sha}\ndetached\n\n\
             worktree {b}/wt4\nHEAD {sha}\nbranch refs/heads/gone/4\nprunable gitdir file points to non-existent location\n\n\
             worktree {b}/missing\nHEAD {sha}\nbranch refs/heads/gone/5\n\n"
        );
        let got = parse_worktrees(&text);
        let summary: Vec<_> = got
            .iter()
            .map(|w| {
                (
                    w.path.as_path().file_name().expect("name").to_string_lossy().into_owned(),
                    w.branch.as_ref().map(|b| b.as_str().to_string()),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("my repo".to_string(), Some("main".to_string())),
                ("my repo-1".to_string(), Some("fix/1-a.b-c".to_string())),
                ("wt3".to_string(), None),
            ]
        );
    }

    #[test]
    fn exclude_adds_the_line_once_and_keeps_the_user_lines() {
        let common = temp_dir("exclude");
        ensure_excluded(&common, ".claude/worktrees/").expect("creates info/exclude");
        let file = common.join("info").join("exclude");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), ".claude/worktrees/\n");

        std::fs::write(&file, "# user\n*.log").expect("write");
        ensure_excluded(&common, ".claude/worktrees/").expect("appends");
        ensure_excluded(&common, ".claude/worktrees/").expect("second call is a no-op");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "# user\n*.log\n.claude/worktrees/\n");
    }

    #[test]
    fn repo_dirs_from_a_linked_worktree_is_the_main_checkout() {
        let base = temp_dir("repo-dirs");
        let main = base.join("main");
        let linked = main.join(".claude").join("worktrees").join("1");
        std::fs::create_dir_all(main.join(".git")).expect("mkdir .git");
        std::fs::create_dir_all(&linked).expect("mkdir linked");
        use crate::proc::fake;
        let (top, common) = (linked.display().to_string(), main.join(".git").display().to_string());
        fake::install(move |call| {
            Ok(if call.contains("--show-toplevel") { top.clone() } else { format!("{common}\n") })
        });
        let dirs = repo_dirs(&linked).expect("dirs");
        assert_eq!(dirs.root, main);
        assert_eq!(dirs.common, main.join(".git"));
    }

    #[test]
    fn markers_from_real_regexp_output() {
        let text = "branch.fix/1-a.b-c.dispatch-issue 1\n\
                    branch.feat/2-x.dispatch-issue abc\n\
                    branch.feat/3.dispatch-issue 3\n";
        let m = parse_markers(text);
        assert_eq!(m.len(), 2);
        assert_eq!(m[&branch("fix/1-a.b-c")], IssueNumber(1));
        assert_eq!(m[&branch("feat/3")], IssueNumber(3));
        assert!(!m.contains_key(&branch("feat/2-x")), "non-numeric issue is dropped");
    }

    #[test]
    fn fetch_splits_remote_and_branch() {
        use crate::proc::fake;
        fake::install(|_| Ok(String::new()));
        fetch(Path::new("/"), "origin/main").expect("ok");
        assert_eq!(fake::calls(), ["git fetch origin main"]);
    }

    #[test]
    fn marker_list_exit_one_is_empty() {
        use crate::proc::fake;
        fake::install(|_| Err(fake::fail("git", 1, "")));
        assert!(list_markers(Path::new("/")).expect("ok").is_empty());
    }
}
