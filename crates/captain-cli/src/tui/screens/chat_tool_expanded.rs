//! Expanded tool-call rendering for the chat transcript.

use super::chat::{
    copyable_tool_command, format_duration_ms, pretty_tool_input, tail_lines, tool_command,
    tool_status_parts, wrap_text, ToolInfo, ToolStatus,
};
use crate::tui::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

#[cfg(test)]
mod tests;

pub(super) fn render_tool_expanded(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    spinner_frame: usize,
    show_copy_button: bool,
) {
    let border_style = tool_border_style(info.status);
    push_tool_header(lines, info, width, spinner_frame, show_copy_button);
    render_tool_input_detail(lines, info, width, border_style);
    render_tool_streams(lines, info, width, border_style);
    render_tool_result(lines, info, width, border_style);
    push_tool_footer(lines, width, border_style);
}

fn tool_border_style(status: ToolStatus) -> Style {
    with_tool_bg(Style::default().fg(tool_border_color(status)))
}

fn push_tool_header(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    spinner_frame: usize,
    show_copy_button: bool,
) {
    let border_style = tool_border_style(info.status);
    let title = tool_header_title(info, spinner_frame, show_copy_button);
    let title_style = tool_header_title_style(info, spinner_frame);
    let fill = "\u{2500}".repeat(width.saturating_sub(title.chars().count() + 3));

    lines.push(Line::from(vec![
        Span::styled("  \u{250c}", border_style),
        Span::styled(title, title_style),
        Span::styled(fill, border_style),
    ]));
}

fn tool_header_title(info: &ToolInfo, spinner_frame: usize, show_copy_button: bool) -> String {
    let (icon, status_label, _) = tool_status_parts(info, Some(spinner_frame));
    let duration = tool_duration_label(info);
    let disclosure = if info.status == ToolStatus::Running {
        ""
    } else {
        "\u{25be} "
    };
    let copy_badge = if show_copy_button && copyable_tool_command(info).is_some() {
        "[copy] "
    } else {
        ""
    };
    format!(
        " {copy_badge}{disclosure}{icon} {} · {status_label} · {duration} ",
        info.name
    )
}

fn tool_header_title_style(info: &ToolInfo, spinner_frame: usize) -> Style {
    let (_, _, status_style) = tool_status_parts(info, Some(spinner_frame));
    with_tool_bg(status_style)
}

fn tool_duration_label(info: &ToolInfo) -> String {
    match info.status {
        ToolStatus::Running => info
            .started_at
            .map(|t| format_duration_ms(t.elapsed().as_millis() as u64))
            .unwrap_or_else(|| "00.0s".to_string()),
        _ => info
            .duration_ms
            .map(format_duration_ms)
            .unwrap_or_else(|| "done".to_string()),
    }
}

fn render_tool_streams(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    border_style: Style,
) {
    render_tool_stream_section(
        lines,
        stdout_label(info),
        &info.stdout,
        width,
        border_style,
        false,
    );
    render_tool_stream_section(lines, "stderr", &info.stderr, width, border_style, true);
}

fn stdout_label(info: &ToolInfo) -> &'static str {
    if info.name.starts_with("browser") {
        "activity"
    } else {
        "stdout"
    }
}

fn render_tool_result(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    border_style: Style,
) {
    if info.result.is_empty() || !info.stdout.is_empty() || !info.stderr.is_empty() {
        return;
    }

    let label = if info.is_error { "error" } else { "result" };
    push_box_line(
        lines,
        border_style,
        width,
        vec![Span::styled(
            format!("{label} "),
            tool_result_label_style(info.is_error),
        )],
    );
    push_box_wrapped(
        lines,
        border_style,
        &info.result,
        width.saturating_sub(6),
        width,
        tool_result_body_style(info.is_error),
        4,
    );
}

fn tool_result_label_style(is_error: bool) -> Style {
    if is_error {
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
    } else {
        theme::dim_style()
    }
}

fn tool_result_body_style(is_error: bool) -> Style {
    if is_error {
        Style::default().fg(theme::RED)
    } else {
        theme::hint_style()
    }
}

fn push_tool_footer(lines: &mut Vec<Line<'static>>, width: usize, border_style: Style) {
    let footer_fill = "\u{2500}".repeat(width.saturating_sub(3));
    lines.push(Line::from(vec![Span::styled(
        format!("  \u{2514}{footer_fill}"),
        border_style,
    )]));
}

fn render_tool_input_detail(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    width: usize,
    border_style: Style,
) {
    if info.input.is_empty() {
        return;
    }

    if render_tool_diff(lines, info, border_style) {
        return;
    }

    if let Some(command) = tool_command(info) {
        push_box_line(
            lines,
            border_style,
            width,
            vec![
                Span::styled("$ ", Style::default().fg(theme::GREEN)),
                Span::styled(command, Style::default().fg(theme::TEXT_PRIMARY)),
            ],
        );
        return;
    }

    push_box_line(
        lines,
        border_style,
        width,
        vec![Span::styled("input", theme::dim_style())],
    );
    let input = pretty_tool_input(&info.input);
    push_box_wrapped(
        lines,
        border_style,
        &input,
        width.saturating_sub(6),
        width,
        Style::default().fg(theme::TEXT_SECONDARY),
        5,
    );
}

