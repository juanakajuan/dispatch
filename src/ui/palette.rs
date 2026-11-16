//! The whole palette. No other module builds a `Style`.
//! Rule: Gruvbox (dark) foregrounds, and a background color only on the tab
//! number badges, so the terminal's own background shows through elsewhere.
//! One accent (blue) marks what is active: the tab, the selected row, the
//! focused field, the keys. The other colors carry state only.

use ratatui::style::{Color, Modifier, Style};

pub const FG: Color = Color::Rgb(0xeb, 0xdb, 0xb2);
pub const GRAY: Color = Color::Rgb(0x92, 0x83, 0x74);
pub const RED: Color = Color::Rgb(0xfb, 0x49, 0x34);
pub const GREEN: Color = Color::Rgb(0xb8, 0xbb, 0x26);
pub const YELLOW: Color = Color::Rgb(0xfa, 0xbd, 0x2f);
pub const BLUE: Color = Color::Rgb(0x83, 0xa5, 0x98);
pub const PURPLE: Color = Color::Rgb(0xd3, 0x86, 0x9b);
pub const AQUA: Color = Color::Rgb(0x8e, 0xc0, 0x7c);
pub const BG0: Color = Color::Rgb(0x28, 0x28, 0x28);
pub const BG2: Color = Color::Rgb(0x50, 0x49, 0x45);

pub const PLAIN: Style = Style::new().fg(FG);
pub const HEADING: Style = Style::new().fg(FG).add_modifier(Modifier::BOLD);
pub const DIM: Style = Style::new().fg(GRAY);
pub const ACCENT: Style = Style::new().fg(BLUE);
pub const ACTIVE: Style = Style::new().fg(BLUE).add_modifier(Modifier::BOLD);
/// The number badge of the shown tab, and of the other tabs.
pub const TAB_KEY_ON: Style = Style::new().fg(BG0).bg(BLUE).add_modifier(Modifier::BOLD);
pub const TAB_KEY_OFF: Style = Style::new().fg(FG).bg(BG2);
pub const CURSOR: Style = Style::new().add_modifier(Modifier::REVERSED);
pub const RUNNING: Style = Style::new().fg(GREEN);
pub const NEEDS_YOU: Style = Style::new().fg(YELLOW).add_modifier(Modifier::BOLD);
pub const EXITED: Style = Style::new().fg(GRAY);
pub const STARTING: Style = Style::new().fg(AQUA);
pub const PASS: Style = Style::new().fg(GREEN);
pub const FAIL: Style = Style::new().fg(RED);
pub const PENDING: Style = Style::new().fg(YELLOW);
pub const MERGED: Style = Style::new().fg(PURPLE);
pub const LABEL: Style = Style::new().fg(AQUA);
pub const ERROR: Style = Style::new().fg(RED);
pub const WARN: Style = Style::new().fg(YELLOW);
