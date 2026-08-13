use super::*;
use futures::{SinkExt, StreamExt};
use reqwest_websocket::Message;

#[test]
fn production_hub_origin_is_exact_https_443_without_credentials_or_path() {
    let mut config = NodeNetworkConfig::new("https://hub.example.com");
    config.proxy = NodeProxyMode::Disabled;
    let client = config.build_client(None).unwrap();
    assert!(config.build_blocking_client(None).is_ok());
    assert!(config
        .build_blocking_client_with_timeout(None, None)
        .is_ok());
    let mut client_headers = HeaderMap::new();
    let mut bearer = reqwest::header::HeaderValue::from_static("Bearer test-client-token");
    bearer.set_sensitive(true);
    client_headers.insert(reqwest::header::AUTHORIZATION, bearer);
    assert!(config
        .build_blocking_client_with_headers(None, Some(Duration::from_secs(5)), client_headers,)
        .is_ok());
    assert_eq!(
        config
            .build_blocking_client_with_timeout(None, Some(Duration::ZERO))
            .unwrap_err(),
        NodeNetworkError::InvalidTimeout
    );
    assert_eq!(
        client.endpoints().connect.as_str(),
        "https://hub.example.com/api/hub/nodes/connect"
    );
    assert_eq!(
        client.endpoints().websocket.as_str(),
        "wss://hub.example.com/api/hub/nodes/ws"
    );

    for invalid in [
        "http://hub.example.com",
        "https://hub.example.com:8443",
        "https://user@hub.example.com",
        "https://hub.example.com/captain",
        "https://hub.example.com/?token=secret",
    ] {
        let mut config = NodeNetworkConfig::new(invalid);
        config.proxy = NodeProxyMode::Disabled;
        assert!(config.build_client(None).is_err());
        assert!(config.build_blocking_client(None).is_err());
    }
}

#[test]
fn environment_proxy_precedence_is_deterministic() {
    let values = std::collections::BTreeMap::from([
        ("HTTPS_PROXY", "http://primary.example.com"),
        ("https_proxy", "http://lowercase.example.com"),
        ("ALL_PROXY", "http://fallback.example.com"),
    ]);
    assert_eq!(
        environment_proxy_url(|key| values.get(key).map(ToString::to_string)).as_deref(),
        Some("http://primary.example.com")
    );
    assert_eq!(
        environment_proxy_url(|key| {
            (key == "ALL_PROXY").then(|| "http://fallback.example.com".to_string())
        })
        .as_deref(),
        Some("http://fallback.example.com")
    );
}

#[test]
fn explicit_proxy_credentials_require_a_named_resolved_secret() {
    let mut config = NodeNetworkConfig::new("https://hub.example.com");
    config.proxy = NodeProxyMode::Explicit {
        url: "http://proxy.example.com:8080".to_string(),
        username: Some("captain-node".to_string()),
        password_secret: Some("proxy-password".to_string()),
    };
    assert_eq!(
        config.build_client(None).unwrap_err(),
        NodeNetworkError::ProxyPasswordRequired
    );
    assert_eq!(
        config.build_blocking_client(None).unwrap_err(),
        NodeNetworkError::ProxyPasswordRequired
    );
    assert!(config
        .build_client(Some(&ResolvedProxyPassword::new(
            "proxy-password",
            "not-rendered"
        )))
        .is_ok());
    assert_eq!(
        config
            .build_client(Some(&ResolvedProxyPassword::new("other", "not-rendered")))
            .unwrap_err(),
        NodeNetworkError::ProxyPasswordRequired
    );

    config.proxy = NodeProxyMode::Explicit {
        url: "http://user:password@proxy.example.com:8080".to_string(),
        username: None,
        password_secret: None,
    };
    assert_eq!(
        config.build_client(None).unwrap_err(),
        NodeNetworkError::ProxyCredentialsInUrl
    );
}

