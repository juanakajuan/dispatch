use super::keys::{Key, key_of};
use super::msg::{Cmd, Job};
use super::{Focus, State};
use crate::config::settings::{Change, SETTINGS, Setting};
use crossterm::event::{KeyCode, KeyEvent};

/// The settings overlay. Browsing has no `edit`; a typed value has one.
pub struct SettingsMenu {
    pub at: Setting,
    pub edit: Option<SettingEdit>,
}

/// A typed value being edited. The error belongs here, so a browsing menu
/// can not show one.
pub struct SettingEdit {
    pub input: tui_textarea::TextArea<'static>,
    pub error: Option<String>,
}

pub(super) fn keybinds_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Keybinds { scroll } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Char('j') | Key::Code(KeyCode::Down)) => {
            *scroll = scroll.saturating_add(1).min(state.hits.keybinds_max_scroll);
        }
        Some(Key::Char('k') | Key::Code(KeyCode::Up)) => *scroll = scroll.saturating_sub(1),
        Some(Key::Code(KeyCode::Esc) | Key::Char('?')) => state.focus = Focus::List,
        _ => {}
    }
    Vec::new()
}

pub(super) fn settings_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Settings(menu) = &mut state.focus else { return Vec::new() };
    let key = key_of(ev);
    let Some(edit) = &mut menu.edit else {
        let at = SETTINGS.iter().position(|s| *s == menu.at).unwrap_or(0);
        match key {
            Some(Key::Char('j') | Key::Code(KeyCode::Down)) => menu.at = SETTINGS[(at + 1).min(SETTINGS.len() - 1)],
            Some(Key::Char('k') | Key::Code(KeyCode::Up)) => menu.at = SETTINGS[at.saturating_sub(1)],
            Some(Key::Code(KeyCode::Esc) | Key::Char(',')) => state.focus = Focus::List,
            Some(Key::Code(KeyCode::Enter)) => {
                let setting = menu.at;
                match setting.change(&mut state.config) {
                    Change::Applied => return changed(state, setting),
                    Change::Edit(text) => {
                        let mut input = tui_textarea::TextArea::new(vec![text]);
                        input.move_cursor(tui_textarea::CursorMove::End);
                        menu.edit = Some(SettingEdit { input, error: None });
                    }
                }
            }
            _ => {}
        }
        return Vec::new();
    };
    match key {
        Some(Key::Code(KeyCode::Esc)) => menu.edit = None,
        Some(Key::Code(KeyCode::Enter)) => {
            let setting = menu.at;
            match setting.set_text(&mut state.config, &edit.input.lines().join("")) {
                Ok(()) => {
                    menu.edit = None;
                    return changed(state, setting);
                }
                Err(e) => edit.error = Some(e.to_string()),
            }
        }
        _ => {
            edit.input.input(*ev);
            edit.error = None;
        }
    }
    Vec::new()
}

/// `state.config` already holds the new value. Save it.
pub(super) fn changed(state: &State, setting: Setting) -> Vec<Cmd> {
    vec![Cmd::Job(Job::SaveConfig { setting, config: state.config.clone() })]
}
