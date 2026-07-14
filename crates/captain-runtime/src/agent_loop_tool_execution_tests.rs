use crate::agent_loop_tool_execution::{
    execute_tool_calls, execute_tool_calls_streaming, StreamingToolExecutionInput,
    ToolExecutionInput,
};
use crate::context_budget::ContextBudget;
use crate::hooks::{HookContext, HookHandler, HookRegistry};
use crate::llm_driver::CompletionResponse;
use crate::loop_guard::{LoopGuard, LoopGuardConfig};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentManifest, HookEvent};
use captain_types::message::{ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage};
use captain_types::tool::{ToolCall, ToolDefinition};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

struct BlockBeforeToolHook;

impl HookHandler for BlockBeforeToolHook {
    fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
        Err("blocked by test hook".to_string())
    }
}

struct BlockByNameHook(&'static str);

impl HookHandler for BlockByNameHook {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
        if ctx.data["tool_name"] == self.0 {
            Err(format!("blocked by test hook: {}", self.0))
        } else {
            Ok(())
        }
    }
}

struct ObservePersistedToolUseHook {
    memory: Arc<MemorySubstrate>,
    session_id: captain_types::agent::SessionId,
    observed: Arc<AtomicBool>,
}

impl HookHandler for ObservePersistedToolUseHook {
    fn on_event(&self, _ctx: &HookContext) -> Result<(), String> {
        let session = self
            .memory
            .get_session(self.session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "session was not persisted before tool preflight".to_string())?;
        let persisted = session.messages.last().is_some_and(|message| {
            matches!(&message.content, MessageContent::Blocks(blocks)
                if blocks.iter().any(|block| matches!(block, ContentBlock::ToolUse { .. })))
        });
        self.observed.store(persisted, Ordering::SeqCst);
        persisted
            .then_some(())
            .ok_or_else(|| "persisted session has no ToolUse boundary".to_string())
    }
}

fn test_manifest() -> AgentManifest {
    AgentManifest {
        name: "test-agent".to_string(),
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

fn tool_call(name: &str, input: serde_json::Value) -> ToolCall {
    ToolCall {
        id: format!("{name}-1"),
        name: name.to_string(),
        input,
    }
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: "test tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn tool_use_response(tool_call: ToolCall) -> CompletionResponse {
    multi_tool_use_response(vec![tool_call])
}

fn multi_tool_use_response(tool_calls: Vec<ToolCall>) -> CompletionResponse {
    let mut content = vec![ContentBlock::Text {
        text: "I will run some tools.".to_string(),
        provider_metadata: None,
    }];
    content.extend(tool_calls.iter().map(|tc| ContentBlock::ToolUse {
        id: tc.id.clone(),
        name: tc.name.clone(),
        input: tc.input.clone(),
        provider_metadata: None,
    }));
    CompletionResponse {
        content,
        stop_reason: StopReason::ToolUse,
        tool_calls,
        usage: TokenUsage::default(),
    }
}

fn message_blocks(message: &Message) -> &[ContentBlock] {
    match &message.content {
        MessageContent::Blocks(blocks) => blocks,
        MessageContent::Text(_) => panic!("expected block message"),
    }
}

#[tokio::test]
async fn execute_tool_calls_turns_loop_guard_block_into_tool_result() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let blocked_call = tool_call("file_read", serde_json::json!({"path": "Cargo.toml"}));
    let response = tool_use_response(blocked_call);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig {
        block_threshold: 1,
        warn_threshold: 10,
        ..Default::default()
    });
    let mut records = Vec::new();
    let mut visible_tools = vec![tool_definition("file_read")];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();

    let result = execute_tool_calls(ToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: None,
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    assert!(
        records.is_empty(),
        "blocked calls are not recorded as executed"
    );
    assert_eq!(session.messages.len(), 2);
    assert_eq!(messages.len(), 2);
    assert_eq!(session.messages[0].role, Role::Assistant);
    assert_eq!(message_blocks(&session.messages[0]).len(), 1);
    assert!(matches!(
        &message_blocks(&session.messages[0])[0],
        ContentBlock::ToolUse { name, .. } if name == "file_read"
    ));

    let result_blocks = message_blocks(&session.messages[1]);
    assert!(matches!(
        &result_blocks[0],
        ContentBlock::ToolResult {
            tool_name,
            is_error: true,
            content,
            ..
        } if tool_name == "file_read" && content.contains("Blocked")
    ));
    assert!(matches!(
        &result_blocks[1],
        ContentBlock::Text { text, .. } if text.contains("Tool call(s) failed")
    ));
    let saved = memory.get_session(session.id).unwrap().unwrap();
    assert_eq!(saved.messages.len(), 2);
}

#[tokio::test]
async fn execute_tool_calls_honors_before_tool_hook_block() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let blocked_call = tool_call("file_read", serde_json::json!({"path": "Cargo.toml"}));
    let response = tool_use_response(blocked_call);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig::default());
    let mut records = Vec::new();
    let mut visible_tools = vec![tool_definition("file_read")];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let hooks = HookRegistry::new();
    hooks.register(HookEvent::BeforeToolCall, Arc::new(BlockBeforeToolHook));

    let result = execute_tool_calls(ToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: Some(&hooks),
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    assert!(records.is_empty(), "hook-blocked calls are not executed");
    let result_blocks = message_blocks(&session.messages[1]);
    assert!(matches!(
        &result_blocks[0],
        ContentBlock::ToolResult {
            tool_name,
            is_error: true,
            content,
            ..
        } if tool_name == "file_read" && content.contains("blocked by test hook")
    ));
}

