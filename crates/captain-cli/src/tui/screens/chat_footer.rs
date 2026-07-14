use super::chat::{ChatState, ToolStatus};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

struct FooterItem {
    priority: u8,
    spans: Vec<Span<'static>>,
}

type FooterAction = (&'static str, &'static str);

const APPROVAL_ACTIONS: &[FooterAction] = &[
    ("O", "une fois"),
    ("S", "session"),
    ("A", "toujours"),
    ("R", "rejeter"),
];
const MODEL_SWITCH_ACTIONS: &[FooterAction] = &[("1", "new"), ("2", "compact"), ("Esc", "annuler")];
const MODEL_PICKER_ACTIONS: &[FooterAction] = &[
    ("\u{2191}\u{2193}", "choisir"),
    ("Enter", "appliquer"),
    ("Esc", "fermer"),
];
const SESSION_PICKER_ACTIONS: &[FooterAction] = &[
    ("\u{2191}\u{2193}", "choisir"),
    ("Enter", "charger"),
    ("Esc", "fermer"),
];
const SLASH_PICKER_ACTIONS: &[FooterAction] = &[
    ("\u{2191}\u{2193}", "choisir"),
    ("Tab", "compléter"),
    ("Enter", "appliquer"),
    ("Esc", "fermer"),
];
const STREAMING_ACTIONS: &[FooterAction] = &[
    ("Enter", "interject"),
    ("PgUp", "historique"),
    ("PgDn", "bas"),
    ("Esc", "retour"),
];
const IDLE_ACTIONS: &[FooterAction] = &[
    ("Enter", "envoyer"),
    ("Alt+Enter", "ligne"),
    ("PgUp", "historique"),
    ("PgDn", "bas"),
    ("/", "commandes"),
    ("Ctrl+M", "modèle"),
];

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

pub(super) fn draw_input_footer(f: &mut Frame, area: Rect, state: &ChatState) {
    let footer = Paragraph::new(build_input_footer(state, area.width as usize));
    f.render_widget(footer, area);
}

fn build_input_footer(state: &ChatState, width: usize) -> Line<'static> {
    let mut items = vec![footer_status_item(state)];

    if state.is_streaming || state.thinking {
        items.push(footer_stream_item(state));
    }

    items.push(footer_context_item(state));

    if let Some(item) = footer_queue_item(state) {
        items.push(item);
    }
    if let Some(item) = footer_attachments_item(state) {
        items.push(item);
    }
    if let Some(item) = footer_tools_item(state) {
        items.push(item);
    }

    items.push(footer_action_item(state));

    if let Some(item) = footer_telemetry_item(state) {
        items.push(item);
    }

    let max_width = width.saturating_sub(1);
    while footer_items_width(&items) > max_width {
        let Some((idx, item)) = items
            .iter()
            .enumerate()
            .max_by_key(|(_, item)| item.priority)
        else {
            break;
        };
        if item.priority == 0 {
            break;
        }
        items.remove(idx);
    }

    let mut line = footer_line_from_items(items);
    if line_width(&line) > width {
        let text = truncate_line(&line_plain_text(&line), width.saturating_sub(1));
        line = Line::from(vec![Span::styled(text, theme::hint_style())]);
    }
    line
}

fn footer_status_item(state: &ChatState) -> FooterItem {
    let (label, style) = if state.pending_approval.is_some() {
        ("\u{25cf} approval", bold(theme::RED))
    } else if state.pending_model_switch.is_some() {
        ("\u{25cf} model switch", bold(theme::YELLOW))
    } else if state.show_model_picker {
        ("\u{25c6} modèle", bold(theme::PURPLE))
    } else if state.show_session_picker {
        ("\u{25c6} sessions", bold(theme::PURPLE))
    } else if state.slash_picker_active() {
        ("\u{2318} commandes", bold(theme::PURPLE))
    } else if state.is_streaming || state.thinking {
        ("\u{25cf} stream", bold(theme::BLUE))
    } else {
        ("\u{25cf} prêt", bold(theme::GREEN))
    };

    FooterItem {
        priority: 0,
        spans: vec![Span::styled(label.to_string(), style)],
    }
}

