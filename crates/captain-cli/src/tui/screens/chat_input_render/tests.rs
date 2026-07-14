use super::*;

fn state_with_input(input: &str, cursor: usize) -> ChatState {
    let mut state = ChatState::new();
    state.input = input.to_string();
    state.input_cursor = cursor;
    state
}

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
fn empty_input_renders_prompt_and_cursor() {
    let state = state_with_input("", 0);

    assert_eq!(line_texts(&build_input_lines(&state)), vec![" > \u{2588}"]);
}

#[test]
fn multiline_input_uses_continuation_prefix_and_cursor_line() {
    let state = state_with_input("first\nsecond", "first\nsec".len());

    assert_eq!(
        line_texts(&build_input_lines(&state)),
        vec![" > first", "   sec\u{2588}ond"]
    );
}

#[test]
fn slash_command_is_split_when_cursor_is_on_another_line() {
    let state = state_with_input("/status now\nnext", "/status now\nn".len());
    let lines = build_input_lines(&state);

    assert_eq!(
        line_texts(&lines),
        vec![" > /status now", "   n\u{2588}ext"]
    );
    assert_eq!(lines[0].spans[1].content.as_ref(), "/status");
    assert_eq!(lines[0].spans[2].content.as_ref(), " now");
}

#[test]
fn streaming_staged_badge_is_appended_to_last_line() {
    let mut state = state_with_input("hello", 5);
    state.is_streaming = true;
    state.staged_messages.push("queued".to_string());
    state.staged_messages.push("queued again".to_string());

    assert_eq!(
        line_texts(&build_input_lines(&state)),
        vec![" > hello\u{2588}  (2 staged)"]
    );
}

#[test]
fn cursor_split_respects_utf8_boundaries() {
    let state = state_with_input("caf\u{e9}", 4);

    assert_eq!(
        line_texts(&build_input_lines(&state)),
        vec![" > caf\u{2588}\u{e9}"]
    );
}

#[test]
fn raw_input_lines_preserves_empty_and_explicit_newlines() {
    assert_eq!(raw_input_lines(""), vec![""]);
    assert_eq!(raw_input_lines("one\ntwo\n"), vec!["one", "two", ""]);
}

#[test]
fn slash_highlight_is_disabled_on_cursor_line() {
    assert!(should_highlight_slash_command(0, "/status now", false));
    assert!(!should_highlight_slash_command(0, "/status now", true));
    assert!(!should_highlight_slash_command(1, "/status now", false));
}
