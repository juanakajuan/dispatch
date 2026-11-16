use super::dialog::PROMPT_MIN_ROWS;
use super::header::since;
use super::text::{age, cells, wrap};
use super::*;
use crate::app::feed::FeedStatus;
use crate::app::list::{RowKey, Tab};
use crate::app::mouse::HitTarget;
use crate::app::{self};
use crate::app::{Context, msg::Msg};
use crate::config::Config;
use crate::domain::{BranchName, WorktreePath};
use crate::domain::{Checks, IssueNumber, PrState, PullRequest};
use crate::glyphs::IconSet;
use crate::session::Session;
use crate::session::{StartPreview, Window, WindowId};
use crate::testutil::{issue, pr};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod dialog;
mod footer;
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
    state.issues.status = FeedStatus::Fresh { at: Instant::now() };
    state.prs.items = vec![pr(10, "fix/3-x", &[3])];
    state.prs.status = FeedStatus::Fresh { at: Instant::now() };
    state.inflight.issues = false;
    state.inflight.prs = false;
    let id = WindowId::parse("@3").expect("id");
    state.sessions.insert(
        IssueNumber(3),
        Session {
            issue: IssueNumber(3),
            branch: BranchName::parse("fix/3-x").expect("branch"),
            worktree: WorktreePath::planned(PathBuf::from("/w/orbit-3")),
            window: Some(Window { id, alive: true, waiting: true }),
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
