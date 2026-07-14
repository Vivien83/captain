//! Transcript layout helpers for chat messages.

use super::chat::{ToolClickAction, ToolClickZone};
use ratatui::layout::Rect;
use ratatui::text::Line;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingToolZone {
    pub(super) line_idx: usize,
    pub(super) message_idx: usize,
    pub(super) can_toggle: bool,
    pub(super) can_copy: bool,
    pub(super) expanded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TranscriptScroll {
    pub(super) total_lines: u16,
    pub(super) scroll: u16,
    pub(super) visible_start: usize,
    pub(super) visible_end: usize,
}

pub(super) fn pad_between_logo_and_tail(
    lines: &mut Vec<Line<'static>>,
    pending_tool_zones: &mut [PendingToolZone],
    visible_height: u16,
    logo_len: usize,
) {
    if (lines.len() as u16) >= visible_height {
        return;
    }

    let pad = visible_height - lines.len() as u16;
    let split_at = logo_len.min(lines.len());
    for zone in pending_tool_zones {
        if zone.line_idx >= split_at {
            zone.line_idx += pad as usize;
        }
    }

    let mut padded: Vec<Line<'static>> = Vec::with_capacity(visible_height as usize);
    let tail = lines.split_off(split_at);
    padded.append(lines);
    for _ in 0..pad {
        padded.push(Line::from(""));
    }
    padded.extend(tail);
    *lines = padded;
}

pub(super) fn transcript_scroll(
    line_count: usize,
    visible_height: u16,
    scroll_offset: &mut u16,
) -> TranscriptScroll {
    let total_lines = line_count as u16;
    let max_scroll = total_lines.saturating_sub(visible_height);
    if *scroll_offset > max_scroll {
        *scroll_offset = max_scroll;
    }
    let scroll = max_scroll.saturating_sub(*scroll_offset).min(max_scroll);
    let visible_start = scroll as usize;
    let visible_end = visible_start + visible_height as usize;
    TranscriptScroll {
        total_lines,
        scroll,
        visible_start,
        visible_end,
    }
}

pub(super) fn register_visible_tool_zones(
    pending_tool_zones: &[PendingToolZone],
    visible_start: usize,
    visible_end: usize,
    area: Rect,
    tool_click_zones: &mut Vec<ToolClickZone>,
) {
    let max_x = area.x.saturating_add(area.width.saturating_sub(1));
    for zone in pending_tool_zones {
        if zone.line_idx < visible_start || zone.line_idx >= visible_end {
            continue;
        }

        let y = area.y + (zone.line_idx - visible_start) as u16;
        let mut push_zone = |start: u16, end: u16, action: ToolClickAction| {
            let x_start = area.x.saturating_add(start).min(max_x);
            let x_end = area.x.saturating_add(end).min(max_x);
            if x_start <= x_end {
                tool_click_zones.push(ToolClickZone {
                    x_start,
                    x_end,
                    y,
                    message_idx: zone.message_idx,
                    action,
                });
            }
        };

        if zone.can_copy {
            let (start, end) = if zone.expanded { (4, 9) } else { (2, 7) };
            push_zone(start, end, ToolClickAction::CopyCommand);
        }
        if zone.can_toggle {
            let start = if zone.can_copy {
                if zone.expanded {
                    11
                } else {
                    9
                }
            } else {
                0
            };
            push_zone(start, start.saturating_add(14), ToolClickAction::Toggle);
        }
    }
}

pub(super) fn scroll_indicator(
    area: Rect,
    total_lines: u16,
    visible_height: u16,
    scroll: u16,
    scroll_offset: u16,
) -> Option<(String, Rect)> {
    if scroll_offset == 0 || total_lines <= visible_height {
        return None;
    }

    let above = scroll;
    let below = total_lines.saturating_sub(scroll + visible_height);
    let indicator = format!("{}↑ {}↓", above, below);
    let area = Rect {
        x: area.x + area.width.saturating_sub(indicator.len() as u16 + 1),
        y: area.y + area.height.saturating_sub(1),
        width: indicator.len() as u16,
        height: 1,
    };
    Some((indicator, area))
}
