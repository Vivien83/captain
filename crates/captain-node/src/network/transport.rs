//! Bounded authenticated HTTPS adapters for the durable Node rail.

use super::{valid_access_token_shape, NodeHttpClient, NodeNetworkError};
use captain_wire::{
    HubNodeCloseRequest, HubNodeConnectRequest, HubNodeDeliveryBatch, HubNodeEnvelope,
    HubNodeIngressRequest, HubNodePullRequest, HubNodeStreamRequest, HubNodeWebSocketFrame,
    NodeTransport, MAX_HUB_NODE_FRAME_BYTES,
};
use futures::{SinkExt, StreamExt};
use reqwest::{header, header::RETRY_AFTER, Response, StatusCode, Url};
use reqwest_websocket::{Message, WebSocket};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{fmt, time::Duration};
use zeroize::Zeroizing;

impl NodeHttpClient {
    pub async fn connect_http(
        &self,
        access_token: &str,
        transport: NodeTransport,
        hello: &HubNodeEnvelope,
    ) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
        if transport == NodeTransport::WebSocket {
            return Err(NodeNetworkError::InvalidHubResponse);
        }
        let request = HubNodeConnectRequest {
            transport,
            hello: hello.clone(),
        };
        request
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let batch = self
            .authenticated_post(&self.endpoints.connect, access_token, &request)
            .await?;
        validate_batch(batch)
    }

    pub async fn send_http_envelope(
        &self,
        access_token: &str,
        transport: NodeTransport,
        envelope: &HubNodeEnvelope,
    ) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
        if transport == NodeTransport::WebSocket {
            return Err(NodeNetworkError::InvalidHubResponse);
        }
        let request = HubNodeIngressRequest {
            transport,
            envelope: envelope.clone(),
        };
        request
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let batch = self
            .authenticated_post(&self.endpoints.envelope, access_token, &request)
            .await?;
        validate_batch(batch)
    }

    pub async fn pull_long_poll(
        &self,
        access_token: &str,
        request: &HubNodePullRequest,
    ) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
        request
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let batch = self
            .authenticated_post(&self.endpoints.pull, access_token, request)
            .await?;
        validate_batch(batch)
    }

    pub async fn close_http(
        &self,
        access_token: &str,
        request: &HubNodeCloseRequest,
    ) -> Result<(), NodeNetworkError> {
        request
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let response: CloseResponse = self
            .authenticated_post(&self.endpoints.close, access_token, request)
            .await?;
        if response.status == "offline" {
            Ok(())
        } else {
            Err(NodeNetworkError::InvalidHubResponse)
        }
    }

    pub async fn open_http_stream(
        &self,
        access_token: &str,
        request: &HubNodeStreamRequest,
    ) -> Result<NodeHttpStream, NodeNetworkError> {
        if !valid_access_token_shape(access_token) {
            return Err(NodeNetworkError::InvalidAccessToken);
        }
        request
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let response = tokio::time::timeout(
            self.request_timeout,
            self.client
                .get(self.endpoints.stream.clone())
                .bearer_auth(access_token)
                .header(header::ACCEPT, "text/event-stream")
                .query(request)
                .send(),
        )
        .await
        .map_err(|_| NodeNetworkError::RequestTimedOut)?
        .map_err(classify_request_failure)?;
        let status = response.status();
        if status != StatusCode::OK {
            let retry_after_secs = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let body = tokio::time::timeout(self.request_timeout, bounded_response_body(response))
                .await
                .map_err(|_| NodeNetworkError::RequestTimedOut)??;
            return Err(classify_hub_error(status, retry_after_secs, &body));
        }
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim() == "text/event-stream")
        {
            return Err(NodeNetworkError::InvalidHubResponse);
        }
        Ok(NodeHttpStream {
            response,
            buffer: Zeroizing::new(Vec::new()),
        })
    }

    pub async fn open_rail_websocket(
        &self,
        access_token: &str,
    ) -> Result<NodeWebSocket, NodeNetworkError> {
        self.open_websocket(access_token)
            .await
            .map(|socket| NodeWebSocket { socket })
    }

    async fn authenticated_post<T: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        endpoint: &Url,
        access_token: &str,
        request: &T,
    ) -> Result<R, NodeNetworkError> {
        if !valid_access_token_shape(access_token) {
            return Err(NodeNetworkError::InvalidAccessToken);
        }
        let response = tokio::time::timeout(
            self.request_timeout,
            self.client
                .post(endpoint.clone())
                .bearer_auth(access_token)
                .json(request)
                .send(),
        )
        .await
        .map_err(|_| NodeNetworkError::RequestTimedOut)?
        .map_err(classify_request_failure)?;
        let status = response.status();
        let retry_after_secs = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        let body = tokio::time::timeout(self.request_timeout, bounded_response_body(response))
            .await
            .map_err(|_| NodeNetworkError::RequestTimedOut)??;
        if status != StatusCode::OK {
            return Err(classify_hub_error(status, retry_after_secs, &body));
        }
        serde_json::from_slice(&body).map_err(|_| NodeNetworkError::InvalidHubResponse)
    }
}

