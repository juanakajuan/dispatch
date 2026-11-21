use super::*;

/// The glyph and colour on the row of `#n`, in the cell two cells left of it.
fn glyph_of(buf: &Buffer, n: u32) -> (String, Color) {
    let screen = text(buf);
    let needle = format!("#{n} ");
    let (y, line) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(&needle) && !l.contains("──"))
        .unwrap_or_else(|| panic!("no row {needle:?} in\n{screen}"));
    let x = cells(&line[..line.find(&needle).expect("number")]) - 2;
    let cell = &buf[(u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))];
    (cell.symbol().to_string(), cell.fg)
}

/// The colour of the first cell of `word` on the row of `#n`.
fn color_of_word(buf: &Buffer, n: u32, word: &str) -> Color {
    let screen = text(buf);
    let needle = format!("#{n} ");
    let (y, line) = screen.lines().enumerate().find(|(_, l)| l.contains(&needle)).expect("row drawn");
    let x = cells(&line[..line.find(word).unwrap_or_else(|| panic!("no {word:?} in {line:?}"))]);
    buf[(u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))].fg
}

fn with_pr(state: PrState, checks: Checks) -> State {
    let mut s = full_state();
    s.sessions = crate::session::Sessions::new();
    s.prs.items = vec![PullRequest { state, checks, ..pr(10, "fix/3-x", &[3]) }];
    s
}

#[test]
fn a_pr_shows_the_same_glyph_and_colour_on_the_issues_tab_and_the_pull_requests_tab() {
    for set in [IconSet::Unicode, IconSet::NerdFont] {
        for pr_state in [PrState::Open, PrState::Draft, PrState::Merged] {
            for checks in [Checks::Pass, Checks::Fail, Checks::Pending, Checks::None] {
                let mut state = with_pr(pr_state, checks);
                state.config.icons = set;
                let on_issues = glyph_of(&render(&state, 120, 20), 3);
                send(&mut state, KeyCode::Char('2'));
                let on_prs = glyph_of(&render(&state, 120, 20), 10);
                assert_eq!(on_issues, on_prs, "{set:?} {pr_state:?} {checks:?}");
            }
        }
    }
}

#[test]
fn an_open_pr_never_shows_the_running_glyph_or_a_checks_glyph_in_the_glyph_column() {
    for set in [IconSet::Unicode, IconSet::NerdFont] {
        let icons = set.icons();
        for checks in [Checks::Pass, Checks::Fail, Checks::Pending, Checks::None] {
            let mut state = with_pr(PrState::Open, checks);
            state.config.icons = set;
            assert_eq!(glyph_of(&render(&state, 120, 20), 3).0, icons.pr_open, "{set:?} {checks:?} issues tab");
            send(&mut state, KeyCode::Char('2'));
            assert_eq!(glyph_of(&render(&state, 120, 20), 10).0, icons.pr_open, "{set:?} {checks:?} PR tab");
        }
    }
}

#[test]
fn a_closed_pr_has_one_colour_for_its_glyph_and_its_word() {
    let mut state = full_state();
    state.prs.items = vec![PullRequest { state: PrState::Closed, checks: Checks::Fail, ..pr(10, "fix/3-x", &[3]) }];
    send(&mut state, KeyCode::Char('2'));
    let (glyph, color) = glyph_of(&render(&state, 120, 20), 10);
    assert_eq!((glyph.as_str(), color), ("✗", palette::GRAY), "the glyph on the Pull requests tab");
    send(&mut state, KeyCode::Char('3'));
    let word = color_of_word(&render(&state, 120, 20), 3, "closed");
    assert_eq!(word, color, "the word on the Sessions tab");
}
