//! git adapter. Private to `session`. Parsers are pure and tested on text
//! captured from git 2.56.

use super::error::{FsAction, FsError, GitError, SessionError};
use crate::domain::{BranchName, CommitOid, OriginRef, RepoSlug, WorktreePath};
use crate::proc::{self, ProcError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Worktree {
    pub path: WorktreePath,
    /// `None` for a detached HEAD or a bare entry.
    pub branch: Option<BranchName>,
}

fn git<I, S>(root: &Path, args: I) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    proc::run("git", args, root)
}

/// The repo's two fixed dirs, both canonical.
pub(super) struct RepoDirs {
    /// The main worktree's root, even when `cwd` is inside a linked worktree,
    /// so session worktrees always go under the main checkout.
    pub root: PathBuf,
    /// The git dir that all worktrees share (`.git` of the main checkout).
    pub common: PathBuf,
}

pub(super) fn repo_dirs(cwd: &Path) -> Result<RepoDirs, GitError> {
    let top = git(cwd, ["rev-parse", "--show-toplevel"]).map_err(GitError::Run)?;
    let common = git(cwd, ["rev-parse", "--path-format=absolute", "--git-common-dir"]).map_err(GitError::Run)?;
    let common = PathBuf::from(common.trim());
    let root = match (common.file_name(), common.parent()) {
        (Some(name), Some(parent)) if name == ".git" => parent.to_path_buf(),
        _ => PathBuf::from(top.trim()),
    };
    let canonical =
        |p: &Path| std::fs::canonicalize(p).map_err(|e| GitError::Fs(FsError::new(FsAction::Resolve, p, &e)));
    Ok(RepoDirs { root: canonical(&root)?, common: canonical(&common)? })
}