fn footer_context_item(state: &ChatState) -> FooterItem {
    let current = current_context_tokens_with_stream(state);
    let style = context_pressure_style(current);
    let ratio = (current as f64 / 64_000.0).clamp(0.0, 1.0);
    let mut spans = vec![
        Span::styled("ctx ", theme::dim_style()),
        Span::styled(pressure_sparkline(ratio), style),
        Span::styled(format!(" {}", compact_token_count(current)), style),
    ];

    if let Some((input, _)) = state.last_tokens {
        if state.last_cached_input_tokens > 0 && input > 0 {
            let pct = ((state.last_cached_input_tokens as f64 / input as f64) * 100.0).round();
            spans.push(Span::styled(
                format!(" cache {:.0}%", pct),
                Style::default().fg(theme::BLUE),
            ));
        }
    }

    if current >= 48_000 {
        spans.push(Span::styled(
            " /compact",
            Style::default().fg(theme::YELLOW),
        ));
    }

    FooterItem { priority: 1, spans }
}

fn footer_stream_item(state: &ChatState) -> FooterItem {
    let est_tokens = (state.streaming_chars / 4) as u64;
    let (label, style) = if state.thinking && state.streaming_chars == 0 {
        ("think ", Style::default().fg(theme::YELLOW))
    } else {
        ("out ", Style::default().fg(theme::BLUE))
    };

    FooterItem {
        priority: 1,
        spans: vec![
            Span::styled(label.to_string(), theme::dim_style()),
            Span::styled(activity_sparkline(state.spinner_frame).to_string(), style),
            Span::styled(format!(" ~{} tok", est_tokens), style),
        ],
    }
}

fn footer_queue_item(state: &ChatState) -> Option<FooterItem> {
    if state.staged_messages.is_empty() {
        return None;
    }
    Some(FooterItem {
        priority: 2,
        spans: vec![
            Span::styled("queued ", theme::dim_style()),
            Span::styled(state.staged_messages.len().to_string(), bold(theme::YELLOW)),
        ],
    })
}

fn footer_attachments_item(state: &ChatState) -> Option<FooterItem> {
    if state.pending_attachments.is_empty() {
        return None;
    }
    Some(FooterItem {
        priority: 3,
        spans: vec![
            Span::styled("files ", theme::dim_style()),
            Span::styled(
                state.pending_attachments.len().to_string(),
                bold(theme::BLUE),
            ),
        ],
    })
}

fn footer_tools_item(state: &ChatState) -> Option<FooterItem> {
    let mut running = usize::from(state.active_tool.is_some());
    let mut errors = 0usize;
    for msg in &state.messages {
        let Some(info) = msg.tool.as_ref() else {
            continue;
        };
        match info.status {
            ToolStatus::Running => running += 1,
            ToolStatus::Error => errors += 1,
            ToolStatus::Success => {}
        }
    }

    if running == 0 && errors == 0 {
        return None;
    }

    let mut spans = Vec::new();
    if running > 0 {
        spans.push(Span::styled("tools ", theme::dim_style()));
        spans.push(Span::styled(format!("{running} run"), bold(theme::BLUE)));
    }
    if errors > 0 {
        if !spans.is_empty() {
            spans.push(Span::styled(" ", theme::hint_style()));
        }
        spans.push(Span::styled(format!("err {errors}"), bold(theme::RED)));
    }

    Some(FooterItem { priority: 2, spans })
}

fn footer_action_item(state: &ChatState) -> FooterItem {
    let (priority, actions) = footer_action_spec(state);
    action_footer_item(priority, actions)
}

fn footer_action_spec(state: &ChatState) -> (u8, &'static [FooterAction]) {
    if state.pending_approval.is_some() {
        return (1, APPROVAL_ACTIONS);
    }
    if state.pending_model_switch.is_some() {
        return (1, MODEL_SWITCH_ACTIONS);
    }
    if state.show_model_picker {
        return (2, MODEL_PICKER_ACTIONS);
    }
    if state.show_session_picker {
        return (2, SESSION_PICKER_ACTIONS);
    }
    if state.slash_picker_active() {
        return (2, SLASH_PICKER_ACTIONS);
    }
    if state.is_streaming {
        return (3, STREAMING_ACTIONS);
    }

    (3, IDLE_ACTIONS)
}

fn action_footer_item(priority: u8, actions: &[FooterAction]) -> FooterItem {
    let mut spans = Vec::new();
    for (idx, (key, label)) in actions.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" · ", theme::hint_style()));
        }
        spans.push(Span::styled((*key).to_string(), bold(theme::ACCENT)));
        if !label.is_empty() {
            spans.push(Span::styled(format!(" {label}"), theme::dim_style()));
        }
    }
    FooterItem { priority, spans }
}

