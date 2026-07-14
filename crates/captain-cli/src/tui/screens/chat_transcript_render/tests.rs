use super::*;
use crate::tui::screens::chat::{ChatMessage, Role};

fn joined_lines(lines: &[Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn empty_transcript_is_marked_and_padded_without_tool_zones() {
    let state = ChatState::new();

    let transcript = build_transcript_lines(&state, 80, 30);

    assert!(transcript.empty_state);
    assert!(transcript.pending_tool_zones.is_empty());
    assert!(transcript.lines.len() >= 30);
}

#[test]
fn transcript_lines_keep_history_live_text_and_streaming_usage() {
    let mut state = ChatState::new();
    state.messages.push(ChatMessage {
        role: Role::User,
        text: "hello from the user".to_string(),
        tool: None,
    });
    state.streaming_text = "live agent text".to_string();
    state.is_streaming = true;
    state.streaming_chars = 40;

    let transcript = build_transcript_lines(&state, 80, 24);
    let text = joined_lines(&transcript.lines);

    assert!(!transcript.empty_state);
    assert!(transcript.pending_tool_zones.is_empty());
    assert!(text.contains("hello from the user"));
    assert!(text.contains("live agent text"));
    assert!(text.contains("~10 tokens"));
    assert!(transcript.lines.len() >= 24);
}
