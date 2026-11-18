use super::*;

#[test]
fn init_asks_for_all_three_feeds_and_the_login() {
    let (state, cmds) = init(Instant::now(), Config::default(), Context::default());
    let jobs: Vec<_> =
        cmds.iter().filter_map(|c| if let Cmd::Job(Job::Read(j)) = c { Some(*j) } else { None }).collect();
    assert_eq!(jobs, [ReadJob::FetchIssues, ReadJob::FetchPrs, ReadJob::ObserveSessions, ReadJob::FetchViewer]);
    assert!(state.issues.poll.in_flight() && state.prs.poll.in_flight() && state.observe.in_flight());
}

#[test]
fn tick_does_not_stack_a_second_fetch_while_one_is_inflight() {
    let (mut state, now) = app();
    state.due.github = now;
    let _ = state.issues.poll.request(ReadJob::FetchIssues);
    let cmds = press(&mut state, Msg::Tick(now + Duration::from_millis(1)));
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchIssues)))));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
    let later = press(&mut state, Msg::Tick(now + Duration::from_millis(2)));
    assert!(later.is_empty(), "due time moved forward");
}

#[test]
fn tick_observes_sessions_every_two_seconds_once() {
    let (mut state, now) = app();
    let t = now + SESSIONS_EVERY;
    let cmds = press(&mut state, Msg::Tick(t));
    assert_eq!(cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::ObserveSessions)))).count(), 1);
    let cmds = press(&mut state, Msg::Tick(t + Duration::from_millis(100)));
    assert!(cmds.is_empty());
}

#[test]
fn issues_error_keeps_the_old_items_and_stamps_the_failure() {
    let (mut state, now) = app();
    let _ = state.issues.poll.request(ReadJob::FetchIssues);
    let failed = now + Duration::from_secs(5);
    press(&mut state, Msg::Issues { result: Err(gh_error("offline")), at: failed });
    assert_eq!(state.issues.items.len(), 3);
    assert!(
        matches!(&state.issues.last, LastFetch::Failed { error, at } if *error == gh_error("offline") && *at == failed)
    );
    assert!(!state.issues.poll.in_flight());
    let arrived = now + Duration::from_secs(9);
    press(&mut state, Msg::Issues { result: Ok(vec![issue(9, "new", &[])]), at: arrived });
    assert_eq!(state.issues.items.len(), 1);
    assert!(matches!(state.issues.last, LastFetch::Fresh { at } if at == arrived));
}

#[test]
fn new_data_resets_the_refresh_time_on_the_timer_and_after_r() {
    let (mut state, _) = app();
    let due = state.due.github;
    let cmds = press(&mut state, Msg::Tick(due));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
    let first = due + Duration::from_secs(2);
    press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: first });
    assert!(matches!(state.prs.last, LastFetch::Fresh { at } if at == first));

    let cmds = press(&mut state, key('r'));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
    let second = first + Duration::from_secs(30);
    press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: second });
    assert!(matches!(state.prs.last, LastFetch::Fresh { at } if at == second));
}

#[test]
fn r_refreshes_without_stacking_and_q_quits() {
    let (mut state, _) = app();
    let cmds = press(&mut state, key('r'));
    assert_eq!(cmds.len(), 3);
    assert!(press(&mut state, key('r')).is_empty());
    assert!(matches!(press(&mut state, key('q')).as_slice(), [Cmd::Quit]));
    assert!(matches!(press(&mut state, ctrl('c')).as_slice(), [Cmd::Quit]));
}

fn github_fetches(cmds: &[Cmd]) -> usize {
    cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchIssues | ReadJob::FetchPrs)))).count()
}

