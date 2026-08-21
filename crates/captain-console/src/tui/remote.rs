use super::model::{
    captain_agent, messages_from_body, session_from_value, sessions_from_body, AgentInfo,
    ChatMessage, SessionInfo, StreamSignal,
};
use crate::manager::ConsoleConnection;
use bytes::{Bytes, BytesMut};
use captain_node::ClientAccessTransport;
use futures::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Method, Response, StatusCode,
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

const MAX_AGENTS_BODY_BYTES: usize = 1024 * 1024;
const MAX_SESSIONS_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_SESSION_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
const MAX_SSE_BUFFER_BYTES: usize = 512 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const BODY_DEADLINE: Duration = Duration::from_secs(10);
const STREAM_IDLE_DEADLINE: Duration = Duration::from_secs(45);

pub(super) struct ChatBootstrap {
    pub agent: AgentInfo,
    pub sessions: Vec<SessionInfo>,
    pub selected_session: usize,
    pub messages: Vec<ChatMessage>,
}

pub(super) async fn bootstrap(
    connection: &ConsoleConnection,
) -> Result<ChatBootstrap, ConsoleRemoteError> {
    let agents = json_request(
        &connection.transport,
        Method::GET,
        "/api/agents",
        None,
        MAX_AGENTS_BODY_BYTES,
    )
    .await?;
    let agent = captain_agent(&agents).ok_or(ConsoleRemoteError::CaptainUnavailable)?;
    let mut sessions = list_sessions(&connection.transport, &agent.id).await?;
    if sessions.is_empty() {
        sessions.push(create_session(&connection.transport, &agent.id).await?);
    }
    let selected_session = sessions
        .iter()
        .position(|session| session.active)
        .unwrap_or(0);
    let messages = load_session(&connection.transport, &sessions[selected_session].id).await?;
    Ok(ChatBootstrap {
        agent,
        sessions,
        selected_session,
        messages,
    })
}

pub(super) async fn list_sessions(
    transport: &Arc<ClientAccessTransport>,
    agent_id: &str,
) -> Result<Vec<SessionInfo>, ConsoleRemoteError> {
    let body = json_request(
        transport,
        Method::GET,
        &format!("/api/agents/{agent_id}/sessions"),
        None,
        MAX_SESSIONS_BODY_BYTES,
    )
    .await?;
    Ok(sessions_from_body(&body))
}

pub(super) async fn load_session(
    transport: &Arc<ClientAccessTransport>,
    session_id: &str,
) -> Result<Vec<ChatMessage>, ConsoleRemoteError> {
    let body = json_request(
        transport,
        Method::GET,
        &format!("/api/sessions/{session_id}"),
        None,
        MAX_SESSION_BODY_BYTES,
    )
    .await?;
    Ok(messages_from_body(&body))
}

pub(super) async fn create_session(
    transport: &Arc<ClientAccessTransport>,
    agent_id: &str,
) -> Result<SessionInfo, ConsoleRemoteError> {
    let body = json_request(
        transport,
        Method::POST,
        &format!("/api/agents/{agent_id}/sessions"),
        Some(serde_json::json!({
            "label": "Captain Console",
            "activate": false,
        })),
        MAX_RESPONSE_BODY_BYTES,
    )
    .await?;
    session_from_value(&body).ok_or(ConsoleRemoteError::InvalidResponse)
}

pub(super) fn spawn_message_stream(
    transport: Arc<ClientAccessTransport>,
    agent_id: String,
    session_id: String,
    message: String,
    sender: mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
) -> Result<JoinHandle<()>, ConsoleRemoteError> {
    if message.trim().is_empty() || message.len() > MAX_INPUT_BYTES {
        return Err(ConsoleRemoteError::InvalidMessage);
    }
    Ok(tokio::spawn(async move {
        let result = stream_message(transport, &agent_id, &session_id, message, &sender).await;
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    }))
}

