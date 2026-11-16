use super::list::{Move, RowKey, Tab, move_selection, select};
use super::msg::{Cmd, DOUBLE_CLICK};
use super::sessions::start_issue;
use super::{Focus, State};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::time::Instant;

/// Clickable regions from the last frame. Written by the shell after each
/// draw (`ui::draw` returns it); read by `update` on mouse events.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(ratatui::layout::Rect, HitTarget)>,
    /// How far the keybinds overlay can scroll at the last drawn size.
    pub keybinds_max_scroll: u16,
}

impl HitMap {
    pub(super) fn at(&self, column: u16, row: u16) -> Option<HitTarget> {
        let pos = ratatui::layout::Position { x: column, y: row };
        self.targets.iter().find(|(rect, _)| rect.contains(pos)).map(|(_, t)| *t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Tab(Tab),
    Row(RowKey),
}

pub(super) fn on_mouse(state: &mut State, ev: MouseEvent, now: Instant) -> Vec<Cmd> {
    if !matches!(state.focus, Focus::List) {
        return Vec::new();
    }
    match ev.kind {
        MouseEventKind::ScrollDown => move_selection(state, Move::Down),
        MouseEventKind::ScrollUp => move_selection(state, Move::Up),
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(target) = state.hits.at(ev.column, ev.row) else { return Vec::new() };
            let double = state.last_click.is_some_and(|(t, at)| t == target && now.duration_since(at) <= DOUBLE_CLICK);
            state.last_click = if double { None } else { Some((target, now)) };
            match target {
                HitTarget::Tab(tab) => state.tab = tab,
                HitTarget::Row(key) => {
                    select(state, key);
                    if let (true, RowKey::Issue(n)) = (double, key) {
                        start_issue(state, n);
                    }
                }
            }
        }
        _ => {}
    }
    Vec::new()
}
