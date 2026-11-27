use super::*;
use crate::config::Harness;
use crate::domain::OriginRef;
use crate::session::error::{AttachError, StartWarning};
use crate::transcript::Transcript;

#[test]
fn a_attaches_a_live_session_and_resumes_an_exited_one() {
    let (mut state, _) = app();
    let cmds = press(&mut state, key('a'));
    assert!(cmds.is_empty());
    assert!(state.toast.as_ref().is_some_and(|t| t.text().contains("Press s")));

    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let cmds = press(&mut state, key('a'));
    assert!(matches!(cmds.as_slice(), [Cmd::Attach { key, to: AttachTo::Claude }] if *key == issue_key(3)));

    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    assert!(
        matches!(press(&mut state, key('a')).as_slice(), [Cmd::Attach { to: AttachTo::Claude, .. }]),
        "needs-you attaches too"
    );

    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    state.transcripts.insert(issue_key(3), Transcript { said: None, active_at: jiff::Timestamp::UNIX_EPOCH });
    let cmds = press(&mut state, key('a'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Write(WriteJob::Resume(n, _)))] if *n == issue_key(3)));
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("Resuming #3 with claude --continue…"));
}

#[test]
fn a_resumes_a_codex_session_with_no_transcript_using_the_session_harness() {
    let (mut state, _) = app();
    let mut exited = session(3, SessionState::Exited);
    exited.harness = Harness::Codex;
    state.sessions.insert(issue_key(3), exited);
    assert!(!state.transcripts.contains_key(&issue_key(3)));
    let cmds = press(&mut state, key('a'));
    let [Cmd::Job(Job::Write(WriteJob::Resume(n, launch)))] = cmds.as_slice() else { panic!("one resume") };
    assert_eq!(*n, issue_key(3));
    assert_eq!(launch.harness, Harness::Codex);
    assert_eq!(launch.command.words(), "codex");
    assert_eq!(state.toast.as_ref().map(Toast::text), Some("Resuming #3 with codex resume --last…"));
}

#[test]
fn a_resume_keeps_a_claude_session_on_claude_when_the_config_changes() {
    let (mut state, _) = app();
    state.config.harness = Harness::Codex;
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    state.transcripts.insert(issue_key(3), Transcript { said: None, active_at: jiff::Timestamp::UNIX_EPOCH });
    let cmds = press(&mut state, key('a'));
    let [Cmd::Job(Job::Write(WriteJob::Resume(_, launch)))] = cmds.as_slice() else { panic!("one resume") };
    assert_eq!((launch.harness, launch.command.words()), (Harness::Claude, "claude".to_string()));
}

#[test]
fn a_on_an_exited_session_with_no_conversation_opens_the_start_dialog() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    let cmds = press(&mut state, key('a'));
    assert!(cmds.is_empty(), "claude --continue would exit at once");
    let Focus::Dialog(d) = &state.focus else { panic!("dialog") };
    assert_eq!(d.key, issue_key(3));
    assert_eq!(
        state.toast.as_ref().map(|t| t.text().to_string()).as_deref(),
        Some("#3 has no conversation to continue. Start it again.")
    );

    let (mut state, _) = app();
    state.sessions.insert(issue_key(9), session(9, SessionState::Exited));
    state.tab = Tab::Sessions;
    state.tabs.sessions.selected = Some(issue_key(9));
    assert!(press(&mut state, key('a')).is_empty());
    assert!(!matches!(state.focus, Focus::Dialog(_)));
    assert_eq!(
        state.toast.as_ref().map(|t| t.text().to_string()).as_deref(),
        Some("#9 has no conversation to continue, and the issue is not listed.")
    );
}

#[test]
fn x_stops_only_a_live_session_by_issue_number() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('x')).is_empty());
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert!(press(&mut state, key('x')).is_empty(), "exited has nothing to stop");
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let cmds = press(&mut state, key('x'));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Write(WriteJob::Stop(n)))] if *n == issue_key(3)));
}

