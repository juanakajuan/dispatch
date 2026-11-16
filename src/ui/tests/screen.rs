use super::*;

/// Every screen the app can show: no cell has a background, and every
/// foreground is a Gruvbox palette color.
#[test]
fn no_cell_has_a_background_and_every_foreground_is_gruvbox() {
    let mut state = full_state();
    let mut screens: Vec<(String, Buffer)> = Vec::new();
    for (name, key) in [("issues", '1'), ("prs", '2'), ("sessions", '3')] {
        send(&mut state, KeyCode::Char(key));
        screens.push((name.to_string(), render(&state, 100, 30)));
    }
    send(&mut state, KeyCode::Char('1'));
    send(&mut state, KeyCode::Char('j'));
    send(&mut state, KeyCode::Char('s'));
    screens.push(("dialog".to_string(), render(&state, 100, 30)));
    screens.push(("narrow".to_string(), render(&state, 30, 12)));
    for (name, buf) in screens {
        for cell in buf.content() {
            let badge = [palette::BLUE, palette::BG2].contains(&cell.bg);
            assert!(cell.bg == Color::Reset || badge, "{name}: background at {:?}", cell.symbol());
            assert_eq!(cell.underline_color, Color::Reset, "{name}: underline color");
            assert!(in_palette(cell.fg), "{name}: fg {:?} at {:?}", cell.fg, cell.symbol());
        }
    }
}

#[test]
fn selection_is_an_accent_marker_on_one_row() {
    let state = full_state();
    let buf = render(&state, 100, 30);
    let screen = text(&buf);
    let rows: Vec<(usize, &str)> =
        screen.lines().enumerate().filter(|(_, l)| l.contains("❯") && l.contains('#')).collect();
    assert_eq!(rows.len(), 1, "one selected row in\n{screen}");
    let (y, line) = rows[0];
    assert!(line.contains("#3 Retry"), "the first row is selected: {line:?}");
    let x = u16::try_from(cells(&line[..line.find('❯').expect("marker")])).expect("x");
    assert_eq!(buf[(x, u16::try_from(y).expect("y"))].fg, palette::BLUE);
    assert!(
        buf.content().iter().all(|c| !c.modifier.contains(Modifier::REVERSED)),
        "no reversed block outside a text cursor"
    );
}

#[test]
fn issues_screen_shows_groups_rows_and_key_bar() {
    let state = full_state();
    let screen = text(&render(&state, 120, 30));
    for want in [
        "1  Issues 2",
        "2  Pull requests 1",
        "3  Sessions 1",
        "▲ 1 needs you",
        "Needs you 1",
        "▲ #3 Retry queue",
        "PR #10 open",
        "Open 1",
        "· #2 Docs",
        "s to start session",
        "a to attach",
        "x to stop",
        "S sort: attention",
    ] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
}

/// The glyph cell left of issue `n`'s number on its row.
fn mark_cell(buf: &Buffer, n: u32) -> (String, Color) {
    let screen = text(buf);
    let needle = format!("#{n} ");
    let (y, line) =
        screen.lines().enumerate().find(|(_, l)| l.contains(&needle) && !l.contains("──")).expect("row drawn");
    let x = cells(&line[..line.find(&needle).expect("number")]) - 2;
    let cell = &buf[(u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))];
    (cell.symbol().to_string(), cell.fg)
}

#[test]
fn issue_rows_show_the_lead_pr_state_and_follow_a_refresh() {
    let mut state = full_state();
    state.sessions = crate::session::Sessions::new();
    let mark = |state: &State| (mark_cell(&render(state, 100, 20), 3), mark_cell(&render(state, 100, 20), 2));
    assert_eq!(
        mark(&state),
        (("●".to_string(), palette::FG), ("·".to_string(), palette::GRAY)),
        "open, no checks; #2 has no PR"
    );
    for (pr_state, checks, want, color) in [
        (PrState::Draft, Checks::Pass, "◌", palette::GRAY),
        (PrState::Open, Checks::Pending, "●", palette::YELLOW),
        (PrState::Open, Checks::Fail, "●", palette::RED),
        (PrState::Open, Checks::Pass, "●", palette::GREEN),
        (PrState::Merged, Checks::Fail, "✓", palette::PURPLE),
        (PrState::Closed, Checks::Pass, "·", palette::GRAY),
    ] {
        let fresh = vec![PullRequest { state: pr_state, checks, ..pr(10, "fix/3-x", &[3]) }];
        let _ = app::update(&mut state, Msg::Prs { result: Ok(fresh), at: Instant::now() });
        assert_eq!(mark(&state).0, (want.to_string(), color), "{pr_state:?} {checks:?}");
    }
    let screen = text(&render(&state, 100, 20));
    assert_eq!(column_of(&screen, "#3 ", "#3 "), column_of(&screen, "#2 ", "#2 "), "titles stay in one column");
}

