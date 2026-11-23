use super::*;
use crate::app::merge::{CannotFix, NoTarget};
use crate::domain::{BranchNameError, CommitOid, ForkPush, PrBranch, RepoSlug};
use crate::merge::job::DraftStep;
use crate::merge::plan::MergeRefusal;
use crate::merge::{Consent, MergeAsk, MergeError};

pub(super) fn merges(cmds: &[Cmd]) -> Vec<MergeAsk> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Merge(ask))) => Some(*ask),
            _ => None,
        })
        .collect()
}

pub(super) const ASK: MergeAsk = MergeAsk { pr: PrNumber(10), consent: Consent::MergeOnly };
const ASK_FIX: MergeAsk = MergeAsk { pr: PrNumber(10), consent: Consent::StartFixSession };

pub(super) fn gh_failed(stderr: &str) -> MergeError {
    MergeError::Gh(gh_error(stderr))
}

pub(super) fn ran(result: Result<MergeOutcome, MergeError>) -> MergeRun {
    MergeRun { draft: DraftStep::WasReady, result }
}

fn ran_after_ready(result: Result<MergeOutcome, MergeError>) -> MergeRun {
    MergeRun { draft: DraftStep::MarkedReady, result }
}

pub(super) fn handoff(problem: MergeProblem) -> Msg {
    let mut fresh = pr(10, "fix/3-x", &[3]);
    fresh.mergeability = Mergeability::Conflict;
    Msg::Merged {
        ask: ASK,
        run: ran(Ok(MergeOutcome::Handoff { problem, method: MergeMethod::Rebase, pr: Box::new(fresh) })),
    }
}

pub(super) fn with_ask(msg: Msg, ask: MergeAsk) -> Msg {
    match msg {
        Msg::Merged { run, .. } => Msg::Merged { ask, run },
        other => other,
    }
}

#[test]
fn m_twice_starts_a_merge_job_for_the_session_pr() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::NeedsYou));
    press(&mut state, key('3'));
    assert!(shows(&state, "m", "merge"));
    assert!(press(&mut state, key('m')).is_empty(), "the first press only arms");
    assert_eq!(toast(&state), "Press m again to merge PR #10.");
    let cmds = press(&mut state, key('m'));
    assert_eq!(merges(&cmds), [ASK]);
    assert!(sends(&cmds).is_empty(), "no agent turn for the merge itself");
    assert!(state.armed.is_none());
    assert!(press(&mut state, key('m')).is_empty(), "a third press arms again");
}

#[test]
fn m_merges_without_a_session_from_the_pr_or_the_issue() {
    let (mut state, _) = app();
    assert!(shows(&state, "m", "merge"), "issue #3 has PR #10");
    press(&mut state, key('m'));
    assert_eq!(merges(&press(&mut state, key('m'))), [ASK]);

    press(&mut state, key('2'));
    press(&mut state, key('j'));
    press(&mut state, key('m'));
    assert_eq!(toast(&state), "Press m again to merge PR #11.");
    assert_eq!(merges(&press(&mut state, key('m'))), [MergeAsk { pr: PrNumber(11), consent: Consent::MergeOnly }]);
}

#[test]
fn the_first_m_names_the_problem_and_the_agent_of_a_running_session() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    state.prs.items[0].mergeability = Mergeability::Conflict;
    press(&mut state, key('3'));
    press(&mut state, key('m'));
    assert_eq!(
        toast(&state),
        "PR #10 has a merge conflict with main. Press m again to ask #3's agent to fix and merge it."
    );
    assert_eq!(merges(&press(&mut state, key('m'))), [ASK], "the job decides, not the cached list");
}

#[test]
fn without_a_running_session_the_first_m_offers_a_new_session() {
    let (mut state, _) = app();
    state.prs.items[0].mergeability = Mergeability::Conflict;
    press(&mut state, key('m'));
    assert_eq!(
        toast(&state),
        "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it."
    );
    assert_eq!(merges(&press(&mut state, key('m'))), [ASK_FIX]);
}

#[test]
fn a_problem_found_after_the_user_agreed_starts_a_session_on_the_pr_branch() {
    let (mut state, _) = app();
    let cmds = press(&mut state, with_ask(handoff(MergeProblem::Conflict), ASK_FIX));
    let started = starts(&cmds);
    assert_eq!(started.len(), 1);
    let req = started[0];
    assert_eq!(
        (req.key, req.branch.as_str(), req.mode, &req.from),
        (issue_key(3), "fix/3-x", Mode::Implement, &StartFrom::Origin)
    );
    assert_eq!(req.prompt, plan::fix_session_prompt(PrNumber(10), "main", MergeProblem::Conflict, MergeMethod::Rebase));
    assert!(state.starting.contains(&issue_key(3)));
    assert_eq!((state.tab, state.tabs.sessions.selected), (Tab::Sessions, Some(issue_key(3))));
    assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Starting a session for #3 to fix it.");
}