#[test]
fn proxy_password_debug_is_redacted() {
    let password = ResolvedProxyPassword::new("proxy-password", "never-print-this");
    let rendered = format!("{password:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains("never-print-this"));

    let proxy = NodeProxyMode::Explicit {
        url: "http://user:never-print-this@proxy.example.com".to_string(),
        username: Some("user".to_string()),
        password_secret: Some("proxy-password".to_string()),
    };
    let rendered = format!("{proxy:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains("never-print-this"));

    let mut config = NodeNetworkConfig::new("https://private-hub.example.test");
    config.enterprise_ca_bundle = Some(PathBuf::from("/private/company/ca.pem"));
    let rendered = format!("{config:?}");
    assert!(!rendered.contains("private-hub.example.test"));
    assert!(!rendered.contains("/private/company/ca.pem"));

    config.proxy = NodeProxyMode::Disabled;
    config.enterprise_ca_bundle = None;
    let client = config.build_client(None).unwrap();
    let rendered = format!("{client:?}");
    assert!(!rendered.contains("private-hub.example.test"));
    assert!(rendered.contains(HUB_NODE_WEBSOCKET_PATH));
}

#[test]
fn ca_bundle_and_timeout_failures_are_categorical() {
    let temp = tempfile::tempdir().unwrap();
    let invalid_ca = temp.path().join("enterprise.pem");
    std::fs::write(&invalid_ca, b"not a certificate").unwrap();
    let mut config = NodeNetworkConfig::new("https://hub.example.com");
    config.proxy = NodeProxyMode::Disabled;
    config.enterprise_ca_bundle = Some(invalid_ca);
    assert_eq!(
        config.build_client(None).unwrap_err(),
        NodeNetworkError::CaBundleInvalid
    );
    assert_eq!(
        config.build_blocking_client(None).unwrap_err(),
        NodeNetworkError::CaBundleInvalid
    );
    config.enterprise_ca_bundle = None;
    config.connect_timeout_secs = 0;
    assert_eq!(
        config.build_client(None).unwrap_err(),
        NodeNetworkError::InvalidTimeout
    );
    assert_eq!(
        config.build_blocking_client(None).unwrap_err(),
        NodeNetworkError::InvalidTimeout
    );
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn websocket_upgrade_uses_the_shared_reqwest_stack_and_bounded_frames() {
    async fn websocket(ws: axum::extract::ws::WebSocketUpgrade) -> axum::response::Response {
        ws.on_upgrade(|mut socket| async move {
            if let Some(Ok(axum::extract::ws::Message::Text(text))) = socket.recv().await {
                let _ = socket.send(axum::extract::ws::Message::Text(text)).await;
            }
        })
    }

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().route(HUB_NODE_WEBSOCKET_PATH, axum::routing::get(websocket)),
        )
        .await
        .unwrap();
    });
    let mut config = NodeNetworkConfig::new(format!("http://{address}"));
    config.connect_timeout_secs = 1;
    config.request_timeout_secs = 1;
    let client = config.build_loopback_client().unwrap();
    let mut socket = client.open_websocket(&"a".repeat(64)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    socket
        .send(Message::Text("bounded".to_string()))
        .await
        .unwrap();
    let echoed = socket.next().await.unwrap().unwrap();
    assert!(matches!(echoed, Message::Text(text) if text == "bounded"));
    server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn authenticated_stream_transports_exchange_validated_rail_frames() {
    use axum::{
        extract::{ws::WebSocketUpgrade, Query},
        http::{header, HeaderMap},
        response::{
            sse::{Event, Sse},
            Response,
        },
        routing::get,
        Router,
    };
    use captain_wire::{
        CapabilityDescriptor, DeviceRole, HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeMessage,
        HubNodeStreamRequest, HubNodeWebSocketFrame, LogicalWorkspace, NodeTransport,
        HUB_NODE_PROTOCOL_VERSION, HUB_NODE_STREAM_PATH,
    };
    use std::convert::Infallible;

    fn authorize(headers: &HeaderMap) {
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            &format!("Bearer {}", "a".repeat(64))
        );
    }

    fn batch_for(envelope: &HubNodeEnvelope, transport: NodeTransport) -> HubNodeDeliveryBatch {
        HubNodeDeliveryBatch {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: envelope.device_id.clone(),
            connection_id: envelope.connection_id.clone(),
            acknowledged_node_sequence: envelope.sequence,
            messages: vec![HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: envelope.device_id.clone(),
                connection_id: envelope.connection_id.clone(),
                sequence: 1,
                ack_sequence: None,
                sent_at_ms: 20,
                message: HubNodeMessage::Welcome {
                    negotiated_version: HUB_NODE_PROTOCOL_VERSION,
                    transport,
                    heartbeat_interval_ms: 15_000,
                    lease_duration_ms: 60_000,
                },
            }],
            retry_after_ms: None,
        }
    }

    async fn websocket(headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
        authorize(&headers);
        ws.on_upgrade(|mut socket| async move {
            let Some(Ok(axum::extract::ws::Message::Text(payload))) = socket.recv().await else {
                return;
            };
            let frame: HubNodeWebSocketFrame = serde_json::from_str(&payload).unwrap();
            frame.validate().unwrap();
            let HubNodeWebSocketFrame::NodeEnvelope { envelope } = frame else {
                panic!("Node sent a Hub-only WebSocket frame")
            };
            socket
                .send(axum::extract::ws::Message::Ping(vec![1, 2, 3].into()))
                .await
                .unwrap();
            assert!(matches!(
                socket.recv().await,
                Some(Ok(axum::extract::ws::Message::Pong(_)))
            ));
            let delivery = HubNodeWebSocketFrame::HubDelivery {
                batch: batch_for(&envelope, NodeTransport::WebSocket),
            };
            socket
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&delivery).unwrap().into(),
                ))
                .await
                .unwrap();
        })
    }

    async fn stream(
        headers: HeaderMap,
        Query(request): Query<HubNodeStreamRequest>,
    ) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
        authorize(&headers);
        request.validate().unwrap();
        let batch = HubNodeDeliveryBatch {
            protocol_version: request.protocol_version(),
            device_id: request.device_id,
            connection_id: request.connection_id,
            acknowledged_node_sequence: 1,
            messages: Vec::new(),
            retry_after_ms: None,
        };
        let events = futures::stream::iter([Ok(Event::default()
            .event("delivery")
            .data(serde_json::to_string(&batch).unwrap()))]);
        Sse::new(events)
    }

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(HUB_NODE_WEBSOCKET_PATH, get(websocket))
                .route(HUB_NODE_STREAM_PATH, get(stream)),
        )
        .await
        .unwrap();
    });
    let mut config = NodeNetworkConfig::new(format!("http://{address}"));
    config.proxy = NodeProxyMode::Disabled;
    config.connect_timeout_secs = 1;
    config.request_timeout_secs = 2;
    let client = config.build_loopback_client().unwrap();
    let token = "a".repeat(64);
    let hello = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-office".to_string(),
        connection_id: "connection-stable".to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 10,
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: CapabilityDescriptor {
                captain_version: "0.1.0-alpha.14".to_string(),
                platform: "macos-arm64".to_string(),
                transports: vec![NodeTransport::WebSocket, NodeTransport::HttpStream],
                tool_families: vec!["file".to_string()],
                workspaces: vec![LogicalWorkspace {
                    workspace_id: "project-main".to_string(),
                    label: "Main Project".to_string(),
                    read_only: true,
                }],
                supports_streaming_output: true,
            },
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    };

    let mut websocket = client.open_rail_websocket(&token).await.unwrap();
    websocket.send_envelope(&hello).await.unwrap();
    let delivery = websocket
        .next_delivery(Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(delivery.acknowledged_node_sequence, 1);
    assert!(matches!(
        delivery.messages[0].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::WebSocket,
            ..
        }
    ));

    let mut stream = client
        .open_http_stream(
            &token,
            &HubNodeStreamRequest {
                protocol_major: HUB_NODE_PROTOCOL_VERSION.major,
                protocol_minor: HUB_NODE_PROTOCOL_VERSION.minor,
                device_id: hello.device_id,
                connection_id: hello.connection_id,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        stream
            .next_delivery(Duration::from_secs(1))
            .await
            .unwrap()
            .acknowledged_node_sequence,
        1
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn authenticated_http_transport_round_trips_validated_rail_contracts() {
    use axum::{
        http::{header, HeaderMap},
        routing::post,
        Json, Router,
    };
    use captain_wire::{
        CapabilityDescriptor, DeviceRole, HubNodeCloseRequest, HubNodeConnectRequest,
        HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeIngressRequest, HubNodeMessage,
        HubNodePullRequest, LogicalWorkspace, NodeTransport, HUB_NODE_CLOSE_PATH,
        HUB_NODE_CONNECT_PATH, HUB_NODE_ENVELOPE_PATH, HUB_NODE_PROTOCOL_VERSION,
        HUB_NODE_PULL_PATH,
    };

    fn authorize(headers: &HeaderMap) {
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            &format!("Bearer {}", "a".repeat(64))
        );
    }

    fn batch_for(envelope: &HubNodeEnvelope, acknowledged: u64) -> HubNodeDeliveryBatch {
        HubNodeDeliveryBatch {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: envelope.device_id.clone(),
            connection_id: envelope.connection_id.clone(),
            acknowledged_node_sequence: acknowledged,
            messages: Vec::new(),
            retry_after_ms: Some(1_000),
        }
    }

    async fn connect(
        headers: HeaderMap,
        Json(request): Json<HubNodeConnectRequest>,
    ) -> Json<HubNodeDeliveryBatch> {
        authorize(&headers);
        request.validate().unwrap();
        let mut batch = batch_for(&request.hello, 1);
        batch.messages.push(HubNodeEnvelope {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: request.hello.device_id.clone(),
            connection_id: request.hello.connection_id.clone(),
            sequence: 1,
            ack_sequence: None,
            sent_at_ms: 20,
            message: HubNodeMessage::Welcome {
                negotiated_version: HUB_NODE_PROTOCOL_VERSION,
                transport: request.transport,
                heartbeat_interval_ms: 15_000,
                lease_duration_ms: 60_000,
            },
        });
        Json(batch)
    }

    async fn envelope(
        headers: HeaderMap,
        Json(request): Json<HubNodeIngressRequest>,
    ) -> Json<HubNodeDeliveryBatch> {
        authorize(&headers);
        request.validate().unwrap();
        Json(batch_for(&request.envelope, request.envelope.sequence))
    }

    async fn pull(
        headers: HeaderMap,
        Json(request): Json<HubNodePullRequest>,
    ) -> Json<HubNodeDeliveryBatch> {
        authorize(&headers);
        request.validate().unwrap();
        Json(HubNodeDeliveryBatch {
            protocol_version: request.protocol_version,
            device_id: request.device_id,
            connection_id: request.connection_id,
            acknowledged_node_sequence: 2,
            messages: Vec::new(),
            retry_after_ms: Some(1_000),
        })
    }

    async fn close(
        headers: HeaderMap,
        Json(request): Json<HubNodeCloseRequest>,
    ) -> Json<serde_json::Value> {
        authorize(&headers);
        request.validate().unwrap();
        Json(serde_json::json!({"status": "offline"}))
    }

    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(HUB_NODE_CONNECT_PATH, post(connect))
                .route(HUB_NODE_ENVELOPE_PATH, post(envelope))
                .route(HUB_NODE_PULL_PATH, post(pull))
                .route(HUB_NODE_CLOSE_PATH, post(close)),
        )
        .await
        .unwrap();
    });
    let mut config = NodeNetworkConfig::new(format!("http://{address}"));
    config.proxy = NodeProxyMode::Disabled;
    config.connect_timeout_secs = 1;
    config.request_timeout_secs = 2;
    let client = config.build_loopback_client().unwrap();
    let token = "a".repeat(64);
    let hello = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-office".to_string(),
        connection_id: "connection-stable".to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 10,
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: CapabilityDescriptor {
                captain_version: "0.1.0-alpha.14".to_string(),
                platform: "macos-arm64".to_string(),
                transports: vec![NodeTransport::HttpStream, NodeTransport::LongPoll],
                tool_families: vec!["file".to_string()],
                workspaces: vec![LogicalWorkspace {
                    workspace_id: "project-main".to_string(),
                    label: "Main Project".to_string(),
                    read_only: true,
                }],
                supports_streaming_output: true,
            },
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    };
    let opened = client
        .connect_http(&token, NodeTransport::LongPoll, &hello)
        .await
        .unwrap();
    assert_eq!(opened.acknowledged_node_sequence, 1);
    assert!(matches!(
        opened.messages[0].message,
        HubNodeMessage::Welcome { .. }
    ));

    let ack = HubNodeEnvelope {
        sequence: 2,
        ack_sequence: Some(1),
        sent_at_ms: 30,
        message: HubNodeMessage::AckOnly,
        ..hello.clone()
    };
    assert_eq!(
        client
            .send_http_envelope(&token, NodeTransport::LongPoll, &ack)
            .await
            .unwrap()
            .acknowledged_node_sequence,
        2
    );
    let pull_request = HubNodePullRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
        max_messages: 64,
        wait_ms: 1,
    };
    assert_eq!(
        client
            .pull_long_poll(&token, &pull_request)
            .await
            .unwrap()
            .acknowledged_node_sequence,
        2
    );
    client
        .close_http(
            &token,
            &HubNodeCloseRequest {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: hello.device_id,
                connection_id: hello.connection_id,
                error_code: Some("client_shutdown".to_string()),
            },
        )
        .await
        .unwrap();
    server.abort();
}
