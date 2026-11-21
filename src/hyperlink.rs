//! Terminal hyperlinks (OSC 8) over cells that ratatui already drew.
//!
//! ratatui has no hyperlink type, and an escape sequence inside a `Span`
//! counts as width, so the row breaks. The shell instead draws the frame as
//! usual, then writes each linked cell again with the same symbol and style,
//! inside an OSC 8 sequence. The cells keep their width, and a terminal
//! without OSC 8 ignores the sequence and shows the cell as before.

use crate::domain::PageUrl;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The bytes that link each rect of `links` to its page, with the cells of
/// `buf`. The cursor and its style are saved before and restored after, so
/// the next frame of ratatui and a visible text cursor do not move.
pub fn overlay(buf: &Buffer, links: &[(Rect, PageUrl)]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    if links.is_empty() {
        return Ok(out);
    }
    out.extend_from_slice(b"\x1b7");
    for (rect, url) in links {
        let rect = rect.intersection(buf.area);
        let cells: Vec<_> = rect.positions().filter_map(|p| buf.cell(p).map(|c| (p.x, p.y, c))).collect();
        if cells.is_empty() {
            continue;
        }
        out.extend_from_slice(format!("\x1b]8;;{}\x1b\\", url.as_str()).as_bytes());
        CrosstermBackend::new(&mut out).draw(cells.into_iter())?;
        out.extend_from_slice(b"\x1b]8;;\x1b\\");
    }
    out.extend_from_slice(b"\x1b8");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::overlay;
    use crate::domain::PageUrl;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    fn url(raw: &str) -> PageUrl {
        PageUrl::parse(raw).expect("url")
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("utf-8")
    }

    #[test]
    fn no_links_write_nothing() {
        let buf = Buffer::with_lines(["#3 title"]);
        assert!(overlay(&buf, &[]).expect("overlay").is_empty());
    }

    #[test]
    fn a_link_wraps_the_drawn_cells_and_puts_the_cursor_back() {
        let mut buf = Buffer::with_lines(["  #16 title"]);
        buf.set_style(Rect::new(2, 0, 3, 1), Style::default().fg(Color::Yellow));
        let out =
            text(&overlay(&buf, &[(Rect::new(2, 0, 3, 1), url("https://github.com/o/r/issues/16"))]).expect("ok"));
        assert!(out.starts_with("\x1b7"), "saves the cursor first: {out:?}");
        assert!(out.ends_with("\x1b]8;;\x1b\\\x1b8"), "closes the link, then restores the cursor: {out:?}");
        let open = out.find("\x1b]8;;https://github.com/o/r/issues/16\x1b\\").expect("opens the link");
        let close = out.rfind("\x1b]8;;\x1b\\").expect("closes the link");
        let inner = &out[open..close];
        assert!(inner.contains("\x1b[1;3H"), "moves to column 3 of row 1: {inner:?}");
        assert!(inner.contains("\x1b[38;5;3;49m"), "keeps the drawn color: {inner:?}");
        assert!(inner.contains("#16"), "writes the drawn symbols again: {inner:?}");
        assert!(!inner.contains("title"), "only the linked cells: {inner:?}");
    }

    #[test]
    fn a_link_outside_the_buffer_writes_no_link() {
        let buf = Buffer::with_lines(["#3"]);
        let out = text(&overlay(&buf, &[(Rect::new(5, 4, 2, 1), url("https://github.com/o/r/issues/3"))]).expect("ok"));
        assert!(!out.contains("]8;;https"), "{out:?}");
    }
}