#[test]
fn a_problem_the_list_did_not_show_needs_one_more_m_for_a_new_session() {
    let (mut state, _) = app();
    press(&mut state, key('m'));
    press(&mut state, key('m'));
    let cmds = press(&mut state, handoff(MergeProblem::Conflict));
    assert!(cmds.is_empty(), "no session before the user agrees");
    assert_eq!(
        toast(&state),
        "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it."
    );
    let started = press(&mut state, key('m'));
    assert_eq!(starts(&started).iter().map(|r| r.key).collect::<Vec<_>>(), [issue_key(3)]);
    assert!(merges(&started).is_empty());
}

#[test]
fn another_key_cancels_the_new_session() {
    let (mut state, _) = app();
    press(&mut state, handoff(MergeProblem::Conflict));
    press(&mut state, key('j'));
    press(&mut state, key('k'));
    let cmds = press(&mut state, key('m'));
    assert!(starts(&cmds).is_empty() && merges(&cmds).is_empty());
    assert_eq!(toast(&state), "Press m again to merge PR #10.", "the m arms a fresh merge");
}

#[test]
fn an_exited_session_on_the_pr_branch_gets_a_new_session() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    let cmds = press(&mut state, with_ask(handoff(MergeProblem::ChecksFailing), ASK_FIX));
    assert!(sends(&cmds).is_empty());
    assert_eq!(starts(&cmds).iter().map(|r| r.key).collect::<Vec<_>>(), [issue_key(3)]);
}

#[test]
fn a_problem_pr_whose_head_branch_name_is_unusable_only_explains() {
    let (mut state, _) = app();
    let mut fresh = pr(10, "fix 3", &[3]);
    fresh.mergeability = Mergeability::Conflict;
    let outcome =
        MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: Box::new(fresh) };
    let cmds = press(&mut state, Msg::Merged { ask: ASK_FIX, run: ran(Ok(outcome)) });
    assert!(starts(&cmds).is_empty());
    assert_eq!(
        toast(&state),
        "PR #10 has a merge conflict with main. PR #10 has the branch name \"fix 3\": BadChar(' ')."
    );
}

pub(super) fn conflict_handoff(head: PrBranch) -> Msg {
    let fresh = PullRequest { head, mergeability: Mergeability::Conflict, ..pr(10, "fix/3-x", &[3]) };
    let outcome =
        MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: Box::new(fresh) };
    Msg::Merged { ask: ASK_FIX, run: ran(Ok(outcome)) }
}

#[test]
fn a_fork_pr_that_denies_maintainer_edits_merges_but_never_gets_a_fix_session() {
    let (mut state, _) = app();
    let fork = PrBranch::fork(PrNumber(10), "fix/3-x".to_string(), ForkPush::Denied);
    state.prs.items[0].head = fork.clone();
    press(&mut state, key('m'));
    assert_eq!(merges(&press(&mut state, key('m'))), [ASK], "a clean fork PR merges directly");

    let refusal = "PR #10 has a merge conflict with main. PR #10 is from a fork that does not let maintainers \
                   push to it, so a session can not push a fix.";
    state.prs.items[0].mergeability = Mergeability::Conflict;
    assert!(press(&mut state, key('m')).is_empty());
    assert_eq!(toast(&state), refusal, "the first m says why, and arms nothing");
    assert!(state.armed.is_none());
    assert!(starts(&press(&mut state, conflict_handoff(fork))).is_empty());
    assert_eq!(toast(&state), refusal);
}

#[test]
fn a_fork_pr_that_allows_maintainer_edits_gets_a_fix_session_on_a_local_branch() {
    let (mut state, _) = app();
    let repo = RepoSlug::parse("maxbeizer/cli").expect("slug");
    let fork = PrBranch::fork(PrNumber(10), "fix/3-x".to_string(), ForkPush::Allowed(repo.clone()));
    let cmds = press(&mut state, conflict_handoff(fork.clone()));
    let started = starts(&cmds);
    assert_eq!(started.len(), 1);
    let head = BranchName::parse("fix/3-x").expect("branch");
    assert_eq!(
        (started[0].branch.as_str(), &started[0].from),
        ("fork/pr-10", &StartFrom::Fork { repo: repo.clone(), branch: head.clone() })
    );
    assert_eq!(
        started[0].prompt,
        plan::fork_fix_session_prompt(PrNumber(10), &repo, "main", MergeProblem::Conflict, MergeMethod::Rebase)
    );
    assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Starting a session for #3 to fix it.");

    let (mut state, _) = app();
    let mut running = session(3, SessionState::Running);
    running.branch = BranchName::parse("fork/pr-10").expect("branch");
    state.sessions.insert(issue_key(3), running);
    state.prs.items[0].head = fork;
    state.prs.items[0].mergeability = Mergeability::Conflict;
    press(&mut state, key('m'));
    assert_eq!(
        toast(&state),
        "PR #10 has a merge conflict with main. Press m again to ask #3's agent to fix and merge it.",
        "the session on the local branch is the PR's session"
    );
}

