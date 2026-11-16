use super::*;

#[test]
fn a_attaches_a_live_session_and_resumes_an_exited_one() {
    let (mut state, _) = app();
    let cmds = press(&mut state, key('a'));
    assert!(cmds.is_empty());
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("Press s")));

    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    let cmds = press(&mut state, key('a'));
    assert!(matches!(cmds.as_slice(), [Cmd::Attach(n)] if *n == IssueNumber(3)));

    state.sessions.insert(IssueNumber(3), session(3, SessionState::NeedsYou));
    assert!(matches!(press(&mut state, key('a')).as_slice(), [Cmd::Attach(_)]), "needs-you attaches too");

    state.sessions.insert(IssueNumber(3), session(3, SessionState::Exited));
    let cmds = press(&mut state, key('a'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Resume(n, _))] if *n == IssueNumber(3)));
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("claude --continue")));
}

#[test]
fn x_stops_only_a_live_session_by_issue_number() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('x')).is_empty());
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Exited));
    assert!(press(&mut state, key('x')).is_empty(), "exited has nothing to stop");
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    let cmds = press(&mut state, key('x'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Stop(n))] if *n == IssueNumber(3)));
}

#[test]
fn results_of_writes_clear_starting_toast_and_ask_for_an_observation() {
    let (mut state, _) = app();
    state.starting.insert(IssueNumber(3));
    let cmds = press(&mut state, Msg::Started { issue: IssueNumber(3), result: Err("boom".to_string()) });
    assert!(state.starting.is_empty());
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("boom")));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::ObserveSessions)]));
    let again = press(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
    assert!(again.is_empty(), "an observation is already in flight");
}

#[test]
fn a_finished_start_shows_its_session_at_once_with_its_warning() {
    let (mut state, _) = app();
    state.starting.insert(IssueNumber(3));
    let outcome =
        StartOutcome { session: session(3, SessionState::Running), warnings: vec!["fetch failed".to_string()] };
    press(&mut state, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
    assert!(state.sessions.contains_key(&IssueNumber(3)));
    assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("fetch failed")));
}

#[test]
fn toast_is_stamped_then_expires() {
    let (mut state, now) = app();
    say(&mut state, "hi".to_string());
    press(&mut state, Msg::Tick(now));
    assert!(state.toast.is_some());
    press(&mut state, Msg::Tick(now + TOAST_FOR));
    assert!(state.toast.is_none());
}

#[test]
fn a_sent_result_shows_a_toast() {
    let (mut state, _) = app();
    press(&mut state, Msg::Sent { issue: IssueNumber(3), result: Ok(()) });
    assert!(state.toast.as_ref().is_some_and(|t| t.text == "Asked #3's agent to fix and merge its PR"));
    press(&mut state, Msg::Sent { issue: IssueNumber(3), result: Err("gone".to_string()) });
    assert!(state.toast.as_ref().is_some_and(|t| t.text == "Merge request to #3 failed: gone"));
}
