//! WebSocket handler for persistent PTY sessions.
//!
//! Each session is identified by a URL path segment
//! (`/api/sessions/{id}/terminal`). The same `id` can reconnect to a live
//! [`SessionActor`]; closing a browser tab does not kill the PTY. A client can
//! explicitly dispose of the session with `{"type":"terminate"}`.
//!
//! Message protocol (JSON, all frames are text):
//!
//! Client -> Server:
//! ```json
//! {"type":"input","data":"ls\n"}
//! {"type":"resize","rows":40,"cols":120}
//! {"type":"signal","signal":"SIGINT"}
//! {"type":"terminate"}
//! ```
//!
//! Server -> Client:
//! ```json
//! {"type":"output","data":"<utf-8 stdout/stderr bytes>"}
//! {"type":"exit","code":0}
//! {"type":"error","message":"..."}
//! ```

use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, Path, Query, State, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use captain_runtime::pty_session::{PtyEvent, SessionActor, SessionSpec};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

/// Process-wide registry of live terminal sessions.
struct TerminalSession {
    actor: Arc<SessionActor>,
    rx: Arc<Mutex<mpsc::Receiver<PtyEvent>>>,
    replay: Arc<Mutex<Vec<u8>>>,
    active_clients: Arc<AtomicUsize>,
    mode: TerminalMode,
}

#[derive(serde::Serialize)]
struct TerminalSessionSummary {
    id: String,
    mode: String,
    active_clients: usize,
    replay_bytes: usize,
}

static SESSIONS: OnceLock<DashMap<String, TerminalSession>> = OnceLock::new();

type SharedPtyReceiver = Arc<Mutex<mpsc::Receiver<PtyEvent>>>;
type SharedReplayBuffer = Arc<Mutex<Vec<u8>>>;
type ActiveClientCounter = Arc<AtomicUsize>;
type SplitSinkAlias = futures::stream::SplitSink<WebSocket, Message>;

const MAX_TEXT_FRAME_BYTES: usize = 64 * 1024;
const MAX_BINARY_FRAME_BYTES: usize = 64 * 1024;
const MAX_REPLAY_BUFFER_BYTES: usize = 256 * 1024;

fn sessions() -> &'static DashMap<String, TerminalSession> {
    SESSIONS.get_or_init(DashMap::new)
}

/// Handler wired in server.rs: `GET /api/sessions/{id}/terminal`.
pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
) -> impl IntoResponse {
    if !state.kernel.config.web_terminal.enabled {
        return terminal_error(StatusCode::NOT_FOUND, "web terminal is disabled");
    }
    if let Err(e) = validate_session_id(&session_id) {
        return terminal_error(StatusCode::BAD_REQUEST, &e);
    }
    if let Err(e) = authorize(&state, &headers, &uri) {
        return e.into_response();
    }
    if let Err(e) = validate_origin(&headers) {
        warn!(peer = %peer, origin_error = %e, "terminal WS origin rejected");
        return terminal_error(StatusCode::FORBIDDEN, &e);
    }

    let request = match build_terminal_request(&state, &session_id, &headers, &uri) {
        Ok(request) => request,
        Err(e) => return terminal_error(StatusCode::FORBIDDEN, &e),
    };
    let max_sessions = state.kernel.config.web_terminal.max_sessions.max(1);

    ws.on_upgrade(move |socket| handle_terminal_ws(socket, request, max_sessions))
}

/// GET /api/terminal/sessions — List live browser terminal sessions.
pub async fn list_terminal_sessions() -> impl IntoResponse {
    let entries: Vec<(String, usize, SharedReplayBuffer)> = sessions()
        .iter()
        .map(|entry| {
            (
                entry.key().clone(),
                entry.active_clients.load(Ordering::SeqCst),
                Arc::clone(&entry.replay),
            )
        })
        .collect();

    let mut summaries = Vec::with_capacity(entries.len());
    for (key, active_clients, replay) in entries {
        let replay_bytes = replay.lock().await.len();
        let (mode, id) = key.split_once(':').unwrap_or(("captain", key.as_str()));
        summaries.push(TerminalSessionSummary {
            id: id.to_string(),
            mode: mode.to_string(),
            active_clients,
            replay_bytes,
        });
    }
    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    Json(serde_json::json!({ "sessions": summaries }))
}

