use super::*;

fn state_with_data() -> State {
    let (mut state, _) = app();
    state.issues.items[0].labels.push("good first issue".to_string());
    state.issues.items[0].assignees = vec!["octo".to_string(), "hubot".to_string()];
    state.issues.items[0].milestone = Some("v1.0".to_string());
    state.issues.items[1].author = "Hubot".to_string();
    state.issues.items[2].labels.push("bug".to_string());
    press(&mut state, key('/'));
    state
}

fn type_text(state: &mut State, text: &str) {
    for c in text.chars() {
        press(state, key(c));
    }
}

fn tab(state: &mut State) {
    press(state, code(KeyCode::Tab));
}

fn back_tab(state: &mut State) {
    press(state, code(KeyCode::BackTab));
}

#[test]
fn tab_completes_a_key_prefix_and_keeps_a_minus() {
    let mut state = state_with_data();
    type_text(&mut state, "retry LA");
    tab(&mut state);
    assert_eq!(state.filter(), "retry label:");
    state.filter_mut().clear();
    state.completion = None;
    type_text(&mut state, "-m");
    tab(&mut state);
    assert_eq!(state.filter(), "-milestone:");
}

#[test]
fn tab_after_a_sole_key_goes_on_to_its_values() {
    let mut state = state_with_data();
    type_text(&mut state, "ha");
    tab(&mut state);
    assert_eq!(state.filter(), "has:");
    tab(&mut state);
    assert_eq!(state.filter(), "has:pr");
    tab(&mut state);
    assert_eq!(state.filter(), "has:session");
}

#[test]
fn tab_on_an_empty_word_cycles_all_keys_and_wraps_both_ways() {
    let mut state = state_with_data();
    tab(&mut state);
    assert_eq!(state.filter(), "assignee:");
    let mut seen = vec![state.filter().to_string()];
    for _ in 0..7 {
        tab(&mut state);
        seen.push(state.filter().to_string());
    }
    assert_eq!(seen, ["assignee:", "author:", "checks:", "has:", "is:", "label:", "milestone:", "state:"]);
    tab(&mut state);
    assert_eq!(state.filter(), "assignee:", "wraps to the first");
    back_tab(&mut state);
    assert_eq!(state.filter(), "state:", "Shift-Tab wraps to the last");
    back_tab(&mut state);
    assert_eq!(state.filter(), "milestone:");

    let mut state = state_with_data();
    type_text(&mut state, "retry ");
    tab(&mut state);
    assert_eq!(state.filter(), "retry assignee:", "a filter that ends in a space completes a new word");
    let mut state = state_with_data();
    back_tab(&mut state);
    assert_eq!(state.filter(), "state:", "the first Shift-Tab picks the last choice");
}

