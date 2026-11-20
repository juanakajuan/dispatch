use super::*;
use crate::app::filter::{Facets, Has, Is, Query, RowState, Sign, Term, TokenKind, Viewer, Who, tokens};
use crate::domain::{Checks, ForkPush, PrBranch};

fn kinds(raw: &str) -> Vec<(&str, TokenKind)> {
    tokens(raw).into_iter().map(|t| (&raw[t.at.clone()], t.kind)).collect()
}

fn label(s: &str) -> TokenKind {
    TokenKind::Term(Sign::Include, Term::Label(s.to_string()))
}

fn term(term: Term) -> TokenKind {
    TokenKind::Term(Sign::Include, term)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

fn type_filter(state: &mut State, text: &str) {
    press(state, key('/'));
    for c in text.chars() {
        press(state, key(c));
    }
}

#[test]
fn tokens_split_on_spaces_outside_quotes_and_x_term_keys() {
    assert_eq!(
        kinds(r#"  retry label:"good first issue" Milestone:v1 assignee:@ME assignee:@octo"#),
        [
            ("retry", TokenKind::Text),
            (r#"label:"good first issue""#, label("good first issue")),
            ("Milestone:v1", term(Term::Milestone("v1".to_string()))),
            ("assignee:@ME", term(Term::Assignee(Who::Me))),
            ("assignee:@octo", term(Term::Assignee(Who::Login("octo".to_string())))),
        ]
    );
    assert_eq!(kinds("label: color:x"), [("label:", TokenKind::Incomplete), ("color:x", TokenKind::Text)]);
    assert_eq!(kinds(r#"label:"good fi"#), [(r#"label:"good fi"#, label("good fi"))], "an open quote runs to the end");
}

#[test]
fn free_text_is_a_case_insensitive_substring_over_all_parts() {
    let parts = ["#142", "Retry queue", "bug", "fix/142-retry"];
    let matches = |raw| Query::parse(raw, None).matches(&parts, &Facets::default());
    assert!(matches(""));
    assert!(matches("RETRY"));
    assert!(matches("#142"));
    assert!(matches("bug fix"));
    assert!(!matches("docs"));
    assert!(matches("label:"), "a term with no value yet hides nothing");
}

#[test]
fn every_term_must_match_its_field_exactly_ignoring_case() {
    let labels = strings(&["bug", "good first issue"]);
    let assignees = strings(&["Octo", "hubot"]);
    let facets = Facets { labels: &labels, assignees: &assignees, milestone: Some("v1.0"), ..Facets::default() };
    let matches = |raw, viewer| Query::parse(raw, viewer).matches(&["#1", "Retry queue"], &facets);
    assert!(matches("label:BUG", None));
    assert!(!matches("label:bu", None), "a label term is not a prefix match");
    assert!(matches(r#"label:"good first issue" label:bug"#, None));
    assert!(!matches("label:bug label:docs", None), "two labels both have to be on the row");
    assert!(matches("milestone:V1.0 retry", None));
    assert!(!matches("milestone:v1.0 docs", None), "the free text still has to match");
    assert!(matches("assignee:octo", None));
    assert!(matches("assignee:@hubot", None));
    assert!(!matches("assignee:other", None));
    assert!(matches("assignee:@me", Some("octo")));
    assert!(!matches("assignee:@me", Some("other")));
    assert!(!matches("assignee:@me", None), "@me matches nothing while the login is unknown");
    let none = Facets::default();
    assert!(!Query::parse("milestone:v1.0", None).matches(&[], &none), "a row with no milestone");
}

#[test]
fn a_label_term_and_text_filter_the_issues_tab() {
    let (mut state, _) = app();
    state.issues.items.push(issue(4, "Retry docs", &["docs"]));
    type_filter(&mut state, "label:bug");
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3))]);
    state.filter = "retry label:docs".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(4))]);
}

#[test]
fn the_pr_tab_filters_by_its_own_labels_assignees_and_milestone() {
    let (mut state, _) = app();
    state.prs.items[1] = PullRequest {
        labels: strings(&["bug"]),
        assignees: strings(&["octo"]),
        milestone: Some("v1".to_string()),
        ..pr(11, "other", &[])
    };
    press(&mut state, key('2'));
    for raw in ["label:bug", "assignee:octo", "milestone:v1", "#11 label:bug"] {
        state.filter = raw.to_string();
        assert_eq!(visible_rows(&state), [RowKey::Pr(PrNumber(11))], "{raw}");
    }
}

