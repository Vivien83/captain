use super::*;
use crate::tui::screens::chat::{ChatMessage, Role, ToolInfo, ToolStatus};

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
    let mut state = ChatState::new();

    let transcript = build_transcript_lines(&mut state, 80, 30);

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

    let transcript = build_transcript_lines(&mut state, 80, 24);
    let text = joined_lines(&transcript.lines);

    assert!(!transcript.empty_state);
    assert!(transcript.pending_tool_zones.is_empty());
    assert!(text.contains("hello from the user"));
    assert!(text.contains("live agent text"));
    assert!(text.contains("~10 tokens"));
    assert!(transcript.lines.len() >= 24);
}

#[test]
fn static_long_history_is_parsed_once_across_one_hundred_stream_frames() {
    let mut state = ChatState::new();
    for index in 0..200 {
        state.push_message(
            if index % 2 == 0 {
                Role::User
            } else {
                Role::Agent
            },
            format!("message **{index}** with enough text to exercise markdown wrapping"),
        );
    }

    let first = build_transcript_lines(&mut state, 100, 36);
    assert!(joined_lines(&first.lines).contains("message 199"));
    assert_eq!(state.transcript_history_cache.stats(), (0, 1));

    for index in 0..100 {
        state.streaming_text = format!("live tail {index}");
        let transcript = build_transcript_lines(&mut state, 100, 36);
        assert!(joined_lines(&transcript.lines).contains(&state.streaming_text));
    }

    assert_eq!(state.transcript_history_cache.stats(), (100, 1));
}

#[test]
fn history_mutation_and_resize_each_force_one_exact_rebuild() {
    let mut state = ChatState::new();
    state.push_message(Role::Agent, "first".to_string());

    build_transcript_lines(&mut state, 80, 24);
    build_transcript_lines(&mut state, 80, 24);
    state.push_message(Role::Agent, "second".to_string());
    let changed = build_transcript_lines(&mut state, 80, 24);
    let resized = build_transcript_lines(&mut state, 60, 24);

    assert!(joined_lines(&changed.lines).contains("second"));
    assert!(joined_lines(&resized.lines).contains("second"));
    assert_eq!(state.transcript_history_cache.stats(), (1, 3));
}

#[test]
fn success_grace_period_is_never_frozen_in_the_history_cache() {
    let mut state = ChatState::new();
    state.messages.push(ChatMessage {
        role: Role::Tool,
        text: "shell_exec".to_string(),
        tool: Some(ToolInfo {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
            input: r#"{"command":"true"}"#.to_string(),
            result: "ok".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            is_error: false,
            status: ToolStatus::Success,
            started_at: None,
            completed_at: Some(std::time::Instant::now()),
            duration_ms: Some(10),
            expanded: false,
        }),
    });

    let during_grace = build_transcript_lines(&mut state, 80, 24);
    assert!(during_grace.pending_tool_zones[0].expanded);
    assert_eq!(state.transcript_history_cache.stats(), (0, 1));

    state.messages[0].tool.as_mut().unwrap().completed_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(5));
    let after_grace = build_transcript_lines(&mut state, 80, 24);

    assert!(!after_grace.pending_tool_zones[0].expanded);
    assert_eq!(state.transcript_history_cache.stats(), (0, 2));
}
