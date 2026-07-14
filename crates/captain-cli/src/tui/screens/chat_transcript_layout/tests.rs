use super::*;

#[test]
fn padding_keeps_logo_at_top_and_shifts_tool_zones() {
    let mut lines = vec![Line::from("logo"), Line::from("message")];
    let mut zones = vec![PendingToolZone {
        line_idx: 1,
        message_idx: 7,
        can_toggle: true,
        can_copy: false,
        expanded: false,
    }];

    pad_between_logo_and_tail(&mut lines, &mut zones, 5, 1);

    assert_eq!(lines.len(), 5);
    assert_eq!(zones[0].line_idx, 4);
}

#[test]
fn transcript_scroll_clamps_offset_and_reports_visible_range() {
    let mut offset = 99;
    let scroll = transcript_scroll(10, 4, &mut offset);

    assert_eq!(offset, 6);
    assert_eq!(scroll.total_lines, 10);
    assert_eq!(scroll.scroll, 0);
    assert_eq!(scroll.visible_start, 0);
    assert_eq!(scroll.visible_end, 4);

    offset = 2;
    let scroll = transcript_scroll(10, 4, &mut offset);
    assert_eq!(scroll.scroll, 4);
    assert_eq!(scroll.visible_start, 4);
    assert_eq!(scroll.visible_end, 8);
}

#[test]
fn visible_tool_zones_map_relative_columns_to_screen_coordinates() {
    let pending = vec![PendingToolZone {
        line_idx: 5,
        message_idx: 3,
        can_toggle: true,
        can_copy: true,
        expanded: true,
    }];
    let mut zones = Vec::new();

    register_visible_tool_zones(&pending, 4, 8, Rect::new(10, 20, 30, 5), &mut zones);

    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0].x_start, 14);
    assert_eq!(zones[0].x_end, 19);
    assert_eq!(zones[0].y, 21);
    assert_eq!(zones[0].message_idx, 3);
    assert_eq!(zones[0].action, ToolClickAction::CopyCommand);
    assert_eq!(zones[1].x_start, 21);
    assert_eq!(zones[1].x_end, 35);
    assert_eq!(zones[1].action, ToolClickAction::Toggle);
}

#[test]
fn scroll_indicator_reports_above_and_below_counts() {
    let (label, area) = scroll_indicator(Rect::new(2, 3, 20, 8), 30, 8, 12, 10).unwrap();

    assert_eq!(label, "12↑ 10↓");
    assert_eq!(area.x, 10);
    assert_eq!(area.y, 10);
    assert_eq!(area.width, label.len() as u16);
    assert_eq!(area.height, 1);
}

#[test]
fn scroll_indicator_is_hidden_at_bottom() {
    assert!(scroll_indicator(Rect::new(0, 0, 20, 8), 30, 8, 22, 0).is_none());
}
