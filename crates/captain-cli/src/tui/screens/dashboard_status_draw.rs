use super::dashboard_status::StatusSnapshot;
use crate::tui::theme;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub(super) fn draw_status_cockpit(
    f: &mut Frame,
    cards_area: Rect,
    signals_area: Rect,
    status: &StatusSnapshot,
) {
    draw_status_cards(f, cards_area, status);
    draw_status_signals(f, signals_area, status);
}

fn draw_status_cards(f: &mut Frame, area: Rect, status: &StatusSnapshot) {
    let cols = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .split(area);

    draw_card(
        f,
        cols[0],
        "Health",
        vec![
            status_line(&status.runtime_health_state),
            dim_line(format!("{} issue(s)", status.runtime_health_issue_count)),
            dim_line(format!("daemon {}", status.status)),
        ],
    );
    draw_card(
        f,
        cols[1],
        "Work",
        vec![
            value_line(format!("{} agents", status.agent_count)),
            dim_line(format!(
                "{} run(s), {} proc(s)",
                status.active_run_count, status.process_count
            )),
        ],
    );
    draw_card(
        f,
        cols[2],
        "Tool Runs",
        vec![
            status_line(if status.tool_runs_failed > 0 {
                "warn"
            } else {
                "ok"
            }),
            dim_line(format!(
                "{} running / {} done / {} failed",
                status.tool_runs_running, status.tool_runs_completed, status.tool_runs_failed
            )),
        ],
    );
    draw_card(
        f,
        cols[3],
        "Agent API",
        vec![
            status_line(&status.agent_api_state),
            dim_line(format!(
                "{} pending / {} due / {} dead",
                status.agent_api_pending, status.agent_api_due, status.agent_api_dead_letters
            )),
        ],
    );
}

fn draw_status_signals(f: &mut Frame, area: Rect, status: &StatusSnapshot) {
    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let mut left = vec![
        Line::from(vec![
            Span::styled("  Model      ", theme::dim_style()),
            Span::styled(provider_label(status), Style::default().fg(theme::CYAN)),
            Span::styled(
                format!("  up {}", format_uptime(status.uptime_secs)),
                theme::dim_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Workload   ", theme::dim_style()),
            Span::styled(
                format!(
                    "{} active goal(s), {} escalated, {} project attention",
                    status.goals_active, status.goals_escalated, status.project_attention_count
                ),
                state_style(if status.goals_escalated > 0 {
                    "warn"
                } else {
                    "ok"
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Channels   ", theme::dim_style()),
            Span::styled(
                format!(
                    "{}/{} ready",
                    status.channel_ready_count, status.channel_total
                ),
                Style::default().fg(theme::GREEN),
            ),
            Span::styled(
                format!(
                    "  cron {}/{} due/enabled",
                    status.cron_due, status.cron_enabled
                ),
                theme::dim_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Budget     ", theme::dim_style()),
            Span::styled(
                format!("{} tokens used", status.budget_total_tokens_used),
                Style::default().fg(theme::YELLOW),
            ),
        ]),
    ];
    left.extend(runtime_issue_lines(status));

    let right = vec![
        Line::from(vec![
            Span::styled("  Awareness  ", theme::dim_style()),
            Span::styled(
                status.consciousness_state.clone(),
                state_style(&status.consciousness_state),
            ),
            Span::styled(
                format!("  {} signal(s)", status.consciousness_signals.len()),
                theme::dim_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Streaming  ", theme::dim_style()),
            Span::styled(
                format!(
                    "{} active / {} completed",
                    status.streaming_active, status.streaming_completed
                ),
                Style::default().fg(theme::CYAN),
            ),
            Span::styled(streaming_latency_label(status), theme::dim_style()),
        ]),
        Line::from(vec![
            Span::styled("  Shutdown   ", theme::dim_style()),
            Span::styled(
                status.shutdown_status.clone(),
                state_style(&status.shutdown_status),
            ),
            Span::styled(disk_label(status), theme::dim_style()),
        ]),
        Line::from(vec![
            Span::styled("  Native     ", theme::dim_style()),
            Span::styled(native_status_label(status), native_status_style(status)),
        ]),
        Line::from(vec![
            Span::styled("  Action     ", theme::dim_style()),
            Span::styled(operator_action_label(status), theme::dim_style()),
        ]),
    ];

    f.render_widget(Paragraph::new(left), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}

fn draw_card(f: &mut Frame, area: Rect, title: &'static str, lines: Vec<Line<'static>>) {
    let block = Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(theme::CYAN),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn status_line(state: &str) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(" {state}"),
        state_style(state).add_modifier(Modifier::BOLD),
    )])
}

fn value_line(value: String) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(" {value}"),
        Style::default()
            .fg(theme::GREEN)
            .add_modifier(Modifier::BOLD),
    )])
}

