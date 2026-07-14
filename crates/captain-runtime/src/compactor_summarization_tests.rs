use crate::compactor::*;
use crate::compactor_summarization::{
    build_conversation_text, compute_adaptive_chunk_ratio, is_oversized, summarize_in_chunks,
};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use captain_types::message::{ContentBlock, Message, MessageContent, Role, TokenUsage};
use std::sync::Arc;

#[test]
fn test_adaptive_chunk_ratio_short_messages() {
    let config = CompactionConfig::default();
    let messages: Vec<Message> = (0..50).map(|i| Message::user(format!("msg {i}"))).collect();
    let ratio = compute_adaptive_chunk_ratio(&messages, &config);
    assert!(
        (ratio - config.base_chunk_ratio).abs() < f64::EPSILON,
        "Short messages should use base ratio, got {ratio}"
    );
}

#[test]
fn test_adaptive_chunk_ratio_long_messages() {
    let config = CompactionConfig::default();
    let messages: Vec<Message> = (0..20).map(|_| Message::user("x".repeat(1500))).collect();
    let ratio = compute_adaptive_chunk_ratio(&messages, &config);
    assert!(
        (ratio - config.min_chunk_ratio).abs() < f64::EPSILON,
        "Long messages should use min ratio, got {ratio}"
    );
}

#[test]
fn test_adaptive_chunk_ratio_medium_messages() {
    let config = CompactionConfig::default();
    let messages: Vec<Message> = (0..20).map(|_| Message::user("y".repeat(700))).collect();
    let ratio = compute_adaptive_chunk_ratio(&messages, &config);
    let expected = (config.base_chunk_ratio + config.min_chunk_ratio) / 2.0;
    assert!(
        (ratio - expected).abs() < f64::EPSILON,
        "Medium messages should use middle ratio, got {ratio}"
    );
}

#[test]
fn test_adaptive_chunk_ratio_empty() {
    let config = CompactionConfig::default();
    let messages: Vec<Message> = vec![];
    let ratio = compute_adaptive_chunk_ratio(&messages, &config);
    assert!(
        (ratio - config.base_chunk_ratio).abs() < f64::EPSILON,
        "Empty messages should default to base ratio"
    );
}

#[test]
fn test_oversized_message_detection() {
    let config = CompactionConfig::default();
    let small_msg = Message::user("short");
    assert!(!is_oversized(&small_msg, &config));

    let large_msg = Message::user("x".repeat(50_000));
    assert!(is_oversized(&large_msg, &config));

    let boundary_msg = Message::user("x".repeat(40_000));
    assert!(!is_oversized(&boundary_msg, &config));

    let just_over = Message::user("x".repeat(40_001));
    assert!(is_oversized(&just_over, &config));
}

#[tokio::test]
async fn test_chunked_summarization_splits_correctly() {
    use std::sync::atomic::{AtomicU32, Ordering};

    static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

    struct CountingDriver;

    #[async_trait]
    impl LlmDriver for CountingDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let n = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            Ok(text_response(&format!("Chunk summary {n}")))
        }
    }

    CALL_COUNT.store(0, Ordering::SeqCst);

    let messages: Vec<Message> = (0..20)
        .map(|i| Message::user(format!("Message {i}")))
        .collect();
    let config = CompactionConfig::default();

    let result = summarize_in_chunks(Arc::new(CountingDriver), "test-model", &messages, &config)
        .await
        .unwrap();

    let calls = CALL_COUNT.load(Ordering::SeqCst);
    assert!(
        calls >= 2,
        "Should have made multiple LLM calls for chunked summary, got {calls}"
    );
    assert!(!result.is_empty(), "Should produce a summary");
}

#[test]
fn test_build_conversation_text_handles_all_blocks() {
    let config = CompactionConfig::default();
    let messages = vec![
        Message::user("Hello"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![
                ContentBlock::Text {
                    text: "Let me search".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::ToolUse {
                    id: "tu-1".to_string(),
                    name: "web_search".to_string(),
                    input: serde_json::json!({"query": "rust"}),
                    provider_metadata: None,
                },
            ]),
        },
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "tu-1".to_string(),
                tool_name: String::new(),
                content: "Results found".to_string(),
                is_error: false,
            }]),
        },
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "base64data".to_string(),
            }]),
        },
    ];

    let text = build_conversation_text(&messages, &config);
    assert!(text.contains("User: Hello"));
    assert!(text.contains("Assistant: Let me search"));
    assert!(text.contains("web_search"));
    assert!(text.contains("Tool result (OK)"));
    assert!(text.contains("[Image: image/png]"));
}

