use super::palette;
use super::text::{cells, fit, span};
use crate::app::{self, State};
use crate::config::{Config, SETTINGS};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

/// The keybinds menu lines, made from `BINDINGS` and grouped by `KeyGroup`.
pub(super) fn keybinds_lines() -> Vec<Line<'static>> {
    let keys = |b: &app::keys::Binding| b.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join(" / ");
    let key_width = app::keys::BINDINGS.iter().map(|b| cells(&keys(b))).max().unwrap_or(0);
    let mut lines = Vec::new();
    for group in app::keys::KeyGroup::ALL {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(span(group.title(), palette::HEADING)));
        for b in app::keys::BINDINGS.iter().filter(|b| b.group == group) {
            lines.push(Line::from(vec![
                span(format!("  {:key_width$}  ", keys(b)), palette::ACCENT),
                span(b.desc, palette::PLAIN),
            ]));
        }
    }
    lines
}

/// Draws the menu scrolled down by `scroll` lines, and returns how far it
/// can scroll at this size.
pub(super) fn draw_keybinds(frame: &mut Frame, screen: Rect, scroll: u16) -> u16 {
    let lines = keybinds_lines();
    let wanted: u16 = lines.len().try_into().unwrap_or(u16::MAX);
    let width = screen.width.min(64);
    // The margin keeps the footer and its `esc to close` in view.
    let height = screen.height.saturating_sub(2).min(wanted.saturating_add(3));
    let area = Rect::new(screen.x + (screen.width - width) / 2, screen.y + (screen.height - height) / 2, width, height);
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, palette::PLAIN);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(palette::DIM)
        .title(span(" Keys ", palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let max_scroll = wanted.saturating_sub(inner.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll.min(max_scroll), 0)), inner);
    max_scroll
}

pub(super) fn draw_settings(frame: &mut Frame, screen: Rect, state: &State, menu: &app::menus::SettingsMenu) {
    let width = screen.width.min(76);
    let height = screen.height.min(SETTINGS.len().try_into().unwrap_or(4) + 8);
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
        .title(span(" Settings ", palette::HEADING))
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let default = Config::default();
    let name_width = SETTINGS.iter().map(|s| s.key().len()).max().unwrap_or(0);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for setting in SETTINGS {
        let on = setting == menu.at;
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
