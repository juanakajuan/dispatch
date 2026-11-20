use super::markdown;
use super::palette;
use super::text::{cells, fit, span};
use crate::app::keys::{Action, Key};
use crate::app::mouse::{HitMap, HitTarget};
use crate::app::{self, DiffBody, State};
use crate::config::Config;
use crate::config::settings::SETTINGS;
use crate::domain::SessionKey;
use crate::transcript::SummaryKind;
use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

/// The keybinds menu lines, made from `BINDINGS` and grouped by `KeyGroup`.
/// An entry line carries its binding's action; headings and blanks carry none.
pub(super) fn keybinds_lines() -> Vec<(Line<'static>, Option<Action>)> {
    let keys = |b: &app::keys::Binding| b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
    let key_width = app::keys::BINDINGS.iter().map(|b| cells(&keys(b))).max().unwrap_or(0);
    let mut lines = Vec::new();
    for group in app::keys::KeyGroup::ALL {
        if !lines.is_empty() {
            lines.push((Line::raw(""), None));
        }
        lines.push((Line::from(span(group.title(), palette::HEADING)), None));
        for b in app::keys::BINDINGS.iter().filter(|b| b.group == group) {
            let line = Line::from(vec![
                span(format!("  {:key_width$}  ", keys(b)), palette::ACCENT),
                span(b.desc, palette::PLAIN),
            ]);
            lines.push((line, Some(b.action)));
        }
    }
    lines
}

/// Draws the menu scrolled down by `scroll` lines, and returns how far it
/// can scroll at this size.
pub(super) fn draw_keybinds(frame: &mut Frame, screen: Rect, scroll: u16, hits: &mut HitMap) -> u16 {
    let entries = keybinds_lines();
    let wanted: u16 = entries.len().try_into().unwrap_or(u16::MAX);
    // The margin keeps the footer and its `esc to close` in view.
    let height = screen.height.saturating_sub(2).min(wanted.saturating_add(3));
    let inner = draw_box(frame, centered(screen, 64, height), " Keys ".to_string(), hits);
    let max_scroll = wanted.saturating_sub(inner.height);
    let scroll = scroll.min(max_scroll);
    for (row, (_, action)) in (0..).zip(&entries) {
        let Some(action) = action else { continue };
        let line = Rect::new(inner.x, (inner.y + row).saturating_sub(scroll), inner.width, 1).intersection(inner);
        if row >= scroll && !line.is_empty() {
            hits.targets.push((line, HitTarget::Binding(*action)));
        }
    }
    let lines: Vec<Line<'static>> = entries.into_iter().map(|(line, _)| line).collect();
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    max_scroll
}

pub(super) fn draw_settings(
    frame: &mut Frame,
    screen: Rect,
    state: &State,
    menu: &app::menus::SettingsMenu,
    hits: &mut HitMap,
) {
    let height = screen.height.min(SETTINGS.len().try_into().unwrap_or(4) + 8);
    let inner = draw_box(frame, centered(screen, 76, height), " Settings ".to_string(), hits);
    let default = Config::default();
    let name_width = SETTINGS.iter().map(|s| s.key().len()).max().unwrap_or(0);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (row, setting) in (0..).zip(SETTINGS) {
        let on = setting == menu.at;
        let line = Rect::new(inner.x, inner.y.saturating_add(row), inner.width, 1).intersection(inner);
        hits.targets.push((line, HitTarget::Setting(setting)));
        let room = usize::from(inner.width).saturating_sub(4 + name_width);
        let mut spans = vec![
            span(if on { "› " } else { "  " }, palette::ACCENT),
            span(format!("{:name_width$}  ", setting.key()), if on { palette::ACCENT } else { palette::PLAIN }),
        ];
        if let (Some(edit), true) = (&menu.edit, on) {
            let text: Vec<char> = edit.input.lines().join("").chars().collect();
            let shown: String = text[text.len().saturating_sub(room.saturating_sub(2))..].iter().collect();
            spans.push(span(format!("{shown}▏"), palette::ACTIVE));
        } else {
            let default_note = format!("   default: {}", setting.show(&default));
            let value = fit(&setting.show(&state.config), room.saturating_sub(cells(&default_note)).max(1));
            spans.push(span(value, if on { palette::ACTIVE } else { palette::PLAIN }));
            spans.push(span(default_note, palette::DIM));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    let error = menu.edit.as_ref().and_then(|e| e.error.as_ref());
    lines.push(match error {
        Some(e) => Line::from(span(fit(e, usize::from(inner.width)), palette::ERROR)),
        None => menu.at.applies_note().map_or_else(|| Line::raw(""), |note| Line::from(span(note, palette::DIM))),
    });
    lines.push(Line::from(span(
        if menu.edit.is_some() { "enter save · esc cancel" } else { "j/k move · enter change · esc close" },
        palette::DIM,
    )));
    lines.push(Line::from(span("Saved to config.json", palette::DIM)));
    frame.render_widget(Paragraph::new(lines), inner);
}

/// All of what the session's Claude last said, as Markdown wrapped to the box and scrolled down
/// by `scroll` lines. Returns how far it can scroll at this size.
pub(super) fn draw_summary(
    frame: &mut Frame,
    screen: Rect,
    state: &State,
    key: SessionKey,
    scroll: u16,
    hits: &mut HitMap,
) -> u16 {
    let said = state.transcripts.get(&key).and_then(|t| t.said.as_ref());
    let title = match said.map(|s| s.kind) {
        Some(SummaryKind::Recap) => format!(" {key} recap "),
        Some(SummaryKind::Message) | None => format!(" {key} last message "),
    };
    let width = screen.width.min(84);
    let text_width = usize::from(width.saturating_sub(6)).max(1);
    let lines = markdown::block(said.map_or("", |s| s.text.as_str()), text_width);
    let wanted: u16 = lines.len().try_into().unwrap_or(u16::MAX);
    // The margin keeps the footer and its `esc to close` in view.
    let height = screen.height.saturating_sub(2).min(wanted.saturating_add(3));
    let inner = draw_box(frame, centered(screen, width, height), title, hits);
    let max_scroll = wanted.saturating_sub(inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.min(max_scroll), 0)), inner);
    max_scroll
}

