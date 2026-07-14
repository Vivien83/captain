//! Tool-call message rendering for the chat transcript.

use super::{
    chat::{
        copyable_tool_command, format_duration_ms, tool_input_summary, tool_output_summary,
        tool_status_parts, truncate_line, ToolInfo, ToolStatus,
    },
    chat_tool_expanded::render_tool_expanded,
};
use crate::tui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const TOOL_SUCCESS_EXPAND_GRACE: std::time::Duration = std::time::Duration::from_secs(4);

#[cfg(test)]
mod tests;

pub(super) fn render_tool_message(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    spinner_frame: usize,
    show_copy_button: bool,
) {
    lines.push(Line::from(""));
    if should_render_tool_expanded(info) {
        render_tool_expanded(lines, info, width, spinner_frame, show_copy_button);
    } else {
        render_tool_collapsed(lines, info, width, show_copy_button);
    }
}

fn copy_button_style() -> Style {
    Style::default()
        .fg(theme::CYAN)
        .add_modifier(Modifier::BOLD)
}

fn render_tool_collapsed(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    show_copy_button: bool,
) {
    let (icon, status_label, status_style) = tool_status_parts(info, None);
    let can_copy = collapsed_can_copy(info, show_copy_button);

    lines.push(collapsed_tool_header(
        info,
        width,
        can_copy,
        icon,
        status_style,
    ));
    if let Some(output) = collapsed_tool_output_line(info, width, status_label, status_style) {
        lines.push(output);
    }
}

fn collapsed_can_copy(info: &ToolInfo, show_copy_button: bool) -> bool {
    show_copy_button && copyable_tool_command(info).is_some()
}

fn collapsed_summary_width(width: usize, can_copy: bool) -> usize {
    width.saturating_sub(if can_copy { 43 } else { 36 })
}

fn collapsed_tool_summary(info: &ToolInfo, width: usize, can_copy: bool) -> String {
    truncate_line(
        &tool_input_summary(info),
        collapsed_summary_width(width, can_copy),
    )
}

fn collapsed_tool_duration(info: &ToolInfo) -> String {
    info.duration_ms
        .map(format_duration_ms)
        .unwrap_or_else(|| "done".to_string())
}

fn collapsed_tool_header(
    info: &ToolInfo,
    width: usize,
    can_copy: bool,
    icon: &str,
    status_style: Style,
) -> Line<'static> {
    let mut spans = vec![Span::styled("  ", theme::dim_style())];
    if can_copy {
        spans.push(Span::styled("[copy] ", copy_button_style()));
    }
    spans.extend([
        Span::styled(
            "\u{25b8} ",
            theme::hint_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{icon} "), status_style),
        Span::styled(
            info.name.clone(),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", theme::dim_style()),
        Span::styled(
            collapsed_tool_summary(info, width, can_copy),
            Style::default().fg(theme::TEXT_PRIMARY),
        ),
        Span::styled("  ", theme::dim_style()),
        Span::styled(collapsed_tool_duration(info), theme::hint_style()),
    ]);
    Line::from(spans)
}

fn collapsed_tool_output_line(
    info: &ToolInfo,
    width: usize,
    status_label: &'static str,
    status_style: Style,
) -> Option<Line<'static>> {
    let output = tool_output_summary(info);
    if output.is_empty() {
        None
    } else {
        Some(Line::from(vec![
            Span::styled("    ", theme::dim_style()),
            Span::styled(status_label, status_style),
            Span::styled(" · ", theme::hint_style()),
            Span::styled(
                truncate_line(&output, width.saturating_sub(10)),
                theme::hint_style(),
            ),
        ]))
    }
}

pub(super) fn should_render_tool_expanded(info: &ToolInfo) -> bool {
    info.expanded
        || info.status == ToolStatus::Running
        || (info.status == ToolStatus::Success
            && info
                .completed_at
                .is_some_and(|t| t.elapsed() < TOOL_SUCCESS_EXPAND_GRACE))
}
