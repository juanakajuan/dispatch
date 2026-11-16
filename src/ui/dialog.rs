use super::palette;
use super::text::{cells, fit, one_line, span, spans_width, wrap};
use crate::app::keys::Key;
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, State, dialog::DialogField};
use crate::domain::Mode;
use crate::session::request::Launch;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

/// The exact commands the dialog will run, for the "Runs" preview.
pub(super) fn preview(state: &State, dialog: &app::dialog::StartDialog) -> Vec<String> {
    let branch = dialog.branch.lines().join("");
    let prompt = dialog.prompt.lines().join("\n");
    let launch = Launch::from_config(&state.config);
    let commands =
        state.context.start_preview.commands(dialog.issue, branch.trim(), dialog.mode, prompt.trim(), &launch);
    commands
        .into_iter()
        .map(|argv| {
            let shown: Vec<String> = argv
                .into_iter()
                .map(|a| if a.contains([' ', '\n', '"']) { format!("{:?}", one_line(&a)) } else { a })
                .collect();
            shown.join(" ")
        })
        .collect()
}

/// The "Runs" label and each preview command, word-wrapped to `width` cells.
/// A wrapped row hangs under the start of its command.
pub(super) fn run_lines(state: &State, dialog: &app::dialog::StartDialog, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(span("  Runs", palette::DIM))];
    for command in preview(state, dialog) {
        for (i, row) in wrap(&command, width.saturating_sub(4).max(1)).into_iter().enumerate() {
            let indent = if i == 0 { "  $ " } else { "    " };
            lines.push(Line::from(span(format!("{indent}{row}"), palette::DIM)));
        }
    }
    lines
}

#[derive(Clone, Copy)]
pub(super) enum Focus {
    On,
    Off,
}

impl Focus {
    pub(super) fn of(dialog: &app::dialog::StartDialog, field: DialogField) -> Focus {
        if dialog.field == field { Focus::On } else { Focus::Off }
    }
}

pub(super) fn styled_input(source: &tui_textarea::TextArea<'static>, focus: Focus) -> tui_textarea::TextArea<'static> {
    let mut input = source.clone();
    input.set_style(palette::PLAIN);
    input.set_cursor_line_style(palette::PLAIN);
    input.set_cursor_style(match focus {
        Focus::On => palette::CURSOR,
        Focus::Off => palette::PLAIN,
    });
    input
}

pub(super) fn field_label(text: &str, focus: Focus) -> Line<'static> {
    match focus {
        Focus::On => Line::from(vec![span("› ", palette::ACCENT), span(text.to_string(), palette::ACTIVE)]),
        Focus::Off => Line::from(span(format!("  {text}"), palette::DIM)),
    }
}

/// Rows of the start dialog that are not the prompt or the "Runs" preview.
pub(super) const DIALOG_FIXED_ROWS: u16 = 10;
/// The prompt input keeps at least this many rows when the preview is tall.
pub(super) const PROMPT_MIN_ROWS: u16 = 3;