#[test]
fn a_direct_merge_shows_the_method_and_refreshes_the_prs() {
    let (mut state, _) = app();
    let cmds = press(
        &mut state,
        Msg::Merged {
            ask: ASK,
            run: ran(Ok(MergeOutcome::Merged {
                method: MergeMethod::Squash,
                head_oid: CommitOid::new("sha10".to_string()),
            })),
        },
    );
    assert_eq!(toast(&state), "Merged PR #10 (squash).");
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
    assert!(sends(&cmds).is_empty());
    assert!(stops(&cmds).is_empty(), "no session on the branch");
}

fn stops(cmds: &[Cmd]) -> Vec<SessionKey> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::Stop(n))) => Some(*n),
            _ => None,
        })
        .collect()
}

fn merged_squash() -> Msg {
    Msg::Merged {
        ask: ASK,
        run: ran(Ok(MergeOutcome::Merged {
            method: MergeMethod::Squash,
            head_oid: CommitOid::new("sha10".to_string()),
        })),
    }
}

#[test]
fn a_direct_merge_stops_the_live_session_on_the_pr_branch() {
    for live in [SessionState::Running, SessionState::NeedsYou] {
        let (mut state, _) = app();
        state.sessions.insert(issue_key(3), session(3, live));
        let cmds = press(&mut state, merged_squash());
        assert_eq!(stops(&cmds), [issue_key(3)]);
        assert_eq!(toast(&state), "Merged PR #10 (squash) and stopped #3's session.");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
    }
}

#[test]
fn a_direct_merge_closes_the_window_of_an_exited_session_on_the_pr_branch() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    let cmds = press(&mut state, merged_squash());
    assert_eq!(stops(&cmds), [issue_key(3)]);
    assert_eq!(toast(&state), "Merged PR #10 (squash) and closed #3's window.");
}

#[test]
fn a_direct_merge_leaves_a_session_on_another_branch_alone() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(2), session(2, SessionState::Running));
    let cmds = press(&mut state, merged_squash());
    assert!(stops(&cmds).is_empty(), "#2 is on another branch");
    assert_eq!(toast(&state), "Merged PR #10 (squash).");
}

#[test]
fn a_queued_merge_keeps_the_session() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Queued)) });
    assert!(stops(&cmds).is_empty(), "the PR can still drop out of the queue");
}

#[test]
fn a_problem_pr_sends_the_running_agent_a_prompt_with_the_problem_and_method() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    let cmds = press(&mut state, handoff(MergeProblem::ChecksFailing));
    let prompt = plan::handoff_prompt(PrNumber(10), "main", MergeProblem::ChecksFailing, MergeMethod::Rebase);
    assert_eq!(sends(&cmds), [(issue_key(3), prompt.as_str())]);
    assert!(prompt.contains("failing checks") && prompt.contains("--rebase"));
    assert_eq!(toast(&state), "PR #10 has failing checks. Asking #3's agent to fix it.");
}

#[test]
fn a_queued_merge_says_so() {
    let (mut state, _) = app();
    let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Queued)) });
    assert_eq!(toast(&state), "PR #10 is in the merge queue.");
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))));
}

#[test]
fn a_refused_merge_shows_a_toast_and_refreshes_and_a_failed_one_only_the_toast() {
    let (mut state, _) = app();
    let refused = MergeOutcome::Refused(MergeRefusal::ChecksRunning(PrNumber(10)));
    let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(refused)) });
    assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))), "a stale list catches up");
    assert_eq!(toast(&state), "PR #10 has checks that are still running. Press m again when they finish.");
    let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Err(gh_failed("not allowed"))) });
    assert!(cmds.is_empty());
    assert_eq!(toast(&state), "Merging PR #10 failed: gh failed with code 1: not allowed.");
}

const ASK_READY: MergeAsk = MergeAsk { pr: PrNumber(10), consent: Consent::MarkReadyThenMerge };

