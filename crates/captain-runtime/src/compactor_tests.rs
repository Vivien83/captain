use crate::compaction_boundary::coherent_recent_split;
use crate::compactor::*;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use captain_memory::session::Session;
use captain_types::message::{ContentBlock, Message, MessageContent, Role, TokenUsage};
use std::sync::Arc;

#[test]
fn test_needs_compaction_below_threshold() {
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: vec![Message::user("hello")],
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig::default();
    assert!(!needs_compaction(&session, &config));
}

#[test]
fn test_needs_compaction_above_threshold() {
    let messages: Vec<Message> = (0..100)
        .map(|i| Message::user(format!("msg {i}")))
        .collect();
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig::default();
    assert!(needs_compaction(&session, &config));
}

#[test]
fn test_compaction_config_defaults() {
    let config = CompactionConfig::default();
    assert_eq!(config.threshold, 30);
    assert_eq!(config.keep_recent, 10);
    assert_eq!(config.max_summary_tokens, 1024);
    assert!((config.token_threshold_ratio - 0.7).abs() < f64::EPSILON);
    assert_eq!(config.context_window_tokens, 200_000);
}

#[tokio::test]
async fn test_compact_session_few_messages() {
    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(text_response("Summary of conversation"))
        }
    }

    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: vec![Message::user("hello"), Message::assistant("hi")],
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig {
        threshold: 30,
        keep_recent: 10,
        max_summary_tokens: 1024,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FakeDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();
    assert_eq!(result.compacted_count, 0);
    assert_eq!(result.kept_messages.len(), 2);
    assert_eq!(result.chunks_used, 0);
    assert!(!result.used_fallback);
}

/// Pruning old tool outputs brings the session back under the token
/// threshold: no LLM call happens at all this round.
#[tokio::test]
async fn test_compact_session_pruning_alone_skips_llm() {
    struct PanickingDriver;

    #[async_trait]
    impl LlmDriver for PanickingDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            panic!("pruning-only round must not call the LLM");
        }
    }

    // One giant old tool result (~200k estimated tokens), then a recent tool
    // result big enough (~42k tokens) to fill the reserved 40k-token window
    // on its own, pushing the giant one into prunable territory.
    let mut messages = vec![
        Message::user("run the audit"),
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call-old".into(),
                tool_name: "shell_exec".into(),
                content: "x".repeat(800_000),
                is_error: false,
            }]),
        },
        Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "call-recent".into(),
                tool_name: "file_read".into(),
                content: "y".repeat(170_000),
                is_error: false,
            }]),
        },
    ];
    for i in 0..5 {
        messages.push(Message::assistant(format!("progress update {i}")));
    }
    messages.push(Message::user("continue"));

    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    // Count threshold not exceeded; only the token threshold (140k) was.
    let config = CompactionConfig::default();

    let result = compact_session(
        Arc::new(PanickingDriver),
        "test-model",
        &session,
        &config,
        0,
    )
    .await
    .unwrap();

    assert!(result.pruned_only);
    assert_eq!(result.pruned_tool_results, 1);
    assert_eq!(result.compacted_count, 0);
    assert!(result.summary.is_empty());
    assert_eq!(result.kept_messages.len(), session.messages.len());
    // The giant old output was replaced by a placeholder.
    assert!(result.kept_messages[1].content.text_length() < 1_000);
    // The recent tool result inside the reserved window is intact.
    assert_eq!(result.kept_messages[2].content.text_length(), 170_000);
    // Recent messages are intact.
    assert_eq!(
        result.kept_messages.last().unwrap().content.text_content(),
        "continue"
    );
}

#[test]
fn test_coherent_recent_split_keeps_user_turn_boundary() {
    let mut messages = Vec::new();
    for idx in 0..24 {
        messages.push(Message::user(format!("old request {idx}")));
        messages.push(Message::assistant(format!("old response {idx}")));
    }
    messages.push(Message::user("schedule the morning report"));
    messages.push(Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: "call_cron".into(),
            name: "cron_create".into(),
            input: serde_json::json!({ "name": "daily" }),
            provider_metadata: None,
        }]),
    });
    messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "call_cron".into(),
            tool_name: "cron_create".into(),
            content: r#"{"status":"created"}"#.into(),
            is_error: false,
        }]),
    });
    messages.push(Message::assistant("scheduled"));

    let split_at = coherent_recent_split(&messages, 3);

    assert_eq!(messages[split_at].role, Role::User);
    assert_eq!(
        messages[split_at].content.text_content(),
        "schedule the morning report"
    );
}

