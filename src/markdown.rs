//! Renders an issue body's Markdown into styled, word-wrapped terminal lines.

use crate::ui::{cells, palette};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

const MAX_LINES: usize = 300;
const GUTTER: &str = "│ ";
const CELL_SEPARATOR: &str = " │ ";

pub fn render(body: &str, width: u16) -> Vec<Line<'static>> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = Renderer::new(usize::from(width).max(1));
    for event in Parser::new_ext(body, options) {
        r.event(event);
        if r.out.len() > MAX_LINES {
            break;
        }
    }
    r.flush();
    let mut out = r.out;
    out.truncate(MAX_LINES);
    while out.last().is_some_and(is_blank) {
        out.pop();
    }
    out
}

enum Container {
    Quote { started: bool },
    List { next: Option<u64> },
    Item { marker: Option<String>, indent: usize },
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    row: Vec<Vec<Span<'static>>>,
    header_rows: usize,
}

struct Renderer {
    width: usize,
    out: Vec<Line<'static>>,
    styles: Vec<Style>,
    containers: Vec<Container>,
    pending: Vec<Span<'static>>,
    code: Option<String>,
    link: Option<(String, String)>,
    image: Option<String>,
    table: Option<Table>,
    gap: bool,
}

impl Renderer {
    fn new(width: usize) -> Self {
        Self {
            width,
            out: Vec::new(),
            styles: Vec::new(),
            containers: Vec::new(),
            pending: Vec::new(),
            code: None,
            link: None,
            image: None,
            table: None,
            gap: false,
        }
    }

