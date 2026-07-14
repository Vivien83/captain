use super::*;

#[test]
fn empty_input_reserves_one_row() {
    assert_eq!(compute_input_visual_rows("", 80), 1);
}

#[test]
fn short_line_reserves_one_row() {
    assert_eq!(compute_input_visual_rows("hello", 80), 1);
}

#[test]
fn explicit_newlines_each_count_as_a_row() {
    assert_eq!(compute_input_visual_rows("a\nb\nc", 80), 3);
}

#[test]
fn long_single_line_wraps_to_multiple_rows() {
    let long = "x".repeat(50);
    assert_eq!(compute_input_visual_rows(&long, 20), 4);
}

#[test]
fn very_narrow_viewport_does_not_panic_or_div_zero() {
    assert_eq!(compute_input_visual_rows("ab", 1), 2);
    assert_eq!(compute_input_visual_rows("", 0), 1);
}

#[test]
fn locate_cursor_handles_multiline() {
    assert_eq!(locate_cursor("ab\ncd", 4), (1, 1));
    assert_eq!(locate_cursor("ab\ncd", 0), (0, 0));
    assert_eq!(locate_cursor("ab\ncd", 2), (0, 2));
    assert_eq!(locate_cursor("ab\ncd", 3), (1, 0));
    assert_eq!(locate_cursor("", 0), (0, 0));
}

#[test]
fn locate_cursor_clamps_overflow() {
    assert_eq!(locate_cursor("abc", 10), (0, 3));
}
