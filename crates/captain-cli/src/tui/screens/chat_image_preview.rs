//! Inline staged-image previews for the chat screen.

use super::chat::{ChatState, PendingAttachment};
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

#[cfg(test)]
mod tests;

/// Height of the staged-attachment preview strip.
///
/// Returns 0 when there is nothing renderable. Returns 8 rows when at least one
/// image attachment is staged with a local path, enough for a recognisable
/// thumbnail without crowding the messages area.
pub(super) fn staged_image_preview_rows(state: &ChatState) -> u16 {
    let any_image = state.pending_attachments.iter().any(|att| {
        crate::tui::image_preview::is_renderable_image(&att.content_type)
            && att.local_path.is_some()
    });
    if any_image {
        8
    } else {
        0
    }
}

/// Render thumbnails for every staged image attachment side by side.
pub(super) fn draw_staged_image_previews(
    f: &mut Frame,
    area: Rect,
    state: &ChatState,
    cache: &mut crate::tui::image_preview::ImagePreviewCache,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let images: Vec<&PendingAttachment> = state
        .pending_attachments
        .iter()
        .filter(|att| {
            crate::tui::image_preview::is_renderable_image(&att.content_type)
                && att.local_path.is_some()
        })
        .collect();
    if images.is_empty() {
        return;
    }
    let max_slot_w: u16 = 24;
    let count = images.len() as u16;
    let slot_w = (area.width / count).min(max_slot_w).max(8);
    for (i, att) in images.iter().enumerate() {
        let x = area.x + i as u16 * slot_w;
        if x + slot_w > area.x + area.width {
            break;
        }
        let slot = Rect::new(x, area.y, slot_w, area.height);
        let path = att.local_path.as_ref().expect("filtered above");
        if let Some(protocol) = cache.get_or_load(path) {
            let widget = ratatui_image::StatefulImage::default();
            f.render_stateful_widget(widget, slot, protocol);
        } else {
            let label = Paragraph::new(Line::from(vec![Span::styled(
                format!("📎 {}", att.filename),
                theme::dim_style(),
            )]));
            f.render_widget(label, slot);
        }
    }
}
