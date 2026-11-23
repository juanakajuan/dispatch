use super::*;
use crate::config::Harness;
use crate::domain::{ForkPush, PrBranch, RepoSlug};
use crate::session::{Push, Work};
use crate::transcript::{Summary, SummaryKind, Transcript};

#[test]
fn key_bar_follows_the_context() {
    let (mut state, _) = app();
    let names = |s: &State| key_bar(s).iter().map(|b| format!("{} {}", b.keys, b.desc)).collect::<Vec<_>>();
    assert!(names(&state).contains(&"s start session".to_string()));
    assert!(names(&state).contains(&"o open on GitHub".to_string()));
    assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    assert!(names(&state).contains(&"a attach".to_string()));
    assert!(names(&state).contains(&"x stop".to_string()));
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert!(names(&state).contains(&"a resume".to_string()));
    press(&mut state, key('2'));
    assert!(names(&state).contains(&"enter go to issue".to_string()));
    assert_eq!(
        names(&state),
        [
            "? show keys",
            "s start session",
            "enter go to issue",
            "a resume",
            "! open shell",
            "D remove",
            "m merge",
            "o open on GitHub",
            "q quit"
        ]
    );
}

#[test]
fn every_binding_has_a_description() {
    for b in BINDINGS {
        assert!(!b.desc.trim().is_empty(), "{:?} has no description", b.action);
    }
}

#[test]
fn every_binding_key_is_unique() {
    let mut seen: Vec<Key> = Vec::new();
    for k in BINDINGS.iter().flat_map(|b| b.keys) {
        assert!(!seen.contains(k), "{k:?} is bound twice");
        seen.push(*k);
    }
}

#[test]
fn release_events_are_ignored() {
    let (mut state, _) = app();
    let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
    ev.kind = KeyEventKind::Release;
    assert!(press(&mut state, Msg::Key(ev)).is_empty());
}

#[test]
fn every_bound_key_round_trips_through_its_key_event() {
    for key in BINDINGS.iter().flat_map(|b| b.keys) {
        assert_eq!(key_of(&key.event()), Some(*key), "{}", key.label());
    }
}

#[test]
fn every_key_bar_entry_carries_the_first_key_of_its_binding() {
    let (mut state, _) = app();
    let bar_keys = |state: &State| key_bar(state).into_iter().map(|b| (b.keys, b.key)).collect::<Vec<_>>();
    assert_eq!(
        bar_keys(&state),
        [
            ("?", Key::Char('?')),
            ("s", Key::Char('s')),
            ("space", Key::Char(' ')),
            ("m", Key::Char('m')),
            ("o", Key::Char('o')),
            ("q", Key::Char('q'))
        ]
        .map(|(keys, key)| (keys.to_string(), key))
    );
    press(&mut state, key('2'));
    assert!(bar_keys(&state).contains(&("enter".to_string(), Key::Code(KeyCode::Enter))));
}

#[test]
fn clicking_a_key_does_what_pressing_it_does() {
    let (mut state, now) = with_target(HitTarget::Key(Key::Char('?')));
    press(&mut state, click(3, 0, now));
    assert!(matches!(state.focus, Focus::Keybinds { .. }), "the ? key bar entry opens the keys menu");

    let (mut state, now) = with_target(HitTarget::Key(Key::Char('q')));
    assert!(matches!(press(&mut state, click(0, 0, now)).as_slice(), [Cmd::Quit]));

    let (mut pressed, _) = app();
    press(&mut pressed, key('S'));
    let (mut state, now) = with_target(HitTarget::Key(Key::Char('S')));
    press(&mut state, click(0, 0, now));
    assert_eq!(
        state.tabs.sort_label(Tab::Issues),
        pressed.tabs.sort_label(Tab::Issues),
        "the sort footer cycles the sort"
    );
    assert_ne!(state.tabs.sort_label(Tab::Issues), app().0.tabs.sort_label(Tab::Issues));
}