#[test]
fn tab_completes_values_from_the_loaded_items_of_the_shown_tab() {
    let mut state = state_with_data();
    type_text(&mut state, "label:");
    tab(&mut state);
    assert_eq!(state.filter(), "label:bug");
    tab(&mut state);
    assert_eq!(state.filter(), "label:docs");
    tab(&mut state);
    tab(&mut state);
    assert_eq!(state.filter(), r#"label:"good first issue""#, "a value with spaces gets quotes");
    tab(&mut state);
    assert_eq!(state.filter(), "label:bug", "wraps");

    let mut state = state_with_data();
    type_text(&mut state, "-label:g");
    tab(&mut state);
    assert_eq!(state.filter(), r#"-label:"good first issue""#);

    let mut state = state_with_data();
    type_text(&mut state, r#"label:"Good F"#);
    tab(&mut state);
    assert_eq!(state.filter(), r#"label:"good first issue""#, "an opening quote and the case do not matter");

    let mut state = state_with_data();
    type_text(&mut state, "milestone:V");
    tab(&mut state);
    assert_eq!(state.filter(), "milestone:v1.0");
}

#[test]
fn tab_completes_people_with_at_me_first_among_equals() {
    let mut state = state_with_data();
    type_text(&mut state, "assignee:");
    let mut seen = Vec::new();
    for _ in 0..3 {
        tab(&mut state);
        seen.push(state.filter().to_string());
    }
    assert_eq!(seen, ["assignee:@me", "assignee:hubot", "assignee:octo"]);

    let mut state = state_with_data();
    type_text(&mut state, "author:@h");
    tab(&mut state);
    assert_eq!(state.filter(), "author:Hubot", "an author from an issue; the @ is not part of the login");
    tab(&mut state);
    assert_eq!(state.filter(), "author:Hubot", "one choice only");

    let mut state = state_with_data();
    type_text(&mut state, "author:@m");
    tab(&mut state);
    assert_eq!(state.filter(), "author:@me");
}

#[test]
fn tab_completes_the_fixed_words_of_each_tab() {
    let words = |state: &mut State, typed: &str| {
        state.filter_mut().clear();
        state.completion = None;
        type_text(state, typed);
        let mut seen = Vec::new();
        loop {
            tab(state);
            if seen.iter().any(|s: &String| s == state.filter()) {
                break seen;
            }
            seen.push(state.filter().to_string());
        }
    };
    let mut state = state_with_data();
    assert_eq!(
        words(&mut state, "state:"),
        ["state:in-review", "state:merged", "state:needs-you", "state:open", "state:working"]
    );
    assert_eq!(words(&mut state, "checks:p"), ["checks:pass", "checks:pending"]);
    assert_eq!(words(&mut state, "has:"), ["has:pr", "has:session"]);
    assert_eq!(words(&mut state, "-is:"), ["-is:fork", "-is:marked"]);
    press(&mut state, code(KeyCode::Esc));
    press(&mut state, key('2'));
    press(&mut state, key('/'));
    assert_eq!(words(&mut state, "state:"), ["state:closed", "state:draft", "state:merged", "state:open"]);
    press(&mut state, code(KeyCode::Esc));
    press(&mut state, key('3'));
    press(&mut state, key('/'));
    assert_eq!(words(&mut state, "state:"), ["state:exited", "state:merged", "state:needs-you", "state:working"]);
}

#[test]
fn the_pr_tab_completes_from_pr_data_and_the_session_tab_from_its_issues() {
    let (mut state, _) = app();
    state.prs.items[0].labels = vec!["ci".to_string()];
    state.sessions.insert(issue_key(2), session(2, SessionState::Running));
    press(&mut state, key('2'));
    press(&mut state, key('/'));
    type_text(&mut state, "label:");
    tab(&mut state);
    assert_eq!(state.filter(), "label:ci", "PR labels only, not the issue labels");
    press(&mut state, code(KeyCode::Esc));
    press(&mut state, key('3'));
    press(&mut state, key('/'));
    type_text(&mut state, "label:");
    tab(&mut state);
    assert_eq!(state.filter(), "label:docs", "the issue of the only session");
    tab(&mut state);
    assert_eq!(state.filter(), "label:docs", "no other label to offer");
}

#[test]
fn tab_changes_nothing_when_there_is_no_choice() {
    let mut state = state_with_data();
    for raw in ["label:zzz", "color:", "retry"] {
        state.filter_mut().set(raw);
        state.completion = None;
        tab(&mut state);
        back_tab(&mut state);
        assert_eq!(state.filter(), raw);
    }
}

#[test]
fn any_other_key_ends_the_completion() {
    let mut state = state_with_data();
    type_text(&mut state, "label:");
    tab(&mut state);
    tab(&mut state);
    assert_eq!(state.filter(), "label:docs");
    press(&mut state, key('x'));
    assert_eq!(state.filter(), "label:docsx");
    tab(&mut state);
    assert_eq!(state.filter(), "label:docsx", "a new word to complete, with no choice");
    press(&mut state, code(KeyCode::Backspace));
    tab(&mut state);
    assert_eq!(state.filter(), "label:docs", "the next Tab starts over from the typed word");
}

#[test]
fn a_completed_filter_filters_the_rows() {
    let mut state = state_with_data();
    type_text(&mut state, "label:g");
    tab(&mut state);
    assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(3))]);
}
