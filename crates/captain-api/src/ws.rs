//! WebSocket handler for real-time agent chat.
//!
//! Provides a persistent bidirectional channel between the client
//! and an agent. Messages are exchanged as JSON:
//!
//! Client → Server: `{"type":"message","content":"..."}`
//! Server → Client: `{"type":"typing","state":"start|tool|stop"}`
//! Server → Client: `{"type":"text_delta","content":"..."}`
//! Server → Client: `{"type":"response","content":"...","input_tokens":N,"output_tokens":N,"iterations":N}`
//! Server → Client: `{"type":"error","content":"..."}`
//! Server → Client: `{"type":"agents_updated","agents":[...]}`
//! Server → Client: `{"type":"silent_complete"}` (agent chose NO_REPLY)
//! Server → Client: `{"type":"canvas","canvas_id":"...","html":"...","title":"..."}`

use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::StreamEvent;
use captain_runtime::llm_errors;
use captain_types::agent::AgentId;
use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Per-IP WebSocket connection tracker.
/// Max 5 concurrent WS connections per IP address.
const MAX_WS_PER_IP: usize = 5;

/// Idle timeout: close WS after 30 minutes of no client messages.
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Text delta debounce interval.
const DEBOUNCE_MS: u64 = 100;

/// Flush text buffer when it exceeds this many characters.
const DEBOUNCE_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// Verbose Level
// ---------------------------------------------------------------------------

/// Per-connection tool detail verbosity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum VerboseLevel {
    /// Suppress tool details (only tool name + success/fail).
    Off = 0,
    /// Truncated tool details.
    On = 1,
    /// Full tool details (default).
    Full = 2,
}

impl VerboseLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Full,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Full,
            Self::Full => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Full => "full",
        }
    }
}

// ---------------------------------------------------------------------------
// Connection Tracking
// ---------------------------------------------------------------------------

/// Global connection tracker (DashMap<IpAddr, AtomicUsize>).
fn ws_tracker() -> &'static DashMap<IpAddr, AtomicUsize> {
    static TRACKER: std::sync::OnceLock<DashMap<IpAddr, AtomicUsize>> = std::sync::OnceLock::new();
    TRACKER.get_or_init(DashMap::new)
}

/// RAII guard that decrements the connection count on drop.
struct WsConnectionGuard {
    ip: IpAddr,
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        if let Some(entry) = ws_tracker().get(&self.ip) {
            let prev = entry.value().fetch_sub(1, Ordering::Relaxed);
            if prev <= 1 {
                drop(entry);
                ws_tracker().remove(&self.ip);
            }
        }
    }
}

