//! Message-history line building for the chat transcript.

use super::{
    chat::{copyable_tool_command, ChatMessage, Role, ToolStatus},
    chat_tool_message::{render_tool_message, should_render_tool_expanded},
    chat_transcript_layout::PendingToolZone,
};
use crate::tui::theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

#[cfg(test)]
mod tests;

pub(super) fn push_message_history_lines(
    lines: &mut Vec<Line<'static>>,
    pending_tool_zones: &mut Vec<PendingToolZone>,
    messages: &[ChatMessage],
    width: usize,
    spinner_frame: usize,
    mouse_capture_enabled: bool,
) {
    for (message_idx, msg) in messages.iter().enumerate() {
        match msg.role {
            Role::User => push_user_message(lines, &msg.text, width),
            Role::Agent => {
                lines.push(Line::from(""));
                lines.extend(crate::tui::markdown::render(
                    &msg.text,
                    width.saturating_sub(4),
                ));
            }
            Role::System => {
                for sline in msg.text.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {sline}"),
                        theme::dim_style(),
                    )]));
                }
            }
            Role::Tool => push_tool_message(
                lines,
                pending_tool_zones,
                msg,
                message_idx,
                width,
                spinner_frame,
                mouse_capture_enabled,
            ),
        }
    }
}

fn push_user_message(lines: &mut Vec<Line<'static>>, text: &str, width: usize) {
    lines.push(Line::from(""));
    // Markdown-rendered like agent replies (headers, bold, lists, code
    // blocks) instead of plain wrap_text: a pasted structured prompt used to
    // show up as a flat, unstyled block of text with literal `#`/`**`/backticks.
    // Newlines are preserved: what the user pasted line-by-line must stay
    // line-by-line after submit (markdown soft breaks would merge them).
    //
    // The whole message renders as a visually distinct block — accent bar
    // on every line plus a card background on the text — so user turns
    // stand out when re-reading a session (they used to blend into the
    // agent output).
    let rendered = crate::tui::markdown::render_preserving_newlines(text, width.saturating_sub(6));
    let bar_style = Style::default().fg(theme::ACCENT);
    for (i, line) in rendered.into_iter().enumerate() {
        let prefix = if i == 0 {
            Span::styled(
                "  \u{258c}\u{276f} ",
                bar_style.add_modifier(ratatui::style::Modifier::BOLD),
            )
        } else {
            Span::styled("  \u{258c}  ", bar_style)
        };
        let mut spans = Vec::with_capacity(1 + line.spans.len());
        spans.push(prefix);
        spans.extend(
            line.spans
                .into_iter()
                .map(|s| Span::styled(s.content, s.style.bg(theme::BG_CARD))),
        );
        lines.push(Line::from(spans));
    }
}

fn push_tool_message(
    lines: &mut Vec<Line<'static>>,
    pending_tool_zones: &mut Vec<PendingToolZone>,
    msg: &ChatMessage,
    message_idx: usize,
    width: usize,
    spinner_frame: usize,
    mouse_capture_enabled: bool,
) {
    if let Some(ref info) = msg.tool {
        let header_line = lines.len() + 1;
        let expanded = should_render_tool_expanded(info);
        render_tool_message(lines, info, width, spinner_frame, mouse_capture_enabled);
        pending_tool_zones.push(PendingToolZone {
            line_idx: header_line,
            message_idx,
            can_toggle: info.status != ToolStatus::Running,
            can_copy: mouse_capture_enabled && copyable_tool_command(info).is_some(),
            expanded,
        });
    } else {
        lines.push(Line::from(vec![Span::styled(
            format!("  \u{2714} {}", msg.text),
            Style::default().fg(theme::YELLOW),
        )]));
    }
}
