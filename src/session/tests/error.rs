use crate::domain::{IssueNumber, OriginRef};
use crate::proc::fake;
use crate::session::error::{
    FsAction, FsError, GitError, MuxError, SessionError, SetupError, StartConflict, StartWarning,
};
use crate::session::tmux::WindowId;
use std::path::Path;
use std::time::Duration;

fn fs(action: FsAction) -> FsError {
    let source = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    FsError::new(action, Path::new("/r/x"), &source)
}

#[test]
fn each_error_and_warning_keeps_its_sentence() {
    let git = || fake::fail("git", 128, "boom");
    let n = IssueNumber(7);
    let window = WindowId::parse("@3").expect("id");
    let cases: Vec<(String, &str)> = vec![
        (SessionError::NoSession(n).to_string(), "no session for #7"),
        (SessionError::NotRunning(n).to_string(), "#7 has no running session"),
        (SessionError::NotVisibleAfterStart(n).to_string(), "session for #7 is not visible after start"),
        (SessionError::Git(git()).to_string(), "git failed with code 128: boom"),
        (
            SessionError::Conflict(StartConflict::PathTaken("/r/x".into())).to_string(),
            "/r/x exists and is not a git worktree. Move it away and start again.",
        ),
        (
            StartWarning::FetchFailed { base: OriginRef::parse("origin/main").expect("ref"), error: git() }.to_string(),
            "fetch failed, using the local origin/main: git failed with code 128: boom",
        ),
        (
            StartWarning::NotExcluded(fs(FsAction::Update)).to_string(),
            ".claude/worktrees still shows in git status: cannot update /r/x: denied",
        ),
        (MuxError::Run(fake::fail("tmux", 1, "x")).to_string(), "tmux failed with code 1: x"),
        (MuxError::Reply { program: "hc", detail: "eof".into() }.to_string(), "hc printed unexpected JSON: eof"),
        (MuxError::NotAWindowId("no".into()).to_string(), "tmux returned \"no\" instead of a window id"),
        (
            MuxError::Untagged { window, error: fake::fail("tmux", 1, "x") }.to_string(),
            "window @3 died before it was tagged: tmux failed with code 1: x",
        ),
        (
            MuxError::ServerTimeout { program: "herdr", waited: Duration::from_secs(5) }.to_string(),
            "herdr did not start within 5s",
        ),
        (MuxError::Fs(fs(FsAction::Write)).to_string(), "cannot write /r/x: denied"),
        (
            MuxError::UnsafeScriptPath("/a'b".into()).to_string(),
            "cannot launch from /a'b: the path has a quote, a backslash, or a newline",
        ),
        (MuxError::NoHerdrWorkspace.to_string(), "HERDR_ENV is set but HERDR_WORKSPACE_ID is not"),
        (
            SetupError::NotARepo(GitError::Run(git())).to_string(),
            "dispatch must run inside a git repository: git failed with code 128: boom",
        ),
        (
            SetupError::NotARepo(GitError::Fs(fs(FsAction::Resolve))).to_string(),
            "dispatch must run inside a git repository: cannot resolve /r/x: denied",
        ),
        (
            SetupError::NoOriginHead(git()).to_string(),
            "cannot find the default branch of origin: git failed with code 128: boom\nrun `git remote set-head origin --auto` first",
        ),
        (SetupError::Mux(MuxError::NoHerdrWorkspace).to_string(), "HERDR_ENV is set but HERDR_WORKSPACE_ID is not"),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