/// Adds `pattern` as one line to `<common>/info/exclude`, unless a line equal
/// to it is there. Idempotent. dispatch never edits the user's `.gitignore`.
pub(super) fn ensure_excluded(common: &Path, pattern: &str) -> Result<(), FsError> {
    let info = common.join("info");
    let file = info.join("exclude");
    let err = |e: std::io::Error| FsError::new(FsAction::Update, &file, &e);
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

pub(super) fn origin_head(root: &Path) -> Result<String, ProcError> {
    git(root, ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).map(|s| s.trim().to_string())
}

pub(super) fn list_worktrees(root: &Path) -> Result<Vec<Worktree>, ProcError> {
    git(root, ["worktree", "list", "--porcelain"]).map(|t| parse_worktrees(&t))
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

pub(super) fn branch_exists(root: &Path, branch: &BranchName) -> Result<bool, ProcError> {
    let r = format!("refs/heads/{}", branch.as_str());
    match git(root, ["show-ref", "--verify", "--quiet", r.as_str()]) {
        Ok(_) => Ok(true),
        Err(e) if e.exit_code() == Some(1) => Ok(false),
        Err(e) => Err(e),
    }
}

pub(super) fn fetch(root: &Path, remote_ref: &OriginRef) -> Result<(), ProcError> {
    git(root, ["fetch", "origin", remote_ref.branch().as_str()]).map(|_| ())
}

/// The remote dispatch adds for the fork of the session branch `branch`,
/// one per PR, so nothing else uses its name or its refs.
pub(super) fn fork_remote(branch: &BranchName) -> String {
    format!("dispatch-{}", branch.as_str().replace('/', "-"))
}

/// Adds `repo` as a remote with origin's host and protocol. Its fetch and
/// push refspecs map only `head` to `branch`, so `git pull` and `git push`
/// on `branch` need no name from the fork. Then fetches `head` into
/// `branch`; the fetch also sets the remote-tracking ref, so the commits
/// count as pushed. Each `git config` overwrites, so a retry after a crash
/// converges.
pub(super) fn fetch_fork(
    root: &Path,
    repo: &RepoSlug,
    head: &BranchName,
    branch: &BranchName,
) -> Result<(), SessionError> {
    let origin = git(root, ["remote", "get-url", "origin"]).map_err(SessionError::Git)?;
    let url = fork_url(&origin, repo).ok_or_else(|| SessionError::NoForkUrl(origin.trim().to_string()))?;
    let remote = fork_remote(branch);
    let (b, h) = (branch.as_str(), head.as_str());
    let config = [
        (format!("remote.{remote}.url"), url),
        (format!("remote.{remote}.fetch"), format!("+refs/heads/{h}:refs/remotes/{remote}/{h}")),
        (format!("remote.{remote}.push"), format!("refs/heads/{b}:refs/heads/{h}")),
        (format!("branch.{b}.remote"), remote.clone()),
        (format!("branch.{b}.merge"), format!("refs/heads/{h}")),
    ];
    for (key, value) in config {
        git(root, ["config".to_string(), "--replace-all".to_string(), key, value]).map_err(SessionError::Git)?;
    }
    git(root, super::start_argv::fork_fetch_args(&remote, head, branch)).map_err(SessionError::Git)?;
    Ok(())
}

/// Removes the fork remote of `branch`, if it has one.
pub(super) fn remove_fork_remote(root: &Path, branch: &BranchName) -> Result<(), ProcError> {
    let remote = fork_remote(branch);
    match git(root, ["config", "--get", &format!("remote.{remote}.url")]) {
        Ok(_) => git(root, ["remote", "remove", &remote]).map(|_| ()),
        Err(e) if e.exit_code() == Some(1) => Ok(()),
        Err(e) => Err(e),
    }
}

/// `origin` with its `owner/name` path swapped for `repo`. `None` when the
/// URL has no `owner/name` path.
fn fork_url(origin: &str, repo: &RepoSlug) -> Option<String> {
    let url = origin.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let name_at = url.rfind('/')?;
    let owner_at = url[..name_at].rfind(['/', ':'])?;
    let has_path = name_at > owner_at + 1 && name_at + 1 < url.len();
    has_path.then(|| format!("{}{repo}.git", &url[..=owner_at]))
}

pub(super) fn add_worktree(
    root: &Path,
    dir: &Path,
    branch: &BranchName,
    create_from: Option<&OriginRef>,
) -> Result<(), ProcError> {
    git(root, super::start_argv::worktree_add_args(&dir.to_string_lossy(), branch.as_str(), create_from)).map(|_| ())
}

/// Whether `git status --porcelain` in the worktree lists any change,
/// untracked files included.
pub(super) fn has_uncommitted(worktree: &Path) -> Result<bool, ProcError> {
    git(worktree, ["status", "--porcelain"]).map(|t| !t.trim().is_empty())
}

fn commit_exists(worktree: &Path, oid: &CommitOid) -> Result<bool, ProcError> {
    let s = oid.as_str();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let rev = format!("{s}^{{commit}}");
    match git(worktree, ["rev-parse", "--verify", "--quiet", rev.as_str()]) {
        Ok(_) => Ok(true),
        Err(e) if e.exit_code() == Some(1) => Ok(false),
        Err(e) => Err(e),
    }
}

pub(super) fn unpushed_count(
    worktree: &Path,
    branch: &BranchName,
    merged_head: Option<&CommitOid>,
) -> Result<u32, SessionError> {
    let mut args = vec!["rev-list", "--count", branch.as_str(), "HEAD", "--not", "--remotes"];
    if let Some(oid) = merged_head
        && commit_exists(worktree, oid).map_err(SessionError::Git)?
    {
        args.push(oid.as_str());
    }
    let out = git(worktree, args).map_err(SessionError::Git)?;
    out.trim().parse().map_err(|_| SessionError::NotACount(out.trim().to_string()))
}

/// One ref from `tips`: its commit, and how many commits it has that the
/// base does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tip {
    pub commit: String,
    pub ahead_of_base: u32,
}

/// Full ref name -> tip. A ref that does not exist is not in the map.
pub(super) type Tips = BTreeMap<String, Tip>;

/// `refs/heads/<b>`, `refs/remotes/origin/<b>`, and the refs of the fork
/// remote of each branch, in one `for-each-ref`. Without branches it runs nothing: `for-each-ref` with no
/// pattern lists every ref.
pub(super) fn tips(root: &Path, base: &OriginRef, branches: &[&BranchName]) -> Result<Tips, ProcError> {
    if branches.is_empty() {
        return Ok(Tips::new());
    }
    let format = format!("--format=%(refname) %(objectname) %(ahead-behind:refs/remotes/{base})");
    let refs = branches.iter().flat_map(|b| [local_ref(b), origin_ref(b), format!("refs/remotes/{}", fork_remote(b))]);
    git(root, ["for-each-ref".to_string(), format].into_iter().chain(refs)).map(|t| parse_tips(&t))
}

pub(super) fn local_ref(branch: &BranchName) -> String {
    format!("refs/heads/{}", branch.as_str())
}

pub(super) fn origin_ref(branch: &BranchName) -> String {
    format!("refs/remotes/origin/{}", branch.as_str())
}

/// The ref that holds the remote copy of `branch`: its fork's branch, else
/// `origin/<branch>`.
pub(super) fn remote_copy<'t>(tips: &'t Tips, branch: &BranchName) -> Option<(&'t str, &'t Tip)> {
    let fork = format!("refs/remotes/{}/", fork_remote(branch));
    let in_fork = tips.range(fork.clone()..).next().filter(|(name, _)| name.starts_with(&fork));
    in_fork.or_else(|| tips.get_key_value(&origin_ref(branch))).map(|(name, tip)| (name.as_str(), tip))
}

