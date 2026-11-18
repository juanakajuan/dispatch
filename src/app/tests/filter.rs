use super::*;
use crate::app::filter::{Assignee, Facets, Query, Term, TokenKind, Viewer, tokens};

fn kinds(raw: &str) -> Vec<(&str, TokenKind)> {
    tokens(raw).into_iter().map(|t| (&raw[t.at.clone()], t.kind)).collect()
}

fn label(s: &str) -> TokenKind {
    TokenKind::Term(Term::Label(s.to_string()))
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
fn tokens_split_on_spaces_outside_quotes_and_know_three_term_keys() {
    assert_eq!(
        kinds(r#"  retry label:"good first issue" Milestone:v1 assignee:@ME assignee:@octo"#),
        [
            ("retry", TokenKind::Text),
            (r#"label:"good first issue""#, label("good first issue")),
            ("Milestone:v1", TokenKind::Term(Term::Milestone("v1".to_string()))),
            ("assignee:@ME", TokenKind::Term(Term::Assignee(Assignee::Me))),
            ("assignee:@octo", TokenKind::Term(Term::Assignee(Assignee::Login("octo".to_string())))),
        ]
    );
    assert_eq!(kinds("label: author:x"), [("label:", TokenKind::Incomplete), ("author:x", TokenKind::Text)]);
    assert_eq!(kinds(r#"label:"good fi"#), [(r#"label:"good fi"#, label("good fi"))], "an open quote runs to the end");
}

#[test]
fn free_text_is_a_case_insensitive_substring_over_all_parts() {
    let parts = ["#142", "Retry queue", "bug", "fix/142-retry"];
    let matches = |raw| Query::parse(raw, None).matches(&parts, Facets::default());
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
    let facets = Facets { labels: &labels, assignees: &assignees, milestone: Some("v1.0") };
    let matches = |raw, viewer| Query::parse(raw, viewer).matches(&["#1", "Retry queue"], facets);
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
    assert!(!Query::parse("milestone:v1.0", None).matches(&[], none), "a row with no milestone");
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
