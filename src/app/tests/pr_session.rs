use super::merge::{merges, ran};
use super::*;
use crate::app::merge::CannotFix;
use crate::domain::{CommitOid, ForkPush, PrBranch, RepoSlug};
use crate::merge::{Consent, MergeAsk};

fn pr_session(n: u32, state: SessionState) -> Session {
    Session { key: pr_key(n), ..session(n, state) }
}

fn on_pr(state: &mut State, n: u32) {
    press(state, key('2'));
    state.tabs.prs.selected = Some(PrNumber(n));
}

fn opens(cmds: &[Cmd]) -> Vec<GithubRef> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Read(ReadJob::Open(t))) => Some(*t),
            _ => None,
        })
        .collect()
}

fn conflicting_unlinked_pr(state: &mut State) {
    state.prs.items[1].mergeability = Mergeability::Conflict;
    press(state, key('2'));
    state.tabs.prs.selected = Some(PrNumber(11));
}

#[test]
fn m_twice_on_a_conflicting_pr_that_closes_no_issue_starts_a_session_keyed_by_the_pr() {
    let (mut state, _) = app();
    conflicting_unlinked_pr(&mut state);
    assert!(press(&mut state, key('m')).is_empty(), "the first press arms");
    assert_eq!(
        toast(&state),
        "PR #11 has a merge conflict with main. Press m again to start a session for PR #11 that fixes it."
    );
    let ask = MergeAsk { pr: PrNumber(11), consent: Consent::StartFixSession };
    assert_eq!(merges(&press(&mut state, key('m'))), [ask]);
    let mut fresh = pr(11, "other", &[]);
    fresh.mergeability = Mergeability::Conflict;
    let outcome =
        MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: Box::new(fresh) };
    let cmds = press(&mut state, Msg::Merged { ask, run: ran(Ok(outcome)) });
    let started = starts(&cmds);
    assert_eq!(started.len(), 1);
    assert_eq!(
        (started[0].key, started[0].branch.as_str(), &started[0].from),
        (pr_key(11), "other", &StartFrom::Origin)
    );
    assert!(state.starting.contains(&pr_key(11)));
    assert_eq!((state.tab, state.tabs.sessions.selected), (Tab::Sessions, Some(pr_key(11))));
    assert_eq!(toast(&state), "PR #11 has a merge conflict with main. Starting a session for PR #11 to fix it.");
}

#[test]
fn a_fork_pr_with_edits_and_no_issue_starts_a_pr_session_on_the_local_branch() {
    let (mut state, _) = app();
    let repo = RepoSlug::parse("maxbeizer/cli").expect("slug");
    let fork = PrBranch::fork(PrNumber(11), "dep-bump".to_string(), ForkPush::Allowed(repo.clone()));
    let fresh = PullRequest { head: fork, mergeability: Mergeability::Conflict, ..pr(11, "x", &[]) };
    let ask = MergeAsk { pr: PrNumber(11), consent: Consent::StartFixSession };
    let outcome =
        MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: Box::new(fresh) };
    let cmds = press(&mut state, Msg::Merged { ask, run: ran(Ok(outcome)) });
    let started = starts(&cmds);
    assert_eq!(started.len(), 1);
    let head = BranchName::parse("dep-bump").expect("branch");
    assert_eq!(
        (started[0].key, started[0].branch.as_str(), &started[0].from),
        (pr_key(11), "fork/pr-11", &StartFrom::Fork { repo, branch: head })
    );
    assert!(!started[0].prompt.contains("dep-bump"), "a fork's branch name stays out of the prompt");
}

#[test]
fn a_pr_that_closes_no_issue_and_already_has_a_session_or_start_is_not_started_twice() {
    let (mut state, _) = app();
    conflicting_unlinked_pr(&mut state);
    state.starting.insert(pr_key(11));
    press(&mut state, key('m'));
    assert_eq!(
        toast(&state),
        "PR #11 has a merge conflict with main. PR #11 already has a session on another branch, or one is starting."
    );
    assert!(state.armed.is_none());
}

#[test]
fn a_running_pr_session_fixes_its_pr_and_m_on_its_row_finds_the_pr() {
    let (mut state, _) = app();
    conflicting_unlinked_pr(&mut state);
    let mut running = session(11, SessionState::Running);
    running.key = pr_key(11);
    running.branch = BranchName::parse("other").expect("branch");
    state.sessions.insert(pr_key(11), running);
    press(&mut state, key('3'));
    state.tabs.sessions.selected = Some(pr_key(11));
    press(&mut state, key('m'));
    assert_eq!(
        toast(&state),
        "PR #11 has a merge conflict with main. Press m again to ask PR #11's agent to fix and merge it."
    );
}