/// Try to acquire a WS connection slot for the given IP.
/// Returns None if the IP has reached MAX_WS_PER_IP.
fn try_acquire_ws_slot(ip: IpAddr) -> Option<WsConnectionGuard> {
    let entry = ws_tracker()
        .entry(ip)
        .or_insert_with(|| AtomicUsize::new(0));
    let current = entry.value().fetch_add(1, Ordering::Relaxed);
    if current >= MAX_WS_PER_IP {
        entry.value().fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    Some(WsConnectionGuard { ip })
}

// ---------------------------------------------------------------------------
// WS Upgrade Handler
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/ws — Upgrade to WebSocket for real-time chat.
///
/// SECURITY: Authenticates via Bearer token in Authorization header
/// or `?token=` query parameter (for browser WebSocket clients that
/// cannot set custom headers).
pub async fn agent_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    // SECURITY: Authenticate WebSocket upgrades (bypasses middleware).
    // Trim whitespace so empty/whitespace-only api_key disables auth.
    let api_key_raw = &state.kernel.config.api_key;
    let api_key = api_key_raw.trim();
    if !api_key.is_empty() {
        // SECURITY: Use constant-time comparison to prevent timing attacks on API key
        let ct_eq = |token: &str, key: &str| -> bool {
            use subtle::ConstantTimeEq;
            if token.len() != key.len() {
                return false;
            }
            token.as_bytes().ct_eq(key.as_bytes()).into()
        };

        let header_auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|token| ct_eq(token, api_key))
            .unwrap_or(false);

        let query_auth = uri
            .query()
            .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")))
            .map(|token| ct_eq(token, api_key))
            .unwrap_or(false);

        if !header_auth && !query_auth {
            warn!("WebSocket upgrade rejected: invalid auth");
            return axum::http::StatusCode::UNAUTHORIZED.into_response();
        }
    }

    // SECURITY: Enforce per-IP WebSocket connection limit
    let ip = addr.ip();

    let guard = match try_acquire_ws_slot(ip) {
        Some(g) => g,
        None => {
            warn!(ip = %ip, "WebSocket rejected: too many connections from IP (max {MAX_WS_PER_IP})");
            return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Verify agent exists
    if state.kernel.registry.get(agent_id).is_none() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let id_str = id.clone();
    ws.on_upgrade(move |socket| handle_agent_ws(socket, state, agent_id, id_str, guard))
        .into_response()
}

// ---------------------------------------------------------------------------
// WS Connection Handler
// ---------------------------------------------------------------------------

/// Handle a WebSocket connection to an agent.
///
/// The `_guard` is an RAII handle that decrements the per-IP connection
/// counter when this function returns (connection closes).
async fn handle_agent_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    agent_id: AgentId,
    id_str: String,
    _guard: WsConnectionGuard,
) {
    info!(agent_id = %id_str, "WebSocket connected");

    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    // Per-connection verbose level (default: Full)
    let verbose = Arc::new(AtomicU8::new(VerboseLevel::Full as u8));

    // Send initial connection confirmation
    let _ = send_json(
        &sender,
        &serde_json::json!({
            "type": "connected",
            "agent_id": id_str,
        }),
    )
    .await;

    // Catch-up: if an agent response is currently in progress, send the accumulated state
    if let Some(active) = state.kernel.active_streams.get(agent_id) {
        let catch_up = serde_json::json!({
            "type": "catch_up",
            "user_message": active.user_message,
            "user_message_id": active.user_message_id,
            "accumulated_text": active.accumulated_text,
            "tools": active.tools,
            "is_streaming": active.is_streaming,
            "channel": active.channel,
        });
        let _ = send_json(&sender, &catch_up).await;
    }

    // Subscribe to EventBus for real-time sync (broadcasts from other WS/channels).
    // The `is_initiator` flag prevents duplicate delivery: when THIS connection is
    // streaming (initiated a message), it already receives events via the direct
    // mpsc channel, so the broadcast forwarder skips ChatStream events.
    let is_initiator = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let is_initiator_events = Arc::clone(&is_initiator);
    let mut event_rx = state.kernel.event_bus.subscribe_agent(agent_id);
    let sender_events = Arc::clone(&sender);
    let event_forward_handle = tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(event) => {
                    if let captain_types::event::EventPayload::ChatStream(ref chat_ev) =
                        event.payload
                    {
                        // Skip if this connection is the initiator (already gets direct stream)
                        if is_initiator_events.load(Ordering::Relaxed) {
                            continue;
                        }
                        if let Ok(json) = serde_json::to_value(chat_ev) {
                            let ws_msg = serde_json::json!({
                                "type": "broadcast",
                                "event": json,
                            });
                            if send_json(&sender_events, &ws_msg).await.is_err() {
                                break; // Client disconnected
                            }
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    debug!("EventBus subscriber lagged by {n} events, continuing");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Spawn background task: periodic agent list updates with change detection
    let sender_clone = Arc::clone(&sender);
    let state_clone = Arc::clone(&state);
    let update_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut last_hash: u64 = 0;
        loop {
            interval.tick().await;
            let agents: Vec<serde_json::Value> = state_clone
                .kernel
                .registry
                .list()
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "model_provider": e.manifest.model.provider,
                        "model_name": e.manifest.model.model,
                    })
                })
                .collect();

            // Change detection: hash the agent list and only send on change
            let mut hasher = DefaultHasher::new();
            for a in &agents {
                serde_json::to_string(a)
                    .unwrap_or_default()
                    .hash(&mut hasher);
            }
            let new_hash = hasher.finish();
            if new_hash == last_hash {
                continue; // No change — skip broadcast
            }
            last_hash = new_hash;

            if send_json(
                &sender_clone,
                &serde_json::json!({
                    "type": "agents_updated",
                    "agents": agents,
                }),
            )
            .await
            .is_err()
            {
                break; // Client disconnected
            }
        }
    });

    // Channel for injecting user responses into the agent loop (ask_user tool)
    let active_user_input_tx: Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    // Per-connection rate limiting: max 10 messages per 60 seconds
    let mut msg_times: Vec<std::time::Instant> = Vec::new();
    const MAX_PER_MIN: usize = 10;
    const WINDOW: Duration = Duration::from_secs(60);

    // Track last activity for idle timeout
    let mut last_activity = std::time::Instant::now();

    // Main message loop with idle timeout
    loop {
        let msg = tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(m) => m,
                    None => break, // Stream ended
                }
            }
            _ = tokio::time::sleep(WS_IDLE_TIMEOUT.saturating_sub(last_activity.elapsed())) => {
                info!(agent_id = %id_str, "WebSocket idle timeout (30 min)");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Connection closed due to inactivity (30 min timeout)",
                    }),
                ).await;
                break;
            }
        };

        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "WebSocket receive error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                last_activity = std::time::Instant::now();

                // SECURITY: Reject oversized WebSocket messages (64KB max)
                const MAX_WS_MSG_SIZE: usize = 64 * 1024;
                if text.len() > MAX_WS_MSG_SIZE {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Message too large (max 64KB)",
                        }),
                    )
                    .await;
                    continue;
                }

                // SECURITY: Per-connection rate limiting
                let now = std::time::Instant::now();
                msg_times.retain(|t| now.duration_since(*t) < WINDOW);
                if msg_times.len() >= MAX_PER_MIN {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Rate limit exceeded. Max 10 messages per minute.",
                        }),
                    )
                    .await;
                    continue;
                }
                msg_times.push(now);

                // Check if this is a user_response to an ask_user question
                let parsed_check: serde_json::Value =
                    serde_json::from_str(&text).unwrap_or_default();
                if parsed_check["type"].as_str() == Some("user_response") {
                    let content = parsed_check["content"].as_str().unwrap_or("").to_string();
                    info!(agent_id = %id_str, response = %content, "WS: injecting user_response into agent loop");
                    let tx_lock = active_user_input_tx.lock().await;
                    if let Some(ref tx) = *tx_lock {
                        let _ = tx.send(content).await;
                    } else {
                        warn!(agent_id = %id_str, "WS: user_response received but no active ask_user");
                    }
                    continue;
                }

                // Spawn message handling as a task so the WS loop stays responsive
                // for user_response messages during ask_user waits
                let sender_c = Arc::clone(&sender);
                let state_c = Arc::clone(&state);
                let verbose_c = Arc::clone(&verbose);
                let ui_tx_c = Arc::clone(&active_user_input_tx);
                let is_init_c = Arc::clone(&is_initiator);
                let text_owned = text.clone();
                tokio::spawn(async move {
                    handle_text_message(
                        &sender_c,
                        &state_c,
                        agent_id,
                        &text_owned,
                        &verbose_c,
                        &ui_tx_c,
                        &is_init_c,
                    )
                    .await;
                });
            }
            Message::Close(_) => {
                info!(agent_id = %id_str, "WebSocket closed by client");
                break;
            }
            Message::Ping(data) => {
                last_activity = std::time::Instant::now();
                let mut s = sender.lock().await;
                let _ = s.send(Message::Pong(data)).await;
            }
            _ => {} // Ignore binary and pong
        }
    }

    // Cleanup
    update_handle.abort();
    event_forward_handle.abort();
    info!(agent_id = %id_str, "WebSocket disconnected");
}

