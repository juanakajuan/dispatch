//! Block renderer: Markdown into styled lines, wrapped to a width.

use super::options;
use super::table::{self, Table};
use super::wrap::{chunks, wrap};
use crate::ui::palette;
use crate::ui::text::{cells, cut_spans};
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

const GUTTER: &str = "│ ";

/// Renders `text` as lines of at most `width` cells, with one blank line
/// between blocks and none at either end.
pub(super) fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut r = Renderer::new(width.max(1));
    for event in Parser::new_ext(text, options()) {
        r.event(event);
    }
    r.flush();
    r.out
}

/// Whether a line starts a block, so a list item puts its marker on it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    First,
    Continued,
}

enum Container {
    Quote { started: bool },
    List { next: Option<u64> },
    Item { marker: Option<String>, indent: usize },
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
                let marker =
                    if checked { Span::styled("[x] ", palette::PASS) } else { Span::styled("[ ] ", palette::DIM) };
                self.pending.push(marker);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.styles.push(if level == HeadingLevel::H1 { palette::HEADING_1 } else { palette::HEADING });
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
                self.end_code();
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
                    let lines = table::layout(&t, self.avail());
                    self.emit(lines);
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

    /// A code block: each line behind a dim gutter, cut by cell and never
    /// clipped.
    fn end_code(&mut self) {
        let code = self.code.take().unwrap_or_default().replace('\t', "    ");
        let room = self.avail().saturating_sub(cells(GUTTER)).max(1);
        let lines = code
            .trim_end_matches('\n')
            .split('\n')
            .flat_map(|l| chunks(l.trim_end(), room))
            .map(|chunk| vec![Span::styled(GUTTER, palette::DIM), Span::styled(chunk, palette::CODE)])
            .collect();
        self.emit(lines);
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

    fn prefix(&mut self, row: Row) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        for c in &mut self.containers {
            match c {
                Container::Quote { started } => {
                    *started = true;
                    out.push(Span::styled(GUTTER, palette::DIM));
                }
                Container::List { .. } => {}
                Container::Item { marker, indent } => match (row == Row::First).then(|| marker.take()).flatten() {
                    Some(m) => out.push(Span::styled(m, palette::DIM)),
                    None => out.push(Span::raw(" ".repeat(*indent))),
                },
            }
        }
        out
    }

    /// Cells left for text inside the open quotes and list items.
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

    /// Adds `lines` behind the prefix of the open containers. The blank line
    /// between blocks is added here, so none lands at either end. A line that
    /// the prefix leaves too wide, on a very narrow width, is cut.
    fn emit(&mut self, lines: Vec<Vec<Span<'static>>>) {
        if lines.is_empty() {
            return;
        }
        if self.gap && !self.out.is_empty() {
            let gutter = self
                .containers
                .iter()
                .filter(|c| matches!(c, Container::Quote { started: true }))
                .map(|_| Span::styled("│", palette::DIM))
                .collect::<Vec<_>>();
            self.out.push(Line::from(cut_spans(gutter, self.width)));
        }
        self.gap = false;
        for (i, line) in lines.into_iter().enumerate() {
            let mut spans = self.prefix(if i == 0 { Row::First } else { Row::Continued });
            spans.extend(line);
            self.out.push(Line::from(cut_spans(spans, self.width)));
        }
    }
}
