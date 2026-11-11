//! Rendering. Reads `State`, writes nothing but the frame, and returns the
//! clickable regions it drew. Layout: tab bar (1 row), list | detail
//! (40% | 60%), key bar (1 row). The dialog is a centered overlay.

use crate::app::{HitMap, State};
use crate::config::Config;
use ratatui::Frame;

/// The whole palette. No other module builds a `Style`.
/// Rule: foreground from the 16 ANSI colors only; never a background color;
/// selection is `Modifier::REVERSED`, so it uses the terminal's own colors and
/// the user's theme applies everywhere.
pub mod palette {
    use ratatui::style::{Color, Modifier, Style};

    pub const DIM: Style = Style::new().fg(Color::DarkGray);
    pub const NUMBER: Style = Style::new().fg(Color::Cyan);
    pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
    pub const RUNNING: Style = Style::new().fg(Color::Green);
    pub const NEEDS_YOU: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    pub const EXITED: Style = Style::new().fg(Color::DarkGray);
    pub const PASS: Style = Style::new().fg(Color::Green);
    pub const FAIL: Style = Style::new().fg(Color::Red);
    pub const PENDING: Style = Style::new().fg(Color::Yellow);
    pub const LABEL: Style = Style::new().fg(Color::Magenta);
    pub const KEY: Style = Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD);
    pub const ERROR: Style = Style::new().fg(Color::Red);
}

pub fn draw(frame: &mut Frame, state: &State, config: &Config) -> HitMap {
    // TODO: split areas; draw_tabs (push Tab hits); draw_list over app::visible_rows
    //       (push Row hits per visible line); draw_detail by tab; draw_keys (context
    //       keys from app::current_session state); draw_dialog if Focus::Dialog.
    //
    // Issue rows:   #N  title  [labels]  <session badge or "starting">  age
    // PR rows:      #N  title  checks badge  state  age
    // Session rows: #N  issue title (or branch if the issue is closed)  badge  branch
    //
    // Issue detail: title, meta, session box (badge, branch, last 3 tail lines, keys)
    //   or "s Start Claude session"; body; linked PRs (prs where closes contains N).
    // PR detail: state, checks, head -> base, +/-; closes; "session #N" if a session
    //   branch equals head.
    // Session detail: badge, branch, worktree path, skill or "automatic skill",
    //   PR via app::pr_for_session, then the tail (bottom-anchored).
    // Dialog: branch, skill (< choice >, hint from skill_reason), mode, prompt,
    //   and a "Runs:" preview of the exact git and claude argv.
    todo!()
}

/// "3h", "2d", "now". Pure, tested.
pub fn age(created: jiff::Timestamp, now: jiff::Timestamp) -> String {
    todo!()
}
