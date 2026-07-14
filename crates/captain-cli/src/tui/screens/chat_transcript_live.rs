//! Live transcript tail lines for streaming/status state.

use super::chat::ChatState;
use crate::tui::theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

#[cfg(test)]
mod tests;

pub(super) fn push_live_transcript_lines(
    lines: &mut Vec<Line<'static>>,
    state: &ChatState,
    width: usize,
) {
    push_streaming_text_lines(lines, state, width);
    push_thinking_line(lines, state);
    push_active_tool_line(lines, state);
    push_streaming_token_estimate(lines, state);
    push_last_token_usage(lines, state);
    push_status_message(lines, state);
    push_operator_notices(lines, state, width);
}

fn push_streaming_text_lines(lines: &mut Vec<Line<'static>>, state: &ChatState, width: usize) {
    if !state.streaming_text.is_empty() {
        lines.push(Line::from(""));
        lines.extend(crate::tui::markdown::render(
            &state.streaming_text,
            width.saturating_sub(4),
        ));
    }
}

fn push_thinking_line(lines: &mut Vec<Line<'static>>, state: &ChatState) {
    if state.thinking {
        lines.push(spinner_line(
            state.spinner_frame,
            theme::CYAN,
            "thinking\u{2026}".to_string(),
            Style::default().fg(theme::DIM),
        ));
    }
}

fn push_active_tool_line(lines: &mut Vec<Line<'static>>, state: &ChatState) {
    if let Some(ref tool_name) = state.active_tool {
        lines.push(spinner_line(
            state.spinner_frame,
            theme::RED,
            tool_name.clone(),
            Style::default().fg(theme::YELLOW),
        ));
    }
}

fn spinner_line(
    spinner_frame: usize,
    spinner_color: ratatui::style::Color,
    label: String,
    label_style: Style,
) -> Line<'static> {
    let spinner = theme::SPINNER_FRAMES[spinner_frame];
    Line::from(vec![
        Span::styled(format!("  {spinner} "), Style::default().fg(spinner_color)),
        Span::styled(label, label_style),
    ])
}

fn push_streaming_token_estimate(lines: &mut Vec<Line<'static>>, state: &ChatState) {
    if state.is_streaming && state.streaming_chars > 0 {
        lines.push(Line::from(vec![Span::styled(
            streaming_token_estimate_label(state.streaming_chars),
            theme::dim_style(),
        )]));
    }
}

fn streaming_token_estimate_label(streaming_chars: usize) -> String {
    format!("  ~{} tokens", streaming_chars / 4)
}

fn push_last_token_usage(lines: &mut Vec<Line<'static>>, state: &ChatState) {
    if let Some((input, output)) = state.last_tokens {
        if let Some(label) = last_token_usage_label(input, output, state.last_cost_usd) {
            lines.push(Line::from(vec![Span::styled(label, theme::dim_style())]));
        }
    }
}

fn last_token_usage_label(input: u64, output: u64, last_cost_usd: Option<f64>) -> Option<String> {
    if input == 0 && output == 0 {
        return None;
    }
    let cost_str = match last_cost_usd {
        Some(c) if c > 0.0 => format!(" | ${:.4}", c),
        _ => String::new(),
    };
    Some(format!("  [tokens: {input} in / {output} out{cost_str}]"))
}

fn push_status_message(lines: &mut Vec<Line<'static>>, state: &ChatState) {
    if let Some(ref msg) = state.status_msg {
        lines.push(Line::from(vec![Span::styled(
            format!("  {msg}"),
            Style::default().fg(theme::RED),
        )]));
    }
}

fn push_operator_notices(lines: &mut Vec<Line<'static>>, state: &ChatState, width: usize) {
    if state.operator_notices.is_empty() {
        return;
    }
    lines.push(Line::from(""));
    for (idx, notice) in state.operator_notices.iter().enumerate() {
        let style = if idx == 0 {
            Style::default().fg(theme::ACCENT)
        } else {
            theme::hint_style()
        };
        for wrapped in wrap_notice_line(notice, width.saturating_sub(4).max(24)) {
            lines.push(Line::from(vec![Span::styled(
                format!("  {wrapped}"),
                style,
            )]));
        }
    }
}

fn wrap_notice_line(line: &str, width: usize) -> Vec<String> {
    if line.len() <= width {
        return vec![line.to_string()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
            continue;
        }
        if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            out.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
