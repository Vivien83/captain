use super::*;
use crate::agent_loop_tool_record::ToolCallRecord;
use captain_memory::event_log::RangeQuery;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentId, AgentManifest, SessionId};
use captain_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};
use captain_types::tool::{ToolCall, ToolDefinition, ToolResult};
use std::sync::{Arc, Mutex};

struct EndTurnHarness {
    manifest: AgentManifest,
    session: Session,
    memory: MemorySubstrate,
    messages: Vec<Message>,
    total_usage: TokenUsage,
    capability_watchdog_used: bool,
    verification_correction_rounds: u8,
    verification_operation:
        Option<captain_memory::work_verification_progress::WorkVerificationLease>,
    max_iterations: u32,
    visible_tools: Vec<ToolDefinition>,
    records: Vec<ToolCallRecord>,
    phases: Arc<Mutex<Vec<LoopPhase>>>,
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
            verification_correction_rounds: 0,
            verification_operation: None,
            max_iterations: 8,
            visible_tools: Vec::new(),
            records: Vec::new(),
            phases: Arc::new(Mutex::new(Vec::new())),
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
        let phases = Arc::clone(&self.phases);
        let on_phase: PhaseCallback = Arc::new(move |phase| {
            phases.lock().unwrap().push(phase);
        });
        handle_end_turn_response(EndTurnInput {
            manifest: &self.manifest,
            user_message: "request",
            response,
            total_usage: &self.total_usage,
            messages: &mut self.messages,
            iteration,
            any_tools_executed,
            capability_denial_watchdog_used: &mut self.capability_watchdog_used,
            verification_correction_rounds: &mut self.verification_correction_rounds,
            verification_operation: &mut self.verification_operation,
            max_iterations: self.max_iterations,
            visible_tools: &self.visible_tools,
            streaming,
            phantom_action_watchdog,
            session: &mut self.session,
            memory: &self.memory,
            embedding_driver: None,
            on_phase: Some(&on_phase),
            hooks: None,
            agent_id_str: "agent-1",
            tool_calls_recorded: &self.records,
        })
        .await
    }

    fn verification_events(&self) -> Vec<captain_memory::event_log::SessionEvent> {
        self.memory
            .read_session_events_tail_by_type(
                &RangeQuery {
                    session_id: self.session.id.to_string(),
                    from_ts: None,
                    to_ts: None,
                    limit: Some(20),
                },
                captain_memory::work_verification_progress::WORK_VERIFICATION_EVENT_TYPE,
            )
            .unwrap()
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

fn tool_record(
    name: &str,
    input: serde_json::Value,
    is_error: bool,
    sequence: u32,
) -> ToolCallRecord {
    let call = ToolCall {
        id: format!("call-{sequence}"),
        name: name.to_string(),
        input,
    };
    let result = ToolResult {
        tool_use_id: call.id.clone(),
        content: if is_error { "failed" } else { "ok" }.to_string(),
        is_error,
        transient_content: Vec::new(),
    };
    ToolCallRecord {
        tool_name: name.to_string(),
        reason: "Test receipt.".to_string(),
        is_error,
        duration_ms: 12,
        input_summary: String::new(),
        output_summary: result.content.clone(),
        verification: Some(
            crate::work_verification::ToolVerificationReceipt::from_tool_call(
                &call, &result, sequence,
            ),
        ),
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
    harness.records = vec![tool_record(
        "file_read",
        serde_json::json!({"path": "README.md"}),
        false,
        0,
    )];

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
    assert_eq!(result.tool_calls[0].tool_name, "file_read");
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
    assert!(harness.verification_events().is_empty());
}

#[tokio::test]
async fn mutation_without_postcondition_requests_a_targeted_correction() {
    let response = text_response("done");
    let mut harness = EndTurnHarness::new();
    harness.records = vec![tool_record(
        "file_write",
        serde_json::json!({"path": "notes.txt", "content": "done"}),
        false,
        0,
    )];

    let result = harness
        .handle(&response, 0, true, false, false)
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(harness.verification_correction_rounds, 1);
    assert!(harness.verification_operation.is_some());
    assert_eq!(harness.messages.len(), 3);
    assert!(harness.messages[2]
        .content
        .text_content()
        .contains("Delivery verification is incomplete"));
    assert!(harness
        .memory
        .get_session(harness.session.id)
        .unwrap()
        .is_none());
    let events = harness.verification_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["state"], "verifying");
    assert_eq!(events[1].payload["state"], "correcting");
    assert_eq!(
        harness.phases.lock().unwrap().as_slice(),
        &[LoopPhase::Verifying, LoopPhase::Correcting]
    );
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("notes.txt"));
    assert!(!serialized.contains("done"));
}

