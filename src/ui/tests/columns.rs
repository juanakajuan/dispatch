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
fn the_summary_starts_two_fifths_across_on_every_tab_with_or_without_work() {
    for w in [90u16, 120, 160] {
        let want = 2 + usize::from(w - 4) * 2 / 5;
        assert_eq!(column_of(&on_tab('1', None, w), "#3 ", "PR #10"), want, "issues at {w}");
        assert_eq!(column_of(&on_tab('2', None, w), "#10 ", "fix/3-x →"), want, "prs at {w}");
        assert_eq!(column_of(&on_tab('3', None, w), "#3 Retry", "PR #10"), want, "sessions at {w}");
        let work = Work { commits: 2, uncommitted: 1, push: Push::NotPushed };
        let with_work = on_tab('3', Some(work), w);
        assert_eq!(column_of(&with_work, "#3 Retry", "PR #10"), want, "sessions with work at {w}\n{with_work}");
    }
}

#[test]
fn a_title_longer_than_its_column_ends_in_an_ellipsis_and_the_age_stays_right_aligned() {
    let screen = on_tab('1', None, 90);
    let row = screen.lines().find(|l| l.contains("#3 ")).unwrap_or_else(|| panic!("no #3 in\n{screen}"));
    assert!(row.contains('…'), "the long title is not cut: {row:?}");
    assert!(column_of(&screen, "#3 ", "…") < 2 + 86 * 2 / 5, "the cut is inside the name column: {row:?}");
    let age_end = |n: &str| cells(screen.lines().find(|l| l.contains(n)).expect("row").trim_end());
    assert_eq!(age_end("#3 "), 88, "{row:?}");
    assert_eq!(age_end("#2 "), 88, "a short title keeps the age aligned too");
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
