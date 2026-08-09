//! Rendering and bounded standalone projection for Live Runs.

use super::{LiveRunsState, RunFilter};
use crate::i18n::Lang;
use crate::tui::theme;
use captain_runtime::{tool_run_operator::OperatorToolRun, tool_runs::ToolRunStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, state: &mut LiveRunsState) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(2),
    ])
    .split(area);
    draw_header(frame, rows[0], state);

    let horizontal = area.width >= 88;
    let body = Layout::default()
        .direction(if horizontal {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if horizontal {
            vec![Constraint::Percentage(42), Constraint::Percentage(58)]
        } else {
            vec![Constraint::Percentage(43), Constraint::Percentage(57)]
        })
        .split(rows[1]);
    draw_run_list(frame, body[0], state);
    draw_detail(frame, body[1], state);
    draw_footer(frame, rows[2], state, area.width);
}

fn draw_header(frame: &mut Frame, area: Rect, state: &LiveRunsState) {
    let running = state
        .items
        .iter()
        .filter(|run| run.status == ToolRunStatus::Running)
        .count();
    let failed = state
        .items
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                ToolRunStatus::Failed | ToolRunStatus::Interrupted
            )
        })
        .count();
    let filter_spans = RunFilter::ALL
        .into_iter()
        .flat_map(|filter| {
            let selected = filter == state.filter;
            [
                Span::styled(
                    format!(" {} ", filter.label()),
                    if selected {
                        theme::selected_style()
                    } else {
                        theme::dim_style()
                    },
                ),
                Span::raw(" "),
            ]
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" Live Runs", theme::title_style()),
                Span::styled(
                    format!(
                        "  {} retained | {running} active | {failed} attention",
                        state.items.len()
                    ),
                    theme::dim_style(),
                ),
            ]),
            Line::from(filter_spans),
        ]),
        area,
    );
}

