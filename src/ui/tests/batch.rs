use super::*;
use crate::domain::Mode;

fn open_batch(state: &mut State) {
    send(state, KeyCode::Char(' '));
    send(state, KeyCode::Char('j'));
    send(state, KeyCode::Char(' '));
    send(state, KeyCode::Char('s'));
}

#[test]
fn the_batch_dialog_shows_the_title_the_issues_and_the_mode() {
    let mut state = full_state();
    // #3 has a session in `full_state`, so mark #2 only and add one more issue.
    state.issues.items.push(issue(1, "Add json output", &["feature"]));
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char(' '));
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char(' '));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    for want in [
        "Start 2 sessions",
        "Issues",
        "#2 Docs",
        "#1 Add json output",
        "Mode",
        "Plan first, then wait for approval",
        "Implement directly",
        "Each session uses its default branch and prompt.",
        "[ Start ]  [ Cancel ]",
        "ctrl+s to start · space to change mode · esc to cancel",
    ] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
}

#[test]
fn the_batch_dialog_clips_a_long_list_with_a_count_of_the_rest() {
    let mut state = full_state();
    state.sessions.clear();
    state.issues.items = (1..=40).map(|n| issue(n, "Same title", &[])).collect();
    for n in 1..=40 {
        state.marked.insert(IssueNumber(n));
    }
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    assert!(screen.contains("Start 40 sessions"), "{screen}");
    let more = screen.lines().find(|l| l.contains("… and ")).unwrap_or_else(|| panic!("no count line in\n{screen}"));
    let hidden: usize =
        more.split("… and ").nth(1).and_then(|r| r.split(' ').next()).and_then(|n| n.parse().ok()).expect("count");
    let listed = screen.lines().filter(|l| l.contains("Same title")).count();
    assert_eq!(listed + hidden, 40, "{screen}");
    assert!(screen.contains("[ Start ]"), "{screen}");
}

#[test]
fn clicking_a_batch_mode_option_picks_it() {
    let mut state = full_state();
    open_batch(&mut state);
    let screen = draw_into(&mut state, 120, 30);
    let (x, y) = find(&screen, "Plan first, then wait");
    click_at(&mut state, x, y);
    let Focus::Batch(b) = &state.focus else { panic!("batch open") };
    assert_eq!(b.mode, Mode::PlanFirst);
}

#[test]
fn a_marked_issue_row_shows_the_mark_and_an_unmarked_one_does_not() {
    let mut state = full_state();
    for set in [IconSet::Unicode, IconSet::NerdFont] {
        state.config.icons = set;
        state.marked.clear();
        let glyph = set.icons().marked;
        assert!(!text(&render(&state, 120, 30)).contains(glyph));
        send(&mut state, KeyCode::Char('j'));
        send(&mut state, KeyCode::Char(' '));
        let screen = text(&render(&state, 120, 30));
        let row = screen.lines().find(|l| l.contains("#2 Docs")).expect("row");
        assert!(row.contains(glyph), "{set:?}: {row:?}");
        send(&mut state, KeyCode::Char('k'));
    }
}

#[test]
fn the_batch_dialog_title_counts_one_session_in_the_singular() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char(' '));
    send(&mut state, KeyCode::Char('s'));
    let screen = text(&render(&state, 120, 30));
    assert!(screen.contains(" Start 1 session "), "{screen}");
}
