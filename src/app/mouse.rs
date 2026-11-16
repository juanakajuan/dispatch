use super::dialog::{DialogField, click_dialog};
use super::keys::{Action, Key, act};
use super::list::{Move, RowKey, Tab, move_selection, select};
use super::menus::click_setting;
use super::msg::{Cmd, DOUBLE_CLICK, Msg};
use super::{Focus, State, update};
use crate::config::settings::Setting;
use crate::domain::Mode;
use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::time::Instant;

/// Clickable regions from the last frame. Written by the shell after each
/// draw (`ui::draw` returns it); read by `update` on mouse events.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(Rect, HitTarget)>,
    /// How far the keybinds overlay can scroll at the last drawn size.
    pub keybinds_max_scroll: u16,
}

impl HitMap {
    /// The target under a cell, with its rect. Later targets are drawn on top
    /// of earlier ones, so the last match wins.
    pub fn at(&self, column: u16, row: u16) -> Option<(Rect, HitTarget)> {
        let pos = Position { x: column, y: row };
        self.targets.iter().rev().find(|(rect, _)| rect.contains(pos)).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Tab(Tab),
    Row(RowKey),
    /// Press `Key` in the current focus: key bar entries, the `S sort` footer,
    /// footer hints of overlays, overlay close controls, filter open and clear.
    Key(Key),
    /// Start dialog: focus this field (label rows, mode row).
    Field(DialogField),
    /// Start dialog: select this mode (each radio option) and focus Mode.
    Mode(Mode),
    /// Start dialog text input. `top`/`left` are the textarea scroll offset at
    /// draw time, so a click at (x, y) in the rect puts the cursor at
    /// `(top + y - rect.y, left + x - rect.x)`.
    Text {
        field: DialogField,
        top: u16,
        left: u16,
    },
    /// Settings menu row.
    Setting(Setting),
    /// Keybinds menu entry: close the menu and run the binding's action.
    Binding(Action),
}

pub(super) fn on_mouse(state: &mut State, ev: MouseEvent, now: Instant) -> Vec<Cmd> {
    state.pointer = Some(Position { x: ev.column, y: ev.row });
    match ev.kind {
        MouseEventKind::ScrollDown => wheel(state, Move::Down),
        MouseEventKind::ScrollUp => wheel(state, Move::Up),
        MouseEventKind::Down(MouseButton::Left) => click(state, ev.column, ev.row, now),
        _ => Vec::new(),
    }
}

/// The wheel moves the selection, or scrolls the menu that is open.
fn wheel(state: &mut State, step: Move) -> Vec<Cmd> {
    match &state.focus {
        Focus::List | Focus::Filter => move_selection(state, step),
        Focus::Keybinds { .. } => return press_key(state, wheel_key(step)),
        Focus::Settings(menu) if menu.edit.is_none() => return press_key(state, wheel_key(step)),
        Focus::Settings(_) | Focus::Dialog(_) => {}
    }
    Vec::new()
}

fn wheel_key(step: Move) -> Key {
    Key::Code(match step {
        Move::Down => KeyCode::Down,
        Move::Up => KeyCode::Up,
    })
}

pub(super) fn press_key(state: &mut State, key: Key) -> Vec<Cmd> {
    update(state, Msg::Key(key.event()))
}

fn click(state: &mut State, column: u16, row: u16, now: Instant) -> Vec<Cmd> {
    let Some((rect, target)) = state.hits.at(column, row) else { return Vec::new() };
    let double = state.last_click.is_some_and(|(t, at)| t == target && now.duration_since(at) <= DOUBLE_CLICK);
    state.last_click = if double { None } else { Some((target, now)) };
    let in_list = matches!(state.focus, Focus::List);
    match target {
        HitTarget::Key(key) => return press_key(state, key),
        HitTarget::Tab(tab) if in_list => state.tab = tab,
        HitTarget::Row(key) if in_list => {
            select(state, key);
            if double {
                return act(state, Action::Enter, None);
            }
        }
        HitTarget::Tab(_) | HitTarget::Row(_) => {}
        HitTarget::Field(_) | HitTarget::Mode(_) | HitTarget::Text { .. } => {
            if let Focus::Dialog(dialog) = &mut state.focus {
                click_dialog(dialog, target, (column - rect.x, row - rect.y));
            }
        }
        HitTarget::Setting(setting) => return click_setting(state, setting),
        HitTarget::Binding(action) => {
            if matches!(state.focus, Focus::Keybinds { .. }) {
                state.focus = Focus::List;
                return act(state, action, None);
            }
        }
    }
    Vec::new()
}