#[test]
fn the_session_tab_filters_by_the_fields_of_the_sessions_issue() {
    let (mut state, _) = app();
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    state.sessions.insert(IssueNumber(2), session(2, SessionState::Running));
    state.sessions.insert(IssueNumber(9), session(9, SessionState::Running));
    press(&mut state, key('3'));
    state.filter = "label:docs".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(2))], "#9 has no issue, so it has no labels");
}

#[test]
fn assignee_me_uses_the_login_once_it_lands() {
    let (mut state, _) = app();
    state.issues.items[0].assignees = strings(&["octo"]);
    type_filter(&mut state, "assignee:@me");
    assert_eq!(visible_rows(&state), [], "the login is not known yet");
    press(&mut state, Msg::Viewer(Ok("octo".to_string())));
    assert_eq!(state.viewer, Viewer::Login("octo".to_string()));
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3))]);

    press(&mut state, Msg::Viewer(Err(GhError::EmptyLogin)));
    assert_eq!(state.viewer, Viewer::Failed);
    assert_eq!(visible_rows(&state), []);
}

#[test]
fn tokens_parse_the_new_terms_with_and_without_a_minus() {
    assert_eq!(
        kinds("author:@Octo author:@me state:needs-you checks:FAIL has:pr is:fork"),
        [
            ("author:@Octo", term(Term::Author(Who::Login("Octo".to_string())))),
            ("author:@me", term(Term::Author(Who::Me))),
            ("state:needs-you", term(Term::State(RowState::NeedsYou))),
            ("checks:FAIL", term(Term::Checks(Checks::Fail))),
            ("has:pr", term(Term::Has(Has::Pr))),
            ("is:fork", term(Term::Is(Is::Fork))),
        ]
    );
    assert_eq!(
        kinds("-label:wontfix -has:pr -author:dependabot[bot] -label: -state:in-review"),
        [
            ("-label:wontfix", TokenKind::Term(Sign::Exclude, Term::Label("wontfix".to_string()))),
            ("-has:pr", TokenKind::Term(Sign::Exclude, Term::Has(Has::Pr))),
            (
                "-author:dependabot[bot]",
                TokenKind::Term(Sign::Exclude, Term::Author(Who::Login("dependabot[bot]".to_string())))
            ),
            ("-label:", TokenKind::Incomplete),
            ("-state:in-review", TokenKind::Term(Sign::Exclude, Term::State(RowState::InReview))),
        ]
    );
    assert_eq!(kinds("-retry -foo:bar"), [("-retry", TokenKind::Text), ("-foo:bar", TokenKind::Text)]);
}

#[test]
fn a_value_outside_a_fixed_set_is_invalid_and_matches_no_row() {
    for raw in ["state:bogus", "checks:ok", "has:x", "is:big", "-state:bogus"] {
        assert_eq!(kinds(raw), [(raw, TokenKind::Invalid)], "{raw}");
    }
    let (mut state, _) = app();
    for raw in ["state:bogus", "-state:bogus", "retry has:x"] {
        state.filter = raw.to_string();
        assert_eq!(visible_rows(&state), [], "{raw}");
    }
}

#[test]
fn a_minus_keeps_the_rows_that_the_term_does_not_match() {
    let (mut state, _) = app();
    state.issues.items[1].labels.push("wontfix".to_string());
    state.filter = "-label:docs".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3)), RowKey::Issue(IssueNumber(1))]);
    state.filter = "-label:docs -label:bug".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(1))]);
    state.filter = "-label:docs -retry".to_string();
    assert_eq!(visible_rows(&state), [], "a minus before free text is part of the text");
    state.filter = "-label:".to_string();
    assert_eq!(visible_rows(&state).len(), 3, "a term with no value hides nothing");
}

#[test]
fn author_matches_the_login_and_at_me() {
    let (mut state, _) = app();
    state.issues.items[0].author = "hubot".to_string();
    state.filter = "author:hubot".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3))]);
    state.filter = "-author:@HUBOT".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(2)), RowKey::Issue(IssueNumber(1))]);
    state.filter = "author:@me".to_string();
    assert_eq!(visible_rows(&state), [], "the login is not known yet");
    press(&mut state, Msg::Viewer(Ok("octo".to_string())));
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(2)), RowKey::Issue(IssueNumber(1))]);
    press(&mut state, key('2'));
    state.prs.items[1].author = "hubot".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Pr(PrNumber(10))], "the PR tab reads the PR author");
}

