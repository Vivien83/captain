use super::*;
use crate::hooks::{HookContext, HookHandler, HookRegistry};
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentId, HookEvent, SessionId};
use captain_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use std::sync::{Arc, Mutex};

fn test_session(messages: Vec<Message>) -> Session {
    Session {
        id: SessionId::new(),
        agent_id: AgentId::new(),
        messages,
        context_window_tokens: 0,
        label: None,
    }
}

fn completion_response(content: Vec<ContentBlock>) -> CompletionResponse {
    CompletionResponse {
        content,
        stop_reason: StopReason::MaxTokens,
        tool_calls: Vec::new(),
        usage: TokenUsage::default(),
    }
}

fn test_manifest() -> AgentManifest {
    AgentManifest {
        name: "captain".to_string(),
        ..Default::default()
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

struct AgentLoopEndRecorder {
    events: Mutex<Vec<serde_json::Value>>,
}

impl AgentLoopEndRecorder {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn events(&self) -> Vec<serde_json::Value> {
        self.events.lock().unwrap().clone()
    }
}

impl HookHandler for AgentLoopEndRecorder {
    fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
        self.events.lock().unwrap().push(ctx.data.clone());
        Ok(())
    }
}

#[test]
fn max_continuations_constant_matches_legacy_guardrail() {
    assert_eq!(MAX_CONTINUATIONS, 5);
}

#[test]
fn appends_partial_assistant_and_continue_prompt_to_both_histories() {
    let response = completion_response(vec![ContentBlock::Text {
        text: "partial".to_string(),
        provider_metadata: None,
    }]);
    let mut session = test_session(vec![Message::user("previous")]);
    let mut request_messages = vec![Message::user("current")];

    append_max_tokens_continuation(&response, &mut session, &mut request_messages);

    assert_eq!(session.messages.len(), 3);
    assert_eq!(request_messages.len(), 3);
    assert_eq!(session.messages[1].role, Role::Assistant);
    assert_eq!(session.messages[1].content.text_content(), "partial");
    assert_eq!(session.messages[2].role, Role::User);
    assert_eq!(
        session.messages[2].content.text_content(),
        "Please continue."
    );
    assert_eq!(request_messages[1].content.text_content(), "partial");
    assert_eq!(
        request_messages[2].content.text_content(),
        "Please continue."
    );
}

#[test]
fn preserves_provider_metadata_on_partial_assistant_message() {
    let metadata = serde_json::json!({"thoughtSignature": "sig"});
    let response = completion_response(vec![
        ContentBlock::Thinking {
            thinking: "state".to_string(),
            provider_metadata: Some(metadata.clone()),
        },
        ContentBlock::Text {
            text: "raw partial".to_string(),
            provider_metadata: Some(metadata.clone()),
        },
    ]);
    let mut session = test_session(Vec::new());
    let mut request_messages = Vec::new();

    append_max_tokens_continuation(&response, &mut session, &mut request_messages);

    let MessageContent::Blocks(blocks) = &session.messages[0].content else {
        panic!("expected metadata-preserving block message");
    };
    assert!(matches!(blocks[0], ContentBlock::Thinking { .. }));
    assert!(matches!(
        &blocks[1],
        ContentBlock::Text {
            text,
            provider_metadata: Some(meta),
        } if text == "raw partial" && meta == &metadata
    ));
}

#[test]
fn incomplete_continuation_uses_codex_tool_aware_nudge() {
    let response = completion_response(vec![ContentBlock::Text {
        text: "partial".to_string(),
        provider_metadata: None,
    }]);
    let mut session = test_session(Vec::new());
    let mut request_messages = Vec::new();

    append_incomplete_continuation(
        &response,
        "partial".to_string(),
        "openai-codex",
        &mut session,
        &mut request_messages,
    );

    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[0].role, Role::Assistant);
    assert_eq!(session.messages[0].content.text_content(), "partial");
    assert_eq!(session.messages[1].role, Role::User);
    assert!(session.messages[1]
        .content
        .text_content()
        .contains("If the next step needs a tool"));
    assert_eq!(
        request_messages[1].content.text_content(),
        session.messages[1].content.text_content()
    );
}

