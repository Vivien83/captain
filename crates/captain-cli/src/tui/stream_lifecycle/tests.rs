use super::*;
use captain_types::message::{ReplyDirectives, TokenUsage};

fn agent_result(response: &str, usage: TokenUsage, cost_usd: Option<f64>) -> AgentLoopResult {
    AgentLoopResult {
        response: response.to_string(),
        total_usage: usage,
        iterations: 1,
        cost_usd,
        silent: false,
        directives: ReplyDirectives::default(),
        tool_calls: Vec::new(),
    }
}

#[test]
fn prepare_stream_start_resets_turn_telemetry() {
    let mut chat = ChatState::new();
    chat.streaming_chars = 42;
    chat.context_stream_checkpoint_chars = Some(21);
    chat.last_tokens = Some((10, 20));
    chat.last_cached_input_tokens = 5;
    chat.last_cache_creation_tokens = 3;
    chat.last_cost_usd = Some(0.25);
    chat.status_msg = Some("old error".to_string());

    prepare_stream_start(&mut chat);

    assert!(chat.is_streaming);
    assert!(chat.thinking);
    assert_eq!(chat.streaming_chars, 0);
    assert!(chat.context_stream_checkpoint_chars.is_none());
    assert!(chat.last_tokens.is_none());
    assert_eq!(chat.last_cached_input_tokens, 0);
    assert_eq!(chat.last_cache_creation_tokens, 0);
    assert!(chat.last_cost_usd.is_none());
    assert!(chat.status_msg.is_none());
}

#[test]
fn success_finalizes_stream_and_avoids_duplicate_response() {
    let mut chat = ChatState::new();
    chat.is_streaming = true;
    chat.streaming_text = "already streamed".to_string();

    apply_stream_result(
        &mut chat,
        Ok(agent_result(
            "already streamed",
            TokenUsage::default(),
            None,
        )),
    );

    assert!(!chat.is_streaming);
    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages[0].text, "already streamed");
}

#[test]
fn success_pushes_non_streamed_response() {
    let mut chat = ChatState::new();

    apply_stream_result(
        &mut chat,
        Ok(agent_result("final answer", TokenUsage::default(), None)),
    );

    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages[0].text, "final answer");
}

#[test]
fn success_records_tokens_cache_and_cost() {
    let mut chat = ChatState::new();
    let usage = TokenUsage {
        input_tokens: 100,
        output_tokens: 40,
        cached_input_tokens: 30,
        cache_creation_tokens: 7,
    };

    apply_stream_result(&mut chat, Ok(agent_result("", usage, Some(0.12))));

    assert_eq!(chat.last_tokens, Some((100, 40)));
    assert_eq!(chat.current_context_tokens, 140);
    assert_eq!(chat.context_stream_checkpoint_chars, Some(0));
    assert_eq!(chat.last_cached_input_tokens, 30);
    assert_eq!(chat.last_cache_creation_tokens, 7);
    assert_eq!(chat.last_cost_usd, Some(0.12));
    assert_eq!(chat.session_input_tokens, 100);
    assert_eq!(chat.session_output_tokens, 40);
    assert_eq!(chat.session_cached_input_tokens, 30);
    assert_eq!(chat.session_cache_creation_tokens, 7);
    assert!((chat.session_cost_usd - 0.12).abs() < f64::EPSILON);
}

#[test]
fn success_with_zero_usage_keeps_cost_visible_without_counting_session_cost() {
    let mut chat = ChatState::new();

    apply_stream_result(
        &mut chat,
        Ok(agent_result("ok", TokenUsage::default(), Some(0.08))),
    );

    assert!(chat.last_tokens.is_none());
    assert_eq!(chat.last_cost_usd, Some(0.08));
    assert_eq!(chat.session_cost_usd, 0.0);
}

#[test]
fn error_finalizes_stream_and_sets_status() {
    let mut chat = ChatState::new();
    chat.is_streaming = true;
    chat.streaming_text = "partial".to_string();

    apply_stream_result(&mut chat, Err("network down".to_string()));

    assert!(!chat.is_streaming);
    assert_eq!(chat.messages.len(), 1);
    assert_eq!(chat.messages[0].text, "partial");
    assert_eq!(chat.status_msg.as_deref(), Some("Error: network down"));
}