/// Lines like `refs/heads/fix/3-x <sha> 2 0`: ref, commit, ahead, behind.
/// A line that does not parse is dropped.
fn parse_tips(text: &str) -> Tips {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let (name, commit, ahead) = (fields.next()?, fields.next()?, fields.next()?);
            Some((name.to_string(), Tip { commit: commit.to_string(), ahead_of_base: ahead.parse().ok()? }))
        })
        .collect()
}

/// How many paths `git status --porcelain` lists, untracked ones included.
/// `--no-optional-locks`, so the poll never takes the index lock that the
/// session's own git commands need.
pub(super) fn uncommitted_count(worktree: &Path) -> Result<u32, ProcError> {
    let out = git(worktree, ["--no-optional-locks", "status", "--porcelain"])?;
    Ok(u32::try_from(out.lines().filter(|l| !l.is_empty()).count()).unwrap_or(u32::MAX))
}

/// Commits only in `left`, and commits only in `right`.
pub(super) fn left_right(root: &Path, left: &str, right: &str) -> Result<(u32, u32), SessionError> {
    let range = format!("{left}...{right}");
    let out = git(root, ["rev-list", "--left-right", "--count", range.as_str()]).map_err(SessionError::Git)?;
    let bad = || SessionError::NotACount(out.trim().to_string());
    let (l, r) = out.trim().split_once('\t').ok_or_else(bad)?;
    Ok((l.parse().map_err(|_| bad())?, r.parse().map_err(|_| bad())?))
}

/// No `--force`: git itself refuses a worktree with changes.
pub(super) fn remove_worktree(root: &Path, worktree: &Path) -> Result<(), ProcError> {
    git(root, ["worktree".as_ref(), "remove".as_ref(), worktree.as_os_str()]).map(|_| ())
}

/// `-D`, because the caller proved that every commit is on a remote, and
/// `-d` refuses a squash-merged branch. It deletes the `branch.<b>` config
/// section too, so the marker goes with it.
pub(super) fn delete_branch(root: &Path, branch: &BranchName) -> Result<(), ProcError> {
    git(root, ["branch", "-D", branch.as_str()]).map(|_| ())
}