pub(super) async fn answer_question(
    transport: &Arc<ClientAccessTransport>,
    agent_id: &str,
    session_id: &str,
    answer: String,
) -> Result<(), ConsoleRemoteError> {
    if answer.trim().is_empty() || answer.len() > MAX_INPUT_BYTES {
        return Err(ConsoleRemoteError::InvalidMessage);
    }
    json_request(
        transport,
        Method::POST,
        &format!("/api/agents/{agent_id}/message/answer"),
        Some(serde_json::json!({
            "content": answer,
            "session_id": session_id,
        })),
        MAX_RESPONSE_BODY_BYTES,
    )
    .await?;
    Ok(())
}

async fn stream_message(
    transport: Arc<ClientAccessTransport>,
    agent_id: &str,
    session_id: &str,
    message: String,
    sender: &mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
) -> Result<(), ConsoleRemoteError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "message": message,
        "session_id": session_id,
        "attachments": [],
        "channel_type": "console",
    }))
    .map_err(|_| ConsoleRemoteError::InvalidRequest)?;
    let response = transport
        .execute(
            Method::POST,
            &format!("/api/agents/{agent_id}/message/stream"),
            &json_headers()?,
            Bytes::from(body),
        )
        .await
        .map_err(|_| ConsoleRemoteError::AuthorityUnavailable)?;
    ensure_success(response.status())?;

    let mut decoder = SseDecoder::default();
    let mut stream = response.bytes_stream();
    let mut completed = false;
    loop {
        let next = tokio::time::timeout(STREAM_IDLE_DEADLINE, stream.next())
            .await
            .map_err(|_| ConsoleRemoteError::StreamInterrupted)?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|_| ConsoleRemoteError::StreamInterrupted)?;
        for signal in decoder.push(&chunk)? {
            completed |= signal == StreamSignal::Done;
            sender
                .send(Ok(signal))
                .map_err(|_| ConsoleRemoteError::StreamCancelled)?;
            if completed {
                return Ok(());
            }
        }
    }
    if completed {
        Ok(())
    } else {
        Err(ConsoleRemoteError::StreamInterrupted)
    }
}

async fn json_request(
    transport: &Arc<ClientAccessTransport>,
    method: Method,
    path: &str,
    body: Option<Value>,
    max_body_bytes: usize,
) -> Result<Value, ConsoleRemoteError> {
    let (headers, body) = match body {
        Some(body) => (
            json_headers()?,
            Bytes::from(serde_json::to_vec(&body).map_err(|_| ConsoleRemoteError::InvalidRequest)?),
        ),
        None => (HeaderMap::new(), Bytes::new()),
    };
    let response = transport
        .execute(method, path, &headers, body)
        .await
        .map_err(|_| ConsoleRemoteError::AuthorityUnavailable)?;
    ensure_success(response.status())?;
    let bytes = tokio::time::timeout(BODY_DEADLINE, bounded_body(response, max_body_bytes))
        .await
        .map_err(|_| ConsoleRemoteError::AuthorityUnavailable)??;
    serde_json::from_slice(&bytes).map_err(|_| ConsoleRemoteError::InvalidResponse)
}

fn json_headers() -> Result<HeaderMap, ConsoleRemoteError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn ensure_success(status: StatusCode) -> Result<(), ConsoleRemoteError> {
    match status {
        status if status.is_success() => Ok(()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(ConsoleRemoteError::PairingRejected)
        }
        StatusCode::CONFLICT => Err(ConsoleRemoteError::Conflict),
        StatusCode::TOO_MANY_REQUESTS => Err(ConsoleRemoteError::QuotaUnavailable),
        status => Err(ConsoleRemoteError::RequestRejected(status.as_u16())),
    }
}