#[test]
fn s_on_a_pr_opens_the_dialog_on_the_pr_branch_and_ignores_typing_in_it() {
    let (mut state, _) = app();
    on_pr(&mut state, 11);
    assert!(shows(&state, "s", "start session"));
    assert!(press(&mut state, key('s')).is_empty());
    let d = dialog(&state);
    assert_eq!((d.key, &d.from), (pr_key(11), &StartFrom::Origin));
    assert_eq!(d.branch.lines(), ["other"]);
    assert!(!d.branch_editable());
    assert!(d.prompt.lines()[0].starts_with("Work on GitHub PR #11: PR 11"));
    for c in "zz".chars() {
        press(&mut state, key(c));
    }
    assert_eq!(dialog(&state).branch.lines(), ["other"], "the PR branch is not editable");
    assert_eq!(dialog(&state).prompt.lines()[0].chars().count(), "Work on GitHub PR #11: PR 11".chars().count());
}

#[test]
fn ctrl_s_in_a_pr_dialog_starts_a_session_keyed_by_the_pr() {
    let (mut state, _) = app();
    on_pr(&mut state, 11);
    press(&mut state, key('s'));
    let cmds = press(&mut state, ctrl('s'));
    let reqs = starts(&cmds);
    assert_eq!(reqs.len(), 1);
    assert_eq!((reqs[0].key, reqs[0].branch.as_str(), &reqs[0].from), (pr_key(11), "other", &StartFrom::Origin));
    assert!(reqs[0].prompt.contains("`gh pr view 11 --comments`") && reqs[0].prompt.contains("`gh pr diff 11`"));
    assert!(state.starting.contains(&pr_key(11)));
    assert!(state.marked.is_empty());
    assert_eq!((state.tab, state.tabs.sessions.selected), (Tab::Sessions, Some(pr_key(11))));
    assert_eq!(toast(&state), "Starting PR #11…");
}

#[test]
fn s_on_a_fork_pr_with_edits_starts_on_the_local_branch_and_never_names_the_fork_branch_in_the_prompt() {
    let (mut state, _) = app();
    let repo = RepoSlug::parse("maxbeizer/cli").expect("slug");
    state.prs.items[1].head = PrBranch::fork(PrNumber(11), "dep-bump".to_string(), ForkPush::Allowed(repo.clone()));
    on_pr(&mut state, 11);
    press(&mut state, key('s'));
    assert!(dialog(&state).prompt.lines().iter().all(|l| !l.contains("dep-bump")));
    let cmds = press(&mut state, ctrl('s'));
    let head = BranchName::parse("dep-bump").expect("branch");
    assert_eq!(
        (starts(&cmds)[0].branch.as_str(), &starts(&cmds)[0].from),
        ("fork/pr-11", &StartFrom::Fork { repo, branch: head })
    );
}

#[test]
fn s_on_a_pr_that_dispatch_can_not_push_to_says_why_and_opens_no_dialog() {
    let (mut state, _) = app();
    state.prs.items[1].head = PrBranch::fork(PrNumber(11), "dep-bump".to_string(), ForkPush::Denied);
    on_pr(&mut state, 11);
    assert!(press(&mut state, key('s')).is_empty());
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(toast(&state), CannotFix::ForkDeniesEdits(PrNumber(11)).to_string());

    state.prs.items[1].head = PrBranch::parse("a b".to_string());
    press(&mut state, key('s'));
    assert!(matches!(state.focus, Focus::List));
    assert!(toast(&state).starts_with("PR #11 has the branch name \"a b\""));
}

#[test]
fn s_on_a_merged_or_closed_pr_only_says_it_is_not_open() {
    for closed in [PrState::Merged, PrState::Closed] {
        let (mut state, _) = app();
        state.prs.items[1].state = closed;
        on_pr(&mut state, 11);
        assert!(!shows(&state, "s", "start session"));
        assert!(press(&mut state, key('s')).is_empty());
        assert!(matches!(state.focus, Focus::List));
        assert_eq!(toast(&state), "PR #11 is not open.");
    }
}

#[test]
fn s_on_a_pr_with_a_session_on_its_branch_or_key_goes_to_that_session() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    on_pr(&mut state, 10);
    press(&mut state, key('s'));
    assert_eq!((state.tab, selected_row(&state)), (Tab::Sessions, Some(RowKey::Session(issue_key(3)))));
    assert!(matches!(state.focus, Focus::List), "the issue session already works on the PR branch");

    let (mut state, _) = app();
    let mut own = pr_session(11, SessionState::Exited);
    own.branch = BranchName::parse("renamed").expect("branch");
    state.sessions.insert(pr_key(11), own);
    on_pr(&mut state, 11);
    press(&mut state, key('s'));
    assert_eq!(selected_row(&state), Some(RowKey::Session(pr_key(11))));

    let (mut state, _) = app();
    state.starting.insert(pr_key(11));
    on_pr(&mut state, 11);
    press(&mut state, key('s'));
    assert_eq!(toast(&state), "PR #11 is already starting.");
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn a_pr_session_and_an_issue_session_with_one_number_are_two_rows() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(11), session(11, SessionState::Running));
    state.sessions.insert(pr_key(11), pr_session(11, SessionState::Exited));
    press(&mut state, key('3'));
    assert_eq!(visible_rows(&state).len(), 2);
}

