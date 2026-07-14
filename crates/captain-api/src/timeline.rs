//! Shared timeline persistence helper (v3.9f).
//!
//! Maps a [`StreamEvent`] to a `(event_type, payload)` pair and appends
//! it to `sessions_events`, keyed by the agent's real active `SessionId`
//! (not `agent_id` — the web UI switches sessions per agent via
//! `/api/agents/{id}/sessions`, and `GET /api/sessions/{id}/events` reads
//! back by that same session UUID, so keying by anything else means
//! replay silently returns nothing). Failures are logged at `warn` level
//! but never propagated — observability must not affect the turn's
//! outcome.
//!
//! High-frequency events (`TextDelta`, `ToolInputDelta`,
//! `ThinkingDelta`, `ToolOutputDelta`, `ContentComplete`) are skipped:
//! replaying them char-by-char would bloat the log by two orders of
//! magnitude and the UI gets the full picture from the higher-level
//! boundaries (`ToolUseEnd`, `ToolExecutionResult`,
//! `IntermediateMessage`).
//!
//! The function is called from both the WebSocket handler
//! (`ws::stream_task`) and the SSE endpoint
//! (`routes::send_message_stream`) so every code path that streams
//! assistant activity feeds the scrubber — not just the WS one.

use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use std::sync::Arc;

pub fn persist_stream_event(
    memory: &Arc<captain_memory::MemorySubstrate>,
    agent_id: AgentId,
    session_id: &str,
    ev: &StreamEvent,
) {
    let (event_type, payload): (&str, serde_json::Value) = match ev {
        StreamEvent::ToolUseStart { id, name } => (
            "tool_use_start",
            serde_json::json!({ "tool_use_id": id, "name": name }),
        ),
        StreamEvent::ToolUseEnd { id, name, input } => (
            "tool_use_end",
            serde_json::json!({
                "tool_use_id": id,
                "name": name,
                "input": input,
            }),
        ),
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => (
            "tool_execution_result",
            serde_json::json!({
                "tool_use_id": tool_use_id,
                "name": name,
                "result_preview": result_preview,
                "is_error": is_error,
            }),
        ),
        StreamEvent::PhaseChange { phase, detail } => (
            "phase_change",
            serde_json::json!({ "phase": phase, "detail": detail }),
        ),
        StreamEvent::IntermediateMessage { content } => (
            "intermediate_message",
            serde_json::json!({ "content": content }),
        ),
        StreamEvent::AskUser { question, options } => (
            "ask_user",
            serde_json::json!({ "question": question, "options": options }),
        ),
        StreamEvent::UserResponse { content } => {
            ("user_response", serde_json::json!({ "content": content }))
        }
        StreamEvent::TextDelta { .. }
        | StreamEvent::ToolInputDelta { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::ToolOutputDelta { .. }
        | StreamEvent::ContentComplete { .. } => return,
    };

    if let Err(err) = memory.append_session_event(session_id, event_type, &payload) {
        tracing::warn!(
            agent_id = %agent_id,
            event_type = event_type,
            "timeline event persistence failed: {err}",
        );
    }
}