fn render_tool_diff(lines: &mut Vec<Line<'static>>, info: &ToolInfo, border_style: Style) -> bool {
    if render_apply_patch_diff(lines, info, border_style) {
        return true;
    }

    let Ok(parsed) = serde_json::from_str::<Value>(&info.input) else {
        return false;
    };

    match info.name.as_str() {
        "edit_file" => render_edit_file_diff(lines, &parsed, border_style),
        "multi_edit" => render_multi_edit_diff(lines, &parsed, border_style),
        _ => false,
    }
}

fn render_apply_patch_diff(
    lines: &mut Vec<Line<'static>>,
    info: &ToolInfo,
    border_style: Style,
) -> bool {
    if info.name != "apply_patch" {
        return false;
    }
    lines.extend(crate::tui::diff_render::render_apply_patch_input(
        &info.input,
        border_style,
    ));
    true
}

fn render_edit_file_diff(
    lines: &mut Vec<Line<'static>>,
    parsed: &Value,
    border_style: Style,
) -> bool {
    lines.extend(crate::tui::diff_render::render_unified_diff(
        parsed["old_string"].as_str().unwrap_or(""),
        parsed["new_string"].as_str().unwrap_or(""),
        parsed["path"].as_str().unwrap_or("?"),
        border_style,
    ));
    true
}

fn render_multi_edit_diff(
    lines: &mut Vec<Line<'static>>,
    parsed: &Value,
    border_style: Style,
) -> bool {
    let Some(edits) = parsed["edits"].as_array() else {
        return false;
    };
    let path = parsed["path"].as_str().unwrap_or("?");
    for (index, edit) in edits.iter().enumerate() {
        push_multi_edit_header(lines, border_style, path, index + 1);
        lines.extend(crate::tui::diff_render::render_unified_diff(
            edit["old_string"].as_str().unwrap_or(""),
            edit["new_string"].as_str().unwrap_or(""),
            path,
            border_style,
        ));
    }
    true
}

fn push_multi_edit_header(
    lines: &mut Vec<Line<'static>>,
    border_style: Style,
    path: &str,
    index: usize,
) {
    lines.push(Line::from(vec![
        Span::styled("  \u{2502} ", border_style),
        Span::styled(
            format!("edit #{index} · {path}"),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
}

fn render_tool_stream_section(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    text: &str,
    width: usize,
    border_style: Style,
    is_err: bool,
) {
    if text.is_empty() {
        return;
    }
    push_stream_header(lines, label, text, width, border_style, is_err);
    push_stream_tail(lines, text, width, border_style, is_err);
}

fn push_stream_header(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    text: &str,
    width: usize,
    border_style: Style,
    is_err: bool,
) {
    let line_count = text.lines().count();
    push_box_line(
        lines,
        border_style,
        width,
        vec![
            Span::styled(label.to_string(), stream_label_style(is_err)),
            Span::styled(format!(" · {line_count} lines"), theme::hint_style()),
        ],
    );
}

fn stream_label_style(is_err: bool) -> Style {
    if is_err {
        Style::default()
            .fg(theme::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme::BLUE)
            .add_modifier(Modifier::BOLD)
    }
}

fn push_stream_tail(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    border_style: Style,
    is_err: bool,
) {
    let style = stream_body_style(is_err);
    for raw in tail_lines(text, 8) {
        push_stream_wrapped_line(lines, border_style, width, style, &raw);
    }
}

fn stream_body_style(is_err: bool) -> Style {
    if is_err {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::TEXT_SECONDARY)
    }
}

fn push_stream_wrapped_line(
    lines: &mut Vec<Line<'static>>,
    border_style: Style,
    width: usize,
    style: Style,
    raw: &str,
) {
    for wrapped in wrap_text(raw, width.saturating_sub(8)) {
        push_box_line(
            lines,
            border_style,
            width,
            vec![
                Span::styled("  ", theme::hint_style()),
                Span::styled(wrapped, style),
            ],
        );
    }
}

fn push_box_line(
    lines: &mut Vec<Line<'static>>,
    border_style: Style,
    width: usize,
    mut body: Vec<Span<'static>>,
) {
    let mut spans = vec![Span::styled("  \u{2502} ", border_style)];
    for span in &mut body {
        span.style = with_tool_bg(span.style);
    }
    let used = 4 + body
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    spans.extend(body);
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        tool_panel_style(),
    ));
    lines.push(Line::from(spans));
}

fn push_box_wrapped(
    lines: &mut Vec<Line<'static>>,
    border_style: Style,
    text: &str,
    wrap_width: usize,
    total_width: usize,
    style: Style,
    max_lines: usize,
) {
    let mut emitted = 0usize;
    for raw in text.lines() {
        for wrapped in wrap_text(raw, wrap_width) {
            if emitted >= max_lines {
                push_box_line(
                    lines,
                    border_style,
                    total_width,
                    vec![Span::styled("  …", theme::hint_style())],
                );
                return;
            }
            push_box_line(
                lines,
                border_style,
                total_width,
                vec![
                    Span::styled("  ", theme::hint_style()),
                    Span::styled(wrapped, style),
                ],
            );
            emitted += 1;
        }
    }
}

fn tool_panel_style() -> Style {
    Style::default().bg(theme::BG_CODE)
}

fn with_tool_bg(style: Style) -> Style {
    style.bg(theme::BG_CODE)
}

fn tool_border_color(status: ToolStatus) -> ratatui::style::Color {
    match status {
        ToolStatus::Running => theme::BLUE,
        ToolStatus::Success => theme::GREEN,
        ToolStatus::Error => theme::RED,
    }
}