/// DELETE /api/terminal/sessions/{id} — Terminate a live browser PTY session.
pub async fn terminate_terminal_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if !state.kernel.config.web_terminal.enabled {
        return terminal_error(StatusCode::NOT_FOUND, "web terminal is disabled");
    }
    if let Err(e) = validate_session_id(&session_id) {
        return terminal_error(StatusCode::BAD_REQUEST, &e);
    }

    let modes = match params
        .get("mode")
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("all") => vec![TerminalMode::Captain, TerminalMode::Shell],
        Some("shell" | "raw") => vec![TerminalMode::Shell],
        Some("" | "captain" | "tui") | None => vec![TerminalMode::Captain],
        Some(other) => {
            return terminal_error(
                StatusCode::BAD_REQUEST,
                &format!("unknown terminal mode: {other}"),
            )
        }
    };

    let mut terminated = false;
    for mode in modes {
        let key = format!("{}:{session_id}", mode.as_str());
        if let Some((_, session)) = sessions().remove(&key) {
            terminated = true;
            let _ = session.actor.terminate();
        }
    }

    if terminated {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "terminated",
                "session_id": session_id,
            })),
        )
            .into_response()
    } else {
        terminal_error(StatusCode::NOT_FOUND, "terminal session not found")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalMode {
    Captain,
    Shell,
}

impl TerminalMode {
    fn as_str(self) -> &'static str {
        match self {
            TerminalMode::Captain => "captain",
            TerminalMode::Shell => "shell",
        }
    }
}

#[derive(Debug, Clone)]
struct TerminalRequest {
    session_id: String,
    session_key: String,
    mode: TerminalMode,
    spec: SessionSpec,
}

#[derive(Debug, Clone)]
struct TerminalAuthError {
    status: StatusCode,
    message: &'static str,
}

impl TerminalAuthError {
    fn into_response(self) -> Response {
        terminal_error(self.status, self.message)
    }
}

fn authorize(state: &AppState, headers: &HeaderMap, uri: &Uri) -> Result<(), TerminalAuthError> {
    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &state.kernel.config.home_dir,
        &state.kernel.config.api_key,
        &state.kernel.config.auth,
    );
    let api_key = auth_snapshot.api_key.trim();
    let auth_cfg = &auth_snapshot.auth;

    if api_key.is_empty() && !auth_cfg.enabled {
        return Err(TerminalAuthError {
            status: StatusCode::FORBIDDEN,
            message: "web terminal requires api_key or web auth",
        });
    }

    if !api_key.is_empty() {
        let header_auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
            .map(|t| ct_eq(t, api_key))
            .unwrap_or(false);
        let query_auth = query_param(uri, "token")
            .map(|t| ct_eq(&t, api_key))
            .unwrap_or(false);
        if header_auth || query_auth {
            return Ok(());
        }
    }

    if auth_cfg.enabled && !auth_snapshot.session_secret().is_empty() {
        if let Some(token) = extract_session_cookie(headers) {
            if crate::session_auth::verify_session_token_for_auth(&token, &auth_snapshot).is_some()
            {
                return Ok(());
            }
        }
    }

    Err(TerminalAuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "missing or invalid terminal credentials",
    })
}

