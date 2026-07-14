use super::*;

fn line_texts(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn streaming_text_is_rendered_after_blank_separator() {
    let mut state = ChatState::new();
    state.streaming_text = "hello **captain**".to_string();
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);
    let text = line_texts(&lines);

    assert_eq!(text.first().map(String::as_str), Some(""));
    assert!(text.iter().any(|line| line.contains("hello captain")));
}

#[test]
fn thinking_and_active_tool_use_current_spinner_frame() {
    let mut state = ChatState::new();
    state.thinking = true;
    state.active_tool = Some("shell_exec".to_string());
    state.spinner_frame = 1;
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);
    let text = line_texts(&lines);

    assert!(text.iter().any(|line| line.contains("thinking…")));
    assert!(text.iter().any(|line| line.contains("shell_exec")));
}

#[test]
fn streaming_token_estimate_and_last_cost_are_appended() {
    let mut state = ChatState::new();
    state.is_streaming = true;
    state.streaming_chars = 41;
    state.last_tokens = Some((100, 25));
    state.last_cost_usd = Some(0.01234);
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);
    let text = line_texts(&lines);

    assert!(text.iter().any(|line| line == "  ~10 tokens"));
    assert!(text
        .iter()
        .any(|line| line == "  [tokens: 100 in / 25 out | $0.0123]"));
}

#[test]
fn empty_last_token_counts_are_not_rendered() {
    let mut state = ChatState::new();
    state.last_tokens = Some((0, 0));
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);

    assert!(lines.is_empty());
}

#[test]
fn status_message_is_appended_as_plain_operator_line() {
    let mut state = ChatState::new();
    state.status_msg = Some("network unavailable".to_string());
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);
    let text = line_texts(&lines);

    assert_eq!(text, vec!["  network unavailable".to_string()]);
}

#[test]
fn operator_notices_are_rendered_as_live_non_history_lines() {
    let mut state = ChatState::new();
    state.push_operator_notice(vec![
        "Agent API provisioned".to_string(),
        "Token: secret-token-shown-once".to_string(),
    ]);
    let mut lines = Vec::new();

    push_live_transcript_lines(&mut lines, &state, 80);
    let text = line_texts(&lines);

    assert!(state.messages.is_empty());
    assert!(text
        .iter()
        .any(|line| line.contains("Agent API provisioned")));
    assert!(text
        .iter()
        .any(|line| line.contains("Token: secret-token-shown-once")));
}

#[test]
fn streaming_token_estimate_label_uses_four_chars_per_token() {
    assert_eq!(streaming_token_estimate_label(41), "  ~10 tokens");
}

#[test]
fn last_token_usage_label_omits_empty_counts_and_zero_cost() {
    assert_eq!(last_token_usage_label(0, 0, Some(0.05)), None);
    assert_eq!(
        last_token_usage_label(100, 25, Some(0.0)),
        Some("  [tokens: 100 in / 25 out]".to_string())
    );
}
