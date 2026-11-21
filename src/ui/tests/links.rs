use super::*;
use crate::domain::PageUrl;

/// The text under each link of the last frame, with the page it links to.
fn links(state: &mut State, w: u16) -> Vec<(String, String)> {
    let buf = frame(state, w, 20);
    state
        .hits
        .links
        .iter()
        .map(|(rect, url)| {
            let cells: String = rect.positions().map(|p| buf[p].symbol().to_string()).collect();
            (cells, url.as_str().to_string())
        })
        .collect()
}

fn pair(cells: &str, url: &str) -> (String, String) {
    (cells.to_string(), url.to_string())
}

#[test]
fn an_issue_number_links_to_the_issue() {
    let mut state = full_state();
    let got = links(&mut state, 120);
    assert!(got.contains(&pair("#3", "https://github.com/o/r/issues/3")), "{got:?}");
    assert!(got.contains(&pair("#2", "https://github.com/o/r/issues/2")), "{got:?}");
}

#[test]
fn a_pr_number_links_to_the_pr() {
    let mut state = full_state();
    send(&mut state, KeyCode::Char('2'));
    assert_eq!(links(&mut state, 120), [pair("#10", "https://github.com/o/r/pull/10")]);
}

#[test]
fn a_session_number_links_to_its_issue_or_pr() {
    let mut state = full_state();
    let pr_session = Session { key: pr_key(10), ..state.sessions.get(&issue_key(3)).expect("session").clone() };
    state.sessions.insert(pr_key(10), pr_session);
    send(&mut state, KeyCode::Char('3'));
    let got = links(&mut state, 120);
    assert!(got.contains(&pair("#3", "https://github.com/o/r/issues/3")), "{got:?}");
    assert!(got.contains(&pair("PR #10", "https://github.com/o/r/pull/10")), "{got:?}");
}

#[test]
fn a_row_with_a_link_draws_the_same_cells_as_a_row_without_one() {
    let mut linked = full_state();
    let mut plain = full_state();
    for i in &mut plain.issues.items {
        i.url = None;
    }
    let with = frame(&mut linked, 100, 20);
    let without = frame(&mut plain, 100, 20);
    assert!(!linked.hits.links.is_empty());
    assert!(plain.hits.links.is_empty(), "an item with no URL gets no link");
    assert_eq!(with, without, "the link changes no cell, so the row keeps its width");
}

#[test]
fn a_selected_row_still_links_its_number() {
    let mut state = full_state();
    let buf = frame(&mut state, 120, 20);
    let (x, y) = find(&text(&buf), "❯");
    let (rect, url) = state.hits.links.iter().find(|(r, _)| r.y == y).expect("link on the selected row");
    assert!(rect.x > x, "the link starts after the marker");
    assert_eq!(url, &PageUrl::parse("https://github.com/o/r/issues/3").expect("url"));
}