/// The column of `needle` on the first screen line that holds `anchor`.
fn column_of(screen: &str, anchor: &str, needle: &str) -> usize {
    let line = screen.lines().find(|l| l.contains(anchor)).unwrap_or_else(|| panic!("no {anchor:?} in\n{screen}"));
    let byte = line.find(needle).unwrap_or_else(|| panic!("no {needle:?} in {line:?}"));
    cells(&line[..byte])
}

#[test]
fn the_nerd_font_set_draws_its_glyphs_in_the_same_columns_as_unicode() {
    let mut state = full_state();
    let plain = text(&render(&state, 120, 24));
    state.config.icons = IconSet::NerdFont;
    let icons = state.config.icons.icons();
    let buf = render(&state, 120, 24);
    let screen = text(&buf);
    assert_eq!(mark_cell(&buf, 3), (icons.needs_you.to_string(), palette::YELLOW), "the session glyph cell");
    assert_eq!(mark_cell(&buf, 2).0, icons.idle, "the idle glyph cell");
    assert!(screen.contains(&format!("{} 1 needs you", icons.needs_you)), "header count\n{screen}");
    for (anchor, needle) in [("#3 ", "Retry queue"), ("#2 ", "Docs"), ("#3 ", "PR #10"), ("needs you", "needs you")] {
        assert_eq!(column_of(&screen, anchor, needle), column_of(&plain, anchor, needle), "{needle:?} moved");
    }
}

#[test]
fn draw_returns_hits_for_tabs_and_visible_rows() {
    let state = full_state();
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("terminal");
    let mut hits = HitMap::default();
    terminal.draw(|f| hits = draw(f, &state)).expect("draw");
    let screen = text(terminal.backend().buffer());
    let line_of =
        |needle: &str| u16::try_from(screen.lines().position(|l| l.contains(needle)).expect(needle)).expect("y");
    let tabs = hits.targets.iter().filter(|(_, t)| matches!(t, HitTarget::Tab(_))).count();
    let rows: Vec<_> = hits
        .targets
        .iter()
        .filter_map(|(r, t)| match t {
            HitTarget::Row(k) => Some((r.y, *k)),
            HitTarget::Tab(_) => None,
        })
        .collect();
    assert_eq!(tabs, 3);
    assert_eq!(
        rows,
        [(line_of("#3 Retry"), RowKey::Issue(IssueNumber(3))), (line_of("#2 Docs"), RowKey::Issue(IssueNumber(2)))],
        "group headings and blank lines are not rows"
    );
}

#[test]
fn tab_keys_look_like_key_hints_and_counts_like_counts() {
    let mut state = full_state();
    state.issues.items.clear();
    let buf = frame(&mut state, 120, 20);
    let y = 1;
    let row = text(&buf).lines().nth(usize::from(y)).expect("tab row").to_string();
    let column = |needle: &str| u16::try_from(cells(&row[..row.find(needle).expect(needle)])).expect("x");
    for (label, tab) in
        [(" 1  Issues 0", Tab::Issues), (" 2  Pull requests 1", Tab::Prs), (" 3  Sessions 1", Tab::Sessions)]
    {
        let x = column(label);
        let badge = if tab == state.tab { palette::BLUE } else { palette::BG2 };
        assert_eq!(buf[(x + 1, y)].bg, badge, "{label}: the key sits on a badge, blue for the shown tab");
        let last = x + u16::try_from(cells(label)).expect("w") - 1;
        if tab != state.tab {
            assert_eq!(buf[(last, y)].fg, palette::GRAY, "{label}: the count of another tab is dim");
        }
        let hit = |cx: u16| state.hits.targets.iter().find(|(r, _)| r.contains((cx, y).into())).map(|(_, t)| *t);
        assert_eq!(hit(x), Some(HitTarget::Tab(tab)), "{label}: a click on the badge selects the tab");
        assert_eq!(hit(last), Some(HitTarget::Tab(tab)), "{label}: a click on the count selects the tab");
        assert_eq!(hit(last + 1), None, "{label}: the hit area ends at the label");
    }
}