pub struct NodeHttpStream {
    response: Response,
    buffer: Zeroizing<Vec<u8>>,
}

impl NodeHttpStream {
    pub async fn next_delivery(
        &mut self,
        idle_timeout: Duration,
    ) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
        if idle_timeout.is_zero() {
            return Err(NodeNetworkError::RequestTimedOut);
        }
        loop {
            if let Some(batch) = take_sse_delivery(&mut self.buffer)? {
                return Ok(batch);
            }
            let chunk = tokio::time::timeout(idle_timeout, self.response.chunk())
                .await
                .map_err(|_| NodeNetworkError::RequestTimedOut)?
                .map_err(classify_request_failure)?
                .ok_or(NodeNetworkError::TransportClosed)?;
            if self.buffer.len().saturating_add(chunk.len()) > MAX_HUB_NODE_FRAME_BYTES {
                return Err(NodeNetworkError::HubResponseTooLarge);
            }
            self.buffer.extend_from_slice(&chunk);
        }
    }
}

impl fmt::Debug for NodeHttpStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeHttpStream")
            .field("buffered_bytes", &self.buffer.len())
            .finish_non_exhaustive()
    }
}

pub struct NodeWebSocket {
    socket: WebSocket,
}

impl NodeWebSocket {
    pub async fn send_envelope(
        &mut self,
        envelope: &HubNodeEnvelope,
    ) -> Result<(), NodeNetworkError> {
        let frame = HubNodeWebSocketFrame::NodeEnvelope {
            envelope: envelope.clone(),
        };
        frame
            .validate()
            .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let payload =
            serde_json::to_string(&frame).map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        if payload.len() > MAX_HUB_NODE_FRAME_BYTES {
            return Err(NodeNetworkError::HubResponseTooLarge);
        }
        self.socket
            .send(Message::Text(payload))
            .await
            .map_err(|_| NodeNetworkError::NetworkUnavailable)
    }

    pub async fn next_delivery(
        &mut self,
        idle_timeout: Duration,
    ) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
        if idle_timeout.is_zero() {
            return Err(NodeNetworkError::RequestTimedOut);
        }
        loop {
            let message = tokio::time::timeout(idle_timeout, self.socket.next())
                .await
                .map_err(|_| NodeNetworkError::RequestTimedOut)?
                .ok_or(NodeNetworkError::TransportClosed)?
                .map_err(|_| NodeNetworkError::NetworkUnavailable)?;
            match message {
                Message::Text(payload) => {
                    if payload.len() > MAX_HUB_NODE_FRAME_BYTES {
                        return Err(NodeNetworkError::HubResponseTooLarge);
                    }
                    let frame: HubNodeWebSocketFrame = serde_json::from_str(&payload)
                        .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
                    frame
                        .validate()
                        .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
                    return match frame {
                        HubNodeWebSocketFrame::HubDelivery { batch } => Ok(batch),
                        HubNodeWebSocketFrame::NodeEnvelope { .. } => {
                            Err(NodeNetworkError::InvalidHubResponse)
                        }
                    };
                }
                Message::Ping(payload) => self
                    .socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| NodeNetworkError::NetworkUnavailable)?,
                Message::Pong(_) => {}
                Message::Binary(_) => return Err(NodeNetworkError::InvalidHubResponse),
                Message::Close { .. } => return Err(NodeNetworkError::TransportClosed),
            }
        }
    }
}

