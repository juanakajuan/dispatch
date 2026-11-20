//! GFM tables: columns shrink to fit, and a cell that is cut ends in `…`.

use crate::ui::palette;
use crate::ui::text::{fit_spans, spans_width};
use ratatui::text::Span;

const SEPARATOR: &str = " │ ";
const SEPARATOR_WIDTH: usize = 3;

pub(super) type Cell = Vec<Span<'static>>;

#[derive(Default)]
pub(super) struct Table {
    pub rows: Vec<Vec<Cell>>,
    pub row: Vec<Cell>,
    pub header_rows: usize,
}

/// Shrinks the widest column, one cell at a time, until the row fits `avail`.
/// A column keeps at least one cell, so a very narrow `avail` is cut later.
fn column_widths(mut widths: Vec<usize>, avail: usize) -> Vec<usize> {
    let separators = SEPARATOR_WIDTH * widths.len().saturating_sub(1);
    while widths.iter().sum::<usize>() + separators > avail {
        let Some(widest) = widths.iter_mut().filter(|w| **w > 1).max_by_key(|w| **w) else { break };
        *widest -= 1;
    }
    widths
}

/// The lines of `table`, each at most `avail` cells wide when `avail` allows.
pub(super) fn layout(table: &Table, avail: usize) -> Vec<Vec<Span<'static>>> {
    let cols = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    let natural = (0..cols)
        .map(|i| table.rows.iter().filter_map(|r| r.get(i)).map(|c| spans_width(c)).max().unwrap_or(0))
        .collect();
    let widths = column_widths(natural, avail);
    let mut lines = Vec::new();
    for (ri, row) in table.rows.iter().enumerate() {
        let header = ri < table.header_rows;
        let mut line = Vec::new();
        for (ci, width) in widths.iter().enumerate() {
            if ci > 0 {
                line.push(Span::styled(SEPARATOR, palette::DIM));
            }
            let cell: Cell = row
                .get(ci)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|s| if header { Span::styled(s.content, s.style.patch(palette::BOLD)) } else { s })
                .collect();
            let cell = fit_spans(cell, *width);
            let pad = width.saturating_sub(spans_width(&cell));
            line.extend(cell);
            if ci + 1 < cols && pad > 0 {
                line.push(Span::raw(" ".repeat(pad)));
            }
        }
        lines.push(line);
        if header && ri + 1 == table.header_rows {
            let rule = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─");
            lines.push(vec![Span::styled(rule, palette::DIM)]);
        }
    }
    lines
}
