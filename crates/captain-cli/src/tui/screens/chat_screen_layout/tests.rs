use super::*;

#[test]
fn screen_areas_reserve_preview_input_and_footer_rows() {
    let areas = chat_screen_areas(Rect::new(2, 3, 80, 20), "hello", 2);

    assert_eq!(areas.messages, Rect::new(2, 3, 80, 15));
    assert_eq!(areas.separator, Rect::new(2, 18, 80, 1));
    assert_eq!(areas.preview, Rect::new(2, 19, 80, 2));
    assert_eq!(areas.input, Rect::new(2, 21, 80, 1));
    assert_eq!(areas.footer, Rect::new(2, 22, 80, 1));
}

#[test]
fn screen_areas_clamp_wrapped_input_to_ten_rows() {
    let input = "x".repeat(200);
    let areas = chat_screen_areas(Rect::new(0, 0, 20, 30), &input, 0);

    assert_eq!(areas.input.height, 10);
    assert_eq!(areas.footer.height, 1);
    assert_eq!(areas.messages.height, 18);
}

#[test]
fn reasoning_areas_leave_messages_untouched_without_thinking() {
    let messages = Rect::new(0, 1, 80, 12);
    let areas = reasoning_areas(messages, false, true);

    assert_eq!(areas.thinking, None);
    assert_eq!(areas.messages, messages);
}

#[test]
fn reasoning_areas_collapsed_reserve_one_row() {
    let areas = reasoning_areas(Rect::new(0, 1, 80, 12), true, false);

    assert_eq!(areas.thinking, Some(Rect::new(0, 1, 80, 1)));
    assert_eq!(areas.messages, Rect::new(0, 2, 80, 11));
}

#[test]
fn reasoning_areas_expanded_clamp_to_half_with_bounds() {
    let small = reasoning_areas(Rect::new(0, 0, 80, 4), true, true);
    assert_eq!(small.thinking, Some(Rect::new(0, 0, 80, 3)));
    assert_eq!(small.messages, Rect::new(0, 3, 80, 1));

    let large = reasoning_areas(Rect::new(0, 0, 80, 40), true, true);
    assert_eq!(large.thinking, Some(Rect::new(0, 0, 80, 12)));
    assert_eq!(large.messages, Rect::new(0, 12, 80, 28));
}

#[test]
fn separator_line_uses_requested_width() {
    let line = separator_line(5);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert_eq!(text, "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
}