    fn style(&self) -> Style {
        self.styles.iter().fold(palette::PLAIN, |acc, s| acc.patch(*s))
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                if let Some(alt) = &mut self.image {
                    alt.push_str(&t);
                } else {
                    self.note_link_text(&t);
                    self.pending.push(Span::styled(t.to_string(), self.style().patch(palette::CODE)));
                }
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                let avail = self.avail();
                self.emit(vec![vec![Span::styled("─".repeat(avail), palette::DIM)]]);
                self.gap = true;
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { Span::styled("[x] ", palette::PASS) } else { Span::styled("[ ] ", palette::DIM) };
                self.pending.push(marker);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { .. } => {
                self.flush();
                self.styles.push(palette::HEADING);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container::Quote { started: false });
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(first) => {
                self.flush();
                self.containers.push(Container::List { next: first });
            }
            Tag::Item => {
                self.flush();
                let marker = match self.containers.last_mut() {
                    Some(Container::List { next: Some(n) }) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                let indent = cells(&marker);
                self.containers.push(Container::Item { marker: Some(marker), indent });
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table::default());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row.clear();
                }
            }
            Tag::TableCell => self.pending.clear(),
            Tag::Emphasis => self.styles.push(palette::ITALIC),
            Tag::Strong => self.styles.push(palette::BOLD),
            Tag::Strikethrough => self.styles.push(palette::STRIKE),
            Tag::Link { dest_url, .. } => {
                self.styles.push(palette::LINK);
                self.link = Some((dest_url.to_string(), String::new()));
            }
            Tag::Image { .. } => self.image = Some(String::new()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                let lines = code
                    .trim_end_matches('\n')
                    .split('\n')
                    .map(|l| vec![Span::styled(GUTTER, palette::DIM), Span::styled(l.trim_end().to_string(), palette::PLAIN)])
                    .collect();
                self.emit(lines);
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.containers.pop();
                self.gap = !self.containers.iter().any(|c| matches!(c, Container::Item { .. }));
            }
            TagEnd::Item => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.pending);
                if let Some(t) = &mut self.table {
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.rows.push(std::mem::take(&mut t.row));
                    t.header_rows += 1;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    t.rows.push(std::mem::take(&mut t.row));
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.emit_table(&t);
                }
                self.gap = true;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some((url, text)) = self.link.take()
                    && text != url
                {
                    self.pending.push(Span::styled(format!(" ({url})"), palette::DIM));
                }
            }
            TagEnd::Image => {
                if let Some(alt) = self.image.take() {
                    let label = if alt.is_empty() { "[image]".to_string() } else { format!("[image: {alt}]") };
                    self.pending.push(Span::styled(label, palette::DIM));
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, t: &str) {
        if let Some(alt) = &mut self.image {
            alt.push_str(t);
        } else if let Some(code) = &mut self.code {
            code.push_str(t);
        } else {
            self.note_link_text(t);
            self.pending.push(Span::styled(t.to_string(), self.style()));
        }
    }

    fn note_link_text(&mut self, t: &str) {
        if let Some((_, text)) = &mut self.link {
            text.push_str(t);
        }
    }

    fn prefix(&mut self, first: bool) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        for c in &mut self.containers {
            match c {
                Container::Quote { started } => {
                    *started = true;
                    out.push(Span::styled(GUTTER, palette::DIM));
                }
                Container::List { .. } => {}
                Container::Item { marker, indent } => match first.then(|| marker.take()).flatten() {
                    Some(m) => out.push(Span::styled(m, palette::DIM)),
                    None => out.push(Span::raw(" ".repeat(*indent))),
                },
            }
        }
        out
    }

    fn avail(&self) -> usize {
        let used: usize = self
            .containers
            .iter()
            .map(|c| match c {
                Container::Quote { .. } => cells(GUTTER),
                Container::List { .. } => 0,
                Container::Item { indent, .. } => *indent,
            })
            .sum();
        self.width.saturating_sub(used).max(1)
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.pending);
        let avail = self.avail();
        self.emit(wrap(&spans, avail));
    }

    fn emit(&mut self, lines: Vec<Vec<Span<'static>>>) {
        if lines.is_empty() {
            return;
        }
        if self.gap && !self.out.is_empty() && !self.out.last().is_some_and(is_blank) {
            let gutter: Vec<Span<'static>> = self.containers.iter().filter(|c| matches!(c, Container::Quote { started: true })).map(|_| Span::styled("│", palette::DIM)).collect();
            self.out.push(Line::from(gutter));
        }
        self.gap = false;
        for (i, line) in lines.into_iter().enumerate() {
            let mut spans = self.prefix(i == 0);
            spans.extend(line);
            self.out.push(Line::from(spans));
        }
    }

    fn emit_table(&mut self, t: &Table) {
        let cols = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let cell_width = |c: &Vec<Span<'static>>| c.iter().map(Span::width).sum::<usize>();
        let widths: Vec<usize> = (0..cols).map(|i| t.rows.iter().filter_map(|r| r.get(i)).map(cell_width).max().unwrap_or(0)).collect();
        let avail = self.avail();
        let total = widths.iter().sum::<usize>() + cells(CELL_SEPARATOR) * (cols - 1);
        let sep = || Span::styled(CELL_SEPARATOR, palette::DIM);
        let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
        for (ri, row) in t.rows.iter().enumerate() {
            let header = ri < t.header_rows;
            let styled = |c: &Vec<Span<'static>>| -> Vec<Span<'static>> {
                c.iter().map(|s| if header { Span::styled(s.content.to_string(), s.style.patch(palette::BOLD)) } else { s.clone() }).collect()
            };
            if total <= avail {
                let mut line = Vec::new();
                for (ci, w) in widths.iter().enumerate() {
                    if ci > 0 {
                        line.push(sep());
                    }
                    let cell = row.get(ci).map(styled).unwrap_or_default();
                    let pad = w.saturating_sub(cell.iter().map(Span::width).sum::<usize>());
                    line.extend(cell);
                    if ci + 1 < cols && pad > 0 {
                        line.push(Span::raw(" ".repeat(pad)));
                    }
                }
                lines.push(line);
                if header && ri + 1 == t.header_rows {
                    let rule = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─");
                    lines.push(vec![Span::styled(rule, palette::DIM)]);
                }
            } else {
                let mut line = Vec::new();
                for (ci, cell) in row.iter().enumerate() {
                    if ci > 0 {
                        line.push(sep());
                    }
                    line.extend(styled(cell));
                }
                lines.extend(wrap(&line, avail));
            }
        }
        self.emit(lines);
    }
}

