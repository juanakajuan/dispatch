use super::palette;
use super::text::{fit, one_line, span, wrap};
use crate::app::{self, State, dialog::DialogField};
use crate::domain::Mode;
use crate::session::request::Launch;
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

pub(super) fn styled_input(source: &tui_textarea::TextArea<'static>, focused: bool) -> tui_textarea::TextArea<'static> {
    let mut input = source.clone();
    input.set_style(palette::PLAIN);
    input.set_cursor_line_style(palette::PLAIN);
    input.set_cursor_style(if focused { palette::CURSOR } else { palette::PLAIN });
    input
}

pub(super) fn field_label(text: &str, focused: bool) -> Line<'static> {
    if focused {
        Line::from(vec![span("› ", palette::ACCENT), span(text.to_string(), palette::ACTIVE)])
    } else {
        Line::from(span(format!("  {text}"), palette::DIM))
    }
}

/// Rows of the start dialog that are not the prompt or the "Runs" preview.
pub(super) const DIALOG_FIXED_ROWS: u16 = 9;
/// The prompt input keeps at least this many rows when the preview is tall.
pub(super) const PROMPT_MIN_ROWS: u16 = 3;

pub(super) fn draw_dialog(frame: &mut Frame, screen: Rect, state: &State, d: &app::dialog::StartDialog) {
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
    let [branch_l, branch, _, mode, _, prompt_l, prompt, _, runs, error] = Layout::vertical([
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
    ])
    .areas(inner);
    let indent = |r: Rect| Rect { x: r.x.saturating_add(2), width: r.width.saturating_sub(2), ..r };
    let focus = |f: DialogField| d.field == f;
    frame.render_widget(Paragraph::new(field_label("Branch", focus(DialogField::Branch))), branch_l);
    frame.render_widget(&styled_input(&d.branch, focus(DialogField::Branch)), indent(branch));

    let icons = state.config.icons.icons();
    let radio = |on: bool, text: &str| -> Vec<Span<'static>> {
        if on {
            vec![span(format!("{} ", icons.radio_on), palette::ACCENT), Span::raw(text.to_string())]
        } else {
            vec![span(format!("{} {text}", icons.radio_off), palette::DIM)]
        }
    };
    let mut modes = vec![Span::raw("  ")];
    modes.extend(radio(d.mode == Mode::PlanFirst, "Plan first, then wait for approval"));
    modes.push(Span::raw("    "));
    modes.extend(radio(d.mode == Mode::Implement, "Implement directly"));
    frame.render_widget(Paragraph::new(vec![field_label("Mode", focus(DialogField::Mode)), Line::from(modes)]), mode);
    frame.render_widget(Paragraph::new(field_label("Prompt", focus(DialogField::Prompt))), prompt_l);
    frame.render_widget(&styled_input(&d.prompt, focus(DialogField::Prompt)), indent(prompt));

    frame.render_widget(Paragraph::new(run_lines), runs);
    if let Some(e) = &d.error {
        frame.render_widget(Paragraph::new(span(format!("  {e}"), palette::ERROR)), error);
    }
}