#[tokio::test]
async fn execute_tool_calls_persists_tool_use_before_preflight_hook() {
    let memory = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
    let manifest = test_manifest();
    let mut session = test_session();
    let session_id = session.id;
    let mut messages = Vec::new();
    let response = tool_use_response(tool_call(
        "file_read",
        serde_json::json!({"path": "Cargo.toml"}),
    ));
    let mut loop_guard = LoopGuard::new(LoopGuardConfig::default());
    let mut records = Vec::new();
    let mut visible_tools = vec![tool_definition("file_read")];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let observed = Arc::new(AtomicBool::new(false));
    let hooks = HookRegistry::new();
    hooks.register(
        HookEvent::BeforeToolCall,
        Arc::new(ObservePersistedToolUseHook {
            memory: Arc::clone(&memory),
            session_id,
            observed: Arc::clone(&observed),
        }),
    );

    execute_tool_calls(ToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: memory.as_ref(),
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: Some(&hooks),
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(
        observed.load(Ordering::SeqCst),
        "BeforeToolCall must observe the durable ToolUse boundary"
    );
}

#[tokio::test]
async fn execute_tool_calls_streaming_turns_loop_guard_block_into_tool_result() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let blocked_call = tool_call("file_read", serde_json::json!({"path": "Cargo.toml"}));
    let response = tool_use_response(blocked_call);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig {
        block_threshold: 1,
        warn_threshold: 10,
        ..Default::default()
    });
    let mut records = Vec::new();
    let mut visible_tools = vec![tool_definition("file_read")];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let (stream_tx, _stream_rx) = mpsc::channel(8);

    let result = execute_tool_calls_streaming(StreamingToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        stream_tx: &stream_tx,
        user_input_rx: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: None,
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    assert!(
        records.is_empty(),
        "blocked streaming calls are not recorded as executed"
    );
    assert_eq!(session.messages.len(), 2);
    let result_blocks = message_blocks(&session.messages[1]);
    assert!(matches!(
        &result_blocks[0],
        ContentBlock::ToolResult {
            tool_name,
            is_error: true,
            content,
            ..
        } if tool_name == "file_read" && content.contains("Blocked")
    ));
    let saved = memory.get_session(session.id).unwrap().unwrap();
    assert_eq!(saved.messages.len(), 2);
}

