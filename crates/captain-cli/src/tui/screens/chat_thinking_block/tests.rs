use super::*;

fn line_texts(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn collapsed_block_renders_only_header() {
    let lines = thinking_lines("reasoning", false, 80, 10);

    assert_eq!(
        line_texts(&lines),
        vec!["  \u{25b6} \u{1f4ad} reasoning  (9 chars)  [Ctrl+T] toggle"]
    );
}

#[test]
fn expanded_block_includes_wrapped_body_lines() {
    let lines = thinking_lines("alpha beta gamma", true, 14, 10);

    assert_eq!(
        line_texts(&lines),
        vec![
            "  \u{25bc} \u{1f4ad} reasoning  (16 chars)  [Ctrl+T] toggle",
            "    alpha beta",
            "    gamma"
        ]
    );
}

#[test]
fn expanded_block_keeps_newline_boundaries() {
    let lines = thinking_lines("one\ntwo", true, 80, 10);

    assert_eq!(
        line_texts(&lines),
        vec![
            "  \u{25bc} \u{1f4ad} reasoning  (7 chars)  [Ctrl+T] toggle",
            "    one",
            "    two"
        ]
    );
}

#[test]
fn expanded_block_trims_to_visible_height_like_legacy_renderer() {
    let lines = thinking_lines("one\ntwo\nthree", true, 80, 2);

    assert_eq!(line_texts(&lines), vec!["    two", "    three"]);
}

#[test]
fn narrow_body_width_keeps_only_header() {
    let lines = thinking_lines("hidden", true, 8, 10);

    assert_eq!(
        line_texts(&lines),
        vec!["  \u{25bc} \u{1f4ad} reasoning  (6 chars)  [Ctrl+T] toggle"]
    );
}