#[test]
fn state_checks_and_has_filter_the_pr_tab() {
    let (mut state, _) = app();
    state.prs.items[0].checks = Checks::Fail;
    state.prs.items[1].state = PrState::Draft;
    state.prs.items[1].checks = Checks::Pass;
    press(&mut state, key('2'));
    let rows = |state: &mut State, raw: &str| {
        state.filter = raw.to_string();
        visible_rows(state)
    };
    assert_eq!(rows(&mut state, "state:open"), [RowKey::Pr(PrNumber(10))]);
    assert_eq!(rows(&mut state, "state:draft"), [RowKey::Pr(PrNumber(11))]);
    assert_eq!(rows(&mut state, "-state:draft"), [RowKey::Pr(PrNumber(10))]);
    assert_eq!(rows(&mut state, "state:merged"), []);
    assert_eq!(rows(&mut state, "checks:fail"), [RowKey::Pr(PrNumber(10))]);
    assert_eq!(rows(&mut state, "-checks:fail"), [RowKey::Pr(PrNumber(11))]);
    assert_eq!(rows(&mut state, "checks:none"), []);
    assert_eq!(rows(&mut state, "has:pr").len(), 2, "a PR row always has a PR");
    assert_eq!(rows(&mut state, "has:session"), []);
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    assert_eq!(rows(&mut state, "has:session"), [RowKey::Pr(PrNumber(10))], "#10 closes #3, which has a session");
}

#[test]
fn is_fork_matches_a_fork_pr_and_the_issue_it_closes() {
    let (mut state, _) = app();
    state.prs.items[0].head = PrBranch::fork(PrNumber(10), "feature".to_string(), ForkPush::Denied);
    state.filter = "is:fork".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3))]);
    press(&mut state, key('2'));
    assert_eq!(visible_rows(&state), [RowKey::Pr(PrNumber(10))]);
    state.filter = "-is:fork".to_string();
    assert_eq!(visible_rows(&state), [RowKey::Pr(PrNumber(11))]);
}

#[test]
fn the_issue_tab_filters_by_state_checks_pr_session_and_marks() {
    let (mut state, _) = app();
    state.prs.items[0].checks = Checks::Pending;
    let rows = |state: &mut State, raw: &str| {
        state.filter = raw.to_string();
        visible_rows(state)
    };
    assert_eq!(rows(&mut state, "state:in-review"), [RowKey::Issue(IssueNumber(3))], "#3 has an open PR");
    assert_eq!(rows(&mut state, "state:open").len(), 2);
    assert_eq!(rows(&mut state, "checks:pending"), [RowKey::Issue(IssueNumber(3))], "through its lead PR");
    assert_eq!(rows(&mut state, "has:pr"), [RowKey::Issue(IssueNumber(3))]);
    assert_eq!(rows(&mut state, "-has:pr").len(), 2);
    assert_eq!(rows(&mut state, "has:session"), []);
    state.sessions.insert(IssueNumber(2), session(2, SessionState::NeedsYou));
    state.starting.insert(IssueNumber(1));
    assert_eq!(rows(&mut state, "has:session"), [RowKey::Issue(IssueNumber(2)), RowKey::Issue(IssueNumber(1))]);
    assert_eq!(rows(&mut state, "state:needs-you"), [RowKey::Issue(IssueNumber(2))]);
    assert_eq!(rows(&mut state, "state:working"), [RowKey::Issue(IssueNumber(1))], "a start counts as working");
    assert_eq!(rows(&mut state, "is:marked"), []);
    state.marked.insert(IssueNumber(3));
    assert_eq!(rows(&mut state, "is:marked"), [RowKey::Issue(IssueNumber(3))]);
    assert_eq!(rows(&mut state, "-is:marked").len(), 2);
}

#[test]
fn the_session_tab_filters_by_state_pr_and_marks() {
    let (mut state, _) = app();
    state.prs.items[0].checks = Checks::Fail;
    state.sessions.insert(IssueNumber(3), session(3, SessionState::Running));
    state.sessions.insert(IssueNumber(2), session(2, SessionState::Exited));
    state.marked.insert(IssueNumber(2));
    press(&mut state, key('3'));
    let rows = |state: &mut State, raw: &str| {
        state.filter = raw.to_string();
        visible_rows(state)
    };
    assert_eq!(rows(&mut state, "state:working"), [RowKey::Session(IssueNumber(3))]);
    assert_eq!(rows(&mut state, "state:exited"), [RowKey::Session(IssueNumber(2))]);
    assert_eq!(rows(&mut state, "has:pr checks:fail"), [RowKey::Session(IssueNumber(3))]);
    assert_eq!(rows(&mut state, "is:marked"), [RowKey::Session(IssueNumber(2))]);
    assert_eq!(rows(&mut state, "has:session").len(), 2, "a session row always has a session");
    assert_eq!(rows(&mut state, "-state:working"), [RowKey::Session(IssueNumber(2))]);
}
