use crate::agent_loop_iteration::{
    complete_iteration, stream_iteration, CompletionIterationInput, IterationCallOutcome,
    StreamingIterationInput,
};
use crate::context_budget::ContextBudget;
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use async_trait::async_trait;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::message::{ContentBlock, Message, StopReason, TokenUsage};
use captain_types::tool::ToolDefinition;
use tokio::sync::mpsc;

fn test_session() -> Session {
    Session {
        id: captain_types::agent::SessionId::new(),
        agent_id: captain_types::agent::AgentId::new(),
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

fn test_manifest(provider: &str) -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
        model: captain_types::agent::ModelConfig {
            provider: provider.to_string(),
            model: format!("{provider}/test-model"),
            system_prompt: "You are a test agent.".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn text_response(text: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        }],
        stop_reason: StopReason::EndTurn,
        tool_calls: Vec::new(),
        usage: TokenUsage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
    }
}

fn test_tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: "test tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            }
        }),
    }
}

struct StaticDriver {
    response: CompletionResponse,
}

#[async_trait]
impl LlmDriver for StaticDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(self.response.clone())
    }
}

#[tokio::test]
async fn complete_iteration_records_usage_and_returns_response() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest("test");
    let mut session = test_session();
    let driver = StaticDriver {
        response: text_response("hello"),
    };
    let mut messages = vec![Message::user("hi")];
    let mut total_usage = TokenUsage::default();
    let visible_tools = Vec::new();

    let outcome = complete_iteration(CompletionIterationInput {
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        driver: &driver,
        messages: &mut messages,
        system_prompt: "system",
        visible_tools: &visible_tools,
        context_budget: &ContextBudget::new(200_000),
        ctx_window: 200_000,
        iteration: 0,
        total_usage: &mut total_usage,
        on_phase: None,
    })
    .await
    .unwrap();

    let IterationCallOutcome::Response(response) = outcome else {
        panic!("expected direct response");
    };
    assert_eq!(response.text(), "hello");
    assert_eq!(total_usage.input_tokens, 7);
    assert_eq!(total_usage.output_tokens, 3);
}

#[tokio::test]
async fn complete_iteration_promotes_recovered_text_tool_call() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest("test");
    let mut session = test_session();
    let driver = StaticDriver {
        response: text_response(
            r#"Let me search. <function=web_search>{"query":"rust async"}</function>"#,
        ),
    };
    let mut messages = vec![Message::user("search")];
    let mut total_usage = TokenUsage::default();
    let visible_tools = vec![test_tool("web_search")];

    let outcome = complete_iteration(CompletionIterationInput {
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        driver: &driver,
        messages: &mut messages,
        system_prompt: "system",
        visible_tools: &visible_tools,
        context_budget: &ContextBudget::new(200_000),
        ctx_window: 200_000,
        iteration: 0,
        total_usage: &mut total_usage,
        on_phase: None,
    })
    .await
    .unwrap();

    let IterationCallOutcome::Response(response) = outcome else {
        panic!("expected promoted response");
    };
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "web_search");
    assert!(matches!(
        &response.content[0],
        ContentBlock::ToolUse { name, .. } if name == "web_search"
    ));
}

#[tokio::test]
async fn stream_iteration_drains_interjection_and_retries_codex_missing_tool_call() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest("codex");
    let mut session = test_session();
    let driver = StaticDriver {
        response: text_response("I will use shell_exec to inspect that."),
    };
    let mut messages = vec![Message::user("inspect")];
    let mut total_usage = TokenUsage::default();
    let visible_tools = vec![test_tool("shell_exec")];
    let (stream_tx, _stream_rx) = mpsc::channel(8);
    let (input_tx, input_rx) = mpsc::channel(2);
    input_tx
        .send("priorite aux tests".to_string())
        .await
        .unwrap();
    drop(input_tx);
    let user_input_rx = Some(std::sync::Arc::new(tokio::sync::Mutex::new(input_rx)));
    let mut watchdog_used = false;

    let outcome = stream_iteration(StreamingIterationInput {
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        driver: &driver,
        messages: &mut messages,
        system_prompt: "system",
        visible_tools: &visible_tools,
        context_budget: &ContextBudget::new(200_000),
        ctx_window: 200_000,
        iteration: 0,
        total_usage: &mut total_usage,
        on_phase: None,
        stream_tx: &stream_tx,
        user_input_rx: &user_input_rx,
        codex_missing_tool_watchdog_used: &mut watchdog_used,
    })
    .await
    .unwrap();

    assert!(matches!(outcome, IterationCallOutcome::Continue));
    assert!(watchdog_used);
    assert_eq!(total_usage.input_tokens, 7);
    assert_eq!(session.messages.len(), 1);
    assert!(session.messages[0]
        .content
        .text_content()
        .contains("priorite aux tests"));
    assert!(messages.iter().any(|message| {
        message
            .content
            .text_content()
            .contains("Call the appropriate tool now")
    }));
}