#[test]
fn o_on_a_pr_session_row_opens_the_pr_not_the_issue() {
    let (mut state, _) = app();
    state.sessions.insert(pr_key(7), pr_session(7, SessionState::Running));
    state.sessions.insert(issue_key(7), session(7, SessionState::Running));
    press(&mut state, key('3'));
    state.tabs.sessions.selected = Some(pr_key(7));
    assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Pr(PrNumber(7))]);
    state.tabs.sessions.selected = Some(issue_key(7));
    assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Issue(IssueNumber(7))]);
}

#[test]
fn a_x_t_and_d_act_on_a_pr_session() {
    let (mut state, _) = app();
    state.sessions.insert(pr_key(11), pr_session(11, SessionState::Running));
    press(&mut state, key('3'));
    state.tabs.sessions.selected = Some(pr_key(11));

    assert!(
        matches!(press(&mut state, key('a')).as_slice(), [Cmd::Attach { key: k, to: AttachTo::Claude }] if *k == pr_key(11))
    );
    assert!(matches!(
        press(&mut state, key('x')).as_slice(),
        [Cmd::Job(Job::Write(WriteJob::Stop(k)))] if *k == pr_key(11)
    ));
    assert_eq!(toast(&state), "Stopping PR #11…");

    press(&mut state, key('t'));
    assert!(matches!(&state.focus, Focus::Message { key: k, .. } if *k == pr_key(11)));
    for c in "hi".chars() {
        press(&mut state, key(c));
    }
    let sent = press(&mut state, code(KeyCode::Enter));
    assert_eq!(sends(&sent), [(pr_key(11), "hi")]);

    assert!(press(&mut state, key('D')).is_empty());
    assert_eq!(toast(&state), "PR #11 is still running. Press x to stop it first.");
    state.sessions.insert(pr_key(11), pr_session(11, SessionState::Exited));
    press(&mut state, key('D'));
    assert_eq!(toast(&state), "Press D again to remove PR #11's worktree and branch.");
    let cmds = press(&mut state, key('D'));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::Job(Job::Write(WriteJob::Remove(t)))] if t.key == pr_key(11) && t.merged_head.is_none()
    ));
}

#[test]
fn a_merged_pr_session_is_removable_with_the_merged_head() {
    let (mut state, _) = app();
    state.prs.items[1].state = PrState::Merged;
    state.prs.items[1].head_oid = CommitOid::new("abc".to_string());
    let mut own = pr_session(11, SessionState::Running);
    own.branch = BranchName::parse("other").expect("branch");
    state.sessions.insert(pr_key(11), own);
    press(&mut state, key('3'));
    state.tabs.sessions.selected = Some(pr_key(11));
    press(&mut state, key('D'));
    let cmds = press(&mut state, key('D'));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::Job(Job::Write(WriteJob::Remove(t)))] if t.key == pr_key(11) && t.merged_head == Some(CommitOid::new("abc".to_string()))
    ));
}

#[test]
fn a_session_row_for_a_pr_takes_its_title_and_filter_facets_from_the_pr() {
    let (mut state, _) = app();
    state.prs.items[1].labels = vec!["dependencies".to_string()];
    state.sessions.insert(pr_key(11), pr_session(11, SessionState::Running));
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('3'));
    state.filter_mut().set("label:dependencies");
    assert_eq!(visible_rows(&state), [RowKey::Session(pr_key(11))]);
    state.filter_mut().set("PR 11");
    assert_eq!(visible_rows(&state), [RowKey::Session(pr_key(11))], "the PR title matches");
}

#[test]
fn a_session_key_on_a_pr_with_no_session_names_the_pr() {
    for c in ['a', 'i', 'd', 'D', 'x', 't', '!'] {
        let (mut state, _) = app();
        on_pr(&mut state, 11);
        assert!(press(&mut state, key(c)).is_empty(), "{c}");
        assert_eq!(toast(&state), "No session for this PR. Press s to start one.", "{c}");
    }
}

#[test]
fn the_no_session_toast_offers_s_only_when_s_can_start() {
    let (mut state, _) = app();
    state.prs.items[1].state = PrState::Merged;
    on_pr(&mut state, 11);
    press(&mut state, key('a'));
    assert_eq!(toast(&state), "No session for this PR.");
}

#[test]
fn s_on_a_pr_whose_branch_has_a_session_offers_to_go_to_it() {
    let (mut state, _) = app();
    let mut existing = session(11, SessionState::Running);
    existing.key = pr_key(11);
    state.sessions.insert(pr_key(11), existing);
    on_pr(&mut state, 11);
    assert!(!shows(&state, "s", "start session"));
    assert!(shows(&state, "s", "go to session"));
    press(&mut state, key('s'));
    assert_eq!(state.tab, Tab::Sessions);
}
