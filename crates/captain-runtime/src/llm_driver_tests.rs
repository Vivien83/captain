use super::*;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};

#[test]
fn cache_hints_default_is_inactive() {
    let h = CacheHints::default();
    assert!(!h.cache_system);
    assert!(!h.cache_tools);
    assert!(
        !h.any(),
        "default must keep callers on legacy uncached path"
    );
}

#[test]
fn cache_hints_full_enables_both() {
    let h = CacheHints::full();
    assert!(h.cache_system && h.cache_tools);
    assert!(h.any());
}

#[test]
fn cache_hints_any_returns_true_when_only_one_set() {
    let h = CacheHints {
        cache_system: false,
        cache_tools: true,
        ..CacheHints::default()
    };
    assert!(h.any());
}

#[test]
fn cache_hints_for_provider_anthropic_enables_full() {
    let h = CacheHints::for_provider("anthropic");
    assert!(h.cache_system && h.cache_tools);
}

#[test]
fn cache_hints_can_carry_system_prefix_split() {
    let h = CacheHints::for_provider("anthropic").with_system_prefix_bytes(Some(42));
    assert_eq!(h.cacheable_system_prefix_bytes, Some(42));
}

#[test]
fn cache_hints_for_provider_is_case_insensitive() {
    assert!(CacheHints::for_provider("Anthropic").any());
    assert!(CacheHints::for_provider("ANTHROPIC").any());
    assert!(CacheHints::for_provider("claude").any());
}

#[test]
fn cache_hints_for_other_providers_stays_inactive() {
    for p in [
        "openai",
        "openrouter",
        "gemini",
        "groq",
        "mistral",
        "ollama",
        "",
    ] {
        assert!(
            !CacheHints::for_provider(p).any(),
            "provider {p:?} must not enable cache_control by default"
        );
    }
}

#[test]
fn test_completion_response_text() {
    let response = CompletionResponse {
        content: vec![
            ContentBlock::Text {
                text: "Hello ".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Text {
                text: "world!".to_string(),
                provider_metadata: None,
            },
        ],
        stop_reason: StopReason::EndTurn,
        tool_calls: vec![],
        usage: TokenUsage::default(),
    };
    assert_eq!(response.text(), "Hello world!");
}

#[test]
fn test_stream_event_clone() {
    let event = StreamEvent::TextDelta {
        text: "hello".to_string(),
    };
    let cloned = event.clone();
    assert!(matches!(cloned, StreamEvent::TextDelta { text } if text == "hello"));
}

#[test]
fn test_stream_event_variants() {
    let events: Vec<StreamEvent> = vec![
        StreamEvent::TextDelta {
            text: "hi".to_string(),
        },
        StreamEvent::ToolUseStart {
            id: "t1".to_string(),
            name: "web_search".to_string(),
        },
        StreamEvent::ToolInputDelta {
            text: "{\"q".to_string(),
        },
        StreamEvent::ToolUseEnd {
            id: "t1".to_string(),
            name: "web_search".to_string(),
            input: serde_json::json!({"query": "rust"}),
        },
        StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
        },
    ];
    assert_eq!(events.len(), 5);
}

#[tokio::test]
async fn test_default_stream_sends_events() {
    use tokio::sync::mpsc;

    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 5,
                    output_tokens: 3,
                    ..Default::default()
                },
            })
        }
    }

    let driver = FakeDriver;
    let (tx, mut rx) = mpsc::channel(16);
    let request = CompletionRequest {
        model: "test".to_string(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        tool_choice: None,
        cache_hints: CacheHints::default(),
    };

    let response = driver.stream(request, tx).await.unwrap();
    assert_eq!(response.text(), "Hello!");

    let ev1 = rx.recv().await.unwrap();
    assert!(matches!(ev1, StreamEvent::TextDelta { text } if text == "Hello!"));

    let ev2 = rx.recv().await.unwrap();
    assert!(matches!(
        ev2,
        StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            ..
        }
    ));
}
