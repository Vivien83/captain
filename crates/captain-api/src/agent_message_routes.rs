//! Agent message route handlers.

use crate::state::AppState;
use crate::types::{MessageRequest, MessageResponse, ToolCallSummary};
use crate::upload_routes::resolve_attachments;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_kernel::CaptainKernel;
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentId, SessionId};
use std::sync::Arc;

const MAX_MESSAGE_SIZE: usize = 64 * 1024;

struct AskUserChannelGuard {
    state: Arc<AppState>,
    key: String,
    sender: tokio::sync::mpsc::Sender<String>,
}

impl AskUserChannelGuard {
    fn new(state: Arc<AppState>, key: String, sender: tokio::sync::mpsc::Sender<String>) -> Self {
        Self { state, key, sender }
    }
}

impl Drop for AskUserChannelGuard {
    fn drop(&mut self) {
        self.state
            .ask_user_channels
            .remove_if(&self.key, |_, sender| sender.same_channel(&self.sender));
    }
}

/// Pre-insert image attachments into an agent's session so the LLM can see them.
pub fn inject_attachments_into_session(
    kernel: &CaptainKernel,
    agent_id: AgentId,
    image_blocks: Vec<captain_types::message::ContentBlock>,
) {
    use captain_types::message::{Message, MessageContent, Role};

    let entry = match kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => return,
    };

    let mut session = match kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => session,
        _ => captain_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        },
    };

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(image_blocks),
    });

    if let Err(e) = kernel.memory.save_session(&session) {
        tracing::warn!(error = %e, "Failed to save session with image attachments");
    }
}

/// POST /api/agents/:id/message - Send a message to an agent.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if req.message.len() > MAX_MESSAGE_SIZE {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Message too large (max 64KB)",
        );
    }

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let requested_session_id = match requested_session_id(req.session_id.as_deref()) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };

    if let Some(response) = handle_daemon_slash(&state, &req).await {
        return response;
    }

    if let Some(response) = handle_project_slash(&state, agent_id, &req.message) {
        return response;
    }

    let content_blocks = if req.attachments.is_empty() {
        None
    } else {
        match resolve_attachments(&req.attachments) {
            blocks if blocks.is_empty() => None,
            blocks => Some(blocks),
        }
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_full_in_session(
            agent_id,
            &req.message,
            Some(kernel_handle),
            content_blocks,
            req.sender_id,
            req.sender_name,
            req.channel_type.or_else(|| Some("web".to_string())),
            requested_session_id,
        )
        .await
    {
        Ok(result) => {
            let cleaned = crate::ws::strip_think_tags(&result.response);
            let response = if result.silent {
                String::new()
            } else if cleaned.trim().is_empty() {
                format!(
                    "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                    result.total_usage.input_tokens,
                    result.total_usage.output_tokens,
                    result.iterations,
                )
            } else {
                cleaned
            };
            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                    tool_calls: result
                        .tool_calls
                        .iter()
                        .map(|tool| ToolCallSummary {
                            name: tool.tool_name.clone(),
                            is_error: tool.is_error,
                            duration_ms: tool.duration_ms,
                        })
                        .collect(),
                })),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            let message = format!("{e}");
            let status = if message.contains("Agent not found") {
                StatusCode::NOT_FOUND
            } else if message.contains("Requested session was not found") {
                StatusCode::NOT_FOUND
            } else if message.contains("Requested session belongs") {
                StatusCode::CONFLICT
            } else if message.contains("quota") || message.contains("Quota") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (
                status,
                Json(serde_json::json!({"error": format!("Message delivery failed: {e}")})),
            )
                .into_response()
        }
    }
}