/// The session's worktree diff, one line per row and cut to the box, never
/// wrapped, scrolled down by `scroll` lines. Returns how far it can scroll at
/// this size.
pub(super) fn draw_diff(
    frame: &mut Frame,
    screen: Rect,
    key: SessionKey,
    scroll: u16,
    body: &DiffBody,
    hits: &mut HitMap,
) -> u16 {
    let width = screen.width.min(120);
    let text_width = usize::from(width.saturating_sub(6)).max(1);
    // A terminal cell has no tab stop, so a tab would shift the rest of the line.
    let line = |text: &str, style: Style| Line::from(span(fit(&text.replace('\t', "    "), text_width), style));
    let lines: Vec<Line<'static>> = match body {
        DiffBody::Reading => vec![line("Reading the diff…", palette::DIM)],
        DiffBody::Read(diff) => diff
            .untracked
            .iter()
            .map(|path| line(&format!("untracked: {path}"), palette::PLAIN))
            .chain(diff.patch.lines().map(|text| line(text, diff_style(text))))
            .collect(),
    };
    let wanted: u16 = lines.len().try_into().unwrap_or(u16::MAX);
    // The margin keeps the footer and its `esc to close` in view.
    let height = screen.height.saturating_sub(2).min(wanted.saturating_add(3));
    let inner = draw_box(frame, centered(screen, width, height), format!(" {key} diff "), hits);
    let max_scroll = wanted.saturating_sub(inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.min(max_scroll), 0)), inner);
    max_scroll
}

fn diff_style(text: &str) -> Style {
    if text.starts_with("diff --git") || text.starts_with("+++") || text.starts_with("---") {
        palette::HEADING
    } else if text.starts_with('+') {
        palette::PASS
    } else if text.starts_with('-') {
        palette::FAIL
    } else if text.starts_with("@@") {
        palette::ACCENT
    } else {
        palette::PLAIN
    }
}

/// A box of at most `width` by `height`, centered on the screen.
fn centered(screen: Rect, width: u16, height: u16) -> Rect {
    let (width, height) = (screen.width.min(width), screen.height.min(height));
    Rect::new(screen.x + (screen.width - width) / 2, screen.y + (screen.height - height) / 2, width, height)
}

/// Clears `area`, draws a titled overlay border with its close control, and
/// returns the area inside the padding.
fn draw_box(frame: &mut Frame, area: Rect, title: String, hits: &mut HitMap) -> Rect {
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(title, palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_close(frame, area, hits);
    inner
}

const CLOSE: &str = " esc ✕ ";

/// A close control in the top-right of an overlay's border: it presses esc.
fn draw_close(frame: &mut Frame, area: Rect, hits: &mut HitMap) {
    let w = u16::try_from(cells(CLOSE)).unwrap_or(u16::MAX);
    if area.width < w + 4 {
        return;
    }
    let rect = Rect::new(area.right() - w - 2, area.y, w, 1);
    frame.render_widget(Paragraph::new(span(CLOSE, palette::DIM)), rect);
    hits.targets.push((rect, HitTarget::Key(Key::Code(KeyCode::Esc))));
}
