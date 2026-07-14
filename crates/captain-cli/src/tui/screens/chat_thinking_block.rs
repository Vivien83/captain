//! Collapsible reasoning block for the chat transcript.

use super::{chat::wrap_text, chat::ChatState};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

#[cfg(test)]
mod tests;

pub(super) fn draw_thinking(f: &mut Frame, area: Rect, state: &ChatState) {
    let lines = thinking_lines(
        &state.thinking_text,
        state.thinking_expanded,
        area.width,
        area.height as usize,
    );
    f.render_widget(Paragraph::new(lines), area);
}

pub(super) fn thinking_lines(
    thinking_text: &str,
    expanded: bool,
    width: u16,
    max_lines: usize,
) -> Vec<Line<'static>> {
    let header = thinking_header(thinking_text.len(), expanded);
    if !expanded {
        return vec![header];
    }

    let mut lines = vec![header];
    let body_w = width.saturating_sub(4) as usize;
    if body_w > 4 {
        for raw in thinking_text.split('\n') {
            for wline in wrap_text(raw, body_w) {
                lines.push(Line::from(vec![Span::styled(
                    format!("    {wline}"),
                    theme::dim_style(),
                )]));
            }
        }
    }
    if lines.len() > max_lines {
        let drop = lines.len() - max_lines;
        lines.drain(0..drop.min(lines.len() - 1));
    }
    lines
}

fn thinking_header(chars: usize, expanded: bool) -> Line<'static> {
    let marker = if expanded { "\u{25bc}" } else { "\u{25b6}" };
    Line::from(vec![
        Span::styled(
            format!("  {marker} \u{1f4ad} reasoning  "),
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("({chars} chars)"), theme::dim_style()),
        Span::styled("  [Ctrl+T] toggle", theme::hint_style()),
    ])
}
