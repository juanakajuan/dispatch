use super::*;
use crate::session::{Push, Work};

fn read(state: &mut State, n: u32, diff: Diff) {
    update(state, Msg::Diff { key: issue_key(n), result: Ok(diff) });
}

fn patch(text: &str) -> Diff {
    Diff { untracked: Vec::new(), patch: text.to_string() }
}

fn reading(state: &State) -> bool {
    matches!(state.focus, Focus::Diff { key, body: DiffBody::Reading, .. } if key == issue_key(3))
}

#[test]
fn d_without_a_session_toasts_and_with_one_starts_a_read() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('d')).is_empty());
    assert_eq!(toast(&state), "No session for this issue. Press s to start one.");
    assert!(matches!(state.focus, Focus::List));

    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    let cmds = press(&mut state, key('d'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::Diff(k)))] if *k == issue_key(3)));
    assert!(reading(&state), "an exited session has a diff too");
}

#[test]
fn the_read_diff_fills_the_overlay() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('d'));
    read(&mut state, 3, patch("+a\n"));
    assert!(matches!(&state.focus, Focus::Diff { body: DiffBody::Read(d), .. } if d.patch == "+a\n"));
}

#[test]
fn an_empty_diff_closes_the_overlay_with_a_toast() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('d'));
    read(&mut state, 3, patch("\n"));
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(toast(&state), "#3 has no changes.");
}

#[test]
fn a_failed_read_closes_the_overlay_with_a_red_toast() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('d'));
    update(&mut state, Msg::Diff { key: issue_key(3), result: Err(git_error("fatal: bad")) });
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(toast(&state), "Reading the diff of #3 failed: git failed with code 128: fatal: bad");
    assert!(state.toast.as_ref().map(Toast::level) == Some(Level::Error));
}

#[test]
fn a_stale_result_is_ignored() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    read(&mut state, 3, patch("+a\n"));
    assert!(matches!(state.focus, Focus::List), "no overlay is open");
    press(&mut state, key('d'));
    read(&mut state, 2, patch("+b\n"));
    assert!(reading(&state), "another issue's result does not land");
    read(&mut state, 3, patch("+a\n"));
    read(&mut state, 3, patch("+c\n"));
    assert!(matches!(&state.focus, Focus::Diff { body: DiffBody::Read(d), .. } if d.patch == "+a\n"));
    press(&mut state, code(KeyCode::Esc));
    update(&mut state, Msg::Diff { key: issue_key(3), result: Err(git_error("x")) });
    assert!(state.toast.is_none(), "a result for a closed overlay says nothing");
}

#[test]
fn j_k_scroll_and_esc_or_d_close() {
    let (mut state, now) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('d'));
    read(&mut state, 3, patch("+a\n"));
    assert!(press(&mut state, key('q')).is_empty(), "list keys do not act while it is open");
    state.hits.max_scroll = 2;
    for _ in 0..3 {
        press(&mut state, key('j'));
    }
    assert!(matches!(state.focus, Focus::Diff { scroll: 2, .. }), "j stops at the end");
    press(&mut state, code(KeyCode::Up));
    assert!(matches!(state.focus, Focus::Diff { scroll: 1, .. }));
    press(&mut state, key('k'));
    press(&mut state, key('k'));
    assert!(matches!(state.focus, Focus::Diff { scroll: 0, .. }));
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    assert!(matches!(state.focus, Focus::Diff { scroll: 1, .. }), "the wheel scrolls it");
    press(&mut state, code(KeyCode::Esc));
    assert!(matches!(state.focus, Focus::List));
    press(&mut state, key('d'));
    press(&mut state, key('d'));
    assert!(matches!(state.focus, Focus::List), "d closes it again");
}

#[test]
fn the_key_bar_shows_diff_and_shell_only_with_a_session() {
    let (mut state, _) = app();
    assert!(!shows(&state, "d", "show diff") && !shows(&state, "!", "open shell"));
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert!(shows(&state, "d", "show diff") && shows(&state, "!", "open shell"));
}

#[test]
fn the_key_bar_shows_diff_only_when_the_session_has_changes() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));

    for tab in ['1', '2', '3'] {
        press(&mut state, key(tab));
        assert!(!shows(&state, "d", "show diff"), "tab {tab}: no work entry");
        assert!(shows(&state, "!", "open shell"), "tab {tab}: shell still shows");
    }

    state.work.insert(issue_key(3), Work { commits: 0, uncommitted: 0, push: Push::NotPushed });
    for tab in ['1', '2', '3'] {
        press(&mut state, key(tab));
        assert!(!shows(&state, "d", "show diff"), "tab {tab}: zero commits and zero uncommitted");
        assert!(shows(&state, "!", "open shell"), "tab {tab}: shell still shows with zero work");
    }

    for work in [
        Work { commits: 1, uncommitted: 0, push: Push::NotPushed },
        Work { commits: 0, uncommitted: 1, push: Push::Pushed },
    ] {
        state.work.insert(issue_key(3), work);
        for tab in ['1', '2', '3'] {
            press(&mut state, key(tab));
            assert!(shows(&state, "d", "show diff"), "tab {tab}: {work:?}");
        }
    }
}
