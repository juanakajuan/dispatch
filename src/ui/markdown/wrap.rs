//! Word-wraps styled spans to a width in cells.

use crate::ui::text::{cells, char_cells};
use ratatui::style::Style;
use ratatui::text::Span;

struct Word {
    space_before: bool,
    space_style: Style,
    parts: Vec<(String, Style)>,
}

fn words(spans: &[Span<'static>]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cur: Option<Word> = None;
    let mut space = false;
    let mut space_style = Style::default();
    for s in spans {
        for (i, seg) in s.content.split(char::is_whitespace).enumerate() {
            if i > 0 {
                out.extend(cur.take());
                space = true;
                space_style = s.style;
            }
            if seg.is_empty() {
                continue;
            }
            cur.get_or_insert_with(|| Word { space_before: space, space_style, parts: Vec::new() })
                .parts
                .push((seg.to_string(), s.style));
            space = false;
        }
    }
    out.extend(cur);
    out
}

pub(super) fn push_text(line: &mut Vec<Span<'static>>, text: &str, style: Style) {
    match line.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(text),
        _ => line.push(Span::styled(text.to_string(), style)),
    }
}

/// Word-wraps styled spans to `width` cells. Each word keeps the style of the
/// span it came from, and a word wider than a whole line is split by cell. A
/// character wider than `width` is dropped, because it fits nowhere.
pub(super) fn wrap(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let mut lines = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for word in words(spans) {
        let w: usize = word.parts.iter().map(|(t, _)| cells(t)).sum();
        let mut gap = usize::from(word.space_before && used > 0);
        if used > 0 && used + gap + w > width {
            lines.push(std::mem::take(&mut cur));
            used = 0;
            gap = 0;
        }
        if gap == 1 {
            push_text(&mut cur, " ", word.space_style);
        }
        if w <= width {
            for (t, s) in &word.parts {
                push_text(&mut cur, t, *s);
            }
            used += gap + w;
            continue;
        }
        for (t, s) in &word.parts {
            for c in t.chars() {
                let cw = char_cells(c);
                if cw > width {
                    continue;
                }
                if used + cw > width {
                    lines.push(std::mem::take(&mut cur));
                    used = 0;
                }
                push_text(&mut cur, c.encode_utf8(&mut [0; 4]), *s);
                used += cw;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Cuts a plain `text` into chunks of at most `width` cells. A character
/// wider than `width` is dropped, because it fits nowhere. Empty text gives
/// one empty chunk, so a blank code line keeps its row.
pub(super) fn chunks(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cw = char_cells(c);
        if cw > width {
            continue;
        }
        if used + cw > width {
            out.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(c);
        used += cw;
    }
    out.push(cur);
    out
}
