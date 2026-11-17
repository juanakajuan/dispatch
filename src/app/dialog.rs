use super::keys::{Key, key_of};
use super::mouse::HitTarget;
use super::msg::Cmd;
use super::sessions::{begin_start, goto_session};
use super::toast;
use super::{Focus, State};
use crate::domain::{BranchName, Issue, IssueNumber, Mode};
use crate::session::request::start_defaults;
use crate::session::request::{StartFrom, StartRequest};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct StartDialog {
    pub issue: IssueNumber,
    pub title: String,
    pub branch: tui_textarea::TextArea<'static>,
    pub mode: Mode,
    pub prompt: tui_textarea::TextArea<'static>,
    pub field: DialogField,
    /// Inline validation error (bad branch name). The dialog stays open.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogField {
    Branch,
    Mode,
    Prompt,
}

pub(super) const FIELDS: [DialogField; 3] = [DialogField::Branch, DialogField::Mode, DialogField::Prompt];

pub(super) fn open_dialog(issue: &Issue) -> StartDialog {
    let defaults = start_defaults(issue);
    let mut branch = tui_textarea::TextArea::new(vec![defaults.branch.as_str().to_string()]);
    branch.move_cursor(tui_textarea::CursorMove::End);
    let prompt = tui_textarea::TextArea::new(defaults.prompt.split('\n').map(str::to_string).collect());
    StartDialog {
        issue: issue.number,
        title: issue.title.clone(),
        branch,
        mode: defaults.mode,
        prompt,
        field: DialogField::Branch,
        error: None,
    }
}

pub(super) fn dialog_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Dialog(dialog) = &mut state.focus else { return Vec::new() };
    let key = key_of(ev);
    let on_text = matches!(dialog.field, DialogField::Branch | DialogField::Prompt);
    match key {
        Some(Key::Code(KeyCode::Esc)) => state.focus = Focus::List,
        Some(Key::Ctrl('s')) => return confirm_dialog(state),
        Some(Key::Code(KeyCode::Enter)) if dialog.field != DialogField::Prompt => return confirm_dialog(state),
        Some(Key::Code(KeyCode::Enter)) if ev.modifiers.contains(KeyModifiers::CONTROL) => {
            return confirm_dialog(state);
        }
        Some(Key::Code(KeyCode::Tab)) => dialog.field = step_field(dialog.field, 1),
        Some(Key::Code(KeyCode::BackTab)) => dialog.field = step_field(dialog.field, FIELDS.len() - 1),
        Some(Key::Code(KeyCode::Down)) if dialog.field != DialogField::Prompt => {
            dialog.field = step_field(dialog.field, 1);
        }
        Some(Key::Code(KeyCode::Up)) if dialog.field != DialogField::Prompt => {
            dialog.field = step_field(dialog.field, FIELDS.len() - 1);
        }
        Some(Key::Code(KeyCode::Left | KeyCode::Right) | Key::Char(' ')) if dialog.field == DialogField::Mode => {
            dialog.mode = match dialog.mode {
                Mode::PlanFirst => Mode::Implement,
                Mode::Implement => Mode::PlanFirst,
            };
        }
        _ if on_text => {
            let target = if dialog.field == DialogField::Branch { &mut dialog.branch } else { &mut dialog.prompt };
            target.input(*ev);
            dialog.error = None;
        }
        _ => {}
    }
    Vec::new()
}

pub(super) fn step_field(field: DialogField, by: usize) -> DialogField {
    let at = FIELDS.iter().position(|f| *f == field).unwrap_or(0);
    FIELDS[(at + by) % FIELDS.len()]
}

pub(super) fn confirm_dialog(state: &mut State) -> Vec<Cmd> {
    let Focus::Dialog(dialog) = &mut state.focus else { return Vec::new() };
    let branch = match BranchName::parse(&dialog.branch.lines().join("")) {
        Ok(b) => b,
        Err(e) => {
            dialog.error = Some(format!("Branch name: {e:?}"));
            dialog.field = DialogField::Branch;
            return Vec::new();
        }
    };
    let prompt = dialog.prompt.lines().join("\n").trim().to_string();
    if prompt.is_empty() {
        dialog.error = Some("The prompt is empty.".to_string());
        dialog.field = DialogField::Prompt;
        return Vec::new();
    }
    let req = StartRequest { issue: dialog.issue, branch, mode: dialog.mode, prompt, from: StartFrom::Base };
    let n = req.issue;
    let cmd = begin_start(state, req);
    state.focus = Focus::List;
    goto_session(state, n);
    toast::info(state, format!("Starting {n}"));
    vec![cmd]
}

/// A click inside the start dialog. `at` is the click's offset in the target's rect.
pub(super) fn click_dialog(dialog: &mut StartDialog, target: HitTarget, at: (u16, u16)) {
    match target {
        HitTarget::Field(field) => dialog.field = field,
        HitTarget::Mode(mode) => {
            dialog.mode = mode;
            dialog.field = DialogField::Mode;
        }
        HitTarget::Text { field, top, left } => {
            dialog.field = field;
            let input = if field == DialogField::Branch { &mut dialog.branch } else { &mut dialog.prompt };
            input.move_cursor(tui_textarea::CursorMove::Jump(top.saturating_add(at.1), left.saturating_add(at.0)));
        }
        HitTarget::Tab(_) | HitTarget::Row(_) | HitTarget::Key(_) | HitTarget::Setting(_) | HitTarget::Binding(_) => {}
    }
}