#[tokio::test]
async fn execute_tool_calls_streaming_handles_ask_user_without_generic_execution() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let ask_call = tool_call(
        "ask_user",
        serde_json::json!({"question": "Go ?", "options": ["oui", "non"]}),
    );
    let response = tool_use_response(ask_call);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig::default());
    let mut records = Vec::new();
    let mut visible_tools = vec![tool_definition("ask_user")];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let (stream_tx, mut stream_rx) = mpsc::channel(8);
    let (answer_tx, answer_rx) = mpsc::channel(1);
    answer_tx.send("oui".to_string()).await.unwrap();
    let user_input_rx = Some(Arc::new(tokio::sync::Mutex::new(answer_rx)));

    let result = execute_tool_calls_streaming(StreamingToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        stream_tx: &stream_tx,
        user_input_rx: user_input_rx.as_ref(),
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: None,
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool_name, "ask_user");
    assert_eq!(records[0].output_summary, "oui");
    let ask_event = stream_rx.recv().await.expect("ask_user event");
    assert!(matches!(
        ask_event,
        crate::llm_driver::StreamEvent::AskUser {
            question,
            options: Some(options),
        } if question == "Go ?" && options == vec!["oui".to_string(), "non".to_string()]
    ));
    let response_event = stream_rx.recv().await.expect("user response event");
    assert!(matches!(
        response_event,
        crate::llm_driver::StreamEvent::UserResponse { content } if content == "oui"
    ));
    let result_event = stream_rx.recv().await.expect("ask_user result event");
    assert!(matches!(
        result_event,
        crate::llm_driver::StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error: false,
        } if tool_use_id == "ask_user-1"
            && name == "ask_user"
            && result_preview == "User response received."
    ));

    assert_eq!(session.messages.len(), 2);
    let result_blocks = message_blocks(&session.messages[1]);
    assert_eq!(result_blocks.len(), 1);
    assert!(matches!(
        &result_blocks[0],
        ContentBlock::ToolResult {
            tool_name,
            content,
            is_error: false,
            ..
        } if tool_name == "ask_user" && content == "oui"
    ));
}

/// Tier 2.3 — native parallelism (execute_parallel_group).
#[tokio::test]
async fn execute_tool_calls_preserves_order_with_a_blocked_call_in_the_middle() {
    // Three side-effect-free calls collapse into one Parallel group
    // (tool_parallelism::is_side_effect). The middle one is hook-blocked
    // during the sequential PRE pass, before the other two even start
    // executing concurrently — its result lands in tool_result_blocks
    // during PRE, while the other two land later during POST. Without the
    // per-call scratch-buffer flush in execute_parallel_group, that would
    // scramble the final order to [web_search, file_read, memory_recall]
    // instead of the original [file_read, web_search, memory_recall].
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let calls = vec![
        tool_call("file_read", serde_json::json!({"path": "a.txt"})),
        tool_call("web_search", serde_json::json!({"query": "captain"})),
        tool_call("memory_recall", serde_json::json!({"query": "notes"})),
    ];
    let response = multi_tool_use_response(calls);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig::default());
    let mut records = Vec::new();
    let mut visible_tools = vec![
        tool_definition("file_read"),
        tool_definition("web_search"),
        tool_definition("memory_recall"),
    ];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let hooks = HookRegistry::new();
    hooks.register(
        HookEvent::BeforeToolCall,
        Arc::new(BlockByNameHook("web_search")),
    );

    let result = execute_tool_calls(ToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: Some(&hooks),
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    let result_blocks = message_blocks(&session.messages[1]);
    let names: Vec<&str> = result_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["file_read", "web_search", "memory_recall"]);

    // The blocked call in the middle carries the hook's rejection reason.
    let web_search_result = result_blocks
        .iter()
        .find(|b| matches!(b, ContentBlock::ToolResult { tool_name, .. } if tool_name == "web_search"))
        .unwrap();
    assert!(matches!(
        web_search_result,
        ContentBlock::ToolResult { content, is_error: true, .. }
            if content.contains("blocked by test hook")
    ));
}

#[tokio::test]
async fn execute_tool_calls_circuit_break_mid_group_stops_later_calls() {
    // global_circuit_breaker: 2 trips on the 3rd tool call regardless of
    // whether it's identical to earlier ones. In the plain sequential loop
    // this would mean the first two calls fully run and the third never
    // does; execute_parallel_group's sequential PRE pass must reproduce
    // that exactly even though EXEC itself is concurrent.
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let calls = vec![
        tool_call("file_read", serde_json::json!({"path": "a.txt"})),
        tool_call("web_search", serde_json::json!({"query": "captain"})),
        tool_call("memory_recall", serde_json::json!({"query": "notes"})),
    ];
    let response = multi_tool_use_response(calls);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig {
        global_circuit_breaker: 2,
        ..LoopGuardConfig::default()
    });
    let mut records = Vec::new();
    let mut visible_tools = vec![
        tool_definition("file_read"),
        tool_definition("web_search"),
        tool_definition("memory_recall"),
    ];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();

    let result = execute_tool_calls(ToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: None,
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await;

    // fail_loop_guard_circuit_break always returns Err (it never produces
    // an Ok(AgentLoopResult) in this codebase, circuit break or not) — the
    // `?` after apply_loop_guard_verdict propagates that immediately, in
    // both the original sequential loop and this parallel-group path.
    assert!(result.is_err());
    // Only the first two calls (file_read, web_search) ever reached PRE
    // with an Execute verdict and ran; memory_recall's PRE was never
    // evaluated.
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].tool_name, "file_read");
    assert_eq!(records[1].tool_name, "web_search");
}