#[test]
fn results_of_writes_clear_starting_toast_and_ask_for_an_observation() {
    let (mut state, _) = app();
    state.starting.insert(issue_key(3));
    let cmds = press(&mut state, Msg::Started { key: issue_key(3), result: Err(git_error("boom")) });
    assert!(state.starting.is_empty());
    assert!(state.toast.as_ref().is_some_and(|t| t.text().contains("boom")));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::ObserveSessions))]));
    let again = press(&mut state, Msg::Stopped { key: issue_key(3), result: Ok(()) });
    assert!(again.is_empty(), "an observation is already in flight");
}

#[test]
fn a_finished_start_shows_its_session_at_once_with_its_warning() {
    let (mut state, _) = app();
    state.starting.insert(issue_key(3));
    let error = crate::proc::fake::fail("git", 128, "no network");
    let warning = StartWarning::FetchFailed { base: OriginRef::parse("origin/main").expect("ref"), error };
    let outcome = StartOutcome { session: session(3, SessionState::Running), warnings: vec![warning] };
    press(&mut state, Msg::Started { key: issue_key(3), result: Ok(outcome) });
    assert!(state.sessions.contains_key(&issue_key(3)));
    let toast = state.toast.as_ref().map(Toast::text);
    assert_eq!(
        toast,
        Some("Started #3 (warning: fetch failed, using the local origin/main: git failed with code 128: no network).")
    );
}

#[test]
fn toast_is_stamped_then_expires() {
    let (mut state, now) = app();
    info(&mut state, "Hi.".to_string());
    press(&mut state, Msg::Tick(now));
    assert!(state.toast.is_some());
    press(&mut state, Msg::Tick(now + TOAST_FOR));
    assert!(state.toast.is_none());
}

#[test]
fn an_error_toast_stays_until_the_next_key() {
    let (mut state, now) = app();
    error(&mut state, "Starting #3 failed: git failed with code 128.".to_string());
    press(&mut state, Msg::Tick(now));
    press(&mut state, Msg::Tick(now + TOAST_FOR * 10));
    assert!(state.toast.is_some(), "time does not remove an error");
    press(&mut state, key('j'));
    assert!(state.toast.is_none(), "a key removes it");
}

#[test]
fn a_sent_result_shows_a_toast() {
    let (mut state, _) = app();
    press(&mut state, Msg::Sent { key: issue_key(3), kind: SendKind::MergeFix, result: Ok(()) });
    assert!(state.toast.as_ref().is_some_and(|t| t.text() == "Asked #3's agent to fix and merge its PR."));
    let result = Err(SessionError::NotRunning(issue_key(3)));
    press(&mut state, Msg::Sent { key: issue_key(3), kind: SendKind::MergeFix, result });
    assert!(
        state
            .toast
            .as_ref()
            .is_some_and(|t| t.text() == "Sending the merge request to #3 failed: #3 has no running session.")
    );
}

#[test]
fn bang_opens_a_shell_for_a_live_or_exited_session() {
    let (mut state, _) = app();
    assert!(press(&mut state, key('!')).is_empty());
    assert_eq!(toast(&state), "No session for this issue. Press s to start one.");
    for st in [SessionState::Running, SessionState::Exited] {
        state.sessions.insert(issue_key(3), session(3, st));
        let cmds = press(&mut state, key('!'));
        assert!(matches!(cmds.as_slice(), [Cmd::Attach { key, to: AttachTo::Shell }] if *key == issue_key(3)));
    }
}

#[test]
fn an_attach_failure_keeps_its_text_and_a_shell_failure_says_so() {
    let (mut state, _) = app();
    let failed = || Err(AttachError::EmptyCommand);
    update(&mut state, Msg::Attached { key: issue_key(3), to: AttachTo::Claude, result: failed() });
    assert_eq!(toast(&state), "Attaching to #3 failed: empty attach command.");
    update(&mut state, Msg::Attached { key: issue_key(3), to: AttachTo::Shell, result: failed() });
    assert_eq!(toast(&state), "Opening a shell in #3 failed: empty attach command.");
    assert_eq!(state.toast.as_ref().map(Toast::level), Some(Level::Error));
}