impl fmt::Debug for NodeWebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeWebSocket")
            .finish_non_exhaustive()
    }
}

fn validate_batch(batch: HubNodeDeliveryBatch) -> Result<HubNodeDeliveryBatch, NodeNetworkError> {
    batch
        .validate()
        .map_err(|_| NodeNetworkError::InvalidHubResponse)?;
    Ok(batch)
}

fn take_sse_delivery(
    buffer: &mut Zeroizing<Vec<u8>>,
) -> Result<Option<HubNodeDeliveryBatch>, NodeNetworkError> {
    loop {
        let Some((event_end, delimiter_len)) = find_sse_event_end(buffer) else {
            return Ok(None);
        };
        let event = Zeroizing::new(buffer.drain(..event_end).collect::<Vec<_>>());
        buffer.drain(..delimiter_len);
        let event =
            std::str::from_utf8(&event).map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        let mut event_name = None;
        let mut data = Zeroizing::new(String::new());
        for raw_line in event.lines() {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                event_name = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        if event_name != Some("delivery") {
            continue;
        }
        if data.is_empty() || data.len() > MAX_HUB_NODE_FRAME_BYTES {
            return Err(NodeNetworkError::InvalidHubResponse);
        }
        let batch =
            serde_json::from_str(&data).map_err(|_| NodeNetworkError::InvalidHubResponse)?;
        return validate_batch(batch).map(Some);
    }
}

fn find_sse_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4));
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, 2));
    match (crlf, lf) {
        (Some(crlf), Some(lf)) => Some(if crlf.0 <= lf.0 { crlf } else { lf }),
        (Some(delimiter), None) | (None, Some(delimiter)) => Some(delimiter),
        (None, None) => None,
    }
}

fn classify_request_failure(error: reqwest::Error) -> NodeNetworkError {
    if error.is_timeout() {
        NodeNetworkError::RequestTimedOut
    } else {
        NodeNetworkError::NetworkUnavailable
    }
}

