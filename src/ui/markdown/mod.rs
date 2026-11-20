//! Markdown as styled terminal text. Pure: text in, spans out.
//! `block` fills the summary overlay, `inline` flattens one message to a
//! single line for a row, and `title` does the same for a GitHub title,
//! which is inline-only there.

mod block;
mod table;
mod wrap;

use super::palette;
use super::text::one_line;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// `text` as lines of at most `width` cells: headings, lists, quotes, code
/// blocks, tables and rules, with one blank line between blocks.
pub(super) fn block(text: &str, width: usize) -> Vec<Line<'static>> {
    block::render(text, width)
}

/// `text` as one line of spans: block marks dropped, blocks and breaks joined
/// by one space, whitespace collapsed. Each span's style is `base` patched
/// with its Markdown style.
pub(super) fn inline(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut flat = Flat { base, styles: Vec::new(), out: Vec::new(), space: false };
    for event in Parser::new_ext(text, options()) {
        flat.event(event);
    }
    flat.out
}

/// A GitHub issue or PR title. When it parses as one paragraph it is
/// `inline`. Otherwise (`1. foo`, `# foo`, `- foo`, `---`, or one with HTML such as
/// `Use <T>`) GitHub shows it verbatim, so it stays one raw span and no text is lost.
pub(super) fn title(text: &str, base: Style) -> Vec<Span<'static>> {
    let text = one_line(text);
    if is_one_paragraph(&text) {
        return inline(&text, base);
    }
    vec![Span::styled(text, base)]
}

fn is_one_paragraph(text: &str) -> bool {
    let mut depth = 0usize;
    let mut blocks = 0usize;
    let mut first_is_paragraph = false;
    for event in Parser::new_ext(text, options()) {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    blocks += 1;
                    first_is_paragraph |= blocks == 1 && matches!(tag, Tag::Paragraph);
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Html(_) | Event::InlineHtml(_) => return false,
            _ if depth == 0 => blocks += 1,
            _ => {}
        }
    }
    blocks == 1 && first_is_paragraph
}

struct Flat {
    base: Style,
    styles: Vec<Style>,
    out: Vec<Span<'static>>,
    /// A space waits before the next word. It lands only if text follows.
    space: bool,
}

impl Flat {
    fn style(&self) -> Style {
        self.styles.iter().fold(self.base, |acc, s| acc.patch(*s))
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(&tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.words(&t, self.style()),
            Event::Code(t) => self.words(&t, self.style().patch(palette::CODE)),
            Event::SoftBreak | Event::HardBreak | Event::Rule => self.space = true,
            Event::TaskListMarker(checked) => self.words(if checked { "[x]" } else { "[ ]" }, self.style()),
            _ => {}
        }
    }

    fn start(&mut self, tag: &Tag<'_>) {
        match tag {
            Tag::Emphasis => self.styles.push(palette::ITALIC),
            Tag::Strong => self.styles.push(palette::BOLD),
            Tag::Strikethrough => self.styles.push(palette::STRIKE),
            Tag::Link { .. } => self.styles.push(palette::LINK),
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::List(_)
            | Tag::Item
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell => self.space = true,
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell => self.space = true,
            _ => {}
        }
    }

    /// Adds the words of `text`, with the whitespace between them collapsed
    /// to one space. A space between two words of one style takes that style,
    /// so the run stays one span.
    fn words(&mut self, text: &str, style: Style) {
        if text.starts_with(char::is_whitespace) {
            self.space = true;
        }
        for (i, word) in text.split_whitespace().enumerate() {
            if i > 0 {
                self.space = true;
            }
            if self.space && !self.out.is_empty() {
                let same_run = self.out.last().is_some_and(|s| s.style == style);
                self.push(" ", if same_run { style } else { self.base });
            }
            self.space = false;
            self.push(word, style);
        }
        if text.ends_with(char::is_whitespace) {
            self.space = true;
        }
    }

    fn push(&mut self, text: &str, style: Style) {
        match self.out.last_mut() {
            Some(last) if last.style == style => last.content.to_mut().push_str(text),
            _ => self.out.push(Span::styled(text.to_string(), style)),
        }
    }
}