async fn handle_terminal_ws(socket: WebSocket, request: TerminalRequest, max_sessions: usize) {
    let (mut sink, mut stream) = socket.split();

    let (actor, rx, replay, active_clients) = match get_or_spawn(
        &request.session_key,
        request.mode,
        request.spec,
        max_sessions,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            let _ = sink
                .send(Message::Text(
                    serde_json::json!({"type":"error","message": e})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    if active_clients.fetch_add(1, Ordering::SeqCst) > 0 {
        active_clients.fetch_sub(1, Ordering::SeqCst);
        let _ = sink
            .send(Message::Text(
                serde_json::json!({
                    "type":"error",
                    "message":"terminal session already has an attached browser"
                })
                .to_string()
                .into(),
            ))
            .await;
        return;
    }

    info!(session_id = %request.session_id, mode = request.mode.as_str(), "terminal WS attached");

    let sink_shared = Arc::new(Mutex::new(sink));
    send_replay_buffer(&sink_shared, &replay).await;
    let output_task = spawn_output_forwarder(
        request.session_key.clone(),
        Arc::clone(&rx),
        Arc::clone(&replay),
        Arc::clone(&sink_shared),
    );

    while let Some(frame) = stream.next().await {
        let msg = match frame {
            Ok(m) => m,
            Err(e) => {
                warn!(session_id = %request.session_id, "terminal WS recv error: {e}");
                break;
            }
        };
        match msg {
            Message::Text(text) => {
                if text.len() > MAX_TEXT_FRAME_BYTES {
                    let _ = send_error(&sink_shared, "terminal frame too large").await;
                    continue;
                }
                match dispatch_client_frame(&actor, text.as_ref()) {
                    Ok(ClientFrameAction::Continue) => {}
                    Ok(ClientFrameAction::Terminate) => {
                        sessions().remove(&request.session_key);
                        break;
                    }
                    Err(e) => {
                        let _ = send_error(&sink_shared, &e).await;
                    }
                }
            }
            Message::Binary(bytes) => {
                if bytes.len() > MAX_BINARY_FRAME_BYTES {
                    let _ = send_error(&sink_shared, "terminal binary frame too large").await;
                    continue;
                }
                let _ = actor.write_stdin(&bytes);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    info!(session_id = %request.session_id, mode = request.mode.as_str(), "terminal WS detached");
    active_clients.fetch_sub(1, Ordering::SeqCst);
    output_task.abort();
}

fn get_or_spawn(
    session_key: &str,
    mode: TerminalMode,
    spec: SessionSpec,
    max_sessions: usize,
) -> Result<
    (
        Arc<SessionActor>,
        SharedPtyReceiver,
        SharedReplayBuffer,
        ActiveClientCounter,
    ),
    String,
> {
    if let Some(existing) = sessions().get(session_key) {
        if existing.mode != mode {
            return Err("terminal session exists with a different mode".to_string());
        }
        return Ok((
            Arc::clone(&existing.actor),
            Arc::clone(&existing.rx),
            Arc::clone(&existing.replay),
            Arc::clone(&existing.active_clients),
        ));
    }
    if sessions().len() >= max_sessions {
        return Err(format!(
            "terminal session limit reached ({max_sessions}); terminate an existing session first"
        ));
    }
    let (tx, rx) = mpsc::channel::<PtyEvent>(512);
    let actor = SessionActor::spawn(spec, tx)?;
    let entry = TerminalSession {
        actor: Arc::new(actor),
        rx: Arc::new(Mutex::new(rx)),
        replay: Arc::new(Mutex::new(Vec::new())),
        active_clients: Arc::new(AtomicUsize::new(0)),
        mode,
    };
    let actor_ref = Arc::clone(&entry.actor);
    let rx_ref = Arc::clone(&entry.rx);
    let replay_ref = Arc::clone(&entry.replay);
    let active_ref = Arc::clone(&entry.active_clients);
    sessions().insert(session_key.to_string(), entry);
    Ok((actor_ref, rx_ref, replay_ref, active_ref))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientFrameAction {
    Continue,
    Terminate,
}

fn dispatch_client_frame(actor: &SessionActor, text: &str) -> Result<ClientFrameAction, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid json: {e}"))?;
    match v["type"].as_str().unwrap_or("") {
        "input" => {
            let data = v["data"].as_str().unwrap_or("");
            actor.write_stdin(data.as_bytes())?;
            Ok(ClientFrameAction::Continue)
        }
        "resize" => {
            let rows = clamp_pty_dimension(v["rows"].as_u64().unwrap_or(24), 1, 200);
            let cols = clamp_pty_dimension(v["cols"].as_u64().unwrap_or(80), 2, 400);
            actor.resize(rows, cols)?;
            Ok(ClientFrameAction::Continue)
        }
        "signal" => {
            let sig = v["signal"].as_str().unwrap_or("");
            let bytes: &[u8] = match sig {
                "SIGINT" | "sigint" | "intr" => b"\x03",
                "SIGQUIT" | "sigquit" => b"\x1c",
                "SIGTSTP" | "sigtstp" | "susp" => b"\x1a",
                "EOF" | "eof" => b"\x04",
                _ => return Err(format!("unknown signal: {sig}")),
            };
            actor.write_stdin(bytes)?;
            Ok(ClientFrameAction::Continue)
        }
        "terminate" => {
            actor.terminate()?;
            Ok(ClientFrameAction::Terminate)
        }
        other => Err(format!("unknown frame type: {other}")),
    }
}

async fn send_error(sink: &Arc<Mutex<SplitSinkAlias>>, message: &str) -> Result<(), ()> {
    sink.lock()
        .await
        .send(Message::Text(
            serde_json::json!({"type":"error","message": message})
                .to_string()
                .into(),
        ))
        .await
        .map_err(|_| ())
}

async fn send_replay_buffer(sink: &Arc<Mutex<SplitSinkAlias>>, replay: &SharedReplayBuffer) {
    let snapshot = { replay.lock().await.clone() };
    if snapshot.is_empty() {
        return;
    }
    let frame = serde_json::json!({
        "type": "output",
        "data": String::from_utf8_lossy(&snapshot).to_string(),
    });
    let _ = sink
        .lock()
        .await
        .send(Message::Text(frame.to_string().into()))
        .await;
}

fn append_replay_buffer(buffer: &mut Vec<u8>, bytes: &[u8]) {
    buffer.extend_from_slice(bytes);
    if buffer.len() > MAX_REPLAY_BUFFER_BYTES {
        let excess = buffer.len() - MAX_REPLAY_BUFFER_BYTES;
        buffer.drain(..excess);
    }
}

#[derive(Default)]
struct PtyUtf8Decoder {
    pending: Vec<u8>,
}

impl PtyUtf8Decoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();

        loop {
            let (valid_up_to, error_len) = match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    output.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => (error.valid_up_to(), error.error_len()),
            };

            if valid_up_to > 0 {
                let valid = std::str::from_utf8(&self.pending[..valid_up_to])
                    .expect("valid_up_to always identifies UTF-8");
                output.push_str(valid);
            }

            match error_len {
                Some(invalid_len) => {
                    output.push('\u{fffd}');
                    self.pending.drain(..valid_up_to + invalid_len);
                }
                None => {
                    self.pending.drain(..valid_up_to);
                    break;
                }
            }
        }

        output
    }

    fn finish(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned()
    }
}

fn spawn_output_forwarder(
    session_key: String,
    rx: SharedPtyReceiver,
    replay: SharedReplayBuffer,
    sink: Arc<Mutex<SplitSinkAlias>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut utf8 = PtyUtf8Decoder::default();
        loop {
            let event_opt = { rx.lock().await.recv().await };
            let Some(event) = event_opt else {
                return;
            };
            let exited = matches!(&event, PtyEvent::Exited(_));
            let frames = match event {
                PtyEvent::Output(bytes) => {
                    {
                        let mut replay_guard = replay.lock().await;
                        append_replay_buffer(&mut replay_guard, &bytes);
                    }
                    let data = utf8.push(&bytes);
                    if data.is_empty() {
                        Vec::new()
                    } else {
                        vec![serde_json::json!({
                            "type": "output",
                            "data": data,
                        })]
                    }
                }
                PtyEvent::Exited(code) => terminal_end_frames(
                    utf8.finish(),
                    serde_json::json!({"type": "exit", "code": code}),
                ),
                PtyEvent::Error(msg) => terminal_end_frames(
                    utf8.finish(),
                    serde_json::json!({"type": "error", "message": msg}),
                ),
            };
            let mut s = sink.lock().await;
            for frame in frames {
                if s.send(Message::Text(frame.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            if exited {
                sessions().remove(&session_key);
                return;
            }
        }
    })
}

fn terminal_end_frames(
    trailing_output: String,
    terminal_frame: serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut frames = Vec::with_capacity(2);
    if !trailing_output.is_empty() {
        frames.push(serde_json::json!({
            "type": "output",
            "data": trailing_output,
        }));
    }
    frames.push(terminal_frame);
    frames
}

fn build_terminal_request(
    state: &AppState,
    session_id: &str,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<TerminalRequest, String> {
    let mode = terminal_mode(&state.kernel.config.web_terminal, uri)?;
    let rows = query_param(uri, "rows")
        .and_then(|s| s.parse::<u64>().ok())
        .map(|v| clamp_pty_dimension(v, 1, 200))
        .unwrap_or(30);
    let cols = query_param(uri, "cols")
        .and_then(|s| s.parse::<u64>().ok())
        .map(|v| clamp_pty_dimension(v, 2, 400))
        .unwrap_or(100);

    let mut spec = match mode {
        TerminalMode::Captain => captain_chat_spec(&state.kernel.config),
        TerminalMode::Shell => SessionSpec::default(),
    };
    spec.rows = rows;
    spec.cols = cols;
    spec.env.extend(terminal_env());
    spec.remove_env.extend(terminal_env_removals());
    append_web_session_env(&mut spec, mode, headers, uri);
    if spec.cwd.is_none() {
        spec.cwd = Some(
            best_terminal_cwd(&state.kernel.config)
                .display()
                .to_string(),
        );
    }

    Ok(TerminalRequest {
        session_id: session_id.to_string(),
        session_key: format!("{}:{session_id}", mode.as_str()),
        mode,
        spec,
    })
}

fn terminal_mode(
    config: &captain_types::config::WebTerminalConfig,
    uri: &Uri,
) -> Result<TerminalMode, String> {
    let raw = query_param(uri, "mode").unwrap_or_else(|| config.default_mode.clone());
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "captain" | "tui" => Ok(TerminalMode::Captain),
        "shell" | "raw" => {
            if config.allow_raw_shell {
                Ok(TerminalMode::Shell)
            } else {
                Err(
                    "raw shell mode is disabled; set [web_terminal].allow_raw_shell = true"
                        .to_string(),
                )
            }
        }
        other => Err(format!("unknown terminal mode: {other}")),
    }
}

fn captain_chat_spec(config: &captain_types::config::KernelConfig) -> SessionSpec {
    let command = std::env::current_exe()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "captain".to_string());
    SessionSpec {
        shell: Some(command),
        args: vec!["chat".to_string()],
        cwd: Some(best_terminal_cwd(config).display().to_string()),
        env: Vec::new(),
        ..SessionSpec::default()
    }
}

fn terminal_env() -> Vec<(String, String)> {
    vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("CAPTAIN_WEB_TERMINAL".to_string(), "1".to_string()),
    ]
}

fn terminal_env_removals() -> Vec<String> {
    vec!["NO_COLOR".to_string()]
}

fn append_web_session_env(
    spec: &mut SessionSpec,
    mode: TerminalMode,
    headers: &HeaderMap,
    uri: &Uri,
) {
    if mode != TerminalMode::Captain {
        return;
    }
    if let Some(token) = extract_session_cookie(headers) {
        spec.env.push(("CAPTAIN_SESSION_TOKEN".to_string(), token));
    }
    if let Some(session_id) =
        query_param(uri, "resume_session").filter(|value| uuid::Uuid::parse_str(value).is_ok())
    {
        spec.env
            .push(("CAPTAIN_WEB_RESUME_SESSION_ID".to_string(), session_id));
    }
}

fn best_terminal_cwd(config: &captain_types::config::KernelConfig) -> PathBuf {
    let workspaces = config.effective_workspaces_dir();
    if workspaces.is_dir() {
        return workspaces;
    }
    config.home_dir.clone()
}

fn validate_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty() || session_id.len() > 80 {
        return Err("terminal session id must be 1..80 characters".to_string());
    }
    if session_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err("terminal session id may only contain letters, numbers, '.', '_' and '-'".to_string())
    }
}