/// Tier 2.4 — native parallelism, streaming (execute_streaming_parallel_group).
#[tokio::test]
async fn execute_tool_calls_streaming_preserves_order_with_a_blocked_call_in_the_middle() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let calls = vec![
        tool_call("file_read", serde_json::json!({"path": "a.txt"})),
        tool_call("web_search", serde_json::json!({"query": "captain"})),
        tool_call("memory_recall", serde_json::json!({"query": "notes"})),
    ];
    let response = multi_tool_use_response(calls);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig::default());
    let mut records = Vec::new();
    let mut visible_tools = vec![
        tool_definition("file_read"),
        tool_definition("web_search"),
        tool_definition("memory_recall"),
    ];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let hooks = HookRegistry::new();
    hooks.register(
        HookEvent::BeforeToolCall,
        Arc::new(BlockByNameHook("web_search")),
    );
    let (stream_tx, mut stream_rx) = mpsc::channel(16);

    let result = execute_tool_calls_streaming(StreamingToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        stream_tx: &stream_tx,
        user_input_rx: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: Some(&hooks),
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await
    .unwrap();

    assert!(result.is_none());
    let result_blocks = message_blocks(&session.messages[1]);
    let names: Vec<&str> = result_blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["file_read", "web_search", "memory_recall"]);

    // Two ToolExecutionResult events (file_read, memory_recall) — the
    // hook-blocked web_search never reaches EXEC/POST, so it never sends
    // one, same as the non-streaming path never records it.
    drop(stream_tx);
    let mut seen = Vec::new();
    while let Some(event) = stream_rx.recv().await {
        if let crate::llm_driver::StreamEvent::ToolExecutionResult { name, .. } = event {
            seen.push(name);
        }
    }
    assert_eq!(seen, vec!["file_read", "memory_recall"]);
}

#[tokio::test]
async fn execute_tool_calls_streaming_circuit_break_mid_group_stops_later_calls() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let manifest = test_manifest();
    let mut session = test_session();
    let mut messages = Vec::new();
    let calls = vec![
        tool_call("file_read", serde_json::json!({"path": "a.txt"})),
        tool_call("web_search", serde_json::json!({"query": "captain"})),
        tool_call("memory_recall", serde_json::json!({"query": "notes"})),
    ];
    let response = multi_tool_use_response(calls);
    let mut loop_guard = LoopGuard::new(LoopGuardConfig {
        global_circuit_breaker: 2,
        ..LoopGuardConfig::default()
    });
    let mut records = Vec::new();
    let mut visible_tools = vec![
        tool_definition("file_read"),
        tool_definition("web_search"),
        tool_definition("memory_recall"),
    ];
    let available_tools = visible_tools.clone();
    let empty_env = Vec::new();
    let (stream_tx, _stream_rx) = mpsc::channel(16);

    let result = execute_tool_calls_streaming(StreamingToolExecutionInput {
        response: &response,
        manifest: &manifest,
        session: &mut session,
        memory: &memory,
        messages: &mut messages,
        loop_guard: &mut loop_guard,
        tool_calls_recorded: &mut records,
        visible_tools: &mut visible_tools,
        available_tools: &available_tools,
        context_budget: &ContextBudget::new(200_000),
        hand_allowed_env: &empty_env,
        kernel: None,
        stream_tx: &stream_tx,
        user_input_rx: None,
        skill_registry: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        workspace_root: None,
        on_phase: None,
        media_engine: None,
        tts_engine: None,
        docker_config: None,
        hooks: None,
        process_manager: None,
        origin_channel: None,
        agent_id_str: "agent-1",
    })
    .await;

    assert!(result.is_err());
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].tool_name, "file_read");
    assert_eq!(records[1].tool_name, "web_search");
}