async fn bounded_response_body(response: Response) -> Result<Zeroizing<Vec<u8>>, NodeNetworkError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HUB_NODE_FRAME_BYTES as u64)
    {
        return Err(NodeNetworkError::HubResponseTooLarge);
    }
    let mut body = Zeroizing::new(Vec::new());
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_request_failure)?;
        if body.len().saturating_add(chunk.len()) > MAX_HUB_NODE_FRAME_BYTES {
            return Err(NodeNetworkError::HubResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct CloseResponse {
    status: String,
}

#[derive(Deserialize)]
struct HubErrorEnvelope {
    error: HubErrorBody,
}

#[derive(Deserialize)]
struct HubErrorBody {
    code: String,
}

fn classify_hub_error(
    status: StatusCode,
    retry_after_secs: Option<u64>,
    body: &[u8],
) -> NodeNetworkError {
    let code = serde_json::from_slice::<HubErrorEnvelope>(body)
        .ok()
        .map(|error| sanitize_error_code(&error.error.code))
        .unwrap_or_else(|| "unknown".to_string());
    match code.as_str() {
        "hub_node_authentication_failed" => NodeNetworkError::HubAuthenticationFailed,
        "hub_node_disabled" => NodeNetworkError::HubTransportDisabled,
        "hub_node_state_conflict" => NodeNetworkError::HubStateConflict,
        "hub_node_transport_busy" => NodeNetworkError::HubTransportBusy {
            retry_after_secs: retry_after_secs.unwrap_or(1),
        },
        "hub_node_unavailable" => NodeNetworkError::HubUnavailable,
        _ if status == StatusCode::UNAUTHORIZED => NodeNetworkError::HubAuthenticationFailed,
        _ if status == StatusCode::TOO_MANY_REQUESTS => NodeNetworkError::HubTransportBusy {
            retry_after_secs: retry_after_secs.unwrap_or(1),
        },
        _ if status == StatusCode::SERVICE_UNAVAILABLE => NodeNetworkError::HubUnavailable,
        _ => NodeNetworkError::HubRejected {
            status: status.as_u16(),
            code,
        },
    }
}

fn sanitize_error_code(value: &str) -> String {
    if !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_wire::{HubNodeEnvelope, HubNodeMessage, NodeTransport, HUB_NODE_PROTOCOL_VERSION};

    fn delivery_batch() -> HubNodeDeliveryBatch {
        HubNodeDeliveryBatch {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: "node-office".to_string(),
            connection_id: "connection-stable".to_string(),
            acknowledged_node_sequence: 1,
            messages: vec![HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: "node-office".to_string(),
                connection_id: "connection-stable".to_string(),
                sequence: 1,
                ack_sequence: None,
                sent_at_ms: 20,
                message: HubNodeMessage::Welcome {
                    negotiated_version: HUB_NODE_PROTOCOL_VERSION,
                    transport: NodeTransport::HttpStream,
                    heartbeat_interval_ms: 15_000,
                    lease_duration_ms: 60_000,
                },
            }],
            retry_after_ms: None,
        }
    }

    fn error_body(code: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"error": {"code": code}})).unwrap()
    }

    #[test]
    fn transport_errors_are_categorical_and_remote_codes_are_sanitized() {
        assert_eq!(
            classify_hub_error(
                StatusCode::UNAUTHORIZED,
                None,
                &error_body("hub_node_authentication_failed"),
            ),
            NodeNetworkError::HubAuthenticationFailed
        );
        assert_eq!(
            classify_hub_error(
                StatusCode::TOO_MANY_REQUESTS,
                Some(7),
                &error_body("hub_node_transport_busy"),
            ),
            NodeNetworkError::HubTransportBusy {
                retry_after_secs: 7
            }
        );
        assert_eq!(
            classify_hub_error(
                StatusCode::BAD_GATEWAY,
                None,
                &error_body("unsafe\nremote detail"),
            ),
            NodeNetworkError::HubRejected {
                status: 502,
                code: "unknown".to_string(),
            }
        );
    }

    #[test]
    fn sse_parser_preserves_fragmented_delivery_after_keepalive() {
        let expected = delivery_batch();
        let encoded = serde_json::to_string(&expected).unwrap();
        let split = encoded.len() / 2;
        let mut buffer = Zeroizing::new(
            format!(
                ": keepalive\n\nevent: delivery\ndata: {}",
                &encoded[..split]
            )
            .into_bytes(),
        );

        assert_eq!(take_sse_delivery(&mut buffer).unwrap(), None);
        buffer.extend_from_slice(&encoded.as_bytes()[split..]);
        buffer.extend_from_slice(b"\n\n");

        assert_eq!(take_sse_delivery(&mut buffer).unwrap(), Some(expected));
        assert!(buffer.is_empty());
    }

    #[test]
    fn sse_parser_accepts_crlf_and_ignores_non_delivery_events() {
        let expected = delivery_batch();
        let encoded = serde_json::to_string(&expected).unwrap();
        let mut buffer = Zeroizing::new(
            format!(
                "event: presence\r\ndata: online\r\n\r\nevent: delivery\r\ndata: {encoded}\r\n\r\n"
            )
            .into_bytes(),
        );

        assert_eq!(take_sse_delivery(&mut buffer).unwrap(), Some(expected));
        assert!(buffer.is_empty());
    }

    #[test]
    fn sse_parser_rejects_invalid_direction_and_oversized_data() {
        let mut wrong_direction = delivery_batch();
        wrong_direction.messages[0].message = HubNodeMessage::Heartbeat {
            active_run_ids: Vec::new(),
        };
        let encoded = serde_json::to_string(&wrong_direction).unwrap();
        let mut buffer =
            Zeroizing::new(format!("event: delivery\ndata: {encoded}\n\n").into_bytes());
        assert_eq!(
            take_sse_delivery(&mut buffer),
            Err(NodeNetworkError::InvalidHubResponse)
        );

        let mut oversized = Zeroizing::new(b"event: delivery\ndata: ".to_vec());
        oversized.extend(std::iter::repeat_n(
            b'x',
            MAX_HUB_NODE_FRAME_BYTES.saturating_add(1),
        ));
        oversized.extend_from_slice(b"\n\n");
        assert_eq!(
            take_sse_delivery(&mut oversized),
            Err(NodeNetworkError::InvalidHubResponse)
        );
    }
}
