use super::dialog::PROMPT_MIN_ROWS;
use super::header::since;
use super::text::{age, cells, wrap};
use super::*;
use crate::app::feed::LastFetch;
use crate::app::keys::Key;
use crate::app::list::{RowKey, Tab};
use crate::app::mouse::HitMap;
use crate::app::mouse::HitTarget;
use crate::app::{self};
use crate::app::{Context, msg::Msg};
use crate::config::Config;
use crate::domain::{BranchName, WorktreePath};
use crate::domain::{Checks, IssueNumber, PrState, PullRequest};
use crate::glyphs::IconSet;
use crate::session::Session;
use crate::session::{SessionState, StartPreview};
use crate::testutil::{issue, issue_key, pr, pr_key};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod batch;
mod columns;
mod dialog;
mod footer;
mod hover;
mod markdown;
mod menus;
mod screen;
mod text;

pub(super) fn full_state() -> State {
    let (mut state, _) =
        app::init(Instant::now(), Config::default(), Context { repo_name: "orbit".to_string(), ..Context::default() });
    state.issues.items = vec![
        issue(
            3,
            "Retry queue drops jobs when Redis reconnects after a very long title that must be cut",
            &["bug", "p1"],
        ),
        issue(2, "Docs", &["docs"]),
    ];
    state.issues.last = LastFetch::Fresh { at: Instant::now() };
    state.prs.items = vec![pr(10, "fix/3-x", &[3])];
    state.prs.last = LastFetch::Fresh { at: Instant::now() };
    state.issues.poll.done();
    state.prs.poll.done();
    state.sessions.insert(
        issue_key(3),
        Session {
            key: issue_key(3),
            branch: BranchName::parse("fix/3-x").expect("branch"),
            checkout: crate::session::Checkout::SessionBranch,
            worktree: WorktreePath::planned(PathBuf::from("/w/orbit-3")),
            harness: crate::config::Harness::Claude,
            state: SessionState::NeedsYou,
        },
    );
    state
}

pub(super) fn render(state: &State, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    terminal
        .draw(|f| {
            let _ = draw(f, state);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

/// Draws like the shell does: the frame's hits go back into `state`, so the
/// reducer sees this frame's targets and size.
pub(super) fn frame(state: &mut State, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    let mut hits = HitMap::default();
    terminal.draw(|f| hits = draw(f, state)).expect("draw");
    state.hits = hits;
    terminal.backend().buffer().clone()
}

pub(super) fn draw_into(state: &mut State, w: u16, h: u16) -> String {
    text(&frame(state, w, h))
}

/// The (x, y) of the first cell of `needle` on the drawn screen.
pub(super) fn find(screen: &str, needle: &str) -> (u16, u16) {
    let (y, line) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(needle))
        .unwrap_or_else(|| panic!("no {needle:?} in\n{screen}"));
    let x = cells(&line[..line.find(needle).expect("found")]);
    (u16::try_from(x).expect("x"), u16::try_from(y).expect("y"))
}

/// The target under the first cell of `needle`, and under its last cell.
fn targets_of(state: &State, screen: &str, needle: &str) -> (Option<HitTarget>, Option<HitTarget>) {
    let (x, y) = find(screen, needle);
    let last = x + u16::try_from(cells(needle)).expect("w") - 1;
    let at = |x: u16| state.hits.at(x, y).map(|(_, t)| t);
    (at(x), at(last))
}

pub(super) fn assert_clickable(state: &State, screen: &str, needle: &str, want: HitTarget) {
    assert_eq!(targets_of(state, screen, needle), (Some(want), Some(want)), "{needle:?} in\n{screen}");
}

pub(super) fn click_at(state: &mut State, x: u16, y: u16) {
    let ev =
        MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE };
    let _ = app::update(state, Msg::Mouse(ev, Instant::now()));
}

pub(super) fn text(buf: &Buffer) -> String {
    buf.content()
        .chunks(usize::from(buf.area.width))
        .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn send(state: &mut State, code: KeyCode) {
    let _ = app::update(state, Msg::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

pub(super) fn in_palette(c: Color) -> bool {
    use palette::{AQUA, BG0, BLUE, FG, GRAY, GREEN, PURPLE, RED, YELLOW};
    [FG, GRAY, RED, GREEN, YELLOW, BLUE, PURPLE, AQUA, BG0].contains(&c)
}

/// The commands in the dialog's "Runs" preview, with wrapped rows joined.
/// Only the cells inside the dialog border are read.
pub(super) fn previewed_commands(screen: &str) -> Vec<String> {
    let rows: Vec<Vec<char>> =
        screen.lines().skip_while(|l| !l.contains("  Runs")).map(|l| l.chars().collect()).collect();
    let label = rows.first().expect("runs label drawn");
    let left = label.iter().position(|&c| c == '│').expect("dialog border");
    let right = left + 1 + label[left + 1..].iter().position(|&c| c == '│').expect("dialog border");
    let mut commands: Vec<String> = Vec::new();
    for row in rows.iter().skip(1) {
        let inside: String = row[left + 1..right].iter().collect();
        let row = inside.trim();
        if let Some(start) = row.strip_prefix("$ ") {
            commands.push(start.to_string());
        } else if let (Some(last), false) = (commands.last_mut(), row.is_empty()) {
            last.push(' ');
            last.push_str(row);
        } else {
            break;
        }
    }
    commands
}