fn draw_run_list(frame: &mut Frame, area: Rect, state: &mut LiveRunsState) {
    let block = Block::default()
        .title(" Executions ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));
    let visible = state.visible_indices();
    if state.loading_runs && state.items.is_empty() {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        frame.render_widget(
            Paragraph::new(format!(" {spinner} Loading live runs..."))
                .block(block)
                .style(theme::dim_style()),
            area,
        );
        return;
    }
    if visible.is_empty() {
        frame.render_widget(
            Paragraph::new(" No runs match this filter.")
                .block(block)
                .style(theme::dim_style()),
            area,
        );
        return;
    }
    let items = visible
        .iter()
        .map(|index| {
            let run = &state.items[*index];
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        truncate(&terminal_safe_inline(&run.tool_name), 30),
                        theme::title_style(),
                    ),
                    Span::styled(
                        format!("  {}", run.status.as_str()),
                        status_style(run.status),
                    ),
                ]),
                Line::from(Span::styled(
                    format!(
                        "{} | {}{}",
                        truncate(&terminal_safe_inline(&run.run_id), 30),
                        format_elapsed(run.elapsed_ms),
                        if run.cancellable {
                            " | cancellable"
                        } else {
                            ""
                        }
                    ),
                    theme::dim_style(),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state.list_state);
}

fn draw_detail(frame: &mut Frame, area: Rect, state: &LiveRunsState) {
    let block = Block::default()
        .title(" Selected run ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(run) = state.selected_run() else {
        frame.render_widget(
            Paragraph::new(" Select a run to inspect retained metadata."),
            inner,
        );
        return;
    };

    let meta_height = if inner.height >= 15 {
        9
    } else {
        inner.height.saturating_div(2).max(3)
    };
    let rows = Layout::vertical([Constraint::Length(meta_height), Constraint::Min(1)]).split(inner);
    let mut metadata = vec![
        Line::from(vec![
            Span::styled(terminal_safe_inline(&run.tool_name), theme::title_style()),
            Span::styled(
                format!("  {}", run.status.as_str()),
                status_style(run.status),
            ),
        ]),
        detail_line("Run", &terminal_safe_inline(&run.run_id)),
        detail_line("Elapsed", &format_elapsed(run.elapsed_ms)),
        detail_line(
            "Agent",
            &terminal_safe_inline(run.caller_agent_id.as_deref().unwrap_or("-")),
        ),
        detail_line(
            "Output",
            &format!(
                "{} / {}",
                format_optional_bytes(run.output_stored_bytes),
                format_optional_bytes(run.output_total_bytes)
            ),
        ),
        detail_line(
            "SHA-256",
            &terminal_safe_inline(run.output_sha256.as_deref().unwrap_or("-")),
        ),
        detail_line(
            "Flags",
            &format!(
                "{}{}{}",
                if run.detached {
                    "detached "
                } else {
                    "foreground "
                },
                if run.output_redacted { "redacted " } else { "" },
                if run.output_capped { "capped" } else { "" }
            ),
        ),
        detail_line(
            "Retry",
            &run.retry_of_run_id
                .as_ref()
                .map(|parent| {
                    format!(
                        "#{} from {}",
                        run.retry_attempt,
                        truncate(&terminal_safe_inline(parent), 24)
                    )
                })
                .unwrap_or_else(|| "-".to_string()),
        ),
    ];
    metadata.truncate(rows[0].height as usize);
    frame.render_widget(Paragraph::new(metadata).wrap(Wrap { trim: false }), rows[0]);
    draw_tail(frame, rows[1], state);
}

fn draw_tail(frame: &mut Frame, area: Rect, state: &LiveRunsState) {
    let block = Block::default()
        .title(" Redacted output tail ")
        .borders(Borders::TOP)
        .border_style(theme::dim_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if state.loading_tail_for.is_some() && state.tail.is_none() {
        frame.render_widget(
            Paragraph::new("Loading retained output...").style(theme::dim_style()),
            inner,
        );
        return;
    }
    let Some(tail) = state.tail.as_ref() else {
        frame.render_widget(
            Paragraph::new("No retained output loaded. Press Enter to refresh.")
                .style(theme::dim_style()),
            inner,
        );
        return;
    };
    let safe_lines = terminal_safe_lines(&tail.content);
    let visible = visible_tail_lines(
        &safe_lines,
        inner.height as usize,
        state.tail_offset_from_end,
    );
    frame.render_widget(
        Paragraph::new(visible.into_iter().map(Line::raw).collect::<Vec<_>>())
            .style(Style::default().fg(theme::TEXT_PRIMARY)),
        inner,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &LiveRunsState, width: u16) {
    let (message, style) = if let Some(run_id) = state.confirm_cancel_for.as_deref() {
        (
            format!(
                " Stop {}? [y] confirm  [n/Esc] keep running",
                truncate(run_id, 28)
            ),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(run_id) = state.cancelling_for.as_deref() {
        (
            format!(" Cancelling {}...", truncate(run_id, 32)),
            Style::default().fg(theme::YELLOW),
        )
    } else if !state.error.is_empty() {
        (format!(" {}", state.error), Style::default().fg(theme::RED))
    } else if width >= 88 {
        (
            " [left/right] Filter  [up/down] Run  [PgUp/PgDn] Tail  [x] Stop  [r] Refresh  [Esc] Close".to_string(),
            theme::hint_style(),
        )
    } else {
        (
            " [left/right] Filter  [up/down] Run  [x] Stop  [r]  [Esc]".to_string(),
            theme::hint_style(),
        )
    };
    frame.render_widget(
        Paragraph::new(message)
            .style(style)
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub fn chat_runs_message(items: &[OperatorToolRun], lang: Lang) -> String {
    if items.is_empty() {
        return match lang {
            Lang::Fr => "Aucune execution d'outil conservee.".to_string(),
            Lang::En => "No retained tool runs.".to_string(),
        };
    }
    let heading = match lang {
        Lang::Fr => "Executions d'outils",
        Lang::En => "Tool runs",
    };
    let count = match lang {
        Lang::Fr => format!("{} conservées", items.len()),
        Lang::En => format!("{} retained", items.len()),
    };
    let mut lines = vec![format!("**{heading}** - {count}")];
    for run in items.iter().take(12) {
        lines.push(format!(
            "- `{}` - `{}` - `{}` - {}{}",
            truncate(&safe_chat_field(&run.run_id), 28),
            safe_chat_field(&run.tool_name),
            run.status.as_str(),
            format_elapsed(run.elapsed_ms),
            if run.cancellable {
                " - cancellable"
            } else {
                ""
            }
        ));
    }
    if items.len() > 12 {
        lines.push(format!("- ... {} more", items.len() - 12));
    }
    lines.push(match lang {
        Lang::Fr => "Tail expurge et arret confirme : TUI complet ou Control Web.".to_string(),
        Lang::En => {
            "Redacted tail and confirmed cancellation: full TUI or Control Web.".to_string()
        }
    });
    lines.join("\n")
}

fn status_style(status: ToolRunStatus) -> Style {
    let color = match status {
        ToolRunStatus::Running => theme::CYAN,
        ToolRunStatus::Completed => theme::GREEN,
        ToolRunStatus::Failed => theme::RED,
        ToolRunStatus::Cancelled => theme::YELLOW,
        ToolRunStatus::Interrupted => theme::YELLOW,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn detail_line(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}"), theme::table_header()),
        Span::raw(value.to_string()),
    ])
}

fn format_elapsed(milliseconds: u128) -> String {
    let seconds = milliseconds / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

fn format_optional_bytes(bytes: Option<u64>) -> String {
    match bytes {
        Some(bytes) if bytes < 1024 => format!("{bytes} B"),
        Some(bytes) if bytes < 1024 * 1024 => format!("{:.1} KiB", bytes as f64 / 1024.0),
        Some(bytes) => format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0)),
        None => "-".to_string(),
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!(
            "{}...",
            captain_types::truncate_str(value, max_chars.saturating_sub(3))
        )
    }
}

fn terminal_safe_lines(content: &str) -> Vec<String> {
    let normalized = content
        .chars()
        .map(|character| match character {
            '\n' => '\n',
            '\t' => ' ',
            character if character.is_control() => '�',
            character => character,
        })
        .collect::<String>();
    let lines = normalized.lines().map(str::to_string).collect::<Vec<_>>();
    if lines.is_empty() {
        vec!["(empty output)".to_string()]
    } else {
        lines
    }
}

fn terminal_safe_inline(content: &str) -> String {
    content
        .chars()
        .map(|character| match character {
            '\t' => ' ',
            character if character.is_control() => '�',
            character => character,
        })
        .collect()
}

fn safe_chat_field(content: &str) -> String {
    terminal_safe_inline(content).replace('`', "'")
}

pub(super) fn visible_tail_lines(
    lines: &[String],
    height: usize,
    offset_from_end: usize,
) -> Vec<String> {
    if height == 0 || lines.is_empty() {
        return Vec::new();
    }
    let maximum_offset = lines.len().saturating_sub(1);
    let end = lines
        .len()
        .saturating_sub(offset_from_end.min(maximum_offset));
    let start = end.saturating_sub(height);
    lines[start..end].to_vec()
}