#[test]
fn test_build_conversation_text_truncates_oversized() {
    let config = CompactionConfig {
        max_chunk_chars: 1000,
        ..CompactionConfig::default()
    };

    let large_msg = Message::user("x".repeat(2000));
    let messages = vec![large_msg];
    let text = build_conversation_text(&messages, &config);
    assert!(
        text.contains("truncated from"),
        "Oversized message should be truncated, got: {}",
        crate::str_utils::safe_truncate_str(&text, 200)
    );
}

#[test]
fn test_context_pressure_from_percent() {
    assert_eq!(ContextPressure::from_percent(30.0), ContextPressure::Low);
    assert_eq!(ContextPressure::from_percent(55.0), ContextPressure::Medium);
    assert_eq!(ContextPressure::from_percent(75.0), ContextPressure::High);
    assert_eq!(
        ContextPressure::from_percent(90.0),
        ContextPressure::Critical
    );
}

#[test]
fn test_generate_context_report_basic() {
    let messages = vec![Message::user("Hello world"), Message::assistant("Hi there")];
    let report = generate_context_report(&messages, Some("You are helpful."), None, 200_000);
    assert!(report.estimated_tokens > 0);
    assert!(report.usage_percent < 1.0);
    assert_eq!(report.pressure, ContextPressure::Low);
    assert_eq!(report.message_count, 2);
    assert!(report.breakdown.system_prompt_tokens > 0);
    assert!(report.breakdown.message_tokens > 0);
}

#[test]
fn test_generate_context_report_critical() {
    let big_msg = "x".repeat(800_000);
    let messages = vec![Message::user(big_msg)];
    let report = generate_context_report(&messages, None, None, 200_000);
    assert_eq!(report.pressure, ContextPressure::Critical);
    assert!(report.usage_percent > 85.0);
}

#[test]
fn test_format_context_report() {
    let messages = vec![Message::user("hi")];
    let report = generate_context_report(&messages, Some("system"), None, 200_000);
    let formatted = format_context_report(&report);
    assert!(formatted.contains("Context Usage"));
    assert!(formatted.contains("Breakdown"));
    assert!(formatted.contains("Pressure"));
}

#[test]
fn test_compaction_strips_base64_blobs() {
    let config = CompactionConfig::default();
    let blob = "A".repeat(2000);
    let tool_content = format!("result: {blob}");
    let messages = vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "t1".to_string(),
            tool_name: String::new(),
            content: tool_content,
            is_error: false,
        }]),
    }];
    let text = build_conversation_text(&messages, &config);
    assert!(text.contains("[base64 blob"));
    assert!(!text.contains(&"A".repeat(2000)));
}

#[test]
fn test_compaction_applies_2k_cap() {
    let config = CompactionConfig::default();
    let large_result = "word ".repeat(500);
    let messages = vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "t2".to_string(),
            tool_name: String::new(),
            content: large_result,
            is_error: false,
        }]),
    }];
    let text = build_conversation_text(&messages, &config);
    let result_part = text.split("[Tool result (OK): ").nth(1).unwrap_or("");
    assert!(
        result_part.len() < 2100,
        "result_part len = {}",
        result_part.len()
    );
}

#[test]
fn test_compaction_short_results_unchanged() {
    let config = CompactionConfig::default();
    let short_result = "Success: 42 records processed";
    let messages = vec![Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "t3".to_string(),
            tool_name: String::new(),
            content: short_result.to_string(),
            is_error: false,
        }]),
    }];
    let text = build_conversation_text(&messages, &config);
    assert!(text.contains(short_result));
}

fn text_response(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        stop_reason: captain_types::message::StopReason::EndTurn,
        tool_calls: vec![],
        usage: TokenUsage {
            input_tokens: 50,
            output_tokens: 20,
            ..Default::default()
        },
    }
}