fn footer_telemetry_item(state: &ChatState) -> Option<FooterItem> {
    let mut spans = Vec::new();

    if state.session_cost_usd > 0.0 {
        spans.push(Span::styled(
            format_cost(state.session_cost_usd),
            Style::default().fg(theme::GREEN),
        ));
    } else if !state.model_label.is_empty() {
        spans.push(Span::styled(
            compact_model_label(&state.model_label),
            theme::hint_style(),
        ));
    }

    if state.mouse_capture_enabled {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", theme::hint_style()));
        }
        spans.push(Span::styled(
            "scroll souris",
            Style::default().fg(theme::PURPLE),
        ));
    }

    if spans.is_empty() {
        None
    } else {
        Some(FooterItem { priority: 6, spans })
    }
}

fn current_context_tokens_with_stream(state: &ChatState) -> u64 {
    let session_total = state
        .session_input_tokens
        .saturating_add(state.session_output_tokens);
    let current_turn = if state.is_streaming || state.thinking {
        state
            .last_tokens
            .map(|(input, output)| input.saturating_add(output))
            .unwrap_or(0)
    } else {
        0
    };
    let fallback_last = state
        .last_tokens
        .map(|(input, output)| input.saturating_add(output))
        .unwrap_or(0);
    let recorded = if session_total > 0 {
        session_total.saturating_add(current_turn)
    } else {
        fallback_last
    };
    recorded.saturating_add((state.streaming_chars / 4) as u64)
}

fn context_pressure_style(total_tokens: u64) -> Style {
    let color = if total_tokens >= 48_000 {
        theme::RED
    } else if total_tokens >= 24_000 {
        theme::YELLOW
    } else {
        theme::GREEN
    };
    bold(color)
}

fn pressure_sparkline(ratio: f64) -> String {
    const BARS: [char; 5] = ['\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}'];
    let filled = (ratio * BARS.len() as f64).ceil() as usize;
    (0..BARS.len())
        .map(|idx| {
            if idx < filled.min(BARS.len()) {
                BARS[idx]
            } else {
                '\u{2581}'
            }
        })
        .collect()
}

fn activity_sparkline(frame: usize) -> &'static str {
    const FRAMES: [&str; 10] = [
        "\u{2581}\u{2582}\u{2584}\u{2586}\u{2585}",
        "\u{2582}\u{2584}\u{2586}\u{2585}\u{2583}",
        "\u{2584}\u{2586}\u{2585}\u{2583}\u{2582}",
        "\u{2586}\u{2585}\u{2583}\u{2582}\u{2581}",
        "\u{2585}\u{2583}\u{2582}\u{2581}\u{2582}",
        "\u{2583}\u{2582}\u{2581}\u{2582}\u{2584}",
        "\u{2582}\u{2581}\u{2582}\u{2584}\u{2586}",
        "\u{2581}\u{2582}\u{2584}\u{2586}\u{2588}",
        "\u{2582}\u{2584}\u{2586}\u{2588}\u{2586}",
        "\u{2584}\u{2586}\u{2588}\u{2586}\u{2584}",
    ];
    FRAMES[frame % FRAMES.len()]
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

fn format_cost(cost: f64) -> String {
    if cost >= 1.0 {
        format!("${cost:.2}")
    } else {
        format!("${cost:.4}")
    }
}

fn compact_model_label(label: &str) -> String {
    let compact = label.rsplit('/').next().unwrap_or(label);
    truncate_line(compact, 24)
}

fn footer_line_from_items(items: Vec<FooterItem>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (idx, item) in items.into_iter().enumerate() {
        if idx > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(theme::BORDER)));
        }
        spans.extend(item.spans);
    }
    Line::from(spans)
}

fn footer_items_width(items: &[FooterItem]) -> usize {
    if items.is_empty() {
        return 0;
    }
    let content = items
        .iter()
        .map(|item| spans_width(&item.spans))
        .sum::<usize>();
    1 + content + (items.len().saturating_sub(1) * 3)
}

fn spans_width(spans: &[Span<'static>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

fn line_width(line: &Line<'static>) -> usize {
    spans_width(&line.spans)
}

fn line_plain_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn truncate_line(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            captain_types::truncate_str(s, max_len.saturating_sub(1))
        )
    }
}

#[cfg(test)]
mod tests;
