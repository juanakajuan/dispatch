use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// "3h", "2d", "now". Pure, tested.
pub(super) fn age(created: jiff::Timestamp, now: jiff::Timestamp) -> String {
    let secs = now.as_second().saturating_sub(created.as_second()).max(0);
    match secs {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=2_591_999 => format!("{}d", secs / 86_400),
        2_592_000..=31_535_999 => format!("{}mo", secs / 2_592_000),
        _ => format!("{}y", secs / 31_536_000),
    }
}

pub(crate) fn cells(text: &str) -> usize {
    Span::raw(text).width()
}

pub(super) fn char_cells(c: char) -> usize {
    cells(c.encode_utf8(&mut [0; 4]))
}

/// Word-wraps `text` to rows of at most `width` cells. A word wider than a
/// whole row is split by cell.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cells(&cur) + 1 + cells(word) > width {
            rows.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        for c in word.chars() {
            let mut buf = [0; 4];
            if !cur.is_empty() && cells(&cur) + cells(c.encode_utf8(&mut buf)) > width {
                rows.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    rows
}

/// Cuts `text` to at most `width` cells and ends it with an ellipsis when cut.
pub(super) fn fit(text: &str, width: usize) -> String {
    if cells(text) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_cells(c);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// Cuts `spans` to at most `width` cells across span boundaries, with no mark.
pub(super) fn cut_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0;
    for s in spans {
        let room = width.saturating_sub(used);
        if s.width() <= room {
            used += s.width();
            out.push(s);
            continue;
        }
        let mut text = String::new();
        for c in s.content.chars() {
            let w = char_cells(c);
            if w > room.saturating_sub(cells(&text)) {
                break;
            }
            text.push(c);
        }
        if !text.is_empty() {
            out.push(Span::styled(text, s.style));
        }
        break;
    }
    out
}

/// Cuts `spans` to at most `width` cells and ends them with an ellipsis, in
/// the style of the last kept span, when cut. Spans that fit stay as they are.
pub(super) fn fit_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    if spans_width(&spans) <= width || width == 0 {
        return if width == 0 { Vec::new() } else { spans };
    }
    let mut out = cut_spans(spans, width - 1);
    let style = out.last().map(|s| s.style).unwrap_or_default();
    out.push(Span::styled("…", style));
    out
}

/// Pads `spans` on the left so they sit in the middle of `width` cells. A
/// line wider than `width` stays at the left edge and is cut at the right.
pub(super) fn centered(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let pad = usize::from(width).saturating_sub(spans_width(&spans)) / 2;
    let mut line = vec![Span::raw(" ".repeat(pad))];
    line.extend(spans);
    Line::from(line)
}

pub(super) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn span(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}

pub(super) fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// One row line: `left` is cut to fit, `right` stays whole at the edge.
pub(super) fn row_line(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>) -> Line<'static> {
    let width = usize::from(width);
    let right_w = spans_width(&right);
    let budget = width.saturating_sub(right_w + 1);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for s in left {
        let room = budget.saturating_sub(used);
        let text = fit(&s.content, room);
        used += cells(&text);
        spans.push(Span::styled(text, s.style));
    }
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w))));
    spans.extend(right);
    Line::from(spans)
}