/// Sends `msg`, then checks that the GitHub fetch waits for the cooldown,
/// goes out once, and pushes the 60 s timer back.
fn assert_triggers_one_refresh(state: &mut State, msg: Msg) {
    let from = state.due.github_sent;
    assert_eq!(github_fetches(&press(state, msg)), 0, "the next tick fetches, not the event");
    assert_eq!(
        github_fetches(&press(state, Msg::Tick(from + GITHUB_COOLDOWN.saturating_sub(Duration::from_millis(1))))),
        0,
        "cooldown"
    );
    let at = from + GITHUB_COOLDOWN;
    assert_eq!(github_fetches(&press(state, Msg::Tick(at))), 2);
    assert_eq!(state.due.github, at + state.config.github_refresh, "the timer starts again");
    assert_eq!(github_fetches(&press(state, Msg::Tick(at + GITHUB_COOLDOWN))), 0, "inflight, and no new event");
    press(state, Msg::Issues { result: Ok(Vec::new()), at });
    press(state, Msg::Prs { result: Ok(Vec::new()), at });
    assert_eq!(github_fetches(&press(state, Msg::Tick(at + GITHUB_COOLDOWN * 2))), 0, "one event, one fetch");
}

#[test]
fn a_start_or_a_stop_refreshes_github() {
    let (mut state, _) = app();
    let outcome = StartOutcome { session: session(3, SessionState::Running), warnings: Vec::new() };
    assert_triggers_one_refresh(&mut state, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
    assert_triggers_one_refresh(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });

    let (mut state, now) = app();
    press(&mut state, Msg::Started { issue: IssueNumber(3), result: Err(git_error("boom")) });
    assert_eq!(
        github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN))),
        0,
        "a failed start changes nothing"
    );
}

#[test]
fn a_session_that_goes_idle_or_exits_refreshes_github() {
    let observe = |sessions: Vec<Session>| {
        let sessions = sessions.into_iter().map(|s| (s.issue, s)).collect();
        Msg::Observed(Ok(Observation { sessions, transcripts: BTreeMap::new(), work: crate::session::Works::new() }))
    };
    let running = || session(3, SessionState::Running);
    let needs_you = || session(3, SessionState::NeedsYou);
    let exited = || session(3, SessionState::Exited);
    for (before, after) in
        [(running(), Some(needs_you())), (running(), Some(exited())), (needs_you(), Some(exited())), (running(), None)]
    {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), before);
        assert_triggers_one_refresh(&mut state, observe(after.into_iter().collect()));
    }
    for (before, after) in
        [(running(), running()), (needs_you(), running()), (needs_you(), needs_you()), (exited(), exited())]
    {
        let (mut state, now) = app();
        state.sessions.insert(IssueNumber(3), before);
        press(&mut state, observe(vec![after]));
        assert_eq!(github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN))), 0);
    }
}

#[test]
fn focus_gained_refreshes_github() {
    let (mut state, _) = app();
    assert_triggers_one_refresh(&mut state, Msg::FocusGained);
}

#[test]
fn a_burst_of_events_makes_one_fetch_and_an_event_during_a_fetch_makes_one_more() {
    let (mut state, now) = app();
    for _ in 0..3 {
        press(&mut state, Msg::FocusGained);
        press(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
    }
    let at = now + GITHUB_COOLDOWN;
    assert_eq!(github_fetches(&press(&mut state, Msg::Tick(at))), 2);

    // This fetch may have started before the change, so another one follows it.
    press(&mut state, Msg::FocusGained);
    let later = at + GITHUB_COOLDOWN;
    assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 0, "waits for the fetch in flight");
    press(&mut state, Msg::Issues { result: Ok(Vec::new()), at: later });
    assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 0, "PRs still in flight");
    press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: later });
    assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 2);
}

#[test]
fn the_timer_firing_during_a_fetch_keeps_the_pending_event() {
    let (mut state, now) = app();
    let _ = state.issues.poll.request(ReadJob::FetchIssues);
    press(&mut state, Msg::FocusGained);
    state.due.github = now + GITHUB_COOLDOWN;
    let cmds = press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN));
    assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Read(ReadJob::FetchPrs)), ..]), "the timer fetches what is idle");
    press(&mut state, Msg::Issues { result: Ok(Vec::new()), at: now + GITHUB_COOLDOWN });
    press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: now + GITHUB_COOLDOWN });
    assert_eq!(
        github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN * 2))),
        2,
        "the issues fetch predated the event"
    );
}
