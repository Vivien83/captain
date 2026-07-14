//! Transcript rendering coordinator for the chat screen.

use super::{
    chat::ChatState,
    chat_transcript_empty::{captain_logo_lines, empty_transcript_lines},
    chat_transcript_layout::{
        pad_between_logo_and_tail, register_visible_tool_zones, scroll_indicator,
        transcript_scroll, PendingToolZone,
    },
    chat_transcript_live::push_live_transcript_lines,
    chat_transcript_messages::push_message_history_lines,
};
use crate::tui::theme;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

#[cfg(test)]
mod tests;

pub(super) fn draw_messages(f: &mut Frame, area: Rect, state: &mut ChatState) {
    state.tool_click_zones.clear();
    let width = area.width as usize;
    if width < 4 {
        return;
    }

    let visible_height = area.height;
    let transcript = build_transcript_lines(state, width, visible_height);
    if transcript.empty_state {
        f.render_widget(Paragraph::new(transcript.lines), area);
        return;
    }

    let scroll = transcript_scroll(
        transcript.lines.len(),
        visible_height,
        &mut state.scroll_offset,
    );
    register_visible_tool_zones(
        &transcript.pending_tool_zones,
        scroll.visible_start,
        scroll.visible_end,
        area,
        &mut state.tool_click_zones,
    );

    let para = Paragraph::new(transcript.lines).scroll((scroll.scroll, 0));
    f.render_widget(para, area);

    if let Some((indicator, ind_area)) = scroll_indicator(
        area,
        scroll.total_lines,
        visible_height,
        scroll.scroll,
        state.scroll_offset,
    ) {
        f.render_widget(
            Paragraph::new(Span::styled(indicator, theme::dim_style())),
            ind_area,
        );
    }
}

struct TranscriptLines {
    lines: Vec<Line<'static>>,
    pending_tool_zones: Vec<PendingToolZone>,
    empty_state: bool,
}

fn build_transcript_lines(state: &ChatState, width: usize, visible_height: u16) -> TranscriptLines {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut pending_tool_zones: Vec<PendingToolZone> = Vec::new();

    let logo_lines = captain_logo_lines(width);
    let logo_len = logo_lines.len();
    lines.extend(logo_lines);

    if state.messages.is_empty() && state.streaming_text.is_empty() && !state.thinking {
        let lines = empty_transcript_lines(lines, logo_len, state, width, visible_height as usize);
        return TranscriptLines {
            lines,
            pending_tool_zones,
            empty_state: true,
        };
    }

    push_message_history_lines(
        &mut lines,
        &mut pending_tool_zones,
        &state.messages,
        width,
        state.spinner_frame,
        state.mouse_capture_enabled,
    );
    push_live_transcript_lines(&mut lines, state, width);
    pad_between_logo_and_tail(
        &mut lines,
        &mut pending_tool_zones,
        visible_height,
        logo_len,
    );

    TranscriptLines {
        lines,
        pending_tool_zones,
        empty_state: false,
    }
}