fn is_blank(line: &Line<'static>) -> bool {
    line.spans.iter().all(|s| matches!(s.content.trim(), "" | "│"))
}

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
        for (i, seg) in s.content.split(' ').enumerate() {
            if i > 0 {
                out.extend(cur.take());
                space = true;
                space_style = s.style;
            }
            if seg.is_empty() {
                continue;
            }
            cur.get_or_insert_with(|| Word { space_before: space, space_style, parts: Vec::new() }).parts.push((seg.to_string(), s.style));
            space = false;
        }
    }
    out.extend(cur);
    out
}

fn push_text(line: &mut Vec<Span<'static>>, text: &str, style: Style) {
    match line.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push_str(text),
        _ => line.push(Span::styled(text.to_string(), style)),
    }
}

/// Word-wraps styled spans to `width` cells. Each word keeps the style of the
/// span it came from, and a word wider than a whole line is split by cell.
fn wrap(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
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
                let mut buf = [0; 4];
                let ch = c.encode_utf8(&mut buf);
                let cw = cells(ch);
                if used + cw > width && used > 0 {
                    lines.push(std::mem::take(&mut cur));
                    used = 0;
                }
                push_text(&mut cur, ch, *s);
                used += cw;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    fn line_text(l: &Line<'_>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn texts(body: &str, width: u16) -> Vec<String> {
        render(body, width).iter().map(line_text).collect()
    }

    fn span_of<'a>(lines: &'a [Line<'static>], content: &str) -> &'a Span<'static> {
        lines.iter().flat_map(|l| l.spans.iter()).find(|s| s.content == content).unwrap_or_else(|| panic!("no span {content:?} in {lines:?}"))
    }

    #[test]
    fn inline_emphasis_code_and_strike_are_styled_without_markers() {
        let lines = render("a **bold** and *it* and ~~gone~~ and `code`", 80);
        assert_eq!(line_text(&lines[0]), "a bold and it and gone and code");
        assert!(span_of(&lines, "bold").style.add_modifier.contains(Modifier::BOLD));
        assert!(span_of(&lines, "it").style.add_modifier.contains(Modifier::ITALIC));
        assert!(span_of(&lines, "gone").style.add_modifier.contains(Modifier::CROSSED_OUT));
        assert_eq!(span_of(&lines, "code").style.fg, palette::CODE.fg);
    }

    #[test]
    fn nested_emphasis_combines_styles() {
        let lines = render("**bold _both_**", 80);
        let both = span_of(&lines, "both").style.add_modifier;
        assert!(both.contains(Modifier::BOLD) && both.contains(Modifier::ITALIC));
    }

    #[test]
    fn link_shows_text_then_dim_url_unless_they_match() {
        let lines = render("see [the docs](https://x.dev/a) now", 80);
        assert_eq!(line_text(&lines[0]), "see the docs (https://x.dev/a) now");
        assert_eq!(span_of(&lines, "the docs").style.fg, palette::LINK.fg);
        assert_eq!(span_of(&lines, " (https://x.dev/a)").style, palette::DIM);
        assert_eq!(texts("<https://x.dev>", 80), ["https://x.dev"]);
    }

    #[test]
    fn headings_are_bold_without_hash_marks() {
        let lines = render("# Title\n\n### Sub\n\ntext", 80);
        assert_eq!(lines.iter().map(line_text).collect::<Vec<_>>(), ["Title", "", "Sub", "", "text"]);
        assert!(span_of(&lines, "Title").style.add_modifier.contains(Modifier::BOLD));
        assert!(span_of(&lines, "Sub").style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn bullet_and_ordered_lists_nest_with_indent() {
        assert_eq!(texts("- a\n  - b\n- c", 80), ["• a", "  • b", "• c"]);
        assert_eq!(texts("1. one\n2. two\n   - sub", 80), ["1. one", "2. two", "   • sub"]);
        assert_eq!(texts("3. x\n4. y", 80), ["3. x", "4. y"]);
    }

    #[test]
    fn wrapping_keeps_styles_and_hangs_under_the_list_text() {
        let lines = render("- alpha **beta gamma** delta epsilon", 14);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, ["• alpha beta", "  gamma delta", "  epsilon"]);
        assert!(got.iter().all(|l| cells(l) <= 14));
        assert!(span_of(&lines, "beta").style.add_modifier.contains(Modifier::BOLD));
        assert!(span_of(&lines, "gamma").style.add_modifier.contains(Modifier::BOLD));
        assert!(!span_of(&lines, " delta").style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_word_longer_than_the_line_is_split() {
        assert_eq!(texts("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn task_list_items_show_checkboxes() {
        let lines = render("- [ ] todo\n- [x] done", 80);
        assert_eq!(lines.iter().map(line_text).collect::<Vec<_>>(), ["• [ ] todo", "• [x] done"]);
        assert_eq!(span_of(&lines, "[x] ").style, palette::PASS);
    }

    #[test]
    fn block_quotes_get_a_dim_gutter_and_wrap_inside_it() {
        let lines = render("> quoted words that wrap around", 14);
        let got: Vec<String> = lines.iter().map(line_text).collect();
        assert_eq!(got, ["│ quoted words", "│ that wrap", "│ around"]);
        assert_eq!(lines[0].spans[0].style, palette::DIM);
    }

    #[test]
    fn fenced_and_indented_code_keep_a_dim_gutter() {
        let lines = render("```rust\nfn main() {}\n\nx\n```\n\n    indented", 80);
        assert_eq!(lines.iter().map(line_text).collect::<Vec<_>>(), ["│ fn main() {}", "│ ", "│ x", "", "│ indented"]);
        assert_eq!(lines[0].spans[0].style, palette::DIM);
    }

    #[test]
    fn horizontal_rule_fills_the_width_dimly() {
        let lines = render("a\n\n---\n\nb", 10);
        assert_eq!(line_text(&lines[2]), "─".repeat(10));
        assert_eq!(lines[2].spans[0].style, palette::DIM);
    }

    #[test]
    fn table_is_aligned_when_it_fits() {
        let got = texts("| name | n |\n|---|---|\n| apple | 1 |\n| fig | 22 |", 40);
        assert_eq!(got, ["name  │ n", "──────┼───", "apple │ 1", "fig   │ 22"]);
    }

    #[test]
    fn table_falls_back_to_one_wrapped_line_per_row_when_narrow() {
        let got = texts("| name | note |\n|---|---|\n| apple | a long note here |", 12);
        assert_eq!(got, ["name │ note", "apple │ a", "long note", "here"]);
        assert!(got.iter().all(|l| cells(l) <= 12));
    }

    #[test]
    fn raw_html_never_appears() {
        let got = texts("before <b>bold</b> after\n\n<div align=\"center\">\n<img src=\"x.png\">\n</div>\n\n<!-- comment -->\n\nend", 80);
        assert_eq!(got, ["before bold after", "", "end"]);
    }

    #[test]
    fn images_show_dim_alt_text() {
        let lines = render("![a cat](http://x/c.png)", 80);
        assert_eq!(line_text(&lines[0]), "[image: a cat]");
        assert_eq!(lines[0].spans[0].style, palette::DIM);
    }

    #[test]
    fn blocks_are_separated_by_exactly_one_blank_line_with_none_at_the_edges() {
        let got = texts("\n\n# H\n\n\n\npara\n\n- a\n- b\n\n> q\n\n", 80);
        assert_eq!(got, ["H", "", "para", "", "• a", "• b", "", "│ q"]);
    }

    #[test]
    fn output_stops_at_300_lines() {
        let body = (0..1000).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n\n");
        let n = render(&body, 80).len();
        assert!((299..=300).contains(&n), "{n} lines");
    }

    #[test]
    fn a_github_issue_template_shows_no_markdown_markers() {
        let body = "## Describe the bug\r\n\r\nThe **retry queue** drops jobs. See [the log](https://ci.example/run/1).\r\n\r\n### Steps\r\n\r\n1. Run `dispatch`\r\n2. Press `s`\r\n\r\n```\r\nerror: boom\r\n```\r\n\r\n- [ ] reproduced\r\n- [x] has logs\r\n\r\n<!-- hidden -->\r\n";
        let got = texts(body, 60).join("\n");
        for marker in ["**", "`", "- [", "](", "##", "<!--"] {
            assert!(!got.contains(marker), "{marker:?} leaked into\n{got}");
        }
        assert!(got.contains("Describe the bug") && got.contains("retry queue") && got.contains("│ error: boom") && got.contains("[x] has logs"));
    }
}
