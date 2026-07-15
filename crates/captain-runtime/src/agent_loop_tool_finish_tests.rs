use super::*;
use captain_types::agent::{AgentId, AgentManifest};

fn manifest() -> AgentManifest {
    AgentManifest {
        name: "captain".to_string(),
        ..Default::default()
    }
}

fn tool_call(name: &str) -> ToolCall {
    ToolCall {
        id: format!("{name}-1"),
        name: name.to_string(),
        input: serde_json::json!({"query": "read file"}),
    }
}

fn tool_result(tool_call: &ToolCall, content: String, is_error: bool) -> ToolResult {
    ToolResult {
        tool_use_id: tool_call.id.clone(),
        content,
        is_error,
        transient_content: Vec::new(),
    }
}

fn core_visible_tools() -> Vec<ToolDefinition> {
    crate::tool_runner::builtin_tool_definitions()
        .into_iter()
        .filter(|tool| crate::core_tools::CORE_TOOLS.contains(&tool.name.as_str()))
        .collect()
}

#[tokio::test]
async fn finish_tool_call_records_warns_expands_and_pushes_result() {
    let manifest = manifest();
    let caller_id = AgentId::new().to_string();
    let call = tool_call("capability_search");
    let result_content = serde_json::json!({
        "results": [{
            "name": "file_read",
            "source": "builtin",
            "status": "available,deferred",
            "metadata": { "core": false }
        }]
    })
    .to_string();
    let result = tool_result(&call, result_content, false);
    let available_tools = crate::tool_runner::builtin_tool_definitions();
    let mut visible_tools = core_visible_tools();
    let mut records = Vec::new();
    let mut blocks = Vec::new();
    assert!(!visible_tools.iter().any(|tool| tool.name == "file_read"));

    finish_tool_call(FinishToolCallInput {
        manifest: &manifest,
        tool_call: &call,
        result,
        verdict: &LoopGuardVerdict::Warn("repeat warning".to_string()),
        context_budget: &ContextBudget::new(200_000),
        available_tools: &available_tools,
        visible_tools: &mut visible_tools,
        tool_calls_recorded: &mut records,
        tool_result_blocks: &mut blocks,
        kernel: None,
        hooks: None,
        caller_id_str: &caller_id,
        tool_elapsed_ms: 42,
        streaming: false,
        stream_tx: None,
    })
    .await;

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tool_name, "capability_search");
    assert_eq!(records[0].duration_ms, 42);
    assert!(visible_tools.iter().any(|tool| tool.name == "file_read"));
    assert_eq!(blocks.len(), 1);
    assert!(matches!(
        &blocks[0],
        ContentBlock::ToolResult {
            tool_name,
            content,
            is_error: false,
            ..
        } if tool_name == "capability_search" && content.contains("[LOOP GUARD] repeat warning")
    ));
}

#[tokio::test]
async fn finish_tool_call_streaming_emits_result_event() {
    let manifest = manifest();
    let caller_id = AgentId::new().to_string();
    let call = tool_call("web_search");
    let result = tool_result(&call, "streamed result body".to_string(), false);
    let mut visible_tools = Vec::new();
    let mut records = Vec::new();
    let mut blocks = Vec::new();
    let (stream_tx, mut stream_rx) = mpsc::channel(4);

    finish_tool_call(FinishToolCallInput {
        manifest: &manifest,
        tool_call: &call,
        result,
        verdict: &LoopGuardVerdict::Allow,
        context_budget: &ContextBudget::new(200_000),
        available_tools: &[],
        visible_tools: &mut visible_tools,
        tool_calls_recorded: &mut records,
        tool_result_blocks: &mut blocks,
        kernel: None,
        hooks: None,
        caller_id_str: &caller_id,
        tool_elapsed_ms: 7,
        streaming: true,
        stream_tx: Some(&stream_tx),
    })
    .await;

    let event = stream_rx.recv().await.expect("tool result event");
    assert!(matches!(
        event,
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error: false,
        } if tool_use_id == "web_search-1"
            && name == "web_search"
            && result_preview.contains("streamed result body")
    ));
    assert_eq!(records.len(), 1);
    assert_eq!(blocks.len(), 1);
}

#[tokio::test]
async fn finish_tool_call_keeps_transient_image_adjacent_to_its_result() {
    let manifest = manifest();
    let caller_id = AgentId::new().to_string();
    let call = tool_call("browser_screenshot");
    let mut result = tool_result(&call, "screenshot metadata".to_string(), false);
    result.transient_content.push(ContentBlock::Image {
        media_type: "image/png".to_string(),
        data: "cG5n".to_string(),
    });
    let mut visible_tools = Vec::new();
    let mut records = Vec::new();
    let mut blocks = Vec::new();

    finish_tool_call(FinishToolCallInput {
        manifest: &manifest,
        tool_call: &call,
        result,
        verdict: &LoopGuardVerdict::Allow,
        context_budget: &ContextBudget::new(200_000),
        available_tools: &[],
        visible_tools: &mut visible_tools,
        tool_calls_recorded: &mut records,
        tool_result_blocks: &mut blocks,
        kernel: None,
        hooks: None,
        caller_id_str: &caller_id,
        tool_elapsed_ms: 3,
        streaming: false,
        stream_tx: None,
    })
    .await;

    assert!(matches!(blocks[0], ContentBlock::ToolResult { .. }));
    assert!(matches!(blocks[1], ContentBlock::Image { .. }));
}