fn validate_origin(headers: &HeaderMap) -> Result<(), String> {
    let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) else {
        return Ok(());
    };
    let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) else {
        return Err("missing Host header for terminal origin check".to_string());
    };
    let Some(origin_host) = normalize_origin_host(origin) else {
        return Err("invalid terminal origin".to_string());
    };
    if origin_host == host.trim().to_ascii_lowercase() {
        Ok(())
    } else {
        Err("terminal WebSocket origin does not match host".to_string())
    }
}

fn normalize_origin_host(origin: &str) -> Option<String> {
    let origin = origin.trim();
    if origin.eq_ignore_ascii_case("null") {
        return None;
    }
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;
    let host = host.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn extract_session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("captain_session=")
                    .map(|v| v.to_string())
            })
        })
}

fn query_param(uri: &Uri, key: &str) -> Option<String> {
    uri.query().and_then(|q| {
        q.split('&').find_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let k = parts.next().unwrap_or("");
            let v = parts.next().unwrap_or("");
            if k == key {
                Some(v.to_string())
            } else {
                None
            }
        })
    })
}

fn ct_eq(token: &str, key: &str) -> bool {
    use subtle::ConstantTimeEq;
    if token.len() != key.len() {
        return false;
    }
    token.as_bytes().ct_eq(key.as_bytes()).into()
}