#[test]
fn incomplete_continuation_uses_generic_nudge_for_other_providers() {
    let response = completion_response(vec![ContentBlock::Text {
        text: "partial".to_string(),
        provider_metadata: None,
    }]);
    let mut session = test_session(Vec::new());
    let mut request_messages = Vec::new();

    append_incomplete_continuation(
        &response,
        "partial".to_string(),
        "anthropic",
        &mut session,
        &mut request_messages,
    );

    assert_eq!(
        session.messages[1].content.text_content(),
        "Please continue from the incomplete response."
    );
    assert_eq!(
        request_messages[1].content.text_content(),
        "Please continue from the incomplete response."
    );
}

#[tokio::test]
async fn handle_max_tokens_continuation_updates_counters_and_continues() {
    let response = completion_response(vec![ContentBlock::Text {
        text: "partial".to_string(),
        provider_metadata: None,
    }]);
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(Vec::new());
    let manifest = test_manifest();
    let mut request_messages = Vec::new();
    let mut consecutive_max_tokens = 1;
    let mut consecutive_incomplete = 3;

    let result = handle_max_tokens_continuation(MaxTokensContinuationInput {
        response: &response,
        session: &mut session,
        memory: &memory,
        manifest: &manifest,
        hooks: None,
        agent_id_str: "agent-1",
        total_usage: &TokenUsage::default(),
        iteration: 1,
        consecutive_max_tokens: &mut consecutive_max_tokens,
        consecutive_incomplete: &mut consecutive_incomplete,
        tool_calls_recorded: &[],
        streaming: false,
        messages: &mut request_messages,
    })
    .await
    .unwrap();

    assert!(result.is_none());
    assert_eq!(consecutive_max_tokens, 2);
    assert_eq!(consecutive_incomplete, 0);
    assert_eq!(session.messages.len(), 2);
    assert_eq!(request_messages.len(), 2);
    assert_eq!(session.messages[0].role, Role::Assistant);
    assert_eq!(
        session.messages[1].content.text_content(),
        "Please continue."
    );
}

#[tokio::test]
async fn handle_incomplete_continuation_returns_limit_when_counter_reaches_guardrail() {
    let response = completion_response(Vec::new());
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(Vec::new());
    let manifest = test_manifest();
    let mut request_messages = Vec::new();
    let mut consecutive_max_tokens = 2;
    let mut consecutive_incomplete = MAX_CONTINUATIONS - 1;

    let result = handle_incomplete_continuation(IncompleteContinuationInput {
        response: &response,
        provider_name: "openai-codex",
        session: &mut session,
        memory: &memory,
        manifest: &manifest,
        hooks: None,
        agent_id_str: "agent-1",
        total_usage: &TokenUsage::default(),
        iteration: 4,
        consecutive_max_tokens: &mut consecutive_max_tokens,
        consecutive_incomplete: &mut consecutive_incomplete,
        tool_calls_recorded: &[],
        streaming: true,
        messages: &mut request_messages,
    })
    .await
    .unwrap()
    .expect("limit should finish the turn");

    assert_eq!(consecutive_max_tokens, 0);
    assert_eq!(consecutive_incomplete, MAX_CONTINUATIONS);
    assert_eq!(
        result.response,
        "[Partial response — Codex ended the turn incomplete with no text output.]"
    );
    assert!(request_messages.is_empty());
    assert_eq!(
        memory
            .get_session(session.id)
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[tokio::test]
async fn finish_continuation_limit_max_tokens_saves_fallback_and_fires_hook() {
    let response = completion_response(Vec::new());
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(Vec::new());
    let manifest = test_manifest();
    let registry = HookRegistry::new();
    let recorder = Arc::new(AgentLoopEndRecorder::new());
    registry.register(HookEvent::AgentLoopEnd, recorder.clone());
    let tool_calls = vec![tool_record()];

    let result = finish_continuation_limit(FinishContinuationLimitInput {
        kind: ContinuationLimitKind::MaxTokens,
        response: &response,
        session: &mut session,
        memory: &memory,
        manifest: &manifest,
        hooks: Some(&registry),
        agent_id_str: "agent-1",
        total_usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        },
        iteration: 3,
        consecutive_count: 5,
        tool_calls_recorded: &tool_calls,
        streaming: true,
    })
    .await
    .unwrap();

    assert_eq!(
        result.response,
        "[Partial response — token limit reached with no text output.]"
    );
    assert_eq!(result.iterations, 4);
    assert_eq!(result.total_usage.total(), 15);
    assert_eq!(result.tool_calls.len(), 1);
    let saved = memory.get_session(session.id).unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].role, Role::Assistant);
    assert_eq!(saved.messages[0].content.text_content(), result.response);

    let events = recorder.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["reason"], "max_continuations");
    assert_eq!(events[0]["iterations"], 4);
}