/// Draws like the shell does: the frame's hits go back into `state`.
fn frame(state: &mut State, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    let mut hits = HitMap::default();
    terminal.draw(|f| hits = draw(f, state)).expect("draw");
    state.hits = hits;
    terminal.backend().buffer().clone()
}

#[test]
fn a_failed_feed_shows_on_the_message_row_above_the_prompt() {
    let mut state = full_state();
    state.issues.status = FeedStatus::Failed { error: "rate limited".to_string(), at: Instant::now() };
    let screen = text(&render(&state, 100, 20));
    let lines: Vec<&str> = screen.lines().collect();
    let at = lines.iter().position(|l| l.contains("gh failed, showing old data: rate limited"));
    let at = at.unwrap_or_else(|| panic!("missing failure in\n{screen}"));
    assert!(lines[at + 1].contains("────") && lines[at + 2].contains("❯ /"), "not above the prompt:\n{screen}");
}

#[test]
fn the_footer_shows_the_shown_tabs_sort_mode() {
    let mut state = full_state();
    let footer = |state: &State| text(&render(state, 120, 24)).lines().last().unwrap_or_default().to_string();
    assert!(footer(&state).trim_end().ends_with("S sort: attention"), "{}", footer(&state));
    let _ = app::update(&mut state, Msg::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)));
    assert!(footer(&state).contains("S sort: newest"));
    send(&mut state, KeyCode::Char('2'));
    assert!(footer(&state).contains("S sort: attention"), "each tab has its own mode");
}

#[test]
fn since_counts_seconds_then_minutes_then_hours() {
    let at = Instant::now();
    for (secs, want) in [(0, "0s"), (12, "12s"), (59, "59s"), (60, "1m"), (3599, "59m"), (3600, "1h"), (7300, "2h")] {
        assert_eq!(since(at, at + Duration::from_secs(secs)), want, "{secs}s");
    }
    assert_eq!(since(at + Duration::from_secs(5), at), "0s", "a response stamped after now is not negative");
}

/// The header text after the repo name, and the style of its first character.
fn freshness_on_screen(state: &State) -> (String, Style) {
    let buf = render(state, 120, 20);
    let screen = text(&buf);
    let (y, row) = screen.lines().enumerate().find(|(_, l)| l.contains("orbit")).expect("repo line");
    let Some(x) = row.find(" · ").map(|x| x + " · ".len()) else {
        return (String::new(), palette::PLAIN);
    };
    let cell = &buf[(u16::try_from(cells(&row[..x])).expect("x"), u16::try_from(y).expect("y"))];
    (row[x..].trim_end().to_string(), Style::new().fg(cell.fg))
}

fn ago(secs: u64) -> Instant {
    Instant::now().checked_sub(Duration::from_secs(secs)).expect("uptime above a minute")
}

#[test]
fn the_header_shows_how_old_the_list_is_and_each_fetch_state() {
    let mut state = full_state();
    state.issues.status = FeedStatus::Fresh { at: ago(12) };
    assert_eq!(freshness_on_screen(&state), ("updated 12s ago".to_string(), palette::DIM));

    state.inflight.issues = true;
    assert_eq!(freshness_on_screen(&state), ("refreshing…".to_string(), palette::DIM));

    state.inflight.issues = false;
    state.issues.status = FeedStatus::Failed { error: "offline".to_string(), at: ago(40) };
    assert_eq!(freshness_on_screen(&state), ("refresh failed 40s ago".to_string(), palette::WARN));

    state.prs.status = FeedStatus::Fresh { at: ago(125) };
    send(&mut state, KeyCode::Char('2'));
    assert_eq!(freshness_on_screen(&state).0, "updated 2m ago", "each tab shows its own feed");

    send(&mut state, KeyCode::Char('3'));
    assert_eq!(freshness_on_screen(&state).0, "", "sessions are not fetched from GitHub, so no age");
}

#[test]
fn empty_and_loading_lists_say_so() {
    let (state, _) = app::init(Instant::now(), Config::default(), Context::default());
    assert!(text(&render(&state, 80, 10)).contains("Loading issues…"));
}
