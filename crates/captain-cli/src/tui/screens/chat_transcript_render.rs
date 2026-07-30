//! Transcript rendering coordinator for the chat screen.

use super::{
    chat::ChatState,
    chat_tool_message::tool_message_render_is_time_sensitive,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HistoryCacheKey {
    revision: u64,
    width: usize,
    running_tool_frame: Option<usize>,
    mouse_capture_enabled: bool,
}

/// Cached rendering of completed history. The live streaming tail remains
/// uncached, so every delta is exact while stable Markdown/tool blocks avoid
/// reparsing on every animation frame.
#[derive(Default)]
pub(super) struct TranscriptHistoryCache {
    key: Option<HistoryCacheKey>,
    lines: Vec<Line<'static>>,
    pending_tool_zones: Vec<PendingToolZone>,
    hits: u64,
    misses: u64,
}

impl TranscriptHistoryCache {
    fn rendered(
        &mut self,
        messages: &[super::chat::ChatMessage],
        revision: u64,
        width: usize,
        spinner_frame: usize,
        mouse_capture_enabled: bool,
    ) -> (Vec<Line<'static>>, Vec<PendingToolZone>) {
        let time_sensitive = messages.iter().any(|message| {
            message
                .tool
                .as_ref()
                .is_some_and(tool_message_render_is_time_sensitive)
        });
        let running_tool_frame = messages
            .iter()
            .any(|message| {
                message
                    .tool
                    .as_ref()
                    .is_some_and(|tool| tool.status == super::chat::ToolStatus::Running)
            })
            .then_some(spinner_frame);
        let key = HistoryCacheKey {
            revision,
            width,
            running_tool_frame,
            mouse_capture_enabled,
        };

        if !time_sensitive && self.key == Some(key) {
            self.hits = self.hits.saturating_add(1);
            return (self.lines.clone(), self.pending_tool_zones.clone());
        }

        let mut lines = Vec::new();
        let mut pending_tool_zones = Vec::new();
        push_message_history_lines(
            &mut lines,
            &mut pending_tool_zones,
            messages,
            width,
            spinner_frame,
            mouse_capture_enabled,
        );
        self.key = (!time_sensitive).then_some(key);
        self.lines = lines;
        self.pending_tool_zones = pending_tool_zones;
        self.misses = self.misses.saturating_add(1);
        (self.lines.clone(), self.pending_tool_zones.clone())
    }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
    }

    pub(super) fn clear(&mut self) {
        self.key = None;
        self.lines.clear();
        self.pending_tool_zones.clear();
    }

    #[cfg(test)]
    fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }
}

fn build_transcript_lines(
    state: &mut ChatState,
    width: usize,
    visible_height: u16,
) -> TranscriptLines {
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

    let history_offset = lines.len();
    let (history_lines, mut history_tool_zones) = state.transcript_history_cache.rendered(
        &state.messages,
        state.transcript_history_revision,
        width,
        state.spinner_frame,
        state.mouse_capture_enabled,
    );
    for zone in &mut history_tool_zones {
        zone.line_idx = zone.line_idx.saturating_add(history_offset);
    }
    lines.extend(history_lines);
    pending_tool_zones.extend(history_tool_zones);
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