#[tokio::test]
async fn test_compact_includes_tool_calls() {
    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let input_text = req.messages[0].content.text_content();
            assert!(
                input_text.contains("web_search"),
                "Should include tool name"
            );
            assert!(
                input_text.contains("Tool result"),
                "Should include tool result"
            );
            Ok(text_response("Summary with tools"))
        }
    }

    let mut messages: Vec<Message> = Vec::new();
    for _ in 0..8 {
        messages.push(Message::user("Query"));
    }
    messages[1] = Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: "tu-1".to_string(),
            name: "web_search".to_string(),
            input: serde_json::json!({"query": "test"}),
            provider_metadata: None,
        }]),
    };
    messages[2] = Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: "tu-1".to_string(),
            tool_name: String::new(),
            content: "Search results here".to_string(),
            is_error: false,
        }]),
    };

    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig {
        threshold: 5,
        keep_recent: 3,
        max_summary_tokens: 512,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FakeDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();
    assert!(result.compacted_count > 0);
    assert!(result.summary.contains("tools"));
    assert_eq!(result.chunks_used, 1);
    assert!(!result.used_fallback);
}

#[test]
fn test_compact_truncates_large_tool_input() {
    let large_input = serde_json::json!({"data": "x".repeat(500)});
    let input_str = serde_json::to_string(&large_input).unwrap();
    assert!(input_str.len() > 200);
    let preview = if input_str.len() > 200 {
        format!(
            "{}...",
            crate::str_utils::safe_truncate_str(&input_str, 200)
        )
    } else {
        input_str.clone()
    };
    assert!(preview.len() < input_str.len());
    assert!(preview.ends_with("..."));
}

#[tokio::test]
async fn test_compact_session_many_messages() {
    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(text_response("Summary: discussed topics 0 through 79"))
        }
    }

    let messages: Vec<Message> = (0..100)
        .map(|i| Message::user(format!("Message about topic {i}")))
        .collect();
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig {
        threshold: 30,
        keep_recent: 10,
        max_summary_tokens: 1024,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FakeDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();
    assert_eq!(result.compacted_count, 90);
    assert_eq!(result.kept_messages.len(), 10);
    assert!(result.summary.contains("Summary"));
    assert_eq!(result.chunks_used, 1);
    assert!(!result.used_fallback);
}

#[test]
fn test_compaction_config_new_defaults() {
    let config = CompactionConfig::default();
    assert_eq!(config.threshold, 30);
    assert_eq!(config.keep_recent, 10);
    assert_eq!(config.max_summary_tokens, 1024);
    assert!((config.base_chunk_ratio - 0.4).abs() < f64::EPSILON);
    assert!((config.min_chunk_ratio - 0.15).abs() < f64::EPSILON);
    assert!((config.safety_margin - 1.2).abs() < f64::EPSILON);
    assert_eq!(config.summarization_overhead_tokens, 4096);
    assert_eq!(config.max_chunk_chars, 80_000);
    assert_eq!(config.max_retries, 3);
    assert!((config.token_threshold_ratio - 0.7).abs() < f64::EPSILON);
    assert_eq!(config.context_window_tokens, 200_000);
}

#[tokio::test]
async fn test_fallback_on_llm_failure() {
    struct FailingDriver;

    #[async_trait]
    impl LlmDriver for FailingDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::Http("connection refused".to_string()))
        }
    }

    let messages: Vec<Message> = (0..30)
        .map(|i| Message::user(format!("Message {i}")))
        .collect();
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig {
        threshold: 10,
        keep_recent: 5,
        max_summary_tokens: 512,
        max_retries: 1,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FailingDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();

    assert!(result.used_fallback, "Should have used fallback");
    assert_eq!(result.chunks_used, 0, "Fallback uses 0 chunks");
    assert!(result.summary.contains("Summarization was unavailable"));
    assert!(result.summary.contains("25 messages removed"));
    assert_eq!(result.compacted_count, 25);
    assert_eq!(result.kept_messages.len(), 5);
}

#[test]
fn test_compaction_result_new_fields() {
    let result = CompactionResult {
        summary: "test".to_string(),
        kept_messages: vec![],
        compacted_count: 10,
        chunks_used: 3,
        used_fallback: false,
        pruned_tool_results: 0,
        pruned_only: false,
    };
    assert_eq!(result.chunks_used, 3);
    assert!(!result.used_fallback);

    let fallback_result = CompactionResult {
        summary: "fallback".to_string(),
        kept_messages: vec![],
        compacted_count: 5,
        chunks_used: 0,
        used_fallback: true,
        pruned_tool_results: 0,
        pruned_only: false,
    };
    assert_eq!(fallback_result.chunks_used, 0);
    assert!(fallback_result.used_fallback);
}

#[test]
fn test_estimate_token_count_basic() {
    let messages = vec![Message::user("Hello world"), Message::assistant("Hi there")];
    let tokens = estimate_token_count(&messages, None, None);
    assert!(tokens > 0);
    assert!(tokens < 100);
}

#[test]
fn test_estimate_token_count_with_system_prompt() {
    let messages = vec![Message::user("hi")];
    let system = "You are a helpful assistant. ".repeat(100);
    let tokens_without = estimate_token_count(&messages, None, None);
    let tokens_with = estimate_token_count(&messages, Some(&system), None);
    assert!(tokens_with > tokens_without);
}