#[tokio::test]
async fn postcondition_after_mutation_allows_delivery() {
    let response = text_response("verified result");
    let mut harness = EndTurnHarness::new();
    harness.records = vec![
        tool_record(
            "file_write",
            serde_json::json!({"path": "notes.txt", "content": "done"}),
            false,
            0,
        ),
        tool_record(
            "file_read",
            serde_json::json!({"path": "notes.txt"}),
            false,
            1,
        ),
    ];

    let result = harness
        .handle(&response, 1, true, false, false)
        .await
        .unwrap()
        .expect("post-condition should close delivery");

    assert_eq!(result.response, "verified result");
    assert_eq!(harness.verification_correction_rounds, 0);
    assert!(harness.verification_operation.is_none());
    let events = harness.verification_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["state"], "verifying");
    assert_eq!(events[1].payload["state"], "verified");
    assert_eq!(
        harness.phases.lock().unwrap().as_slice(),
        &[
            LoopPhase::Verifying,
            LoopPhase::VerificationVerified,
            LoopPhase::Done,
        ]
    );
}

#[tokio::test]
async fn correction_reuses_one_durable_operation_until_verified() {
    let mut harness = EndTurnHarness::new();
    harness.records = vec![tool_record(
        "file_write",
        serde_json::json!({"path": "notes.txt", "content": "done"}),
        false,
        0,
    )];
    assert!(harness
        .handle(&text_response("done"), 0, true, false, false)
        .await
        .unwrap()
        .is_none());
    let operation_id = harness
        .verification_operation
        .as_ref()
        .unwrap()
        .operation_id()
        .to_string();

    harness.records.push(tool_record(
        "file_read",
        serde_json::json!({"path": "notes.txt"}),
        false,
        1,
    ));
    let result = harness
        .handle(&text_response("verified"), 1, true, false, false)
        .await
        .unwrap()
        .expect("corrected delivery should finish");

    assert_eq!(result.response, "verified");
    assert!(harness.verification_operation.is_none());
    let events = harness.verification_events();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].payload["state"], "verifying");
    assert_eq!(events[1].payload["state"], "correcting");
    assert_eq!(events[2].payload["state"], "verifying");
    assert_eq!(events[3].payload["state"], "verified");
    assert!(events
        .iter()
        .all(|event| event.payload["operation_id"] == operation_id));
}

#[tokio::test]
async fn correction_circuit_breaker_returns_an_honest_incomplete_result() {
    let response = text_response("everything is complete");
    let mut harness = EndTurnHarness::new();
    harness.verification_correction_rounds =
        crate::work_verification::MAX_VERIFICATION_CORRECTION_ROUNDS;
    harness.records = vec![tool_record(
        "file_write",
        serde_json::json!({"path": "notes.txt", "content": "done"}),
        false,
        0,
    )];

    let result = harness
        .handle(&response, 2, true, false, false)
        .await
        .unwrap()
        .expect("circuit breaker should finish honestly");

    assert!(result.response.starts_with("Verification incomplete:"));
    assert!(result.response.contains("Unverified draft from the agent:"));
    assert!(result.response.contains("everything is complete"));
    assert!(harness.verification_operation.is_none());
    let events = harness.verification_events();
    assert_eq!(events[0].payload["state"], "verifying");
    assert_eq!(events[1].payload["state"], "incomplete");
    assert_eq!(
        harness.phases.lock().unwrap().as_slice(),
        &[
            LoopPhase::Verifying,
            LoopPhase::VerificationIncomplete,
            LoopPhase::Done,
        ]
    );
}

#[tokio::test]
async fn final_iteration_does_not_schedule_an_impossible_correction() {
    let response = text_response("done");
    let mut harness = EndTurnHarness::new();
    harness.max_iterations = 2;
    harness.records = vec![tool_record(
        "file_write",
        serde_json::json!({"path": "notes.txt", "content": "done"}),
        false,
        0,
    )];

    let result = harness
        .handle(&response, 1, true, false, false)
        .await
        .unwrap()
        .expect("last iteration should finish incomplete");

    assert!(result.response.starts_with("Verification incomplete:"));
    assert_eq!(harness.verification_correction_rounds, 0);
}

#[tokio::test]
async fn executed_tool_without_receipt_cannot_be_reported_complete() {
    let response = text_response("done");
    let mut harness = EndTurnHarness::new();

    let result = harness
        .handle(&response, 0, true, false, false)
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(harness.verification_correction_rounds, 1);
    assert!(harness.messages[2]
        .content
        .text_content()
        .contains("no verifiable receipt"));
}

#[tokio::test]
async fn silent_completion_is_withheld_until_effects_are_verified() {
    let response = text_response("[[silent]]");
    let mut harness = EndTurnHarness::new();
    harness.records = vec![tool_record(
        "file_write",
        serde_json::json!({"path": "notes.txt", "content": "done"}),
        false,
        0,
    )];

    let result = harness
        .handle(&response, 0, true, false, false)
        .await
        .unwrap();

    assert!(result.is_none());
    assert_eq!(harness.verification_correction_rounds, 1);
    assert!(harness.messages[1]
        .content
        .text_content()
        .contains("Silent completion withheld"));
}
