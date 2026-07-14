use super::*;
use crate::agent_loop_tool_record::ToolCallRecord;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentId, AgentManifest, SessionId};
use captain_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use captain_types::tool::ToolDefinition;

struct EndTurnHarness {
    manifest: AgentManifest,
    session: Session,
    memory: MemorySubstrate,
    messages: Vec<Message>,
    total_usage: TokenUsage,
    capability_watchdog_used: bool,
    visible_tools: Vec<ToolDefinition>,
    records: Vec<ToolCallRecord>,
}

impl EndTurnHarness {
    fn new() -> Self {
        Self {
            manifest: AgentManifest {
                name: "captain".to_string(),
                ..Default::default()
            },
            session: Session {
                id: SessionId::new(),
                agent_id: AgentId::new(),
                messages: Vec::new(),
                context_window_tokens: 0,
                label: None,
            },
            memory: MemorySubstrate::open_in_memory(0.01).unwrap(),
            messages: vec![Message::user("request")],
            total_usage: TokenUsage {
                input_tokens: 11,
                output_tokens: 5,
                ..Default::default()
            },
            capability_watchdog_used: false,
            visible_tools: Vec::new(),
            records: Vec::new(),
        }
    }

    async fn handle(
        &mut self,
        response: &CompletionResponse,
        iteration: u32,
        any_tools_executed: bool,
        streaming: bool,
        phantom_action_watchdog: bool,
    ) -> CaptainResult<Option<AgentLoopResult>> {
        handle_end_turn_response(EndTurnInput {
            manifest: &self.manifest,
            user_message: "request",
            response,
            total_usage: &self.total_usage,
            messages: &mut self.messages,
            iteration,
            any_tools_executed,
            capability_denial_watchdog_used: &mut self.capability_watchdog_used,
            visible_tools: &self.visible_tools,
            streaming,
            phantom_action_watchdog,
            session: &mut self.session,
            memory: &self.memory,
            embedding_driver: None,
            on_phase: None,
            hooks: None,
            agent_id_str: "agent-1",
            tool_calls_recorded: &self.records,
        })
        .await
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

fn empty_response() -> CompletionResponse {
    CompletionResponse {
        content: Vec::new(),
        stop_reason: StopReason::EndTurn,
        tool_calls: Vec::new(),
        usage: TokenUsage::default(),
    }
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: "test tool".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn tool_record() -> ToolCallRecord {
    ToolCallRecord {
        tool_name: "shell_exec".to_string(),
        reason: "Run a shell command needed for the current task.".to_string(),
        is_error: false,
        duration_ms: 12,
        input_summary: "{\"cmd\":\"true\"}".to_string(),
        output_summary: "ok".to_string(),
    }
}

#[tokio::test]
async fn handle_end_turn_silent_persists_marker_and_directives() {
    let response = text_response("[[reply:thread-1]] [[silent]] hidden");
    let mut harness = EndTurnHarness::new();

    let result = harness
        .handle(&response, 1, false, false, false)
        .await
        .unwrap()
        .expect("silent turn should finish");

    assert!(result.silent);
    assert_eq!(result.iterations, 2);
    assert_eq!(result.directives.reply_to.as_deref(), Some("thread-1"));
    assert_eq!(harness.session.messages.len(), 1);
    assert_eq!(
        harness.session.messages[0].content.text_content(),
        "[no reply needed]"
    );
    let saved = harness
        .memory
        .get_session(harness.session.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 1);
}

#[tokio::test]
async fn handle_end_turn_empty_retry_adds_retry_prompt_without_finishing() {
    let response = empty_response();
    let mut harness = EndTurnHarness::new();

    let result = harness
        .handle(&response, 0, false, false, false)
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(harness.messages.len(), 3);
    assert_eq!(harness.messages[1].role, Role::Assistant);
    assert_eq!(harness.messages[1].content.text_content(), "[no response]");
    assert_eq!(harness.messages[2].role, Role::User);
    assert_eq!(
        harness.messages[2].content.text_content(),
        "Please provide your response."
    );
    assert!(harness
        .memory
        .get_session(harness.session.id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn handle_end_turn_phantom_retry_demands_real_tool_use() {
    let response = text_response("The Telegram message has been sent successfully.");
    let mut harness = EndTurnHarness::new();

    let result = harness
        .handle(&response, 0, false, false, true)
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(harness.messages.len(), 3);
    assert_eq!(harness.messages[1].role, Role::Assistant);
    assert!(harness.messages[1]
        .content
        .text_content()
        .contains("sent successfully"));
    assert_eq!(harness.messages[2].role, Role::User);
    assert!(harness.messages[2]
        .content
        .text_content()
        .contains("did not call any tools"));
}

#[tokio::test]
async fn handle_end_turn_capability_retry_sets_watchdog_and_adds_nudge() {
    let response = text_response("I don't have access to that tool.");
    let mut harness = EndTurnHarness::new();
    harness.visible_tools = vec![tool_definition("capability_search")];

    let result = harness
        .handle(&response, 1, false, true, false)
        .await
        .unwrap();

    assert!(result.is_none());
    assert!(harness.capability_watchdog_used);
    assert_eq!(harness.messages.len(), 3);
    assert_eq!(harness.messages[1].role, Role::Assistant);
    assert_eq!(
        harness.messages[1].content.text_content(),
        "I don't have access to that tool."
    );
    assert_eq!(harness.messages[2].role, Role::User);
    assert!(harness.messages[2]
        .content
        .text_content()
        .contains("capability_search"));
}

#[tokio::test]
async fn handle_end_turn_complete_saves_assistant_message_and_result() {
    let response = text_response("final answer");
    let mut harness = EndTurnHarness::new();
    harness.records = vec![tool_record()];

    let result = harness
        .handle(&response, 2, true, false, false)
        .await
        .unwrap()
        .expect("complete turn should finish");

    assert!(!result.silent);
    assert_eq!(result.response, "final answer");
    assert_eq!(result.iterations, 3);
    assert_eq!(result.total_usage.input_tokens, 11);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "shell_exec");
    assert_eq!(harness.session.messages.len(), 1);
    assert_eq!(
        harness.session.messages[0].content.text_content(),
        "final answer"
    );
    let saved = harness
        .memory
        .get_session(harness.session.id)
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content.text_content(), "final answer");
}
