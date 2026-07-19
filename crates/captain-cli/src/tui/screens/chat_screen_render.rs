//! Chat screen rendering coordinator.

use super::{
    chat::ChatState,
    chat_footer::draw_input_footer,
    chat_image_preview::{draw_staged_image_previews, staged_image_preview_rows},
    chat_input_render::draw_chat_input,
    chat_model_picker::draw_model_picker,
    chat_quick_action_prompt::draw_quick_action_prompt,
    chat_screen_layout::{chat_screen_areas, draw_separator, reasoning_areas, ChatScreenAreas},
    chat_session_picker::draw_session_picker,
    chat_slash_picker::draw_slash_picker_live,
    chat_status_line::{build_provider_quota_lines, build_status_line, draw_provider_quota_status},
    chat_thinking_block::draw_thinking,
    chat_transcript_render::draw_messages,
};
use crate::tui::{image_preview::ImagePreviewCache, theme};
use ratatui::{
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Padding},
    Frame,
};

#[cfg(test)]
mod tests;

pub(super) fn draw_chat_screen(
    f: &mut Frame,
    area: Rect,
    state: &mut ChatState,
    image_cache: &mut ImagePreviewCache,
) {
    let inner = draw_chat_frame(f, area, state);
    let provider_quota_lines = build_provider_quota_lines(state, inner.width as usize);
    let areas = draw_chat_body(f, inner, state, image_cache, provider_quota_lines);
    draw_chat_overlays(f, inner, areas.input, state);
}

fn draw_chat_frame(f: &mut Frame, area: Rect, state: &ChatState) -> Rect {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" {} ", state.agent_name),
            theme::title_style(),
        )]))
        .title_alignment(Alignment::Left)
        .title_bottom(build_status_line(state))
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}

fn draw_chat_body(
    f: &mut Frame,
    inner: Rect,
    state: &mut ChatState,
    image_cache: &mut ImagePreviewCache,
    provider_quota_lines: Vec<Line<'static>>,
) -> ChatScreenAreas {
    let preview_rows = staged_image_preview_rows(state);
    let quota_rows = provider_quota_lines.len() as u16;
    let areas = chat_screen_areas(inner, &state.input, preview_rows, quota_rows);

    if preview_rows > 0 {
        draw_staged_image_previews(f, areas.preview, state, image_cache);
    }

    draw_chat_transcript(f, areas.messages, state);
    draw_separator(f, areas.separator);
    draw_chat_input(f, areas.input, state);
    draw_input_footer(f, areas.footer, state);
    draw_provider_quota_status(f, areas.provider_quota, provider_quota_lines);
    areas
}

fn draw_chat_transcript(f: &mut Frame, area: Rect, state: &mut ChatState) {
    let transcript_areas = reasoning_areas(
        area,
        !state.thinking_text.is_empty(),
        state.thinking_expanded,
    );
    if let Some(thinking_area) = transcript_areas.thinking {
        draw_thinking(f, thinking_area, state);
    }
    draw_messages(f, transcript_areas.messages, state);
}

fn draw_chat_overlays(f: &mut Frame, inner: Rect, input: Rect, state: &mut ChatState) {
    let overlays = chat_overlay_state(state);
    if overlays.slash_picker {
        draw_slash_picker_live(f, input, state);
    }
    if overlays.model_picker {
        draw_model_picker(f, inner, state);
    }
    if overlays.session_picker {
        draw_session_picker(f, inner, state);
    }

    state.quick_action_click_zones.clear();
    if overlays.quick_action {
        draw_quick_action_prompt(f, inner, state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChatOverlayState {
    slash_picker: bool,
    model_picker: bool,
    session_picker: bool,
    quick_action: bool,
}

fn chat_overlay_state(state: &ChatState) -> ChatOverlayState {
    ChatOverlayState {
        slash_picker: state.slash_picker_active(),
        model_picker: state.show_model_picker,
        session_picker: state.show_session_picker,
        // Delegates to ChatState::has_quick_action_prompt() (single source
        // of truth also used for key routing) instead of re-listing pending
        // states here — the previous hand-copied condition silently missed
        // pending_ask_user (T2) and left the modal never rendering.
        quick_action: state.has_quick_action_prompt(),
    }
}
