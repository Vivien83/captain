use super::{NativeCapabilitiesState, NativeRunDecision, NativeScope};
use crate::tui::theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(frame: &mut Frame, area: Rect, state: &mut NativeCapabilitiesState) {
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Native capabilities ",
            theme::title_style(),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(inner);
    draw_summary(frame, chunks[0], state);

    let direction = if chunks[1].width >= 100 {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let constraints = if direction == Direction::Horizontal {
        [Constraint::Percentage(44), Constraint::Percentage(56)]
    } else {
        [Constraint::Percentage(52), Constraint::Percentage(48)]
    };
    let body = Layout::default()
        .direction(direction)
        .constraints(constraints)
        .split(chunks[1]);
    draw_list(frame, body[0], state);
    draw_detail(frame, body[1], state);

    let hint = if state.confirm_disable {
        " [y] confirm disable  [any] cancel"
    } else if state.confirm_run.is_some() {
        " [y] confirm exact run decision  [any] cancel"
    } else {
        " [↑↓] capability  [←→] revision  [[]] run  [a/x] approve/reject  [T/C/F] retry/confirm/fail run  [r] refresh"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(hint, theme::hint_style())),
        chunks[2],
    );
}

fn draw_summary(frame: &mut Frame, area: Rect, state: &NativeCapabilitiesState) {
    let ready = state.capabilities.iter().filter(|item| item.ready).count();
    let pending = state
        .capabilities
        .iter()
        .filter(|item| item.human_action_required)
        .count();
    let waiting_runs = state
        .runs
        .iter()
        .filter(|run| run.status == "waiting_decision")
        .count();
    let line = Line::from(vec![
        Span::styled(
            format!(" 1 {} ", NativeScope::Effective.label()),
            scope_style(state.scope == NativeScope::Effective),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" 2 {} ", NativeScope::Global.label()),
            scope_style(state.scope == NativeScope::Global),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" 3 {} ", NativeScope::Project.label()),
            scope_style(state.scope == NativeScope::Project),
        ),
        Span::styled(
            format!(
                "   {} total · {ready} ready · {pending} pending · {waiting_runs} uncertain runs",
                state.capabilities.len()
            ),
            theme::dim_style(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(vec![
            line,
            Line::from(Span::styled(&state.status_msg, theme::dim_style())),
        ]),
        area,
    );
}

fn draw_list(frame: &mut Frame, area: Rect, state: &mut NativeCapabilitiesState) {
    let items = state
        .capabilities
        .iter()
        .map(|item| {
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!(" {:<22}", truncate(&item.name, 22)),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(status_label(&item.status), status_style(&item.status)),
                ]),
                Line::from(Span::styled(
                    format!(
                        " {} · {} · {}",
                        item.scope,
                        version_label(&item.version),
                        short_hash(item.selected_hash.as_deref())
                    ),
                    theme::dim_style(),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let title = if state.loading {
        " CapSpecs · loading "
    } else {
        " CapSpecs "
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER)),
        )
        .highlight_style(theme::selected_style())
        .highlight_symbol("›");
    frame.render_stateful_widget(list, area, &mut state.list_state);
}

fn draw_detail(frame: &mut Frame, area: Rect, state: &NativeCapabilitiesState) {
    let Some(item) = state.selected() else {
        let mut lines = vec![Line::from(" No native capability in this scope.")];
        lines.extend(uncertain_run_lines(state));
        frame.render_widget(
            Paragraph::new(lines)
                .block(Block::default().title(" Detail ").borders(Borders::ALL))
                .style(theme::dim_style()),
            area,
        );
        return;
    };
    let revision = item.revisions.get(state.revision_index);
    let mut lines = vec![
        Line::from(vec![
            Span::styled(&item.name, theme::title_style()),
            Span::raw("  "),
            Span::styled(status_label(&item.status), status_style(&item.status)),
        ]),
        Line::from(Span::styled(&item.description, theme::dim_style())),
        Line::from(format!("tool: {}", item.tool_name)),
        Line::from(format!(
            "active: {}  pending: {}",
            short_hash(item.active_hash.as_deref()),
            short_hash(item.pending_hash.as_deref())
        )),
        Line::from(format!(
            "permissions: {}",
            if item.tools.is_empty() {
                "none".to_string()
            } else {
                item.tools.join(", ")
            }
        )),
        Line::from(""),
    ];
    if let Some(revision) = revision {
        let decision = revision
            .approved_by
            .as_deref()
            .map(|actor| format!("approved by {actor}"))
            .or_else(|| {
                revision
                    .rejected_by
                    .as_deref()
                    .map(|actor| format!("rejected by {actor}"))
            })
            .unwrap_or_else(|| "not decided".to_string());
        lines.push(Line::from(format!(
            "revision {}/{}: {}",
            state.revision_index + 1,
            item.revisions.len(),
            short_hash(Some(&revision.source_hash))
        )));
        lines.push(Line::from(Span::styled(
            format!("{} · {decision}", version_label(&revision.version)),
            theme::dim_style(),
        )));
    }
    if let Some(error) = item.last_error.as_deref() {
        lines.push(Line::from(Span::styled(
            format!("source error: {error}"),
            Style::default().fg(theme::RED),
        )));
    }
    if state.source_visible {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("source", theme::table_header())));
        lines.extend(
            item.source
                .as_deref()
                .unwrap_or("loading source…")
                .lines()
                .map(Line::from),
        );
    }
    lines.extend(uncertain_run_lines(state));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Detail ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::BORDER)),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn uncertain_run_lines(state: &NativeCapabilitiesState) -> Vec<Line<'static>> {
    let Some((run, node)) = state.selected_uncertain() else {
        return Vec::new();
    };
    let decision = state.confirm_run.map(|decision| match decision {
        NativeRunDecision::ConfirmSucceeded => "confirm succeeded with null output",
        NativeRunDecision::Retry => "retry this exact attempt",
        NativeRunDecision::MarkFailed => "mark this exact attempt failed",
    });
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "UNCERTAIN RUN {}/{}",
                state.run_index + 1,
                state.uncertain_run_count()
            ),
            Style::default()
                .fg(theme::YELLOW)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{} · {} · {}",
            run.capability_name,
            short_hash(Some(&run.run_id)),
            run.origin
        )),
        Line::from(format!(
            "step {} · tool {} · attempt {}",
            node.step_id, node.tool_name, node.attempts
        )),
        Line::from(Span::styled(
            format!("tool use: {}", short_hash(node.tool_use_id.as_deref())),
            theme::dim_style(),
        )),
    ];
    if let Some(decision) = decision {
        lines.push(Line::from(Span::styled(
            format!("press y to {decision}"),
            Style::default().fg(theme::YELLOW),
        )));
    }
    lines
}

fn scope_style(active: bool) -> Style {
    if active {
        theme::tab_active()
    } else {
        theme::tab_inactive()
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "operational" | "invalid_update_retained" => Style::default().fg(theme::GREEN),
        "pending_approval" | "update_pending_approval" => Style::default().fg(theme::YELLOW),
        _ => Style::default().fg(theme::RED),
    }
}

fn status_label(status: &str) -> &str {
    match status {
        "operational" => "READY",
        "pending_approval" => "APPROVAL",
        "update_pending_approval" => "UPDATE APPROVAL",
        "invalid_update_retained" => "READY / INVALID UPDATE",
        "invalid" => "INVALID",
        "disabled" => "DISABLED",
        "rejected" | "update_rejected" => "REJECTED",
        _ => status,
    }
}

fn short_hash(hash: Option<&str>) -> String {
    hash.filter(|value| !value.is_empty())
        .map(|value| value.chars().take(12).collect())
        .unwrap_or_else(|| "—".to_string())
}

fn version_label(version: &str) -> String {
    if version.is_empty() {
        "no version".to_string()
    } else {
        format!("v{version}")
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value
        .chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}