// ---------------------------------------------------------------------------
// Message Handler
// ---------------------------------------------------------------------------

/// Handle a text message from the WebSocket client.
async fn handle_text_message(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    text: &str,
    verbose: &Arc<AtomicU8>,
    active_user_input_tx: &Arc<tokio::sync::Mutex<Option<tokio::sync::mpsc::Sender<String>>>>,
    is_initiator: &Arc<std::sync::atomic::AtomicBool>,
) {
    // Parse the message
    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // Treat plain text as a message
            serde_json::json!({"type": "message", "content": text})
        }
    };

    let msg_type = parsed["type"].as_str().unwrap_or("message");

    match msg_type {
        "message" => {
            let raw_content = match parsed["content"].as_str() {
                Some(c) if !c.trim().is_empty() => c.to_string(),
                _ => {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Missing or empty 'content' field",
                        }),
                    )
                    .await;
                    return;
                }
            };

            // Sanitize inbound user input
            let content = sanitize_user_input(&raw_content);
            if content.is_empty() {
                let _ = send_json(
                    sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Message content is empty after sanitization",
                    }),
                )
                .await;
                return;
            }

            // v3.11d — intercept /project slash commands before calling the LLM.
            // Instant UX + no token burn.
            {
                use captain_runtime::active_project::{parse_slash, SlashCommand};
                let slash = parse_slash(content.lines().next().unwrap_or(""));
                if !matches!(slash, SlashCommand::None) {
                    let reply =
                        crate::project_slash::handle(&state.kernel, &agent_id.to_string(), slash);
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "response",
                            "content": reply,
                        }),
                    )
                    .await;
                    return;
                }
            }

            // Resolve file attachments into image content blocks
            let mut has_images = false;
            let mut ws_content_blocks: Option<Vec<captain_types::message::ContentBlock>> = None;
            if let Some(attachments) = parsed["attachments"].as_array() {
                let refs: Vec<crate::types::AttachmentRef> = attachments
                    .iter()
                    .filter_map(|a| serde_json::from_value(a.clone()).ok())
                    .collect();
                if !refs.is_empty() {
                    let image_blocks = crate::routes::resolve_attachments(&refs);
                    if !image_blocks.is_empty() {
                        has_images = true;
                        ws_content_blocks = Some(image_blocks);
                    }
                }
            }

            // Warn if the model doesn't support vision but images were attached
            if has_images {
                let model_name = state
                    .kernel
                    .registry
                    .get(agent_id)
                    .map(|e| e.manifest.model.model.clone())
                    .unwrap_or_default();
                let supports_vision = state
                    .kernel
                    .model_catalog
                    .read()
                    .ok()
                    .and_then(|cat| cat.find_model(&model_name).map(|m| m.supports_vision))
                    .unwrap_or(false);
                if !supports_vision {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "command_result",
                            "message": format!(
                                "**Vision not supported** — the current model `{}` cannot analyze images. \
                                 Switch to a vision-capable model (e.g. `gemini-2.5-flash`, `claude-sonnet-4-20250514`, `gpt-4o`) \
                                 with `/model <name>` for image analysis.",
                                model_name
                            ),
                        }),
                    )
                    .await;
                }
            }

            // Mark this connection as the stream initiator (prevents duplicate EventBus delivery)
            is_initiator.store(true, Ordering::Relaxed);

            // Send typing lifecycle: start
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "typing",
                    "state": "start",
                }),
            )
            .await;

            // Generate a unique message ID for broadcast deduplication
            let user_msg_id = uuid::Uuid::new_v4().to_string();

            // Broadcast user message + typing to other WS clients via EventBus
            {
                use captain_types::event::{ChatStreamEvent, TypingState};
                let bus = &state.kernel.event_bus;
                crate::chat_broadcast_publish(
                    bus,
                    agent_id,
                    ChatStreamEvent::UserMessage {
                        message_id: user_msg_id.clone(),
                        content: content.clone(),
                        agent_id,
                        channel: "web".to_string(),
                    },
                )
                .await;
                crate::chat_broadcast_publish(
                    bus,
                    agent_id,
                    ChatStreamEvent::Typing {
                        agent_id,
                        state: TypingState::Start,
                    },
                )
                .await;
            }

            // Start tracking active stream for catch-up on new connections
            state.kernel.active_streams.start(
                agent_id,
                content.clone(),
                user_msg_id.clone(),
                "web".to_string(),
            );

            // Send message to agent with streaming
            let kernel_handle: Arc<dyn KernelHandle> =
                state.kernel.clone() as Arc<dyn KernelHandle>;
            match state.kernel.send_message_streaming(
                agent_id,
                &content,
                Some(kernel_handle),
                None,
                None,
                ws_content_blocks,
                Some("web".to_string()),
            ) {
                Ok((mut rx, handle, user_input_tx)) => {
                    let stream_metric = crate::stream_metrics::StreamMetricHandle::start(
                        agent_id.to_string(),
                        "websocket",
                    );
                    // Store user_input_tx so the WS loop can inject responses
                    {
                        let mut tx_slot = active_user_input_tx.lock().await;
                        *tx_slot = Some(user_input_tx);
                    }

                    // Timeline replay reads back by the agent's real active
                    // session UUID (GET /api/sessions/{id}/events), not by
                    // agent_id — resolve it once here rather than inside the
                    // hot per-event persist call.
                    let session_id = state
                        .kernel
                        .registry
                        .get(agent_id)
                        .map(|entry| entry.session_id.to_string())
                        .unwrap_or_else(|| agent_id.to_string());

                    // Forward stream events to WebSocket with debouncing.
                    //
                    // The stream_task also accumulates the full response text and
                    // captures ContentComplete usage data. This lets us send the
                    // `response` event immediately when the stream channel closes
                    // (after `drop(phase_cb)` in the kernel), WITHOUT waiting for
                    // post-processing (canonical session writes, JSONL, compaction)
                    // that happens in the kernel task after the loop.
                    let sender_stream = Arc::clone(sender);
                    let verbose_clone = Arc::clone(verbose);
                    let state_stream = Arc::clone(state);
                    let stream_task = tokio::spawn(async move {
                        let mut text_buffer = String::new();
                        let mut accumulated_text = String::new();
                        let mut stream_usage: Option<captain_types::message::TokenUsage> = None;
                        let mut is_silent = false;
                        let far_future = tokio::time::Instant::now() + Duration::from_secs(86400);
                        let mut flush_deadline = far_future;

                        loop {
                            let sleep = tokio::time::sleep_until(flush_deadline);
                            tokio::pin!(sleep);

                            tokio::select! {
                                event = rx.recv() => {
                                    let vlevel = VerboseLevel::from_u8(
                                        verbose_clone.load(Ordering::Relaxed),
                                    );
                                    match event {
                                        None => {
                                            // Stream ended — flush remaining text
                                            let _ = flush_text_buffer(
                                                &sender_stream,
                                                &mut text_buffer,
                                            )
                                            .await;
                                            break;
                                        }
                                        Some(ev) => {
                                            stream_metric.observe(&ev);
                                            // Capture ContentComplete for immediate response
                                            if let StreamEvent::ContentComplete { usage, .. } = &ev {
                                                stream_usage = Some(*usage);
                                                // Don't forward — handled below
                                                continue;
                                            }

                                            // Broadcast to other WS clients via EventBus
                                            broadcast_stream_event(
                                                &state_stream.kernel,
                                                agent_id,
                                                &ev,
                                            ).await;

                                            // v3.9f — timeline replay: persist a
                                            // coarse-grained event line for every
                                            // stream event. Best-effort; a failure
                                            // here must never crash the turn, only
                                            // degrade replay quality.
                                            crate::timeline::persist_stream_event(
                                                &state_stream.kernel.memory,
                                                agent_id,
                                                &session_id,
                                                &ev,
                                            );

                                            if let StreamEvent::TextDelta { ref text } = ev {
                                                accumulated_text.push_str(text);
                                                text_buffer.push_str(text);
                                                if text_buffer.len() >= DEBOUNCE_CHARS {
                                                    let _ = flush_text_buffer(
                                                        &sender_stream,
                                                        &mut text_buffer,
                                                    )
                                                    .await;
                                                    flush_deadline = far_future;
                                                } else if flush_deadline >= far_future {
                                                    flush_deadline =
                                                        tokio::time::Instant::now()
                                                            + Duration::from_millis(DEBOUNCE_MS);
                                                }
                                            } else {
                                                // Flush pending text before non-text events
                                                let _ = flush_text_buffer(
                                                    &sender_stream,
                                                    &mut text_buffer,
                                                )
                                                .await;
                                                flush_deadline = far_future;

                                                // Send typing indicator for tool events
                                                if let StreamEvent::ToolUseStart {
                                                    ref name, ..
                                                } = ev
                                                {
                                                    if name != "ask_user" {
                                                        let _ = send_json(
                                                            &sender_stream,
                                                            &serde_json::json!({
                                                                "type": "typing",
                                                                "state": "tool",
                                                                "tool": name,
                                                            }),
                                                        )
                                                        .await;
                                                    }
                                                }

                                                // Reset accumulated text on intermediate message
                                                // so the final response only contains the LAST turn's text
                                                if matches!(ev, StreamEvent::IntermediateMessage { .. }) {
                                                    accumulated_text.clear();
                                                }

                                                // Map event to JSON with verbose filtering
                                                if let Some(json) =
                                                    map_stream_event(&ev, vlevel)
                                                {
                                                    if send_json(&sender_stream, &json)
                                                        .await
                                                        .is_err()
                                                    {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                _ = &mut sleep => {
                                    // Timer fired — flush text buffer
                                    let _ = flush_text_buffer(
                                        &sender_stream,
                                        &mut text_buffer,
                                    )
                                    .await;
                                    flush_deadline = far_future;
                                }
                            }
                        }

                        stream_metric.finish();

                        // Check if the agent signalled NO_REPLY via the stream
                        // (PhaseChange with a "silent" marker — currently the
                        // kernel sets result.silent after the loop, so we detect
                        // it from empty accumulated text when ContentComplete
                        // had no text deltas at all).
                        if accumulated_text.is_empty() && stream_usage.is_some() {
                            is_silent = true;
                        }

                        (accumulated_text, stream_usage, is_silent)
                    });

                    // Wait for the stream to finish (fast — closes as soon as
                    // drop(phase_cb) runs after the agent loop). This does NOT
                    // wait for post-processing.
                    let stream_result = stream_task.await;

                    // Clear user_input_tx — agent is no longer waiting for input
                    {
                        let mut tx_slot = active_user_input_tx.lock().await;
                        *tx_slot = None;
                    }

                    // Spawn the kernel task in the background for cleanup
                    // (canonical session writes, JSONL mirror, compaction).
                    // We don't need its result for the response event.
                    let sender_bg = Arc::clone(sender);
                    let kernel_bg = Arc::clone(&state.kernel);
                    tokio::spawn(async move {
                        match handle.await {
                            Ok(Err(e)) => {
                                warn!("Agent post-processing failed: {e}");
                                let user_msg = classify_streaming_error(&e);
                                let _ = send_json(
                                    &sender_bg,
                                    &serde_json::json!({
                                        "type": "error",
                                        "content": user_msg,
                                    }),
                                )
                                .await;
                            }
                            Err(e) if e.is_panic() => {
                                kernel_bg.supervisor.record_panic();
                                warn!("Agent task panicked outside the supervised future: {e}");
                                let _ = send_json(
                                    &sender_bg,
                                    &serde_json::json!({
                                        "type": "error",
                                        "content": "Internal error occurred",
                                    }),
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!("Agent task cancelled: {e}");
                                let _ = send_json(
                                    &sender_bg,
                                    &serde_json::json!({
                                        "type": "error",
                                        "content": "Run cancelled",
                                    }),
                                )
                                .await;
                            }
                            Ok(Ok(_)) => {
                                // Post-processing completed successfully — nothing to send
                            }
                        }
                    });

                    // Send the response immediately from stream data
                    match stream_result {
                        Ok((accumulated_text, stream_usage, is_silent)) => {
                            // Send typing lifecycle: stop
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing",
                                    "state": "stop",
                                }),
                            )
                            .await;

                            let usage = stream_usage.unwrap_or_default();

                            // Finish active stream tracking + broadcast completion.
                            // Keep is_initiator=true DURING broadcasts so the initiating
                            // connection's event_forward_handle skips them (no duplicates).
                            state.kernel.active_streams.finish(agent_id);
                            {
                                use captain_types::event::{ChatStreamEvent, TypingState};
                                let bus = &state.kernel.event_bus;
                                let cleaned_for_broadcast = strip_think_tags(&accumulated_text);
                                crate::chat_broadcast_publish(
                                    bus,
                                    agent_id,
                                    ChatStreamEvent::Response {
                                        agent_id,
                                        content: cleaned_for_broadcast,
                                        input_tokens: usage.input_tokens,
                                        output_tokens: usage.output_tokens,
                                    },
                                )
                                .await;
                                crate::chat_broadcast_publish(
                                    bus,
                                    agent_id,
                                    ChatStreamEvent::Typing {
                                        agent_id,
                                        state: TypingState::Stop,
                                    },
                                )
                                .await;
                            }
                            // Release initiator lock AFTER broadcasts
                            is_initiator.store(false, Ordering::Relaxed);

                            if is_silent {
                                let _ = send_json(
                                    sender,
                                    &serde_json::json!({
                                        "type": "silent_complete",
                                        "input_tokens": usage.input_tokens,
                                        "output_tokens": usage.output_tokens,
                                    }),
                                )
                                .await;
                                return;
                            }

                            // Strip <think>...</think> blocks
                            let cleaned = strip_think_tags(&accumulated_text);

                            let content = if cleaned.trim().is_empty() {
                                format!(
                                    "[The agent completed processing but returned no text response. ({} in / {} out)]",
                                    usage.input_tokens, usage.output_tokens,
                                )
                            } else {
                                cleaned
                            };

                            // Estimate context pressure
                            let ctx_pct =
                                (usage.input_tokens as f64 / 200_000.0 * 100.0).min(100.0);
                            let pressure = if ctx_pct > 85.0 {
                                "critical"
                            } else if ctx_pct > 70.0 {
                                "high"
                            } else if ctx_pct > 50.0 {
                                "medium"
                            } else {
                                "low"
                            };

                            // Estimate cost from model catalog
                            let (provider_name, model_name) = state
                                .kernel
                                .registry
                                .get(agent_id)
                                .map(|e| {
                                    (
                                        e.manifest.model.provider.clone(),
                                        e.manifest.model.model.clone(),
                                    )
                                })
                                .unwrap_or_default();
                            let pricing_model =
                                captain_kernel::metering::MeteringEngine::catalog_pricing_model_id(
                                    &provider_name,
                                    &model_name,
                                );
                            let cost = captain_kernel::metering::MeteringEngine::estimate_cost_with_catalog(
                                &state.kernel.model_catalog.read().unwrap_or_else(|e| e.into_inner()),
                                &pricing_model,
                                usage.input_tokens,
                                usage.output_tokens,
                            );

                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "response",
                                    "content": content,
                                    "input_tokens": usage.input_tokens,
                                    "output_tokens": usage.output_tokens,
                                    "cached_input_tokens": usage.cached_input_tokens,
                                    "cache_creation_tokens": usage.cache_creation_tokens,
                                    "iterations": 0,
                                    "cost_usd": cost,
                                    "context_pressure": pressure,
                                }),
                            )
                            .await;
                        }
                        Err(e) => {
                            let content = if e.is_panic() {
                                state.kernel.supervisor.record_panic();
                                warn!("Stream relay task panicked: {e}");
                                "Internal error occurred"
                            } else {
                                warn!("Stream relay task cancelled: {e}");
                                "Stream cancelled"
                            };
                            state.kernel.active_streams.finish(agent_id);
                            is_initiator.store(false, Ordering::Relaxed);
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing", "state": "stop",
                                }),
                            )
                            .await;
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "error",
                                    "content": content,
                                }),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    warn!("Streaming setup failed: {e}");
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "typing", "state": "stop",
                        }),
                    )
                    .await;
                    let user_msg = classify_streaming_error(&e);
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": user_msg,
                        }),
                    )
                    .await;
                }
            }
        }
        "command" => {
            let cmd = parsed["command"].as_str().unwrap_or("");
            let args = parsed["args"].as_str().unwrap_or("");
            let response = handle_command(sender, state, agent_id, cmd, args, verbose).await;
            let _ = send_json(sender, &response).await;
        }
        "ping" => {
            let _ = send_json(sender, &serde_json::json!({"type": "pong"})).await;
        }
        other => {
            warn!(msg_type = other, "Unknown WebSocket message type");
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "error",
                    "content": format!("Unknown message type: {other}"),
                }),
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Command Handler
// ---------------------------------------------------------------------------