/// POST /api/agents/:id/message/stream - SSE streaming response.
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;

    if req.message.len() > MAX_MESSAGE_SIZE {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Message too large (max 64KB)",
        );
    }

    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return error(StatusCode::NOT_FOUND, "Agent not found");
    }

    let requested_session_id = match requested_session_id(req.session_id.as_deref()) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };

    if let Some((command, args)) = crate::daemon_commands::parse_daemon_slash(&req.message) {
        let reply = crate::daemon_commands::handle_daemon_command(
            state.kernel.clone(),
            Some(state.started_at),
            Some(state.shutdown_notify.clone()),
            &command,
            &args,
            crate::daemon_commands::DaemonCommandOrigin::api(
                req.channel_type.as_deref(),
                req.sender_id.as_deref(),
            ),
        )
        .await;
        let events: Vec<Result<Event, std::convert::Infallible>> = vec![
            Ok(Event::default()
                .event("chunk")
                .json_data(serde_json::json!({"content": reply, "done": false}))
                .unwrap_or_else(|_| Event::default().data("error"))),
            Ok(Event::default()
                .event("done")
                .json_data(serde_json::json!({
                    "done": true,
                    "usage": {"input_tokens": 0, "output_tokens": 0}
                }))
                .unwrap_or_else(|_| Event::default().data("error"))),
        ];
        return Sse::new(stream::iter(events))
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response();
    }

    let channel_type = req
        .channel_type
        .clone()
        .unwrap_or_else(|| "web".to_string());
    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle, user_input_tx) = match state.kernel.send_message_streaming_in_session(
        agent_id,
        &req.message,
        Some(kernel_handle),
        req.sender_id,
        req.sender_name,
        None,
        Some(channel_type.clone()),
        requested_session_id,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("Streaming message failed for agent {id}: {e}");
            let message = e.to_string();
            let status = if message.contains("Requested session was not found") {
                StatusCode::NOT_FOUND
            } else if message.contains("Requested session belongs") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return error(status, "Streaming message failed");
        }
    };

    // Hold the ask_user answer channel so POST /message/answer can reach
    // this turn while its SSE stream is open (mirrors ws.rs's per-connection
    // active_user_input_tx — the SSE surface has no socket to hold it on, so
    // it lives in shared state keyed by agent and persisted session).
    let ask_user_key = ask_user_channel_key(agent_id, requested_session_id);
    state
        .ask_user_channels
        .insert(ask_user_key.clone(), user_input_tx.clone());
    let ask_user_guard = AskUserChannelGuard::new(Arc::clone(&state), ask_user_key, user_input_tx);

    // Timeline replay reads back by the agent's real active session UUID
    // (GET /api/sessions/{id}/events), not by agent_id — resolve it once
    // here rather than inside the per-event persist call.
    let session_id = requested_session_id
        .or_else(|| {
            state
                .kernel
                .registry
                .get(agent_id)
                .map(|entry| entry.session_id)
        })
        .map(|session_id| session_id.to_string())
        .unwrap_or_else(|| agent_id.to_string());

    let sse_state = (
        rx,
        Arc::clone(&state.kernel.memory),
        crate::stream_metrics::StreamMetricHandle::start(agent_id.to_string(), channel_type),
        session_id,
        ask_user_guard,
    );
    let sse_stream = stream::unfold(
        sse_state,
        move |(mut rx, memory, metric, session_id, ask_user_guard)| async move {
            match rx.recv().await {
                Some(event) => {
                    metric.observe(&event);
                    crate::timeline::persist_stream_event(&memory, agent_id, &session_id, &event);
                    let event = Ok::<Event, std::convert::Infallible>(stream_event_to_sse(event));
                    Some((event, (rx, memory, metric, session_id, ask_user_guard)))
                }
                None => {
                    metric.finish();
                    None
                }
            }
        },
    );

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

#[derive(serde::Deserialize)]
pub struct AnswerAskUserReq {
    pub content: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

/// POST /api/agents/:id/message/answer - Answer a pending ask_user question
/// for an agent currently streaming via send_message_stream (daemon/SSE
/// surface — e.g. the TUI connected to a remote daemon). The web UI answers
/// over its own WebSocket instead; this route exists because SSE has no
/// return channel to inject a reply on.
pub async fn answer_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AnswerAskUserReq>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(agent_id) => agent_id,
        Err(response) => return response,
    };
    let session_id = match requested_session_id(req.session_id.as_deref()) {
        Ok(session_id) => session_id,
        Err(response) => return response,
    };
    let key = ask_user_channel_key(agent_id, session_id);
    let Some((_, tx)) = state.ask_user_channels.remove(&key) else {
        return error(StatusCode::CONFLICT, "No ask_user pending for this agent");
    };
    if tx.send(req.content).await.is_err() {
        return error(
            StatusCode::GONE,
            "The pending ask_user turn is no longer listening",
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "answered"})),
    )
        .into_response()
}

async fn handle_daemon_slash(
    state: &Arc<AppState>,
    req: &MessageRequest,
) -> Option<axum::response::Response> {
    let (command, args) = crate::daemon_commands::parse_daemon_slash(&req.message)?;
    let reply = crate::daemon_commands::handle_daemon_command(
        state.kernel.clone(),
        Some(state.started_at),
        Some(state.shutdown_notify.clone()),
        &command,
        &args,
        crate::daemon_commands::DaemonCommandOrigin::api(
            req.channel_type.as_deref(),
            req.sender_id.as_deref(),
        ),
    )
    .await;
    Some(
        (
            StatusCode::OK,
            Json(serde_json::json!(MessageResponse {
                response: reply,
                input_tokens: 0,
                output_tokens: 0,
                iterations: 0,
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
            })),
        )
            .into_response(),
    )
}

fn handle_project_slash(
    state: &AppState,
    agent_id: AgentId,
    message: &str,
) -> Option<axum::response::Response> {
    use captain_runtime::active_project::{parse_slash, SlashCommand};
    let slash = parse_slash(message.lines().next().unwrap_or(""));
    if matches!(slash, SlashCommand::None) {
        return None;
    }
    let reply = crate::project_slash::handle(&state.kernel, &agent_id.to_string(), slash);
    Some(
        (
            StatusCode::OK,
            Json(serde_json::json!({"response": reply, "slash": true})),
        )
            .into_response(),
    )
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, axum::response::Response> {
    id.parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))
}

