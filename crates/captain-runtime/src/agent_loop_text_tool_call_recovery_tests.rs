use crate::agent_loop::{run_agent_loop, run_agent_loop_streaming};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use captain_types::agent::AgentManifest;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};
use captain_types::tool::ToolDefinition;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

fn test_manifest() -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
        model: captain_types::agent::ModelConfig {
            system_prompt: "You are a test agent.".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn test_session() -> captain_memory::session::Session {
    captain_memory::session::Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

fn web_search_tool(schema: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: "web_search".into(),
        description: "Search the web".into(),
        input_schema: schema,
    }
}

struct TextToolCallDriver {
    call_count: AtomicU32,
}

impl TextToolCallDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for TextToolCallDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#.to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 20,
                    output_tokens: 15,
                    ..Default::default()
                },
            })
        } else {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Based on the search results, Rust async is great!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 30,
                    output_tokens: 12,
                    ..Default::default()
                },
            })
        }
    }
}

struct NormalTextDriver;

#[async_trait]
impl LlmDriver for NormalTextDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "Hello from the agent!".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 8,
                ..Default::default()
            },
        })
    }
}

#[tokio::test]
async fn text_tool_call_recovery_e2e() {
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());
    let tools = vec![web_search_tool(serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"}
        }
    }))];

    let result = run_agent_loop(
        &manifest,
        "Search for rust async programming",
        &mut session,
        &memory,
        driver,
        &tools,
        None, // kernel
        None, // skill_registry
        None, // mcp_connections
        None, // web_ctx
        None, // browser_ctx
        None, // embedding_driver
        None, // workspace_root
        None, // on_phase
        None, // media_engine
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // user_content_blocks
        None, // origin_channel
    )
    .await
    .expect("Agent loop should complete");

    assert!(
        !result.response.contains("<function="),
        "Response should not contain raw function tags, got: {:?}",
        result.response
    );
    assert!(
        result.iterations >= 2,
        "Should have at least 2 iterations (tool call + final response), got: {}",
        result.iterations
    );
    assert!(
        result.response.contains("search results") || result.response.contains("Rust async"),
        "Expected final response text, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn normal_flow_unaffected_by_recovery() {
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(NormalTextDriver);
    let tools = vec![web_search_tool(serde_json::json!({}))];

    let result = run_agent_loop(
        &manifest,
        "Say hello",
        &mut session,
        &memory,
        driver,
        &tools,
        None, // kernel
        None, // skill_registry
        None, // mcp_connections
        None, // web_ctx
        None, // browser_ctx
        None, // embedding_driver
        None, // workspace_root
        None, // on_phase
        None, // media_engine
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // user_content_blocks
        None, // origin_channel
    )
    .await
    .expect("Normal loop should complete");

    assert_eq!(result.response, "Hello from the agent!");
    assert_eq!(
        result.iterations, 1,
        "Normal response should complete in 1 iteration"
    );
}

#[tokio::test]
async fn text_tool_call_recovery_streaming_e2e() {
    let memory = captain_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session();
    let manifest = test_manifest();
    let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());
    let tools = vec![web_search_tool(serde_json::json!({
        "type": "object",
        "properties": {
            "query": {"type": "string"}
        }
    }))];
    let (tx, mut rx) = mpsc::channel(64);

    let result = run_agent_loop_streaming(
        &manifest,
        "Search for rust async programming",
        &mut session,
        &memory,
        driver,
        &tools,
        None, // kernel
        tx,
        None, // skill_registry
        None, // mcp_connections
        None, // web_ctx
        None, // browser_ctx
        None, // embedding_driver
        None, // workspace_root
        None, // on_phase
        None, // media_engine
        None, // tts_engine
        None, // docker_config
        None, // hooks
        None, // context_window_tokens
        None, // process_manager
        None, // user_content_blocks
        None, // user_input_rx
        None, // origin_channel
    )
    .await
    .expect("Streaming loop should complete");

    assert!(
        !result.response.contains("<function="),
        "Streaming: response should not contain raw function tags, got: {:?}",
        result.response
    );
    assert!(
        result.iterations >= 2,
        "Streaming: should have at least 2 iterations, got: {}",
        result.iterations
    );

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(!events.is_empty(), "Should have received stream events");
}