pub(super) fn draw_dialog(
    frame: &mut Frame,
    screen: Rect,
    state: &State,
    d: &app::dialog::StartDialog,
    hits: &mut HitMap,
) {
    let width = screen.width.min(96);
    let height = screen.height.min(28);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(format!(" Start session for {} {} ", d.issue, fit(&d.title, 50)), palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let run_lines = run_lines(state, d, usize::from(inner.width));
    let room = inner.height.saturating_sub(DIALOG_FIXED_ROWS + PROMPT_MIN_ROWS);
    let runs_h = u16::try_from(run_lines.len()).unwrap_or(u16::MAX).min(room);
    let [branch_l, branch, _, mode, _, prompt_l, prompt, _, runs, error, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(PROMPT_MIN_ROWS),
        Constraint::Length(1),
        Constraint::Length(runs_h),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let indent = |r: Rect| Rect { x: r.x.saturating_add(2), width: r.width.saturating_sub(2), ..r };
    let focus = |f: DialogField| Focus::of(d, f);
    frame.render_widget(Paragraph::new(field_label("Branch", focus(DialogField::Branch))), branch_l);
    frame.render_widget(&styled_input(&d.branch, focus(DialogField::Branch)), indent(branch));
    hits.targets.push((branch_l, HitTarget::Field(DialogField::Branch)));
    hits.targets.push(text_target(&d.branch, DialogField::Branch, indent(branch)));

    let icons = state.config.icons.icons();
    let radio = |on: bool, text: &str| -> Vec<Span<'static>> {
        if on {
            vec![span(format!("{} ", icons.radio_on), palette::ACCENT), Span::raw(text.to_string())]
        } else {
            vec![span(format!("{} {text}", icons.radio_off), palette::DIM)]
        }
    };
    let mut modes = vec![Span::raw("  ")];
    modes.extend(radio(d.mode == Mode::PlanFirst, PLAN_FIRST));
    modes.push(Span::raw("    "));
    modes.extend(radio(d.mode == Mode::Implement, IMPLEMENT));
    frame.render_widget(Paragraph::new(vec![field_label("Mode", focus(DialogField::Mode)), Line::from(modes)]), mode);
    hits.targets.push((mode, HitTarget::Field(DialogField::Mode)));
    let mut option_x = 2;
    for (m, text) in [(Mode::PlanFirst, PLAN_FIRST), (Mode::Implement, IMPLEMENT)] {
        let w = spans_width(&radio(d.mode == m, text));
        let rect = Rect::new(mode.x.saturating_add(option_x), mode.y + 1, u16::try_from(w).unwrap_or(0), 1);
        hits.targets.push((rect.intersection(mode), HitTarget::Mode(m)));
        option_x = option_x.saturating_add(u16::try_from(w + 4).unwrap_or(0));
    }
    frame.render_widget(Paragraph::new(field_label("Prompt", focus(DialogField::Prompt))), prompt_l);
    frame.render_widget(&styled_input(&d.prompt, focus(DialogField::Prompt)), indent(prompt));
    hits.targets.push((prompt_l, HitTarget::Field(DialogField::Prompt)));
    hits.targets.push(text_target(&d.prompt, DialogField::Prompt, indent(prompt)));

    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(format!("  {e}"), palette::ERROR)), error);
    }
    draw_buttons(frame, buttons, hits);
}

const PLAN_FIRST: &str = "Plan first, then wait for approval";
const IMPLEMENT: &str = "Implement directly";

/// `[ Start ]` presses ctrl-s and `[ Cancel ]` presses esc.
fn draw_buttons(frame: &mut Frame, row: Rect, hits: &mut HitMap) {
    let (start, cancel) = ("[ Start ]", "[ Cancel ]");
    let start_w = u16::try_from(cells(start)).unwrap_or(0);
    let cancel_w = u16::try_from(cells(cancel)).unwrap_or(0);
    let at = Rect { x: row.x.saturating_add(2), width: row.width.saturating_sub(2), ..row };
    let start_rect = Rect::new(at.x, row.y, start_w, 1).intersection(at);
    let cancel_rect = Rect::new(at.x + start_w + 2, row.y, cancel_w, 1).intersection(at);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            span(start, palette::ACTIVE),
            Span::raw("  "),
            span(cancel, palette::DIM),
        ])),
        row,
    );
    hits.targets.push((start_rect, HitTarget::Key(Key::Ctrl('s'))));
    hits.targets.push((cancel_rect, HitTarget::Key(Key::Code(KeyCode::Esc))));
}

/// A click target for a text input drawn in `rect`.
fn text_target(input: &tui_textarea::TextArea<'static>, field: DialogField, rect: Rect) -> (Rect, HitTarget) {
    // `styled_input` draws a clone whose scroll starts at 0. tui-textarea then
    // scrolls only as far as keeps the cursor in view, which is this formula.
    let (row, col) = input.cursor();
    let scrolled = |cursor: usize, len: u16| {
        let cursor = u16::try_from(cursor).unwrap_or(u16::MAX);
        cursor.saturating_sub(len.saturating_sub(1))
    };
    (rect, HitTarget::Text { field, top: scrolled(row, rect.height), left: scrolled(col, rect.width) })
}
