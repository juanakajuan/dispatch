use super::keys::{Key, key_of};
use super::mouse::press_key;
use super::msg::{Cmd, Job, WriteJob};
use super::text_field::TextField;
use super::{Focus, State};
use crate::config::Config;
use crate::config::settings::{self, SETTINGS, Setting, Step};
use crossterm::event::{KeyCode, KeyEvent};

/// The settings overlay. Browsing has no `edit`; a typed value has one.
/// `pending` is a copy of the config that the keys change. `enter` saves it,
/// and `esc` drops it.
pub struct SettingsMenu {
    pub at: Setting,
    pub pending: Config,
    pub edit: Option<SettingEdit>,
}

impl SettingsMenu {
    pub fn new(config: &Config) -> SettingsMenu {
        SettingsMenu { at: SETTINGS[0], pending: config.clone(), edit: None }
    }
}

/// A typed value being edited. The error belongs here, so a browsing menu
/// can not show one.
pub struct SettingEdit {
    pub input: TextField,
    pub error: Option<String>,
}

pub(super) fn keybinds_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Keybinds { scroll } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Char('j') | Key::Code(KeyCode::Down)) => {
            *scroll = scroll.saturating_add(1).min(state.hits.max_scroll);
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
            Some(Key::Char('h') | Key::Code(KeyCode::Left)) => step(menu, &state.config, Step::Back),
            Some(Key::Char('l') | Key::Code(KeyCode::Right)) => step(menu, &state.config, Step::Forward),
            Some(Key::Code(KeyCode::Esc) | Key::Char(',')) => state.focus = Focus::List,
            Some(Key::Code(KeyCode::Enter)) if menu.at.is_typed() => {
                let input = TextField::new(&menu.at.show(&menu.pending));
                menu.edit = Some(SettingEdit { input, error: None });
            }
            Some(Key::Code(KeyCode::Enter)) => return save(state),
            _ => {}
        }
        return Vec::new();
    };
    match key {
        Some(Key::Code(KeyCode::Esc)) => menu.edit = None,
        Some(Key::Code(KeyCode::Enter)) => match menu.at.set_text(&mut menu.pending, edit.input.text()) {
            Ok(()) => return save(state),
            Err(e) => edit.error = Some(e.to_string()),
        },
        _ => {
            edit.input.edit(ev);
            edit.error = None;
        }
    }
    Vec::new()
}

/// `left`/`right` on the selected setting. A new harness clears the command
/// override. Back on the saved harness, the saved override comes back, so
/// going there and back changes nothing.
fn step(menu: &mut SettingsMenu, saved: &Config, step: Step) {
    menu.at.step(&mut menu.pending, step);
    if menu.at == Setting::Harness && menu.pending.harness == saved.harness {
        menu.pending.command.clone_from(&saved.command);
    }
}

/// `enter`: the menu's pending config becomes the live one, the changed
/// settings are saved in one write, and the menu closes.
fn save(state: &mut State) -> Vec<Cmd> {
    let Focus::Settings(menu) = std::mem::replace(&mut state.focus, Focus::List) else { return Vec::new() };
    let settings = settings::changed(&state.config, &menu.pending);
    state.config = menu.pending;
    if settings.is_empty() {
        return Vec::new();
    }
    vec![Cmd::Job(Job::Write(WriteJob::SaveConfig { settings, config: state.config.clone() }))]
}

/// A click on a settings row selects it. A click on the selected row selects
/// its next value, or opens the editor of a typed value. It never saves.
pub(super) fn click_setting(state: &mut State, setting: Setting) -> Vec<Cmd> {
    let Focus::Settings(menu) = &mut state.focus else { return Vec::new() };
    if menu.edit.is_some() {
        return Vec::new();
    }
    if menu.at != setting {
        menu.at = setting;
        return Vec::new();
    }
    let key = if setting.is_typed() { Key::Code(KeyCode::Enter) } else { Key::Char('l') };
    press_key(state, key)
}
