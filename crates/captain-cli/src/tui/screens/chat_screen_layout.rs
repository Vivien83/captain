//! Chat screen area calculations and simple separators.

use super::chat_input_layout::compute_input_visual_rows;
use crate::tui::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ChatScreenAreas {
    pub messages: Rect,
    pub separator: Rect,
    pub preview: Rect,
    pub input: Rect,
    pub footer: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReasoningAreas {
    pub thinking: Option<Rect>,
    pub messages: Rect,
}

pub(super) fn chat_screen_areas(inner: Rect, input: &str, preview_rows: u16) -> ChatScreenAreas {
    let input_rows = compute_input_visual_rows(input, inner.width).clamp(1, 10);
    let [messages, separator, preview, input, footer] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(preview_rows),
        Constraint::Length(input_rows),
        Constraint::Length(1),
    ])
    .areas(inner);

    ChatScreenAreas {
        messages,
        separator,
        preview,
        input,
        footer,
    }
}

pub(super) fn reasoning_areas(
    messages: Rect,
    has_thinking_text: bool,
    expanded: bool,
) -> ReasoningAreas {
    if !has_thinking_text {
        return ReasoningAreas {
            thinking: None,
            messages,
        };
    }

    let thinking_h = if expanded {
        (messages.height / 2).clamp(3, 12)
    } else {
        1
    };
    let [thinking, messages] =
        Layout::vertical([Constraint::Length(thinking_h), Constraint::Min(1)]).areas(messages);
    ReasoningAreas {
        thinking: Some(thinking),
        messages,
    }
}

pub(super) fn separator_line(width: u16) -> Line<'static> {
    Line::from(vec![Span::styled(
        "\u{2500}".repeat(width as usize),
        Style::default().fg(theme::BORDER),
    )])
}

pub(super) fn draw_separator(f: &mut Frame, area: Rect) {
    f.render_widget(Paragraph::new(separator_line(area.width)), area);
}
