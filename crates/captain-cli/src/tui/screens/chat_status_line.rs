//! Title/status line rendering for the chat screen.

use super::chat::ChatState;
use crate::tui::theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::time::Duration;

#[cfg(test)]
mod tests;

/// Build the bottom title status line:
///   [spinner] model | mode | duration | tokens | cost
pub(super) fn build_status_line(state: &ChatState) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ")];

    push_activity_spinner(&mut spans, state);
    push_background_activity(&mut spans, state);
    push_model_and_mode(&mut spans, state);
    push_session_duration(&mut spans, state);
    push_last_tokens(&mut spans, state);
    push_last_cost(&mut spans, state);
    push_session_totals(&mut spans, state);

    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn separator_span() -> Span<'static> {
    Span::styled(" \u{2502} ", Style::default().fg(theme::BORDER))
}

fn push_separator(spans: &mut Vec<Span<'static>>) {
    spans.push(separator_span());
}

fn push_activity_spinner(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.is_streaming || state.thinking {
        let frame = theme::SPINNER_FRAMES[state.spinner_frame % theme::SPINNER_FRAMES.len()];
        spans.push(Span::styled(
            format!("{frame} "),
            Style::default().fg(theme::YELLOW),
        ));
    }
}

/// Persistent badge for sub-agents/detached tool_runs still in flight —
/// unlike `push_activity_spinner`, this isn't scoped to the current HTTP
/// turn and stays visible even after the turn that started the background
/// work has ended.
fn push_background_activity(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.background_activity.is_empty() {
        return;
    }
    spans.push(Span::styled(
        background_activity_label(state.background_activity.len()),
        Style::default().fg(theme::YELLOW),
    ));
}

fn background_activity_label(count: usize) -> String {
    if count == 1 {
        "\u{23f3} 1 en arrière-plan  ".to_string()
    } else {
        format!("\u{23f3} {count} en arrière-plan  ")
    }
}

fn push_model_and_mode(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    spans.push(Span::styled(
        state.model_label.clone(),
        Style::default().fg(theme::ACCENT),
    ));
    push_separator(spans);
    spans.push(Span::styled(state.mode_label.clone(), theme::dim_style()));
}

fn push_session_duration(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some(start) = state.session_start {
        push_separator(spans);
        spans.push(Span::styled(
            duration_label(start.elapsed()),
            theme::dim_style(),
        ));
    }
}

fn duration_label(elapsed: Duration) -> String {
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    format!("{mins}m{secs:02}s")
}

fn push_last_tokens(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some((input, output)) = state.last_tokens {
        push_separator(spans);
        spans.push(Span::styled(
            token_usage_label(input, output, state.last_cached_input_tokens),
            Style::default().fg(theme::TEXT_PRIMARY),
        ));
    }
}

fn token_usage_label(input: u64, output: u64, cached_input: u64) -> String {
    if cached_input > 0 {
        let effective_input = input.saturating_sub(cached_input);
        return format!(
            "{input}\u{2191} {output}\u{2193} · eff {}",
            compact_token_count(effective_input)
        );
    }
    format!("{input}\u{2191} {output}\u{2193}")
}

fn push_last_cost(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if let Some(cost) = state.last_cost_usd {
        push_separator(spans);
        spans.push(Span::styled(
            format!("${cost:.4}"),
            Style::default().fg(theme::GREEN),
        ));
    }
}

fn push_session_totals(spans: &mut Vec<Span<'static>>, state: &ChatState) {
    if state.session_input_tokens + state.session_output_tokens > 0 {
        push_separator(spans);
        let total = state.session_input_tokens + state.session_output_tokens;
        spans.push(Span::styled(
            format!("\u{03A3} {total} tok"),
            theme::dim_style(),
        ));
        if state.session_cost_usd > 0.0 {
            spans.push(Span::styled(
                format!(" / ${:.4}", state.session_cost_usd),
                Style::default().fg(theme::GREEN),
            ));
        }
    }
}

fn compact_token_count(tokens: u64) -> String {
    if tokens >= 1_000 {
        let value = tokens as f64 / 1_000.0;
        if tokens >= 10_000 {
            format!("{value:.0}k tok")
        } else {
            format!("{value:.1}k tok")
        }
    } else {
        format!("{tokens} tok")
    }
}