#[test]
fn test_codex_economy_compaction_profile_is_stricter() {
    let default = CompactionConfig::default();
    let codex = CompactionConfig::codex_economy();
    assert!(codex.threshold < default.threshold);
    assert!(codex.keep_recent < default.keep_recent);
    assert!(codex.token_threshold_ratio < default.token_threshold_ratio);
    assert_eq!(
        CompactionConfig::for_provider("codex").threshold,
        codex.threshold
    );
    assert_eq!(
        CompactionConfig::for_provider("anthropic").threshold,
        default.threshold
    );
}

#[test]
fn test_estimate_token_count_with_tools() {
    use captain_types::tool::ToolDefinition;
    let messages = vec![Message::user("hi")];
    let tools = vec![ToolDefinition {
        name: "web_search".into(),
        description: "Search the web for information".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}}),
    }];
    let tokens_without = estimate_token_count(&messages, None, None);
    let tokens_with = estimate_token_count(&messages, None, Some(&tools));
    assert!(tokens_with > tokens_without);
}

#[test]
fn test_needs_compaction_by_tokens_below() {
    let config = CompactionConfig::default();
    assert!(!needs_compaction_by_tokens(100_000, &config));
}

#[test]
fn test_needs_compaction_by_tokens_above() {
    let config = CompactionConfig::default();
    assert!(needs_compaction_by_tokens(150_000, &config));
}

#[tokio::test]
async fn test_compact_session_forces_active_marker_when_ending_mid_tool_activity() {
    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            // The summarizer wrongly reports nothing pending, even though
            // the session ends on an unresolved tool call. All 8 sections
            // are present so normalize_handoff_summary passes it through
            // instead of routing it through the unstructured-wrap fallback.
            Ok(text_response(
                "# Demande active\n- (rien)\n\n\
                 # Objectif global\n- Test complet demande.\n\n\
                 # Etat courant\n- En cours.\n\n\
                 # Decisions\n- (rien)\n\n\
                 # Questions utilisateur\n- (rien)\n\n\
                 # Fichiers / artefacts\n- (rien)\n\n\
                 # Erreurs / risques\n- (rien)\n\n\
                 # Travail restant\n- (rien)",
            ))
        }
    }

    let messages = vec![
        Message::user("lance le test complet en 12 points"),
        Message {
            role: Role::Assistant,
            content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                id: "call-1".to_string(),
                name: "shell_exec".to_string(),
                input: serde_json::json!({}),
                provider_metadata: None,
            }]),
        },
    ];
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    // keep_recent: 0 forces every message into `to_compact`, so the session's
    // last message (the unresolved ToolUse) is guaranteed to be the one
    // `ends_mid_tool_activity` inspects.
    let config = CompactionConfig {
        keep_recent: 0,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FakeDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();

    let active_section = demande_active_section(&result.summary);
    // The deterministic task checkpoint replaces the summarizer's "- (rien)":
    // exact user request, tool activity, and the still-in-progress warning.
    assert!(active_section.contains("\"lance le test complet en 12 points\""));
    assert!(active_section.contains("shell_exec x1"));
    assert!(active_section.contains("Travail probablement encore en cours"));
    assert!(!active_section.contains("- (rien)"));
    assert!(result
        .summary
        .contains("# Objectif global\n- Test complet demande."));
}

/// Extract the "# Demande active" section only, so assertions don't get
/// confused by "- (rien)" placeholders legitimately present in other
/// sections of a test fixture summary.
fn demande_active_section(summary: &str) -> &str {
    let start = summary.find("# Demande active").expect("section present");
    let after = start + "# Demande active".len();
    let end = summary[after..]
        .find("\n# ")
        .map(|offset| after + offset)
        .unwrap_or(summary.len());
    &summary[start..end]
}

#[tokio::test]
async fn test_compact_session_keeps_summary_when_ending_at_completed_reply() {
    struct FakeDriver;

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(text_response(
                "# Demande active\n- (rien)\n\n\
                 # Objectif global\n- Test termine.\n\n\
                 # Etat courant\n- Rapport livre.\n\n\
                 # Decisions\n- (rien)\n\n\
                 # Questions utilisateur\n- (rien)\n\n\
                 # Fichiers / artefacts\n- (rien)\n\n\
                 # Erreurs / risques\n- (rien)\n\n\
                 # Travail restant\n- (rien)",
            ))
        }
    }

    let messages = vec![
        Message::user("lance le test complet"),
        Message::assistant("Voila le rapport final, tout est termine."),
    ];
    let session = Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    };
    let config = CompactionConfig {
        keep_recent: 0,
        ..CompactionConfig::default()
    };

    let result = compact_session(Arc::new(FakeDriver), "test-model", &session, &config, 0)
        .await
        .unwrap();

    let active_section = demande_active_section(&result.summary);
    assert!(active_section.contains("- (rien)"));
    assert!(!active_section.contains("Travail probablement en cours"));
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
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        },
    }
}