pub(super) fn prune_worktrees(root: &Path) -> Result<(), ProcError> {
    git(root, ["worktree", "prune"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::fake;
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
        let (top, common) = (linked.display().to_string(), main.join(".git").display().to_string());
        fake::install(move |call| {
            Ok(if call.contains("--show-toplevel") { top.clone() } else { format!("{common}\n") })
        });
        let dirs = repo_dirs(&linked).expect("dirs");
        assert_eq!(dirs.root, main);
        assert_eq!(dirs.common, main.join(".git"));
    }

    #[test]
    fn fetch_splits_remote_and_branch() {
        fake::install(|_| Ok(String::new()));
        fetch(Path::new("/"), &OriginRef::parse("origin/release/1.0").expect("ref")).expect("ok");
        assert_eq!(fake::calls(), ["git fetch origin release/1.0"]);
    }

    #[test]
    fn a_fork_url_keeps_origin_host_and_protocol() {
        let fork = RepoSlug::parse("maxbeizer/cli").expect("slug");
        for (origin, want) in [
            ("https://github.com/cli/cli.git\n", Some("https://github.com/maxbeizer/cli.git")),
            ("https://github.com/cli/cli/", Some("https://github.com/maxbeizer/cli.git")),
            ("git@github.com:cli/cli.git", Some("git@github.com:maxbeizer/cli.git")),
            ("ssh://git@ghe.example.com/cli/cli", Some("ssh://git@ghe.example.com/maxbeizer/cli.git")),
            ("cli", None),
            ("git@github.com:cli", None),
        ] {
            assert_eq!(fork_url(origin, &fork).as_deref(), want, "{origin}");
        }
    }

    #[test]
    fn fetch_fork_adds_the_remote_tracks_the_head_and_fetches_it() {
        fake::install(|call| {
            Ok(if call.contains("get-url") { "git@github.com:cli/cli.git\n".to_string() } else { String::new() })
        });
        let fork = RepoSlug::parse("maxbeizer/cli").expect("slug");
        fetch_fork(Path::new("/"), &fork, &branch("fix-scope"), &branch("fork/pr-7")).expect("ok");
        assert_eq!(
            fake::calls(),
            [
                "git remote get-url origin",
                "git config --replace-all remote.dispatch-fork-pr-7.url git@github.com:maxbeizer/cli.git",
                "git config --replace-all remote.dispatch-fork-pr-7.fetch \
                 +refs/heads/fix-scope:refs/remotes/dispatch-fork-pr-7/fix-scope",
                "git config --replace-all remote.dispatch-fork-pr-7.push refs/heads/fork/pr-7:refs/heads/fix-scope",
                "git config --replace-all branch.fork/pr-7.remote dispatch-fork-pr-7",
                "git config --replace-all branch.fork/pr-7.merge refs/heads/fix-scope",
                "git fetch dispatch-fork-pr-7 refs/heads/fix-scope:refs/heads/fork/pr-7",
            ]
        );
    }

    #[test]
    fn the_remote_copy_of_a_fork_branch_is_in_its_fork() {
        let tip = |commit: &str| Tip { commit: commit.to_string(), ahead_of_base: 0 };
        let tips: Tips = [
            ("refs/remotes/dispatch-fork-pr-7/fix-scope", tip("f")),
            ("refs/remotes/dispatch-fork-pr-70/other", tip("x")),
            ("refs/remotes/origin/fix/3-x", tip("o")),
        ]
        .into_iter()
        .map(|(name, tip)| (name.to_string(), tip))
        .collect();
        let copy = |b: &str| remote_copy(&tips, &branch(b)).map(|(name, tip)| (name, tip.commit.as_str()));
        assert_eq!(copy("fork/pr-7"), Some(("refs/remotes/dispatch-fork-pr-7/fix-scope", "f")));
        assert_eq!(copy("fix/3-x"), Some(("refs/remotes/origin/fix/3-x", "o")));
        assert_eq!(copy("fork/pr-8"), None);
    }
}