/// Handle a WS command and return the response JSON.
async fn handle_command(
    _sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    cmd: &str,
    args: &str,
    verbose: &Arc<AtomicU8>,
) -> serde_json::Value {
    match cmd {
        "new" | "reset" => match state.kernel.reset_session(agent_id) {
            Ok(()) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "New session started. The previous session remains available in history."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Reset failed: {e}")}),
        },
        "compact" => match state.kernel.compact_agent_session(agent_id).await {
            Ok(msg) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Compaction failed: {e}")})
            }
        },
        "stop" => match state.kernel.stop_agent_run(agent_id) {
            Ok(true) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "Run cancelled."})
            }
            Ok(false) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "No active run to cancel."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Stop failed: {e}")}),
        },
        "model" => {
            if args.is_empty() {
                if let Some(entry) = state.kernel.registry.get(agent_id) {
                    serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Current model: {} (provider: {})", entry.manifest.model.model, entry.manifest.model.provider)})
                } else {
                    serde_json::json!({"type": "error", "content": "Agent not found"})
                }
            } else {
                match state.kernel.set_agent_model(agent_id, args, None) {
                    Ok(()) => {
                        if let Some(entry) = state.kernel.registry.get(agent_id) {
                            let model = &entry.manifest.model.model;
                            let provider = &entry.manifest.model.provider;
                            serde_json::json!({
                                "type": "command_result",
                                "command": cmd,
                                "message": format!("Model switched to: {model} (provider: {provider})"),
                                "model": model,
                                "provider": provider
                            })
                        } else {
                            serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Model switched to: {args}")})
                        }
                    }
                    Err(e) => {
                        serde_json::json!({"type": "error", "content": format!("Model switch failed: {e}")})
                    }
                }
            }
        }
        "usage" => match state.kernel.session_usage_cost(agent_id) {
            Ok((input, output, cost)) => {
                let mut msg = format!(
                    "Session usage: ~{input} in / ~{output} out (~{} total)",
                    input + output
                );
                if cost > 0.0 {
                    msg.push_str(&format!(" | ${cost:.4}"));
                }
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Usage query failed: {e}")})
            }
        },
        "context" => match state.kernel.context_report(agent_id) {
            Ok(report) => {
                let formatted = captain_runtime::compactor::format_context_report(&report);
                serde_json::json!({
                    "type": "command_result",
                    "command": cmd,
                    "message": formatted,
                    "context_pressure": format!("{:?}", report.pressure).to_lowercase(),
                })
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Context report failed: {e}")})
            }
        },
        "verbose" => {
            let new_level = match args.to_lowercase().as_str() {
                "off" => VerboseLevel::Off,
                "on" => VerboseLevel::On,
                "full" => VerboseLevel::Full,
                _ => {
                    // Cycle to next level
                    let current = VerboseLevel::from_u8(verbose.load(Ordering::Relaxed));
                    current.next()
                }
            };
            verbose.store(new_level as u8, Ordering::Relaxed);
            serde_json::json!({
                "type": "command_result",
                "command": cmd,
                "message": format!("Verbose level: **{}**", new_level.label()),
            })
        }
        "queue" => {
            let is_running = state.kernel.running_tasks.contains_key(&agent_id);
            let msg = if is_running {
                "Agent is processing a request..."
            } else {
                "Agent is idle."
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "budget" => {
            let budget = &state.kernel.config.budget;
            let status = state.kernel.metering.budget_status(budget);
            let fmt = |v: f64| -> String {
                if v > 0.0 {
                    format!("${v:.2}")
                } else {
                    "unlimited".to_string()
                }
            };
            let msg = format!(
                "Hourly: ${:.4} / {}  |  Daily: ${:.4} / {}  |  Monthly: ${:.4} / {}",
                status.hourly_spend,
                fmt(status.hourly_limit),
                status.daily_spend,
                fmt(status.daily_limit),
                status.monthly_spend,
                fmt(status.monthly_limit),
            );
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "peers" => {
            let msg = if !state.kernel.config.network_enabled {
                "OFP network disabled.".to_string()
            } else {
                match state.kernel.peer_registry.get() {
                    Some(registry) => {
                        let peers = registry.all_peers();
                        if peers.is_empty() {
                            "No peers connected.".to_string()
                        } else {
                            peers
                                .iter()
                                .map(|p| format!("{} — {} ({:?})", p.node_id, p.address, p.state))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    None => "OFP peer node not started.".to_string(),
                }
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "a2a" => {
            let agents = state
                .kernel
                .a2a_external_agents
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let msg = if agents.is_empty() {
                "No external A2A agents discovered.".to_string()
            } else {
                agents
                    .iter()
                    .map(|(url, card)| format!("{} — {}", card.name, url))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        _ => serde_json::json!({"type": "error", "content": format!("Unknown command: {cmd}")}),
    }
}

// ---------------------------------------------------------------------------
// Stream Event Mapping (verbose-aware)
// ---------------------------------------------------------------------------

/// Map a stream event to a JSON value, applying verbose filtering.
fn map_stream_event(event: &StreamEvent, verbose: VerboseLevel) -> Option<serde_json::Value> {
    match event {
        StreamEvent::TextDelta { .. } => None, // Handled by debounce buffer
        StreamEvent::ToolUseStart { name, .. } if name == "ask_user" => None,
        StreamEvent::ToolUseStart { id, name } => Some(serde_json::json!({
            "type": "tool_start",
            "id": id,
            "tool_use_id": id,
            "tool": name,
        })),
        StreamEvent::ToolUseEnd { name, .. } if name == "ask_user" => None,
        StreamEvent::ToolUseEnd { name, input, .. } if name == "canvas_present" => {
            let html = input.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Canvas");
            Some(serde_json::json!({
                "type": "canvas",
                "canvas_id": uuid::Uuid::new_v4().to_string(),
                "html": html,
                "title": title,
            }))
        }
        StreamEvent::ToolUseEnd { id, name, input } => match verbose {
            VerboseLevel::Off => None,
            VerboseLevel::On => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(100)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool_use_id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
            VerboseLevel::Full => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool_use_id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
        },
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => match verbose {
            VerboseLevel::Off => Some(serde_json::json!({
                "type": "tool_result",
                "id": tool_use_id,
                "tool_use_id": tool_use_id,
                "tool": name,
                "is_error": is_error,
            })),
            VerboseLevel::On => {
                let truncated: String = result_preview.chars().take(200).collect();
                Some(serde_json::json!({
                    "type": "tool_result",
                    "id": tool_use_id,
                    "tool_use_id": tool_use_id,
                    "tool": name,
                    "result": truncated,
                    "is_error": is_error,
                }))
            }
            VerboseLevel::Full => Some(serde_json::json!({
                "type": "tool_result",
                "id": tool_use_id,
                "tool_use_id": tool_use_id,
                "tool": name,
                "result": result_preview,
                "is_error": is_error,
            })),
        },
        StreamEvent::PhaseChange { phase, detail } => Some(serde_json::json!({
            "type": "phase",
            "phase": phase,
            "detail": detail,
        })),
        StreamEvent::ToolOutputDelta {
            tool_use_id,
            stream,
            chunk,
        } => Some(serde_json::json!({
            "type": "tool_output_delta",
            "tool_use_id": tool_use_id,
            "stream": stream,
            "chunk": chunk,
        })),
        StreamEvent::IntermediateMessage { content } => Some(serde_json::json!({
            "type": "intermediate_message",
            "content": content,
        })),
        StreamEvent::AskUser { question, options } => Some(serde_json::json!({
            "type": "ask_user",
            "question": question,
            "options": options,
        })),
        _ => None, // Skip ToolInputDelta, ContentComplete, ThinkingDelta, UserResponse
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flush accumulated text buffer as a single text_delta event.
async fn flush_text_buffer(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    buffer: &mut String,
) -> Result<(), axum::Error> {
    if buffer.is_empty() {
        return Ok(());
    }
    let result = send_json(
        sender,
        &serde_json::json!({
            "type": "text_delta",
            "content": buffer.as_str(),
        }),
    )
    .await;
    buffer.clear();
    result
}

/// Helper to send a JSON value over WebSocket.
async fn send_json(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    value: &serde_json::Value,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut s = sender.lock().await;
    s.send(Message::Text(text.into()))
        .await
        .map_err(axum::Error::new)
}

/// Sanitize inbound user input.
///
/// - If content looks like a JSON envelope, extract the `content` field.
/// - Strip control characters (except \n, \t).
/// - Trim excessive whitespace.
fn sanitize_user_input(content: &str) -> String {
    // If content looks like a JSON envelope, try to extract the content field
    if content.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(inner) = val.get("content").and_then(|v| v.as_str()) {
                return sanitize_text(inner);
            }
        }
    }
    sanitize_text(content)
}

/// Strip control characters and normalize whitespace.
fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Classify a streaming/setup error into a user-friendly message.
///
/// Uses the proper LLM error classifier from `captain_runtime::llm_errors`
/// for comprehensive 20-provider coverage with actionable advice.
fn classify_streaming_error(err: &captain_kernel::error::KernelError) -> String {
    let inner = format!("{err}");

    // Check for agent-specific errors first (not LLM errors)
    if inner.contains("Agent not found") {
        return "Agent not found. It may have been stopped or deleted.".to_string();
    }
    if inner.contains("quota") || inner.contains("Quota") {
        return "Token quota exceeded. Try /compact or /new to free up space.".to_string();
    }

    // Use the LLM error classifier for everything else
    let status = extract_status_code(&inner);
    let classified = llm_errors::classify_error(&inner, status);

    // Build a user-facing message. The classified.sanitized_message now
    // includes a redacted excerpt of the raw error (issue #493 fix), so we
    // use it as the base and only override for cases that need extra context.
    match classified.category {
        llm_errors::LlmErrorCategory::ContextOverflow => {
            "Context is full. Try /compact or /new.".to_string()
        }
        llm_errors::LlmErrorCategory::RateLimit => {
            if let Some(delay_ms) = classified.suggested_delay_ms {
                let secs = (delay_ms / 1000).max(1);
                format!("Rate limited. Wait ~{secs}s and try again.")
            } else {
                "Rate limited. Wait a moment and try again.".to_string()
            }
        }
        llm_errors::LlmErrorCategory::Billing => {
            format!("Billing issue. {}", classified.sanitized_message)
        }
        llm_errors::LlmErrorCategory::Auth => {
            // Show the actual error detail so users can diagnose (issue #493).
            // The sanitized_message already redacts secrets.
            classified.sanitized_message.clone()
        }
        llm_errors::LlmErrorCategory::ModelNotFound => {
            if inner.contains("localhost:11434") || inner.contains("ollama") {
                "Model not found on Ollama. Run `ollama pull <model>` first. Use /model to see options.".to_string()
            } else {
                format!(
                    "{}. Use /model to see options.",
                    classified.sanitized_message
                )
            }
        }
        llm_errors::LlmErrorCategory::Format => {
            // Claude Code CLI errors have actionable messages — pass them through
            if inner.contains("Claude Code CLI") || inner.contains("claude auth") {
                classified.raw_message.clone()
            } else {
                classified.sanitized_message.clone()
            }
        }
        _ => classified.sanitized_message,
    }
}

/// Try to extract an HTTP status code from an error string.
fn extract_status_code(s: &str) -> Option<u16> {
    // "API error (NNN):" — the format produced by LlmError::Api Display impl
    if let Some(idx) = s.find("API error (") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse::<u16>() {
            return Some(code);
        }
    }
    // "status: NNN"
    if let Some(idx) = s.find("status: ") {
        let after = &s[idx + 8..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "HTTP NNN"
    if let Some(idx) = s.find("HTTP ") {
        let after = &s[idx + 5..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "StatusCode(NNN)"
    if let Some(idx) = s.find("StatusCode(") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    None
}

/// Strip `<think>...</think>` blocks from model output.
///
/// Some models (MiniMax, DeepSeek, etc.) wrap their reasoning in `<think>` tags.
/// These are internal chain-of-thought and shouldn't be shown to the user.
pub fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[(start + end + 8)..]; // 8 = "</think>".len()
        } else {
            // Unclosed <think> tag — strip to end
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Broadcast a StreamEvent to all other WS clients via the EventBus.
/// Also updates the ActiveStreamTracker for catch-up on new connections.
async fn broadcast_stream_event(
    kernel: &Arc<captain_kernel::CaptainKernel>,
    agent_id: AgentId,
    ev: &StreamEvent,
) {
    use captain_types::event::ChatStreamEvent;

    let chat_event = match ev {
        StreamEvent::TextDelta { text } => {
            kernel.active_streams.append_text(agent_id, text);
            Some(ChatStreamEvent::TextDelta {
                agent_id,
                delta: text.clone(),
            })
        }
        StreamEvent::ToolUseStart { id, name } => {
            kernel
                .active_streams
                .tool_start(agent_id, id.clone(), name.clone());
            Some(ChatStreamEvent::ToolStart {
                agent_id,
                tool_name: name.clone(),
                tool_use_id: id.clone(),
            })
        }
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name: _,
            result_preview,
            is_error,
        } => {
            let maybe_tid = if !tool_use_id.is_empty() {
                Some(tool_use_id.clone())
            } else if let Some(stream) = kernel.active_streams.get(agent_id) {
                stream
                    .tools
                    .iter()
                    .rev()
                    .find(|t| !t.completed)
                    .map(|tool| tool.tool_use_id.clone())
            } else {
                None
            };

            maybe_tid.map(|tid| {
                kernel
                    .active_streams
                    .tool_end(agent_id, &tid, result_preview.clone(), *is_error);
                ChatStreamEvent::ToolEnd {
                    agent_id,
                    tool_use_id: tid,
                    result_preview: result_preview.clone(),
                    is_error: *is_error,
                }
            })
        }
        StreamEvent::PhaseChange { phase, detail } => Some(ChatStreamEvent::Phase {
            agent_id,
            phase: phase.clone(),
            detail: detail.clone(),
        }),
        StreamEvent::IntermediateMessage { content } => {
            Some(ChatStreamEvent::IntermediateMessage {
                agent_id,
                content: content.clone(),
            })
        }
        StreamEvent::AskUser { question, options } => Some(ChatStreamEvent::AskUser {
            agent_id,
            question: question.clone(),
            options: options.clone(),
        }),
        // ThinkingDelta, ToolInputDelta, ToolUseEnd, UserResponse, ContentComplete — not broadcast
        _ => None,
    };

    if let Some(evt) = chat_event {
        captain_kernel::chat_broadcast::publish_chat_event(&kernel.event_bus, agent_id, evt).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_module_loads() {
        // Verify module compiles and loads correctly
        let _ = VerboseLevel::Off;
    }

    #[test]
    fn test_verbose_level_cycle() {
        assert_eq!(VerboseLevel::Off.next(), VerboseLevel::On);
        assert_eq!(VerboseLevel::On.next(), VerboseLevel::Full);
        assert_eq!(VerboseLevel::Full.next(), VerboseLevel::Off);
    }

    #[test]
    fn test_verbose_level_roundtrip() {
        for v in [VerboseLevel::Off, VerboseLevel::On, VerboseLevel::Full] {
            assert_eq!(VerboseLevel::from_u8(v as u8), v);
        }
    }

    #[test]
    fn test_verbose_level_labels() {
        assert_eq!(VerboseLevel::Off.label(), "off");
        assert_eq!(VerboseLevel::On.label(), "on");
        assert_eq!(VerboseLevel::Full.label(), "full");
    }

    #[test]
    fn test_sanitize_user_input_plain_text() {
        assert_eq!(sanitize_user_input("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_user_input_strips_control_chars() {
        assert_eq!(sanitize_user_input("hello\x00world"), "helloworld");
        // Newlines and tabs are preserved
        assert_eq!(sanitize_user_input("hello\nworld"), "hello\nworld");
        assert_eq!(sanitize_user_input("hello\tworld"), "hello\tworld");
    }

    #[test]
    fn test_sanitize_user_input_extracts_json_content() {
        let envelope = r#"{"type":"message","content":"actual message"}"#;
        assert_eq!(sanitize_user_input(envelope), "actual message");
    }

    #[test]
    fn test_sanitize_user_input_leaves_non_envelope_json() {
        // JSON that doesn't have a content field is left as-is (after control-char stripping)
        let json = r#"{"key":"value"}"#;
        assert_eq!(sanitize_user_input(json), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_extract_status_code() {
        assert_eq!(extract_status_code("status: 429, body: ..."), Some(429));
        assert_eq!(
            extract_status_code("HTTP 503 Service Unavailable"),
            Some(503)
        );
        assert_eq!(extract_status_code("StatusCode(401)"), Some(401));
        assert_eq!(extract_status_code("some random error"), None);
        // LlmError::Api Display format (issue #493 fix)
        assert_eq!(
            extract_status_code("LLM driver error: API error (403): quota exceeded"),
            Some(403)
        );
        assert_eq!(
            extract_status_code("API error (401): invalid api key"),
            Some(401)
        );
    }

    #[test]
    fn test_sanitize_trims_whitespace() {
        assert_eq!(sanitize_user_input("  hello  "), "hello");
    }

    #[test]
    fn test_strip_think_tags() {
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>The answer is 42."),
            "The answer is 42."
        );
        assert_eq!(
            strip_think_tags("Hello <think>\nsome thinking\n</think> world"),
            "Hello  world"
        );
        assert_eq!(strip_think_tags("No thinking here"), "No thinking here");
        assert_eq!(strip_think_tags("<think>all thinking</think>"), "");
    }
}