fn dim_line(value: String) -> Line<'static> {
    Line::from(vec![Span::styled(format!(" {value}"), theme::dim_style())])
}

fn state_style(state: &str) -> Style {
    match state.to_ascii_lowercase().as_str() {
        "ok" | "running" | "idle" | "in-process" | "local" => Style::default().fg(theme::GREEN),
        "watch" | "retrying" | "attention" | "draining" | "pending" => {
            Style::default().fg(theme::YELLOW)
        }
        "warn" | "dead_letter" | "unavailable" | "failed" | "critical" => {
            Style::default().fg(theme::RED)
        }
        _ => Style::default().fg(theme::CYAN),
    }
}

fn provider_label(status: &StatusSnapshot) -> String {
    if status.provider.is_empty() {
        "not set".to_string()
    } else if status.model.is_empty() {
        status.provider.clone()
    } else {
        format!("{}/{}", status.provider, status.model)
    }
}

fn runtime_issue_lines(status: &StatusSnapshot) -> Vec<Line<'static>> {
    if status.runtime_health_issues.is_empty() {
        return vec![Line::from(vec![
            Span::styled("  Issue      ", theme::dim_style()),
            Span::styled("none", Style::default().fg(theme::GREEN)),
        ])];
    }

    status
        .runtime_health_issues
        .iter()
        .take(2)
        .map(|issue| {
            Line::from(vec![
                Span::styled("  Issue      ", theme::dim_style()),
                Span::styled(
                    format!("{}: {}", issue.kind, truncate(&issue.summary, 52)),
                    state_style(&issue.severity),
                ),
            ])
        })
        .collect()
}

fn streaming_latency_label(status: &StatusSnapshot) -> String {
    match (
        status.streaming_last_first_signal_ms,
        status.streaming_last_first_token_ms,
        status.streaming_last_total_ms,
    ) {
        (Some(signal), Some(token), Some(total)) => {
            format!("  last {signal}/{token}/{total}ms")
        }
        (Some(signal), _, _) => format!("  last signal {signal}ms"),
        _ => String::new(),
    }
}

fn disk_label(status: &StatusSnapshot) -> String {
    let Some(available) = status.disk_available_gib else {
        return String::new();
    };
    if status.disk_cleanup_recommended {
        format!("  disk {available:.1} GiB cleanup")
    } else {
        format!("  disk {available:.1} GiB")
    }
}

fn native_status_label(status: &StatusSnapshot) -> String {
    let embeddings = ready_label(status.native_embeddings_ready);
    let tts = ready_label(status.native_voice_tts_ready);
    let stt = ready_label(status.native_voice_stt_ready);
    format!("embeddings {embeddings}, tts {tts}, stt {stt}")
}

fn native_status_style(status: &StatusSnapshot) -> Style {
    if status.native_embeddings_ready == Some(false)
        || status.native_voice_tts_ready == Some(false)
        || status.native_voice_stt_ready == Some(false)
    {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::GREEN)
    }
}

fn ready_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "ready",
        Some(false) => "pending",
        None => "unknown",
    }
}

fn operator_action_label(status: &StatusSnapshot) -> String {
    status
        .consciousness_actions
        .first()
        .or_else(|| {
            status
                .runtime_health_issues
                .first()
                .map(|issue| &issue.action)
        })
        .map(|action| truncate(action, 72))
        .unwrap_or_else(|| "no operator action".to_string())
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            captain_types::truncate_str(s, max.saturating_sub(1))
        )
    }
}
