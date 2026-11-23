use super::*;
use crate::ui::markdown::{block, inline, title};
use crate::ui::text::fit_spans;
use ratatui::text::{Line, Span};

fn text_of(spans: &[Span<'_>]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

fn texts(md: &str, width: usize) -> Vec<String> {
    block(md, width).iter().map(|l| text_of(&l.spans)).collect()
}

fn span_of<'a>(lines: &'a [Line<'static>], content: &str) -> &'a Span<'static> {
    lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .find(|s| s.content == content)
        .unwrap_or_else(|| panic!("no span {content:?} in {lines:?}"))
}

fn has(style: Style, m: Modifier) -> bool {
    style.add_modifier.contains(m)
}

#[test]
fn headings_are_bold_and_blocks_have_one_blank_line_between() {
    let lines = block("# Title\n\n## Sub\n\ntext", 40);
    let got: Vec<String> = lines.iter().map(|l| text_of(&l.spans)).collect();
    assert_eq!(got, ["Title", "", "Sub", "", "text"]);
    assert!(has(span_of(&lines, "Title").style, Modifier::BOLD));
    assert!(has(span_of(&lines, "Sub").style, Modifier::BOLD));
    assert!(!has(span_of(&lines, "text").style, Modifier::BOLD));
}

#[test]
fn inline_styles_show_without_their_marks() {
    let lines = block("a **bold** and *it* and ~~gone~~ and `code`", 80);
    assert_eq!(text_of(&lines[0].spans), "a bold and it and gone and code");
    assert!(has(span_of(&lines, "bold").style, Modifier::BOLD));
    assert!(has(span_of(&lines, "it").style, Modifier::ITALIC));
    assert!(has(span_of(&lines, "gone").style, Modifier::CROSSED_OUT));
    assert_eq!(span_of(&lines, "code").style.fg, Some(palette::ORANGE));
}

#[test]
fn a_fenced_code_block_wraps_by_cell_and_loses_nothing() {
    let got = texts("```rust\nlet answer = 42;\n\nx\n```", 10);
    assert_eq!(got, ["│ let answ", "│ er = 42;", "│ ", "│ x"]);
    assert_eq!(texts("    indented", 40), ["│ indented"]);
}

#[test]
fn nested_lists_hang_under_the_item_text() {
    assert_eq!(texts("- a\n  - b\n- c", 40), ["• a", "  • b", "• c"]);
    assert_eq!(texts("1. one\n2. two\n   - sub", 40), ["1. one", "2. two", "   • sub"]);
    assert_eq!(texts("- alpha beta gamma delta epsilon", 14), ["• alpha beta", "  gamma delta", "  epsilon"]);
}

#[test]
fn task_list_items_show_checkboxes() {
    assert_eq!(texts("- [ ] todo\n- [x] done", 40), ["• [ ] todo", "• [x] done"]);
}

#[test]
fn a_blockquote_has_a_dim_gutter_on_every_wrapped_line() {
    let lines = block("> quoted words that wrap around", 14);
    let got: Vec<String> = lines.iter().map(|l| text_of(&l.spans)).collect();
    assert_eq!(got, ["│ quoted words", "│ that wrap", "│ around"]);
    assert_eq!(lines[0].spans[0].style, palette::DIM);
}

#[test]
fn a_table_shrinks_its_columns_and_cuts_cells_to_fit() {
    assert_eq!(texts("| name | n |\n|---|---|\n| apple | 1 |", 40), ["name  │ n", "──────┼──", "apple │ 1"]);
    let got = texts("| name | note |\n|---|---|\n| apple | a long note here |", 14);
    assert_eq!(got, ["name  │ note", "──────┼───────", "apple │ a lon…"]);
    assert!(got.iter().all(|l| cells(l) <= 14));
}

#[test]
fn a_link_shows_its_text_then_a_dim_url_unless_they_match() {
    let lines = block("see [the docs](https://x.dev/a) now", 80);
    assert_eq!(text_of(&lines[0].spans), "see the docs (https://x.dev/a) now");
    assert!(has(span_of(&lines, "the docs").style, Modifier::UNDERLINED));
    assert_eq!(span_of(&lines, " (https://x.dev/a)").style, palette::DIM);
    assert_eq!(texts("<https://x.dev>", 80), ["https://x.dev"]);
}

#[test]
fn images_rules_and_html() {
    assert_eq!(texts("![a cat](http://x/c.png)", 80), ["[image: a cat]"]);
    assert_eq!(texts("a\n\n---\n\nb", 6), ["a", "", "──────", "", "b"]);
    assert_eq!(texts("before <b>bold</b> after", 80), ["before bold after"]);
}

#[test]
fn no_line_is_wider_than_the_width_whatever_it_holds() {
    let md = "# 見出し 🎉\n\n日本語のとても長い文章がここにあります 🎉🎉🎉\n\n- 項目 **太字**\n  - 入れ子 🎉\n\n> 引用 🎉🎉\n\n```\n🎉🎉🎉🎉🎉\n```\n\n| 名前 | 値 |\n|---|---|\n| りんご | 🎉🎉🎉 |\n\n---\n";
    for width in [1, 2, 3, 5, 8, 20, 80] {
        for line in block(md, width) {
            let w: usize = line.spans.iter().map(Span::width).sum();
            assert!(w <= width, "width {width}: {w} cells in {line:?}");
        }
    }
    assert!(texts(md, 80).iter().any(|l| l.contains("日本語のとても長い文章がここにあります")));
}

#[test]
fn empty_text_gives_no_lines() {
    assert!(block("", 10).is_empty());
    assert!(block("\n\n", 10).is_empty());
}

#[test]
fn inline_flattens_blocks_to_one_styled_line() {
    let spans = inline("## Heading\n\n- **bold** and `code`", palette::DIM);
    assert_eq!(text_of(&spans), "Heading bold and code");
    assert_eq!(spans[0].style, palette::DIM);
    let style_of = |s: &str| spans.iter().find(|x| x.content == s).map(|x| x.style).expect("span");
    assert!(has(style_of("bold"), Modifier::BOLD));
    assert_eq!(style_of("bold").fg, palette::DIM.fg, "the base color is kept");
    assert_eq!(style_of("code").fg, Some(palette::ORANGE));
}

#[test]
fn inline_joins_lines_and_trims() {
    let spans = inline("  one\ntwo  \n\n> three\n\n1. four\n2. five\n\n```\nsix\n```\n", palette::DIM);
    assert_eq!(text_of(&spans), "one two three four five six");
    assert!(spans.iter().all(|s| s.style == palette::DIM), "plain text merges into one span");
}

#[test]
fn a_title_is_inline_markdown() {
    let spans = title("Fix `foo` in **bar**", Style::default());
    assert_eq!(text_of(&spans), "Fix foo in bar");
    let foo = spans.iter().find(|s| s.content == "foo").expect("foo");
    assert_eq!(foo.style.fg, Some(palette::ORANGE));
    let bar = spans.iter().find(|s| s.content == "bar").expect("bar");
    assert!(has(bar.style, Modifier::BOLD));
}

#[test]
fn a_title_that_is_not_one_paragraph_stays_raw() {
    for raw in
        ["1. Not a list", "# Not a heading", "- not a bullet", "> not a quote", "---", "<b>x</b>", "Use <T> here"]
    {
        let spans = title(raw, palette::DIM);
        assert_eq!(spans.len(), 1, "{raw}");
        assert_eq!(spans[0].content, raw);
        assert_eq!(spans[0].style, palette::DIM);
    }
    assert_eq!(text_of(&title("two\nlines", Style::default())), "two lines");
}

#[test]
fn fit_spans_cuts_across_spans_with_one_ellipsis() {
    let spans = vec![Span::styled("abc", palette::DIM), Span::styled("def", palette::PASS)];
    assert_eq!(fit_spans(spans.clone(), 6), spans, "fits: unchanged");
    let cut = fit_spans(spans.clone(), 5);
    assert_eq!(text_of(&cut), "abcd…");
    assert_eq!(cut.last().expect("last").style, palette::PASS);
    let cut = fit_spans(spans.clone(), 3);
    assert_eq!(text_of(&cut), "ab…");
    assert_eq!(cut.last().expect("last").style, palette::DIM);
    assert!(fit_spans(spans, 0).is_empty());
    let wide = fit_spans(vec![Span::raw("日本語")], 4);
    assert_eq!(text_of(&wide), "日…");
}

fn md_state(title: &str, said: &str) -> State {
    use crate::transcript::{Summary, SummaryKind, Transcript};
    let mut state = full_state();
    state.issues.items[0].title = title.to_string();
    let summary = Summary { kind: SummaryKind::Message, text: said.to_string() };
    state.transcripts.insert(issue_key(3), Transcript { said: Some(summary), active_at: jiff::Timestamp::UNIX_EPOCH });
    state
}

#[test]
fn the_summary_overlay_shows_rendered_markdown() {
    let mut state = md_state("Docs", "## Done\n\nFixed **the queue** with `retry`.\n\n- first\n- second");
    send(&mut state, KeyCode::Char('3'));
    send(&mut state, KeyCode::Char('i'));
    let screen = draw_into(&mut state, 60, 30);
    for want in ["Done", "Fixed the queue with retry.", "• first", "• second"] {
        assert!(screen.contains(want), "missing {want:?} in\n{screen}");
    }
    for raw in ["**", "`", "##", "- first"] {
        assert!(!screen.contains(raw), "{raw:?} leaked into\n{screen}");
    }
}

#[test]
fn a_markdown_title_and_summary_keep_the_columns_aligned() {
    let mut plain = md_state("Docs", "plain words");
    let mut fancy = md_state("Fix `foo` in **bar** now", "## Head\n\n- **bold** and `code`");
    let before = draw_into(&mut plain, 100, 20);
    let after = draw_into(&mut fancy, 100, 20);
    assert!(after.contains("#3 Fix foo in bar now"), "{after}");
    assert!(after.contains("Head bold and code"), "{after}");
    for raw in ["**", "`", "##"] {
        assert!(!after.contains(raw), "{raw:?} leaked into\n{after}");
    }
    let col = |screen: &str, summary: &str| {
        let line = screen.lines().find(|l| l.contains("#3 ")).expect("row");
        cells(&line[..line.find(summary).expect("summary")])
    };
    assert_eq!(col(&before, "plain words"), col(&after, "Head bold"), "the summary column does not move");
    let narrow = draw_into(&mut fancy, 50, 20);
    let row = narrow.lines().find(|l| l.contains("#3 ")).expect("row");
    assert_eq!(cells(row), 50);
    assert!(!row.contains('`') && !row.contains('*'), "{row}");
}