#[test]
fn m_on_a_draft_says_it_marks_it_ready_then_the_second_m_merges_with_consent() {
    let (mut state, _) = app();
    state.prs.items[0].state = PrState::Draft;
    assert!(shows(&state, "m", "merge"));
    assert!(press(&mut state, key('m')).is_empty());
    assert_eq!(toast(&state), "PR #10 is a draft. Press m again to mark it ready and merge it.");
    let cmds = press(&mut state, key('m'));
    assert_eq!(merges(&cmds), [ASK_READY]);
}

#[test]
fn a_merge_that_marked_the_pr_ready_but_did_not_merge_says_so_and_refreshes() {
    let handoff_after_ready = || {
        let mut fresh = pr(10, "fix/3-x", &[3]);
        fresh.mergeability = Mergeability::Conflict;
        Ok(MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: Box::new(fresh) })
    };
    let outcomes = [
        (
            Ok(MergeOutcome::Refused(MergeRefusal::StillDraft(PrNumber(10)))),
            "PR #10 is still a draft. PR #10 is now ready for review.",
        ),
        (
            handoff_after_ready(),
            "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it. PR #10 is now ready for review.",
        ),
        (Err(gh_failed("boom")), "Merging PR #10 failed: gh failed with code 1: boom. PR #10 is now ready for review."),
    ];
    for (result, text) in outcomes {
        let (mut state, _) = app();
        let cmds = press(&mut state, Msg::Merged { ask: ASK_READY, run: ran_after_ready(result) });
        assert_eq!(toast(&state), text);
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchPrs)))), "{text}");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Read(ReadJob::FetchIssues)))), "{text}");
    }
}

#[test]
fn a_merge_after_marking_ready_has_no_ready_note() {
    let (mut state, _) = app();
    let msg = Msg::Merged {
        ask: ASK_READY,
        run: ran_after_ready(Ok(MergeOutcome::Merged {
            method: MergeMethod::Squash,
            head_oid: CommitOid::new("sha10".to_string()),
        })),
    };
    press(&mut state, msg);
    assert_eq!(toast(&state), "Merged PR #10 (squash).");
}

#[test]
fn m_from_the_pr_row_reaches_the_same_pr() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('2'));
    press(&mut state, key('m'));
    assert_eq!(merges(&press(&mut state, key('m'))), [ASK]);
}

#[test]
fn another_key_between_the_two_m_presses_disarms() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Running));
    press(&mut state, key('3'));
    press(&mut state, key('m'));
    press(&mut state, key('r'));
    assert!(merges(&press(&mut state, key('m'))).is_empty());
}

#[test]
fn m_explains_why_it_can_not_merge() {
    let (mut state, _) = app();
    press(&mut state, key('3'));
    press(&mut state, key('m'));
    assert_eq!(toast(&state), "Nothing is selected.");

    press(&mut state, key('1'));
    state.prs.items[0].state = PrState::Draft;
    press(&mut state, key('m'));
    assert_eq!(toast(&state), "PR #10 is a draft. Press m again to mark it ready and merge it.");
    assert!(shows(&state, "m", "merge"));

    state.prs.items.remove(0);
    for _ in 0..2 {
        assert!(press(&mut state, key('m')).is_empty());
    }
    assert_eq!(toast(&state), "#3 has no PR yet.");
}

#[test]
fn each_reason_m_can_not_act_keeps_its_sentence() {
    let (pr, issue) = (PrNumber(10), IssueNumber(3));
    let unusable = CannotFix::UnusableHead { pr, raw: "a..b".to_string(), why: BranchNameError::BadSequence("..") };
    let cases = [
        (NoTarget::NothingSelected.to_string(), "Nothing is selected."),
        (NoTarget::NotListed(pr).to_string(), "PR #10 is not in the list."),
        (NoTarget::NoPr(issue).to_string(), "#3 has no PR yet."),
        (NoTarget::NotOpen(pr).to_string(), "PR #10 is not open."),
        (CannotFix::PrBusy(pr).to_string(), "PR #10 already has a session on another branch, or one is starting."),
        (
            CannotFix::EachIssueBusy(pr).to_string(),
            "Each issue that PR #10 closes already has a session on another branch.",
        ),
        (unusable.to_string(), "PR #10 has the branch name \"a..b\": BadSequence(\"..\")."),
        (
            CannotFix::ForkDeniesEdits(pr).to_string(),
            "PR #10 is from a fork that does not let maintainers push to it, so a session can not push a fix.",
        ),
    ];
    for (got, want) in cases {
        assert_eq!(got, want);
    }
}
