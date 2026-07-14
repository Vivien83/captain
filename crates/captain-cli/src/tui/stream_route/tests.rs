use super::*;
use captain_types::message::{StopReason, TokenUsage};
use serde_json::json;

#[test]
fn text_delta_clears_transient_tool_and_appends_visible_stream() {
    let mut chat = ChatState::new();
    chat.thinking = true;
    chat.active_tool = Some("shell_exec".to_string());

    apply_stream_event(
        &mut chat,
        StreamEvent::TextDelta {
            text: "hello".to_string(),
        },
    );

    assert!(!chat.thinking);
    assert!(chat.active_tool.is_none());
    assert_eq!(chat.streaming_text, "hello");
}

#[test]
fn tool_start_flushes_visible_stream_before_spinner() {
    let mut chat = ChatState::new();
    chat.streaming_text = "partial answer".to_string();

    apply_stream_event(
        &mut chat,
        StreamEvent::ToolUseStart {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
        },
    );

    assert!(chat.streaming_text.is_empty());
    assert_eq!(chat.messages.len(), 1);
    assert!(matches!(chat.messages[0].role, Role::Agent));
    assert_eq!(chat.messages[0].text, "partial answer");
    assert_eq!(chat.active_tool.as_deref(), Some("shell_exec"));
}

#[test]
fn tool_end_prefers_incremental_input_buffer() {
    let mut chat = ChatState::new();
    apply_stream_event(
        &mut chat,
        StreamEvent::ToolInputDelta {
            text: "{\"cmd\":\"cargo test\"}".to_string(),
        },
    );

    apply_stream_event(
        &mut chat,
        StreamEvent::ToolUseEnd {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
            input: json!({"cmd": "ignored"}),
        },
    );

    let tool = chat
        .messages
        .last()
        .and_then(|message| message.tool.as_ref());
    assert!(tool.is_some());
    let tool = tool.unwrap();
    assert_eq!(tool.input, "{\"cmd\":\"cargo test\"}");
    assert!(chat.tool_input_buf.is_empty());
}

#[test]
fn tool_end_falls_back_to_json_input_without_buffer() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::ToolUseEnd {
            id: "tool-1".to_string(),
            name: "shell_exec".to_string(),
            input: json!({"cmd": "cargo check"}),
        },
    );

    let tool = chat
        .messages
        .last()
        .and_then(|message| message.tool.as_ref())
        .unwrap();
    assert_eq!(tool.input, "{\"cmd\":\"cargo check\"}");
}

#[test]
fn content_complete_updates_token_usage() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 12,
                output_tokens: 7,
                cached_input_tokens: 5,
                cache_creation_tokens: 3,
            },
        },
    );

    assert_eq!(chat.last_tokens, Some((12, 7)));
    assert_eq!(chat.last_cached_input_tokens, 5);
    assert_eq!(chat.last_cache_creation_tokens, 3);
}

#[test]
fn phase_change_routes_tool_thinking_and_model_fallback() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::PhaseChange {
            phase: "tool_use".to_string(),
            detail: Some("browser".to_string()),
        },
    );
    assert_eq!(chat.active_tool.as_deref(), Some("browser"));

    apply_stream_event(
        &mut chat,
        StreamEvent::PhaseChange {
            phase: "thinking".to_string(),
            detail: None,
        },
    );
    assert!(chat.thinking);

    apply_stream_event(
        &mut chat,
        StreamEvent::PhaseChange {
            phase: "model_fallback".to_string(),
            detail: Some("Fallback model selected".to_string()),
        },
    );
    assert_eq!(
        chat.messages.last().map(|message| message.text.as_str()),
        Some("Fallback model selected")
    );

    apply_stream_event(
        &mut chat,
        StreamEvent::PhaseChange {
            phase: "model_fallback".to_string(),
            detail: Some("   ".to_string()),
        },
    );
    assert_eq!(chat.messages.len(), 1);
}

#[test]
fn thinking_delta_stays_out_of_visible_answer_stream() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::ThinkingDelta {
            text: "reasoning".to_string(),
        },
    );

    assert!(chat.thinking);
    assert_eq!(chat.thinking_text, "reasoning");
    assert!(chat.streaming_text.is_empty());
}

#[test]
fn intermediate_message_flushes_previous_stream_first() {
    let mut chat = ChatState::new();
    chat.streaming_text = "before".to_string();

    apply_stream_event(
        &mut chat,
        StreamEvent::IntermediateMessage {
            content: "after".to_string(),
        },
    );

    assert_eq!(chat.messages.len(), 2);
    assert_eq!(chat.messages[0].text, "before");
    assert_eq!(chat.messages[1].text, "after");
}

#[test]
fn ask_user_adds_visible_operator_question() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::AskUser {
            question: "Continue?".to_string(),
            options: None,
        },
    );

    assert_eq!(
        chat.messages.last().map(|message| message.text.as_str()),
        Some("\u{2753} Continue?")
    );
}

#[test]
fn ask_user_with_options_opens_modal_instead_of_a_message() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::AskUser {
            question: "Couleur ?".to_string(),
            options: Some(vec!["bleu".to_string(), "rouge".to_string()]),
        },
    );

    let pending = chat.pending_ask_user.as_ref().expect("pending ask_user");
    assert_eq!(pending.question, "Couleur ?");
    assert_eq!(pending.options, vec!["bleu", "rouge"]);
    // Non-regression: unlike the no-options case, nothing was pushed as a
    // plain chat message — the modal owns the question instead.
    assert!(chat.messages.is_empty());
}

#[test]
fn ask_user_tool_result_closes_running_tool_block() {
    let mut chat = ChatState::new();

    apply_stream_event(
        &mut chat,
        StreamEvent::ToolUseStart {
            id: "ask-1".to_string(),
            name: "ask_user".to_string(),
        },
    );
    apply_stream_event(
        &mut chat,
        StreamEvent::ToolUseEnd {
            id: "ask-1".to_string(),
            name: "ask_user".to_string(),
            input: json!({"question": "Continue?"}),
        },
    );
    apply_stream_event(
        &mut chat,
        StreamEvent::AskUser {
            question: "Continue?".to_string(),
            options: None,
        },
    );
    apply_stream_event(
        &mut chat,
        StreamEvent::ToolExecutionResult {
            tool_use_id: "ask-1".to_string(),
            name: "ask_user".to_string(),
            result_preview: "User response received.".to_string(),
            is_error: false,
        },
    );

    let tool = chat
        .messages
        .iter()
        .find_map(|message| message.tool.as_ref())
        .expect("ask_user tool block");
    assert_eq!(tool.name, "ask_user");
    assert_eq!(tool.result, "User response received.");
    assert!(matches!(
        tool.status,
        crate::tui::screens::chat::ToolStatus::Success
    ));
    assert!(chat.active_tool.is_none());
}
