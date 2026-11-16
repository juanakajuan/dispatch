use super::*;

fn opens(cmds: &[Cmd]) -> Vec<GithubRef> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Read(ReadJob::Open(t))) => Some(*t),
            _ => None,
        })
        .collect()
}

#[test]
fn o_opens_the_selected_issue_pr_or_session_issue() {
    let (mut state, _) = app();
    press(&mut state, key('j'));
    assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Issue(IssueNumber(2))]);
    assert!(state.toast.as_ref().is_some_and(|t| t.text() == "Opening issue #2 on GitHub"));

    press(&mut state, key('2'));
    press(&mut state, key('j'));
    assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Pr(PrNumber(11))]);

    state.sessions.insert(IssueNumber(7), session(7, SessionState::Running));
    press(&mut state, key('3'));
    assert_eq!(
        opens(&press(&mut state, key('o'))),
        [GithubRef::Issue(IssueNumber(7))],
        "an issue not in the list still opens"
    );
}

#[test]
fn o_on_an_empty_list_sends_nothing() {
    let (mut state, _) = app();
    press(&mut state, key('3'));
    assert!(press(&mut state, key('o')).is_empty());
    assert!(state.toast.as_ref().is_some_and(|t| t.text() == "Nothing to open."));

    press(&mut state, key('1'));
    press(&mut state, key('/'));
    for c in "zzz".chars() {
        press(&mut state, key(c));
    }
    press(&mut state, code(KeyCode::Enter));
    assert!(press(&mut state, key('o')).is_empty(), "a filter that hides every row");
}

#[test]
fn o_while_filtering_types_into_the_filter() {
    let (mut state, _) = app();
    press(&mut state, key('/'));
    assert!(press(&mut state, key('o')).is_empty());
    assert_eq!(state.filter, "o");
}

#[test]
fn o_over_ssh_copies_the_url_instead_of_opening_a_browser() {
    let (mut state, _) = app();
    state.context.open_with = OpenWith::Clipboard;
    press(&mut state, key('j'));
    let cmds = press(&mut state, key('o'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::FetchPageUrl(GithubRef::Issue(IssueNumber(2)))))]));
    assert!(state.toast.as_ref().is_some_and(|t| t.text() == "Copying the issue #2 URL"));
    let bar: Vec<_> = key_bar(&state).into_iter().filter(|(k, _)| *k == "o").collect();
    assert_eq!(bar, [("o", "copy URL")]);

    let target = GithubRef::Issue(IssueNumber(2));
    let url = "https://github.com/o/r/issues/2".to_string();
    let cmds = press(&mut state, Msg::PageUrl { target, result: Ok(url.clone()) });
    assert!(matches!(cmds.as_slice(), [Cmd::Copy { target: t, text }] if *t == target && *text == url));

    press(&mut state, Msg::Copied { target, result: Ok(url) });
    assert!(
        state.toast.as_ref().is_some_and(|t| t.text() == "Copied https://github.com/o/r/issues/2"),
        "the toast shows the URL, because a terminal can ignore OSC 52 without an error"
    );
}

#[test]
fn a_failed_url_lookup_or_copy_shows_the_error() {
    let (mut state, _) = app();
    let target = GithubRef::Pr(PrNumber(10));
    assert!(press(&mut state, Msg::PageUrl { target, result: Err(gh_error("gh down")) }).is_empty());
    assert_eq!(toast(&state), "Copy PR #10 URL failed: gh failed with code 1: gh down");
    let no_tmux = crate::clipboard::CopyError::Tmux(crate::proc::fake::fail("tmux", 1, "no tmux"));
    press(&mut state, Msg::Copied { target, result: Err(no_tmux) });
    assert_eq!(toast(&state), "Copy PR #10 URL failed: tmux failed with code 1: no tmux");
}

#[test]
fn a_failed_open_shows_the_error_and_a_success_is_quiet() {
    let (mut state, _) = app();
    let target = GithubRef::Pr(PrNumber(10));
    assert!(press(&mut state, Msg::Opened { target, result: Ok(()) }).is_empty());
    assert!(state.toast.is_none());
    press(&mut state, Msg::Opened { target, result: Err(gh_error("no browser")) });
    assert_eq!(toast(&state), "Open PR #10 failed: gh failed with code 1: no browser");
}