#[tokio::test]
async fn finish_continuation_limit_incomplete_saves_fallback_without_hook() {
    let response = completion_response(Vec::new());
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(Vec::new());
    let manifest = test_manifest();
    let registry = HookRegistry::new();
    let recorder = Arc::new(AgentLoopEndRecorder::new());
    registry.register(HookEvent::AgentLoopEnd, recorder.clone());

    let result = finish_continuation_limit(FinishContinuationLimitInput {
        kind: ContinuationLimitKind::Incomplete,
        response: &response,
        session: &mut session,
        memory: &memory,
        manifest: &manifest,
        hooks: Some(&registry),
        agent_id_str: "agent-1",
        total_usage: TokenUsage::default(),
        iteration: 2,
        consecutive_count: 5,
        tool_calls_recorded: &[],
        streaming: false,
    })
    .await
    .unwrap();

    assert_eq!(
        result.response,
        "[Partial response — Codex ended the turn incomplete with no text output.]"
    );
    assert_eq!(result.iterations, 3);
    let saved = memory.get_session(session.id).unwrap().unwrap();
    assert_eq!(saved.messages[0].content.text_content(), result.response);
    assert!(recorder.events().is_empty());
}

#[tokio::test]
async fn finish_continuation_limit_keeps_non_empty_partial_text() {
    let response = completion_response(vec![ContentBlock::Text {
        text: "partial answer".to_string(),
        provider_metadata: None,
    }]);
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(Vec::new());
    let manifest = test_manifest();

    let result = finish_continuation_limit(FinishContinuationLimitInput {
        kind: ContinuationLimitKind::Incomplete,
        response: &response,
        session: &mut session,
        memory: &memory,
        manifest: &manifest,
        hooks: None,
        agent_id_str: "agent-1",
        total_usage: TokenUsage::default(),
        iteration: 0,
        consecutive_count: 5,
        tool_calls_recorded: &[],
        streaming: false,
    })
    .await
    .unwrap();

    assert_eq!(result.response, "partial answer");
    assert_eq!(
        memory.get_session(session.id).unwrap().unwrap().messages[0]
            .content
            .text_content(),
        "partial answer"
    );
}

#[tokio::test]
async fn fail_max_iterations_saves_session_before_error() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut session = test_session(vec![Message::user("preserve me")]);
    let manifest = test_manifest();

    let err = fail_max_iterations(&manifest, &mut session, &memory, None, "agent", 7)
        .await
        .unwrap_err();

    assert!(matches!(err, CaptainError::MaxIterationsExceeded(7)));
    let saved = memory.get_session(session.id).unwrap().unwrap();
    assert_eq!(saved.messages.len(), 1);
    assert_eq!(saved.messages[0].content.text_content(), "preserve me");
}
