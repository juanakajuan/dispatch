use super::*;
use crate::session::{Push, Work};

fn on_tab(tab: char, work: Option<Work>, w: u16) -> String {
    let mut state = full_state();
    if let Some(work) = work {
        state.work.insert(IssueNumber(3), work);
    }
    send(&mut state, KeyCode::Char(tab));
    text(&render(&state, w, 20))
}

fn column_of(screen: &str, anchor: &str, needle: &str) -> usize {
    let line = screen.lines().find(|l| l.contains(anchor)).unwrap_or_else(|| panic!("no {anchor:?} in\n{screen}"));
    let byte = line.find(needle).unwrap_or_else(|| panic!("no {needle:?} in {line:?}"));
    cells(&line[..byte])
}

#[test]
fn every_summary_on_a_tab_starts_in_one_column_and_a_work_column_does_not_cut_it() {
    for w in [90u16, 120, 160] {
        let mut state = full_state();
        state.issues.items.push(crate::testutil::issue(4, "A longer title in the same group as Docs", &["ui"]));
        let issues = text(&render(&state, w, 20));
        assert_eq!(column_of(&issues, "#4 ", "octo"), column_of(&issues, "#2 ", "octo"), "issues at {w}\n{issues}");
        let work = Work { commits: 2, uncommitted: 1, push: Push::NotPushed };
        let with_work = on_tab('3', Some(work), w);
        let row = with_work.lines().find(|l| l.contains("#3 Retry")).expect("row");
        assert!(row.contains("PR #10 open"), "sessions with work at {w}: {row:?}");
    }
}

#[test]
fn when_the_summaries_are_short_a_long_title_takes_the_empty_space() {
    let wide = on_tab('1', None, 160);
    assert!(wide.contains("a very long title that must be cut  "), "the title is cut on a wide screen:\n{wide}");
    let narrow = on_tab('1', None, 90);
    let row = narrow.lines().find(|l| l.contains("#3 ")).unwrap_or_else(|| panic!("no #3 in\n{narrow}"));
    assert!(row.contains('…'), "the long title is not cut at 90: {row:?}");
    assert!(column_of(&narrow, "#3 ", "PR #10") > 2 + 86 * 2 / 5, "the name took the free space: {row:?}");
    let age_end = |n: &str| cells(narrow.lines().find(|l| l.contains(n)).expect("row").trim_end());
    assert_eq!(age_end("#3 "), 88, "{row:?}");
    assert_eq!(age_end("#2 "), 88, "a short title keeps the age aligned too");
}

#[test]
fn when_a_summary_is_long_the_name_keeps_two_fifths_and_ends_in_an_ellipsis() {
    let mut state = full_state();
    let labels: Vec<String> = (0..12).map(|i| format!("label-number-{i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let title = "Docs for every setting, key, and screen that dispatch has today, in one long page";
    state.issues.items[1] = crate::testutil::issue(2, title, &["docs"]);
    state.issues.items.push(crate::testutil::issue(4, "Labels", &labels));
    let screen = text(&render(&state, 120, 20));
    let row = screen.lines().find(|l| l.contains("#2 ")).unwrap_or_else(|| panic!("no #2 in\n{screen}"));
    assert!(row.contains('…'), "the long title is not cut: {row:?}");
    assert_eq!(column_of(&screen, "#2 ", "octo"), 2 + 116 * 2 / 5, "{row:?}\n{screen}");
}

#[test]
fn a_group_heading_starts_in_the_same_column_as_the_row_icons() {
    let screen = on_tab('3', None, 100);
    let heading = screen.lines().find(|l| l.contains("Needs you")).unwrap_or_else(|| panic!("no heading in\n{screen}"));
    let row = screen.lines().find(|l| l.contains("#3 Retry")).expect("row");
    let heading_col = cells(&heading[..heading.find("Needs you").unwrap_or(0)]);
    let name_col = column_of(&screen, "#3 Retry", "#3 Retry");
    assert_eq!(heading_col, name_col - 2, "the icon sits two cells before the name: {row:?}\n{screen}");
}

#[test]
fn a_long_summary_in_one_group_does_not_narrow_the_titles_of_another() {
    let mut state = full_state();
    let long = "Claude says a lot of things here ".repeat(6);
    let said = crate::transcript::Summary { kind: crate::transcript::SummaryKind::Message, text: long };
    state.transcripts.insert(IssueNumber(3), crate::transcript::Transcript { said: Some(said) });
    let title = "Docs for every setting, key, and screen that dispatch has today";
    state.issues.items[1] = crate::testutil::issue(2, title, &["docs"]);
    let screen = text(&render(&state, 160, 20));
    assert!(screen.contains(&format!("#2 {title}  ")), "the Open title is cut:\n{screen}");
    let row = screen.lines().find(|l| l.contains("#3 ")).unwrap_or_else(|| panic!("no #3 in\n{screen}"));
    assert!(row.contains('…'), "the title next to the long summary keeps two fifths: {row:?}");
}
