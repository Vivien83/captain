use ratatui::{
    crossterm::event::KeyCode,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortcutAction {
    Prev,
    Next,
    Index(usize),
}

pub(crate) fn index<T: Copy + PartialEq>(items: &[T], value: T) -> usize {
    items.iter().position(|&item| item == value).unwrap_or(0)
}

pub(crate) fn next<T: Copy + PartialEq>(items: &[T], value: T) -> T {
    let idx = index(items, value);
    items[(idx + 1) % items.len()]
}

pub(crate) fn prev<T: Copy + PartialEq>(items: &[T], value: T) -> T {
    let idx = index(items, value);
    let prev = if idx == 0 { items.len() - 1 } else { idx - 1 };
    items[prev]
}

pub(crate) fn shortcut_action(code: KeyCode, item_count: usize) -> Option<ShortcutAction> {
    match code {
        KeyCode::Left => Some(ShortcutAction::Prev),
        KeyCode::Right => Some(ShortcutAction::Next),
        KeyCode::Char(ch) if ('1'..='9').contains(&ch) => {
            let idx = ch as usize - '1' as usize;
            (idx < item_count).then_some(ShortcutAction::Index(idx))
        }
        _ => None,
    }
}

pub(crate) fn line(title: &str, labels: &[&str], active_idx: usize) -> Line<'static> {
    let hint = format!(" Alt+1..{} / Alt+←→ ", labels.len());
    let mut spans = vec![
        Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(hint, theme::hint_style()),
    ];

    for (idx, label) in labels.iter().enumerate() {
        let text = format!(" {} {} ", idx + 1, label);
        let style = if idx == active_idx {
            theme::tab_active()
        } else {
            theme::tab_inactive()
        };
        spans.push(Span::styled(text, style));
    }

    Line::from(spans)
}

pub(crate) fn draw(frame: &mut Frame, area: Rect, title: &str, labels: &[&str], active_idx: usize) {
    let bar =
        Paragraph::new(line(title, labels, active_idx)).style(Style::default().bg(theme::BG_CARD));
    frame.render_widget(bar, area);
}

#[cfg(test)]
#[path = "hub_nav/tests.rs"]
mod tests;