#[allow(clippy::result_large_err)]
fn requested_session_id(
    value: Option<&str>,
) -> Result<Option<SessionId>, axum::response::Response> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<uuid::Uuid>()
                .map(SessionId)
                .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid session ID"))
        })
        .transpose()
}

fn ask_user_channel_key(agent_id: AgentId, session_id: Option<SessionId>) -> String {
    format!(
        "{agent_id}:{}",
        session_id
            .map(|session_id| session_id.to_string())
            .unwrap_or_else(|| "active".to_string())
    )
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

fn stream_event_to_sse(
    event: captain_runtime::llm_driver::StreamEvent,
) -> axum::response::sse::Event {
    use axum::response::sse::Event;
    use captain_runtime::llm_driver::StreamEvent;

    match event {
        StreamEvent::TextDelta { text } => Event::default()
            .event("chunk")
            .json_data(serde_json::json!({"content": text, "done": false}))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::ToolUseStart { id, name } => Event::default()
            .event("tool_use")
            .json_data(serde_json::json!({
                "type": "tool_start",
                "id": id,
                "tool_use_id": id,
                "tool": name,
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::ToolUseEnd { id, name, input } => Event::default()
            .event("tool_result")
            .json_data(serde_json::json!({
                "type": "tool_end",
                "id": id,
                "tool_use_id": id,
                "tool": name,
                "input": input,
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => Event::default()
            .event("tool_result")
            .json_data(serde_json::json!({
                "type": "tool_result",
                "id": tool_use_id,
                "tool_use_id": tool_use_id,
                "tool": name,
                "result": result_preview,
                "is_error": is_error,
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::ToolOutputDelta {
            tool_use_id,
            stream,
            chunk,
        } => Event::default()
            .event("tool_output_delta")
            .json_data(serde_json::json!({
                "type": "tool_output_delta",
                "tool_use_id": tool_use_id,
                "stream": stream,
                "chunk": chunk,
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::ContentComplete { usage, .. } => Event::default()
            .event("done")
            .json_data(serde_json::json!({
                "done": true,
                "usage": {
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                }
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        StreamEvent::PhaseChange { phase, detail } => Event::default()
            .event("phase")
            .json_data(serde_json::json!({"phase": phase, "detail": detail}))
            .unwrap_or_else(|_| Event::default().data("error")),
        // Same shape as ws.rs's web relay ("type":"ask_user") — without this
        // arm, the wildcard below silently swallows the question as an SSE
        // comment and the daemon/TUI surface hangs until the 300s timeout,
        // with no way to answer since the client never learns a question is
        // pending.
        StreamEvent::AskUser { question, options } => Event::default()
            .event("ask_user")
            .json_data(serde_json::json!({
                "type": "ask_user",
                "question": question,
                "options": options,
            }))
            .unwrap_or_else(|_| Event::default().data("error")),
        _ => Event::default().comment("skip"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        (tmp, state)
    }

    #[test]
    fn requested_session_id_accepts_only_uuid_values() {
        assert!(requested_session_id(None).unwrap().is_none());
        assert!(requested_session_id(Some(" ")).unwrap().is_none());

        let session_id = SessionId::new();
        assert_eq!(
            requested_session_id(Some(&session_id.to_string())).unwrap(),
            Some(session_id)
        );
        assert!(requested_session_id(Some("web-local-terminal")).is_err());
    }

    #[test]
    fn ask_user_channels_are_isolated_by_persisted_session() {
        let agent_id = AgentId::new();
        let first = SessionId::new();
        let second = SessionId::new();

        assert_ne!(
            ask_user_channel_key(agent_id, Some(first)),
            ask_user_channel_key(agent_id, Some(second))
        );
        assert_ne!(
            ask_user_channel_key(agent_id, Some(first)),
            ask_user_channel_key(agent_id, None)
        );
    }

    #[test]
    fn ask_user_channel_guard_cleans_up_abandoned_stream() {
        let (_tmp, state) = test_state();
        let key = "agent:session".to_string();
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        state.ask_user_channels.insert(key.clone(), tx.clone());

        let guard = AskUserChannelGuard::new(Arc::clone(&state), key.clone(), tx);
        assert!(state.ask_user_channels.contains_key(&key));
        drop(guard);

        assert!(!state.ask_user_channels.contains_key(&key));
    }

    #[test]
    fn stale_ask_user_guard_keeps_newer_stream_channel() {
        let (_tmp, state) = test_state();
        let key = "agent:session".to_string();
        let (first_tx, _first_rx) = tokio::sync::mpsc::channel(1);
        state
            .ask_user_channels
            .insert(key.clone(), first_tx.clone());
        let first_guard = AskUserChannelGuard::new(Arc::clone(&state), key.clone(), first_tx);

        let (replacement_tx, _replacement_rx) = tokio::sync::mpsc::channel(1);
        state
            .ask_user_channels
            .insert(key.clone(), replacement_tx.clone());
        drop(first_guard);

        let current = state.ask_user_channels.get(&key).expect("replacement kept");
        assert!(current.same_channel(&replacement_tx));
    }
}