fn clamp_pty_dimension(value: u64, min: u16, max: u16) -> u16 {
    value.clamp(min as u64, max as u64) as u16
}

fn terminal_error(status: StatusCode, message: &str) -> Response {
    (
        status,
        [("content-type", "application/json")],
        serde_json::json!({ "error": message }).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_actor() -> SessionActor {
        let (tx, _rx) = mpsc::channel::<PtyEvent>(8);
        SessionActor::spawn(
            SessionSpec {
                shell: Some("/bin/sh".to_string()),
                ..SessionSpec::default()
            },
            tx,
        )
        .expect("spawn ok")
    }

    #[tokio::test]
    async fn dispatch_input_writes_bytes_to_actor() {
        let actor = shell_actor();
        let action = dispatch_client_frame(&actor, r#"{"type":"input","data":"echo 9b\n"}"#)
            .expect("input frame should succeed");
        assert_eq!(action, ClientFrameAction::Continue);
    }

    #[tokio::test]
    async fn dispatch_resize_delegates() {
        let actor = shell_actor();
        let action = dispatch_client_frame(&actor, r#"{"type":"resize","rows":40,"cols":120}"#)
            .expect("resize frame should succeed");
        assert_eq!(action, ClientFrameAction::Continue);
    }

    #[tokio::test]
    async fn dispatch_signal_sigint_sends_ctrl_c() {
        let actor = shell_actor();
        let action = dispatch_client_frame(&actor, r#"{"type":"signal","signal":"SIGINT"}"#)
            .expect("SIGINT signal frame should succeed");
        assert_eq!(action, ClientFrameAction::Continue);
    }

    #[tokio::test]
    async fn dispatch_unknown_type_returns_error() {
        let actor = shell_actor();
        let result = dispatch_client_frame(&actor, r#"{"type":"nope"}"#);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dispatch_invalid_json_returns_error() {
        let actor = shell_actor();
        let result = dispatch_client_frame(&actor, "not-json");
        assert!(result.is_err());
    }

    #[test]
    fn validates_terminal_session_ids() {
        assert!(validate_session_id("main-01").is_ok());
        assert!(validate_session_id("../bad").is_err());
        assert!(validate_session_id("").is_err());
    }

    #[test]
    fn terminal_mode_defaults_to_captain_and_gates_shell() {
        let cfg = captain_types::config::WebTerminalConfig::default();
        let uri: Uri = "/api/sessions/main/terminal".parse().unwrap();
        assert_eq!(terminal_mode(&cfg, &uri).unwrap(), TerminalMode::Captain);

        let uri: Uri = "/api/sessions/main/terminal?mode=shell".parse().unwrap();
        assert!(terminal_mode(&cfg, &uri).is_err());

        let cfg = captain_types::config::WebTerminalConfig {
            allow_raw_shell: true,
            ..captain_types::config::WebTerminalConfig::default()
        };
        assert_eq!(terminal_mode(&cfg, &uri).unwrap(), TerminalMode::Shell);
    }

    #[test]
    fn captain_terminal_uses_chat_only_cli() {
        let cfg = captain_types::config::KernelConfig::default();
        let spec = captain_chat_spec(&cfg);
        assert_eq!(spec.args, vec!["chat".to_string()]);
    }

    #[test]
    fn web_terminal_removes_no_color_inherited_from_daemon() {
        assert!(terminal_env_removals().iter().any(|key| key == "NO_COLOR"));
    }

    #[test]
    fn replay_buffer_keeps_recent_terminal_output() {
        let mut buffer = vec![b'a'; MAX_REPLAY_BUFFER_BYTES - 2];
        append_replay_buffer(&mut buffer, b"bcdef");
        assert_eq!(buffer.len(), MAX_REPLAY_BUFFER_BYTES);
        assert_eq!(&buffer[buffer.len() - 5..], b"bcdef");
    }

    #[test]
    fn pty_utf8_decoder_preserves_an_emoji_split_across_chunks() {
        let mut decoder = PtyUtf8Decoder::default();
        let response = "Oui, tout roule 👍\n\nJe suis opérationnel.".as_bytes();
        let emoji_offset = "Oui, tout roule ".len();

        let first = decoder.push(&response[..emoji_offset + 2]);
        let second = decoder.push(&response[emoji_offset + 2..]);

        assert_eq!(
            format!("{first}{second}"),
            "Oui, tout roule 👍\n\nJe suis opérationnel."
        );
        assert!(decoder.finish().is_empty());
    }

    #[test]
    fn pty_utf8_decoder_replaces_invalid_bytes_without_losing_valid_output() {
        let mut decoder = PtyUtf8Decoder::default();

        assert_eq!(decoder.push(b"ok\xffdone"), "ok\u{fffd}done");
    }

    #[test]
    fn origin_must_match_host_when_present() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "captain.example.com".parse().unwrap());
        headers.insert("origin", "https://captain.example.com".parse().unwrap());
        assert!(validate_origin(&headers).is_ok());

        headers.insert("origin", "https://evil.example.com".parse().unwrap());
        assert!(validate_origin(&headers).is_err());
    }

    #[test]
    fn web_session_env_only_flows_to_captain_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "cookie",
            "theme=dark; captain_session=session-token; other=1"
                .parse()
                .unwrap(),
        );
        let uri: Uri = "/api/sessions/main/terminal".parse().unwrap();

        let mut spec = SessionSpec::default();
        append_web_session_env(&mut spec, TerminalMode::Captain, &headers, &uri);
        assert!(spec
            .env
            .iter()
            .any(|(k, v)| { k == "CAPTAIN_SESSION_TOKEN" && v == "session-token" }));

        let mut shell_spec = SessionSpec::default();
        append_web_session_env(&mut shell_spec, TerminalMode::Shell, &headers, &uri);
        assert!(!shell_spec
            .env
            .iter()
            .any(|(k, _)| k == "CAPTAIN_SESSION_TOKEN"));
    }

    #[test]
    fn web_resume_session_env_requires_uuid() {
        let headers = HeaderMap::new();
        let uuid = uuid::Uuid::new_v4().to_string();
        let uri: Uri = format!("/api/sessions/main/terminal?resume_session={uuid}")
            .parse()
            .unwrap();
        let mut spec = SessionSpec::default();
        append_web_session_env(&mut spec, TerminalMode::Captain, &headers, &uri);
        assert!(spec
            .env
            .iter()
            .any(|(k, v)| { k == "CAPTAIN_WEB_RESUME_SESSION_ID" && v == &uuid }));

        let uri: Uri = "/api/sessions/main/terminal?resume_session=web-old"
            .parse()
            .unwrap();
        let mut spec = SessionSpec::default();
        append_web_session_env(&mut spec, TerminalMode::Captain, &headers, &uri);
        assert!(!spec
            .env
            .iter()
            .any(|(k, _)| k == "CAPTAIN_WEB_RESUME_SESSION_ID"));
    }
}
