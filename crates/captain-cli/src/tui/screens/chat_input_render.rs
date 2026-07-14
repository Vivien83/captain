//! Chat input line rendering.

use super::{chat::ChatState, chat_input_layout::locate_cursor};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

#[cfg(test)]
mod tests;

pub(super) fn draw_chat_input(f: &mut Frame, area: Rect, state: &ChatState) {
    f.render_widget(
        Paragraph::new(build_input_lines(state)).wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn build_input_lines(state: &ChatState) -> Vec<Line<'static>> {
    let styles = input_render_styles(state);
    let input_text = state.input.as_str();
    let (cursor_line, cursor_col_byte) = locate_cursor(input_text, state.input_cursor);
    let raw_lines = raw_input_lines(input_text);
    let last_idx = raw_lines.len() - 1;

    raw_lines
        .iter()
        .enumerate()
        .map(|(idx, raw)| {
            build_input_line(
                raw,
                idx,
                last_idx,
                cursor_line,
                cursor_col_byte,
                state,
                styles,
            )
        })
        .collect()
}

#[derive(Clone, Copy)]
struct InputRenderStyles {
    prompt: Style,
    cursor: Style,
}

fn input_render_styles(state: &ChatState) -> InputRenderStyles {
    let cursor_color = if state.is_streaming {
        theme::YELLOW
    } else {
        theme::ACCENT
    };
    let prompt = if state.is_streaming {
        Style::default().fg(theme::YELLOW)
    } else {
        theme::input_style()
    };
    InputRenderStyles {
        prompt,
        cursor: Style::default()
            .fg(cursor_color)
            .add_modifier(Modifier::SLOW_BLINK),
    }
}

fn raw_input_lines(input_text: &str) -> Vec<&str> {
    if input_text.is_empty() {
        vec![""]
    } else {
        input_text.split('\n').collect()
    }
}

fn build_input_line(
    raw: &str,
    idx: usize,
    last_idx: usize,
    cursor_line: usize,
    cursor_col_byte: usize,
    state: &ChatState,
    styles: InputRenderStyles,
) -> Line<'static> {
    let cursor_here = idx == cursor_line;
    let mut spans = vec![Span::styled(input_prefix(idx), styles.prompt)];
    let (before_cursor, after_cursor) = split_at_cursor(raw, cursor_col_byte, cursor_here);

    if should_highlight_slash_command(idx, raw, cursor_here) {
        push_slash_command_spans(&mut spans, raw);
    } else {
        push_cursor_text_spans(&mut spans, before_cursor, after_cursor, cursor_here, styles);
    }

    if should_show_staged_badge(idx, last_idx, state) {
        push_staged_badge(&mut spans, state.staged_messages.len());
    }
    Line::from(spans)
}

fn input_prefix(idx: usize) -> &'static str {
    if idx == 0 {
        " > "
    } else {
        "   "
    }
}

fn should_highlight_slash_command(idx: usize, raw: &str, cursor_here: bool) -> bool {
    idx == 0 && raw.starts_with('/') && !cursor_here
}

fn push_slash_command_spans(spans: &mut Vec<Span<'static>>, raw: &str) {
    let (cmd_part, rest_part) = match raw.find(' ') {
        Some(idx) => raw.split_at(idx),
        None => (raw, ""),
    };
    spans.push(Span::styled(
        cmd_part.to_string(),
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    ));
    if !rest_part.is_empty() {
        spans.push(Span::raw(rest_part.to_string()));
    }
}

fn push_cursor_text_spans(
    spans: &mut Vec<Span<'static>>,
    before_cursor: &str,
    after_cursor: &str,
    cursor_here: bool,
    styles: InputRenderStyles,
) {
    spans.push(Span::raw(before_cursor.to_string()));
    if cursor_here {
        spans.push(Span::styled("\u{2588}", styles.cursor));
        spans.push(Span::raw(after_cursor.to_string()));
    }
}

fn should_show_staged_badge(idx: usize, last_idx: usize, state: &ChatState) -> bool {
    idx == last_idx && state.is_streaming && !state.staged_messages.is_empty()
}

fn push_staged_badge(spans: &mut Vec<Span<'static>>, staged_count: usize) {
    spans.push(Span::styled(
        format!("  ({staged_count} staged)"),
        Style::default().fg(theme::PURPLE),
    ));
}

fn split_at_cursor(raw: &str, cursor_col_byte: usize, cursor_here: bool) -> (&str, &str) {
    if !cursor_here {
        return (raw, "");
    }
    let off = cursor_col_byte.min(raw.len());
    let off = (0..=off)
        .rev()
        .find(|idx| raw.is_char_boundary(*idx))
        .unwrap_or(0);
    (&raw[..off], &raw[off..])
}
