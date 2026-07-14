use crate::agent_loop::{run_agent_loop, run_agent_loop_streaming, AgentLoopResult};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::message::{ContentBlock, MessageContent, StopReason, TokenUsage};
use captain_types::tool::ToolCall;
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

fn test_session() -> Session {
    Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

fn response(
    content: Vec<ContentBlock>,
    stop_reason: StopReason,
    tool_calls: Vec<ToolCall>,
    output_tokens: u64,
) -> CompletionResponse {
    CompletionResponse {
        content,
        stop_reason,
        tool_calls,
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens,
            ..Default::default()
        },
    }
}

fn empty_response(stop_reason: StopReason) -> CompletionResponse {
    response(Vec::new(), stop_reason, Vec::new(), 0)
}

fn text_response(text: &str) -> CompletionResponse {
    response(
        vec![ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        StopReason::EndTurn,
        Vec::new(),
        8,
    )
}

async fn run_loop_with(driver: Arc<dyn LlmDriver>, message: &str) -> (AgentLoopResult, Session) {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let result = run_agent_loop(
        &manifest,
        message,
        &mut session,
        &memory,
        driver,
        &[],
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
    .expect("Loop should complete without error");
    (result, session)
}

async fn run_streaming_with(driver: Arc<dyn LlmDriver>, message: &str) -> AgentLoopResult {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let (tx, _rx) = mpsc::channel(64);
    run_agent_loop_streaming(
        &manifest,
        message,
        &mut session,
        &memory,
        driver,
        &[],
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
    .expect("Streaming loop should complete without error")
}

struct EmptyAfterToolUseDriver {
    call_count: AtomicU32,
}

impl EmptyAfterToolUseDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for EmptyAfterToolUseDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            let tool_call = ToolCall {
                id: "tool_1".to_string(),
                name: "fake_tool".to_string(),
                input: serde_json::json!({"query": "test"}),
            };
            Ok(response(
                vec![ContentBlock::ToolUse {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    input: tool_call.input.clone(),
                    provider_metadata: None,
                }],
                StopReason::ToolUse,
                vec![tool_call],
                5,
            ))
        } else {
            Ok(empty_response(StopReason::EndTurn))
        }
    }
}

struct EmptyMaxTokensDriver;

#[async_trait]
impl LlmDriver for EmptyMaxTokensDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(empty_response(StopReason::MaxTokens))
    }
}

struct NormalDriver;

#[async_trait]
impl LlmDriver for NormalDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(text_response("Hello from the agent!"))
    }
}

struct EmptyThenNormalDriver {
    call_count: AtomicU32,
}

impl EmptyThenNormalDriver {
    fn new() -> Self {
        Self {
            call_count: AtomicU32::new(0),
        }
    }
}

#[async_trait]
impl LlmDriver for EmptyThenNormalDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let call = self.call_count.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            Ok(empty_response(StopReason::EndTurn))
        } else {
            Ok(text_response("Recovered after retry!"))
        }
    }
}

struct AlwaysEmptyDriver;

#[async_trait]
impl LlmDriver for AlwaysEmptyDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(empty_response(StopReason::EndTurn))
    }
}

#[tokio::test]
async fn empty_response_after_tool_use_returns_fallback() {
    let (result, _) = run_loop_with(
        Arc::new(EmptyAfterToolUseDriver::new()),
        "Do something with tools",
    )
    .await;

    assert!(
        !result.response.trim().is_empty(),
        "Response should not be empty after tool use, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("Task completed"),
        "Expected fallback message, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn tool_error_injects_no_fabrication_guidance() {
    let (_, session) = run_loop_with(
        Arc::new(EmptyAfterToolUseDriver::new()),
        "Do something with tools",
    )
    .await;

    let guidance_seen = session.messages.iter().any(|msg| {
        match &msg.content {
        MessageContent::Blocks(blocks) => blocks.iter().any(|block| {
            matches!(block, ContentBlock::Text { text, .. } if text.contains("Tool call(s) failed"))
        }),
        _ => false,
    }
    });

    assert!(
        guidance_seen,
        "Expected tool error guidance in session messages after failed tool call"
    );
}

#[tokio::test]
async fn empty_response_max_tokens_returns_fallback() {
    let (result, _) = run_loop_with(Arc::new(EmptyMaxTokensDriver), "Tell me something long").await;

    assert!(
        !result.response.trim().is_empty(),
        "Response should not be empty on max tokens, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("token limit"),
        "Expected max-tokens fallback message, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn normal_response_not_replaced_by_fallback() {
    let (result, _) = run_loop_with(Arc::new(NormalDriver), "Say hello").await;
    assert_eq!(result.response, "Hello from the agent!");
}

#[tokio::test]
async fn streaming_empty_response_after_tool_use_returns_fallback() {
    let result = run_streaming_with(
        Arc::new(EmptyAfterToolUseDriver::new()),
        "Do something with tools",
    )
    .await;

    assert!(
        !result.response.trim().is_empty(),
        "Streaming response should not be empty after tool use, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("Task completed"),
        "Expected fallback message in streaming, got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn empty_first_response_retries_and_recovers() {
    let (result, _) = run_loop_with(Arc::new(EmptyThenNormalDriver::new()), "Hello").await;

    assert_eq!(result.response, "Recovered after retry!");
    assert_eq!(
        result.iterations, 2,
        "Should have taken 2 iterations (retry)"
    );
}

#[tokio::test]
async fn empty_first_response_fallback_when_retry_also_empty() {
    let (result, _) = run_loop_with(Arc::new(AlwaysEmptyDriver), "Hello").await;

    assert!(
        result.response.contains("empty response"),
        "Expected empty response fallback (no tools executed), got: {:?}",
        result.response
    );
}

#[tokio::test]
async fn streaming_empty_response_max_tokens_returns_fallback() {
    let result = run_streaming_with(Arc::new(EmptyMaxTokensDriver), "Tell me something long").await;

    assert!(
        !result.response.trim().is_empty(),
        "Streaming response should not be empty on max tokens, got: {:?}",
        result.response
    );
    assert!(
        result.response.contains("token limit"),
        "Expected max-tokens fallback in streaming, got: {:?}",
        result.response
    );
}