#[test]
fn clicking_a_keybinds_entry_closes_the_menu_and_runs_its_action() {
    let (mut state, now) = app();
    press(&mut state, key('?'));
    state.hits.targets = vec![(Rect::new(0, 0, 10, 1), HitTarget::Binding(Action::Settings))];
    press(&mut state, click(1, 0, now));
    assert!(matches!(state.focus, Focus::Settings(_)), "the , entry opens settings");

    let (mut state, now) = app();
    press(&mut state, key('?'));
    state.hits.targets = vec![(Rect::new(0, 0, 10, 1), HitTarget::Binding(Action::Refresh))];
    let cmds = press(&mut state, click(1, 0, now));
    assert!(matches!(state.focus, Focus::List));
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))), "the r entry refreshes");
}

fn bar_pairs(state: &State) -> Vec<(String, &'static str)> {
    key_bar(state).into_iter().map(|h| (h.keys, h.desc)).collect()
}

fn pairs<const N: usize>(want: [(&str, &'static str); N]) -> Vec<(String, &'static str)> {
    want.into_iter().map(|(keys, desc)| (keys.to_string(), desc)).collect()
}

#[test]
fn empty_tabs_show_only_keys_and_quit() {
    let only_global = pairs([("?", "show keys"), ("q", "quit")]);

    let (mut state, _) = app();
    press(&mut state, key('3'));
    assert_eq!(bar_pairs(&state), only_global, "empty sessions tab with browser");
    state.context.open_with = OpenWith::Clipboard;
    assert_eq!(bar_pairs(&state), only_global, "empty sessions tab with clipboard");

    let (mut state, _) = app();
    state.issues.items.clear();
    assert_eq!(bar_pairs(&state), only_global, "empty issues tab");

    let (mut state, _) = app();
    press(&mut state, key('2'));
    state.prs.items.clear();
    assert_eq!(bar_pairs(&state), only_global, "empty pull requests tab");

    let (mut state, _) = app();
    *state.filter_mut() = "zzz-no-match".to_string();
    assert_eq!(bar_pairs(&state), only_global, "filtered empty issues tab");

    press(&mut state, key('2'));
    *state.filter_mut() = "zzz-no-match".to_string();
    assert_eq!(bar_pairs(&state), only_global, "filtered empty pull requests tab");
}

#[test]
fn enter_shows_only_on_prs_that_close_an_issue_and_never_duplicates_attach() {
    let (mut state, _) = app();
    assert!(!key_bar(&state).iter().any(|h| h.keys == "enter"), "issues tab uses s");

    press(&mut state, key('2'));
    state.tabs.prs.selected = Some(PrNumber(10));
    assert!(shows(&state, "enter", "go to issue"), "PR #10 closes #3");
    state.tabs.prs.selected = Some(PrNumber(11));
    assert!(!key_bar(&state).iter().any(|h| h.keys == "enter"), "PR #11 closes no issue");

    press(&mut state, key('3'));
    for live in [SessionState::Running, SessionState::NeedsYou] {
        state.sessions.insert(issue_key(3), session(3, live));
        let bar = bar_pairs(&state);
        assert_eq!(
            bar.iter().filter(|(k, d)| (k.as_str(), *d) == ("a", "attach")).count(),
            1,
            "{live:?}: one attach hint"
        );
        assert!(!bar.iter().any(|(k, _)| *k == "enter"), "{live:?}: sessions tab uses a");
    }

    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    let bar = bar_pairs(&state);
    assert_eq!(bar.iter().filter(|(k, d)| (k.as_str(), *d) == ("a", "resume")).count(), 1, "one resume hint");
    assert!(!bar.iter().any(|(k, _)| *k == "enter"), "exited session uses a");
}

#[test]
fn space_to_mark_hides_when_issue_has_a_session_or_is_starting() {
    let (mut state, _) = app();
    assert!(shows(&state, "space", "mark"), "unmarked issue without a session");

    press(&mut state, key(' '));
    assert!(shows(&state, "space", "mark"), "marked issue can be unmarked");
    press(&mut state, key(' '));

    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    assert!(!shows(&state, "space", "mark"), "issue with a running session");
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert!(!shows(&state, "space", "mark"), "issue with an exited session");

    state.sessions.clear();
    state.starting.insert(issue_key(3));
    assert!(!shows(&state, "space", "mark"), "issue that is starting");
}

#[test]
fn s_to_start_session_hides_when_selected_row_cannot_start() {
    let (mut state, _) = app();
    state.starting.insert(issue_key(3));
    assert!(!shows(&state, "s", "start session"), "issue already starting with no marks");
    state.marked.insert(IssueNumber(2));
    assert!(shows(&state, "s", "start session"), "marks let s open the batch dialog");

    let (mut state, _) = app();
    press(&mut state, key('2'));
    state.tabs.prs.selected = Some(PrNumber(11));

    state.prs.items[1].head = PrBranch::fork(PrNumber(11), "dep-bump".to_string(), ForkPush::Denied);
    assert!(!shows(&state, "s", "start session"), "fork PR denying edits");

    let mut existing = session(11, SessionState::Running);
    existing.key = pr_key(11);
    state.sessions.insert(pr_key(11), existing);
    assert!(shows(&state, "s", "start session"), "existing session lets s jump to it");
    state.sessions.clear();

    let repo = RepoSlug::parse("maxbeizer/cli").expect("slug");
    state.prs.items[1].head = PrBranch::fork(PrNumber(11), "dep-bump".to_string(), ForkPush::Allowed(repo));
    assert!(shows(&state, "s", "start session"), "fork PR allowing edits");

    state.prs.items[1].head = PrBranch::parse("a b".to_string());
    assert!(!shows(&state, "s", "start session"), "unusable PR head branch");

    state.prs.items[1].head = PrBranch::parse("other".to_string());
    state.starting.insert(pr_key(11));
    assert!(!shows(&state, "s", "start session"), "PR already starting");
}

#[test]
fn i_to_read_summary_hides_for_harnesses_without_transcripts() {
    let (mut state, _) = app();
    let mut running = session(3, SessionState::Running);
    running.harness = Harness::Codex;
    state.sessions.insert(issue_key(3), running);
    let summary = Summary { kind: SummaryKind::Message, text: "done".to_string() };
    state.transcripts.insert(issue_key(3), Transcript { said: Some(summary), active_at: jiff::Timestamp::UNIX_EPOCH });
    assert!(!shows(&state, "i", "read summary"), "Codex does not report what it said");
}

#[test]
fn key_bar_never_emits_duplicate_keys_or_descriptions() {
    let assert_unique = |label: &str, state: &State| {
        let mut keys = Vec::new();
        let mut descs = Vec::new();
        for h in key_bar(state) {
            assert!(!keys.contains(&h.keys), "{label}: duplicate key {:?} in {:?}", h.keys, bar_pairs(state));
            assert!(!descs.contains(&h.desc), "{label}: duplicate desc {:?} in {:?}", h.desc, bar_pairs(state));
            keys.push(h.keys);
            descs.push(h.desc);
        }
    };

    let (mut state, _) = app();
    assert_unique("issues plain", &state);
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    state.work.insert(issue_key(3), Work { commits: 1, uncommitted: 0, push: Push::NotPushed });
    let summary = Summary { kind: SummaryKind::Message, text: "hi".to_string() };
    state.transcripts.insert(issue_key(3), Transcript { said: Some(summary), active_at: jiff::Timestamp::UNIX_EPOCH });
    assert_unique("issues live session", &state);
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    assert_unique("issues exited session", &state);

    press(&mut state, key('2'));
    assert_unique("prs linked session", &state);
    state.tabs.prs.selected = Some(PrNumber(11));
    assert_unique("prs unlinked", &state);

    press(&mut state, key('3'));
    assert_unique("sessions exited", &state);
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    assert_unique("sessions running", &state);
    state.prs.items[0].state = PrState::Merged;
    assert_unique("sessions merged", &state);
}