async fn bounded_body(
    response: Response,
    max_body_bytes: usize,
) -> Result<Bytes, ConsoleRemoteError> {
    let mut body = BytesMut::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ConsoleRemoteError::AuthorityUnavailable)?;
        if body.len().saturating_add(chunk.len()) > max_body_bytes {
            return Err(ConsoleRemoteError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[derive(Default)]
struct SseDecoder {
    buffer: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<StreamSignal>, ConsoleRemoteError> {
        if self.buffer.len().saturating_add(chunk.len()) > MAX_SSE_BUFFER_BYTES {
            return Err(ConsoleRemoteError::ResponseTooLarge);
        }
        self.buffer.extend_from_slice(chunk);
        let mut signals = Vec::new();
        while let Some((boundary, separator_len)) = event_boundary(&self.buffer) {
            let event = self.buffer[..boundary].to_vec();
            self.buffer.drain(..boundary + separator_len);
            if event.len() > MAX_SSE_EVENT_BYTES {
                return Err(ConsoleRemoteError::ResponseTooLarge);
            }
            if let Some(signal) = decode_event(&event)? {
                signals.push(signal);
            }
        }
        Ok(signals)
    }
}

fn event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(left), Some(right)) if left <= right => Some((left, 2)),
        (Some(_), Some(right)) => Some((right, 4)),
        (Some(left), None) => Some((left, 2)),
        (None, Some(right)) => Some((right, 4)),
        (None, None) => None,
    }
}

fn decode_event(event: &[u8]) -> Result<Option<StreamSignal>, ConsoleRemoteError> {
    let text = std::str::from_utf8(event).map_err(|_| ConsoleRemoteError::InvalidResponse)?;
    let mut data = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let data = data.join("\n");
    if data.len() > MAX_SSE_EVENT_BYTES {
        return Err(ConsoleRemoteError::ResponseTooLarge);
    }
    let value = serde_json::from_str(&data).map_err(|_| ConsoleRemoteError::InvalidResponse)?;
    Ok(super::model::stream_signal(&value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum ConsoleRemoteError {
    #[error("the selected Captain is unavailable")]
    AuthorityUnavailable,
    #[error("the selected Captain no longer authorizes this Console")]
    PairingRejected,
    #[error("Captain has no primary agent available")]
    CaptainUnavailable,
    #[error("Captain rejected the request with HTTP {0}")]
    RequestRejected(u16),
    #[error("Captain rejected the action because its state changed")]
    Conflict,
    #[error("the Captain usage quota is currently unavailable")]
    QuotaUnavailable,
    #[error("Captain returned an invalid response")]
    InvalidResponse,
    #[error("Captain returned more data than the Console accepts")]
    ResponseTooLarge,
    #[error("the Console could not encode the request")]
    InvalidRequest,
    #[error("the message is empty or exceeds 64 KiB")]
    InvalidMessage,
    #[error("the Captain stream was interrupted")]
    StreamInterrupted,
    #[error("the Captain stream was cancelled")]
    StreamCancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_preserves_split_events_and_discards_raw_tool_output() {
        let mut decoder = SseDecoder::default();
        assert!(decoder
            .push(b"event: chunk\ndata: {\"content\":\"hel")
            .unwrap()
            .is_empty());
        assert_eq!(
            decoder
                .push(b"lo\",\"done\":false}\n\nevent: tool_result\ndata: {\"type\":\"tool_result\",\"tool\":\"shell_exec\",\"result\":\"secret\",\"is_error\":false}\n\n")
                .unwrap(),
            vec![
                StreamSignal::Text("hello".to_string()),
                StreamSignal::ToolFinished {
                    name: "shell_exec".to_string(),
                    failed: false,
                },
            ]
        );
    }

    #[test]
    fn decoder_accepts_crlf_and_ignores_keepalive_comments() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder
                .push(b": keep-alive\r\n\r\nevent: done\r\ndata: {\"done\":true}\r\n\r\n")
                .unwrap(),
            vec![StreamSignal::Done]
        );
    }

    #[test]
    fn decoder_rejects_unbounded_or_malformed_events() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder.push(&vec![b'x'; MAX_SSE_BUFFER_BYTES + 1]),
            Err(ConsoleRemoteError::ResponseTooLarge)
        );
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder.push(b"data: not-json\n\n"),
            Err(ConsoleRemoteError::InvalidResponse)
        );
    }
}
