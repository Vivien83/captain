use super::*;
use crate::{NodeNetworkConfig, NodePairingProgress, NodePairingStore, NodeProxyMode};
use captain_wire::{
    DeviceRole, HubNodeConnectRequest, HubNodeIngressRequest, HubNodeWebSocketFrame,
    LogicalWorkspace, HUB_NODE_CLOSE_PATH, HUB_NODE_CONNECT_PATH, HUB_NODE_ENVELOPE_PATH,
    HUB_NODE_PULL_PATH, HUB_NODE_STREAM_PATH, HUB_NODE_WEBSOCKET_PATH,
};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

fn capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![
            NodeTransport::LongPoll,
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
        ],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            read_only: true,
        }],
        supports_streaming_output: true,
    }
}

fn paired_store(root: &Path, hub_sha256: &str) -> NodePairingStore {
    let store = NodePairingStore::open(root).unwrap();
    let state = serde_json::json!({
        "schema_version": 1,
        "hub_sha256": hub_sha256,
        "phase": {
            "state": "paired",
            "credential": "b".repeat(64),
            "device_id": "node-office",
            "protocol_version": {
                "major": HUB_NODE_PROTOCOL_VERSION.major,
                "minor": HUB_NODE_PROTOCOL_VERSION.minor,
            }
        }
    });
    fs::write(
        root.join("pairing.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store.status().unwrap(),
        Some(NodePairingProgress::Paired {
            device_id: "node-office".to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        })
    );
    store
}

fn test_token() -> NodeAccessToken {
    let now_ms = current_time_ms().unwrap();
    NodeAccessToken::for_test("a".repeat(64), now_ms - 1_000, now_ms + 60_000)
}

#[test]
fn fallback_policy_is_preferred_bounded_and_excludes_terminal_errors() {
    let hello = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-office".to_string(),
        connection_id: "connection-stable".to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 10,
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: capabilities(),
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    };
    assert_eq!(
        advertised_transports(&hello).unwrap(),
        vec![
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
            NodeTransport::LongPoll,
        ]
    );
    for error in [
        NodeNetworkError::WebSocketUpgradeFailed,
        NodeNetworkError::RequestTimedOut,
        NodeNetworkError::NetworkUnavailable,
        NodeNetworkError::TransportClosed,
        NodeNetworkError::HubUnavailable,
    ] {
        assert!(is_fallback_safe(&error));
    }
    for error in [
        NodeNetworkError::HubAuthenticationFailed,
        NodeNetworkError::HubStateConflict,
        NodeNetworkError::HubTransportBusy {
            retry_after_secs: 1,
        },
        NodeNetworkError::InvalidHubResponse,
        NodeNetworkError::HubResponseTooLarge,
    ] {
        assert!(!is_fallback_safe(&error));
    }
}

#[tokio::test]
async fn link_rejects_expired_tokens_and_cross_hub_rail_binding_before_network() {
    fn loopback_client(port: u16) -> NodeHttpClient {
        let mut config = NodeNetworkConfig::new(format!("http://127.0.0.1:{port}"));
        config.proxy = NodeProxyMode::Disabled;
        config.connect_timeout_secs = 1;
        config.request_timeout_secs = 1;
        config.build_loopback_client().unwrap()
    }

    let trusted = loopback_client(31_001);
    let other = loopback_client(31_002);
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), &trusted.hub_sha256());
    let rail = NodeRailStore::open(&pairing).unwrap();
    assert!(matches!(
        NodeRailLink::connect(other, rail, test_token(), &capabilities(), &[]).await,
        Err(NodeLinkError::Rail(NodeRailError::IdentityConflict))
    ));

    let trusted = loopback_client(31_003);
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), &trusted.hub_sha256());
    let rail = NodeRailStore::open(&pairing).unwrap();
    let now_ms = current_time_ms().unwrap();
    let expired = NodeAccessToken::for_test("a".repeat(64), now_ms - 2_000, now_ms - 1_000);
    assert!(matches!(
        NodeRailLink::connect(trusted, rail, expired, &capabilities(), &[]).await,
        Err(NodeLinkError::InvalidAccessToken)
    ));
}

#[derive(Default)]
struct MockHubState {
    hellos: std::sync::Mutex<Vec<Vec<u8>>>,
    ingress_sequences: std::sync::Mutex<Vec<u64>>,
    pull_waits_ms: std::sync::Mutex<Vec<u64>>,
    closes: std::sync::Mutex<usize>,
    websocket_connections: AtomicUsize,
    fail_next_pull: AtomicBool,
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn link_falls_back_through_explicit_proxy_with_exact_durable_hello() {
    use axum::{
        extract::{ws::WebSocketUpgrade, State},
        http::{header, HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
        Json, Router,
    };

    fn authorize(headers: &HeaderMap) {
        assert_eq!(
            headers.get(header::AUTHORIZATION).unwrap(),
            &format!("Bearer {}", "a".repeat(64))
        );
    }

    fn record_hello(state: &MockHubState, hello: &HubNodeEnvelope) {
        state
            .hellos
            .lock()
            .unwrap()
            .push(serde_json::to_vec(hello).unwrap());
    }

    fn delivery_for(hello: &HubNodeEnvelope, transport: NodeTransport) -> HubNodeDeliveryBatch {
        let superseded = match transport {
            NodeTransport::HttpStream => vec![(1, "welcome", '1')],
            NodeTransport::LongPoll => vec![(1, "welcome", '1'), (2, "welcome", '2')],
            NodeTransport::WebSocket => Vec::new(),
        };
        let mut messages = superseded
            .into_iter()
            .map(|(sequence, kind, digest)| HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: hello.device_id.clone(),
                connection_id: hello.connection_id.clone(),
                sequence,
                ack_sequence: None,
                sent_at_ms: 100 + sequence as i64,
                message: HubNodeMessage::Superseded {
                    original_message_kind: kind.to_string(),
                    original_message_sha256: digest.to_string().repeat(64),
                },
            })
            .collect::<Vec<_>>();
        let welcome_sequence = messages.len() as u64 + 1;
        messages.push(HubNodeEnvelope {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: hello.device_id.clone(),
            connection_id: hello.connection_id.clone(),
            sequence: welcome_sequence,
            ack_sequence: None,
            sent_at_ms: 200 + welcome_sequence as i64,
            message: HubNodeMessage::Welcome {
                negotiated_version: HUB_NODE_PROTOCOL_VERSION,
                transport,
                heartbeat_interval_ms: 15_000,
                lease_duration_ms: 60_000,
            },
        });
        HubNodeDeliveryBatch {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: hello.device_id.clone(),
            connection_id: hello.connection_id.clone(),
            acknowledged_node_sequence: 1,
            messages,
            retry_after_ms: None,
        }
    }

    async fn websocket(
        State(state): State<Arc<MockHubState>>,
        headers: HeaderMap,
        upgrade: WebSocketUpgrade,
    ) -> Response {
        authorize(&headers);
        if state.websocket_connections.fetch_add(1, Ordering::SeqCst) == 0 {
            return StatusCode::BAD_GATEWAY.into_response();
        }
        upgrade.on_upgrade(move |mut socket| async move {
            let Some(Ok(axum::extract::ws::Message::Text(payload))) = socket.recv().await else {
                return;
            };
            let frame: HubNodeWebSocketFrame = serde_json::from_str(&payload).unwrap();
            frame.validate().unwrap();
            let HubNodeWebSocketFrame::NodeEnvelope { envelope } = frame else {
                panic!("Node sent a Hub-only frame")
            };
            record_hello(&state, &envelope);
            let welcome = HubNodeDeliveryBatch {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: envelope.device_id.clone(),
                connection_id: envelope.connection_id.clone(),
                acknowledged_node_sequence: 3,
                messages: vec![HubNodeEnvelope {
                    protocol_version: HUB_NODE_PROTOCOL_VERSION,
                    device_id: envelope.device_id.clone(),
                    connection_id: envelope.connection_id.clone(),
                    sequence: 4,
                    ack_sequence: None,
                    sent_at_ms: 404,
                    message: HubNodeMessage::Welcome {
                        negotiated_version: HUB_NODE_PROTOCOL_VERSION,
                        transport: NodeTransport::WebSocket,
                        heartbeat_interval_ms: 15_000,
                        lease_duration_ms: 60_000,
                    },
                }],
                retry_after_ms: None,
            };
            socket
                .send(axum::extract::ws::Message::Text(
                    serde_json::to_string(&HubNodeWebSocketFrame::HubDelivery { batch: welcome })
                        .unwrap()
                        .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(axum::extract::ws::Message::Text(payload))) = socket.recv().await {
                let frame: HubNodeWebSocketFrame = serde_json::from_str(&payload).unwrap();
                frame.validate().unwrap();
                let HubNodeWebSocketFrame::NodeEnvelope { envelope } = frame else {
                    panic!("Node sent a Hub-only frame")
                };
                state
                    .ingress_sequences
                    .lock()
                    .unwrap()
                    .push(envelope.sequence);
                let acknowledgement = HubNodeDeliveryBatch {
                    protocol_version: HUB_NODE_PROTOCOL_VERSION,
                    device_id: envelope.device_id,
                    connection_id: envelope.connection_id,
                    acknowledged_node_sequence: envelope.sequence,
                    messages: Vec::new(),
                    retry_after_ms: None,
                };
                socket
                    .send(axum::extract::ws::Message::Text(
                        serde_json::to_string(&HubNodeWebSocketFrame::HubDelivery {
                            batch: acknowledgement,
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await
                    .unwrap();
            }
        })
    }

    async fn connect(
        State(state): State<Arc<MockHubState>>,
        headers: HeaderMap,
        Json(request): Json<HubNodeConnectRequest>,
    ) -> Json<HubNodeDeliveryBatch> {
        authorize(&headers);
        request.validate().unwrap();
        record_hello(&state, &request.hello);
        Json(delivery_for(&request.hello, request.transport))
    }

    async fn unavailable_stream(headers: HeaderMap) -> Response {
        authorize(&headers);
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": {"code": "hub_node_unavailable"}
            })),
        )
            .into_response()
    }

    async fn envelope(
        State(state): State<Arc<MockHubState>>,
        headers: HeaderMap,
        Json(request): Json<HubNodeIngressRequest>,
    ) -> Json<HubNodeDeliveryBatch> {
        authorize(&headers);
        request.validate().unwrap();
        state
            .ingress_sequences
            .lock()
            .unwrap()
            .push(request.envelope.sequence);
        Json(HubNodeDeliveryBatch {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: request.envelope.device_id,
            connection_id: request.envelope.connection_id,
            acknowledged_node_sequence: request.envelope.sequence,
            messages: Vec::new(),
            retry_after_ms: Some(1_000),
        })
    }

    async fn pull(
        State(state): State<Arc<MockHubState>>,
        headers: HeaderMap,
        Json(request): Json<HubNodePullRequest>,
    ) -> Response {
        authorize(&headers);
        request.validate().unwrap();
        state.pull_waits_ms.lock().unwrap().push(request.wait_ms);
        if state.fail_next_pull.swap(false, Ordering::SeqCst) {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": {"code": "hub_node_unavailable"}
                })),
            )
                .into_response();
        }
        Json(HubNodeDeliveryBatch {
            protocol_version: request.protocol_version,
            device_id: request.device_id,
            connection_id: request.connection_id,
            acknowledged_node_sequence: 3,
            messages: Vec::new(),
            retry_after_ms: Some(1_000),
        })
        .into_response()
    }

    async fn close(
        State(state): State<Arc<MockHubState>>,
        headers: HeaderMap,
        Json(request): Json<HubNodeCloseRequest>,
    ) -> Json<serde_json::Value> {
        authorize(&headers);
        request.validate().unwrap();
        *state.closes.lock().unwrap() += 1;
        Json(serde_json::json!({"status": "offline"}))
    }

    let state = Arc::new(MockHubState::default());
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server_state = Arc::clone(&state);
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(HUB_NODE_WEBSOCKET_PATH, get(websocket))
                .route(HUB_NODE_CONNECT_PATH, post(connect))
                .route(HUB_NODE_STREAM_PATH, get(unavailable_stream))
                .route(HUB_NODE_ENVELOPE_PATH, post(envelope))
                .route(HUB_NODE_PULL_PATH, post(pull))
                .route(HUB_NODE_CLOSE_PATH, post(close))
                .with_state(server_state),
        )
        .await
        .unwrap();
    });

    let mut config = NodeNetworkConfig::new(format!("http://127.0.0.2:{}", address.port()));
    config.proxy = NodeProxyMode::Explicit {
        url: format!("http://{address}"),
        username: None,
        password_secret: None,
    };
    config.connect_timeout_secs = 1;
    config.request_timeout_secs = 2;
    let client = config.build_loopback_client().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), &client.hub_sha256());
    let rail = NodeRailStore::open(&pairing).unwrap();

    let mut link = NodeRailLink::connect(client, rail, test_token(), &capabilities(), &[])
        .await
        .unwrap();
    assert_eq!(link.transport(), NodeTransport::LongPoll);
    assert_eq!(
        link.heartbeat_policy().interval(),
        std::time::Duration::from_secs(15)
    );
    assert_eq!(
        link.heartbeat_policy().lease_duration(),
        std::time::Duration::from_secs(60)
    );
    assert_eq!(
        link.fallbacks(),
        &[
            NodeTransportFallback {
                transport: NodeTransport::WebSocket,
                reason: NodeTransportFailure::UpgradeFailed,
            },
            NodeTransportFallback {
                transport: NodeTransport::HttpStream,
                reason: NodeTransportFailure::HubUnavailable,
            },
        ]
    );
    let snapshot = link.snapshot().unwrap();
    assert_eq!(snapshot.last_hub_sequence, 3);
    assert_eq!(snapshot.acknowledged_node_sequence, 3);
    assert_eq!(snapshot.pending_outbound, 0);
    assert_eq!(snapshot.pending_inbound, 0);
    {
        let hellos = state.hellos.lock().unwrap();
        assert_eq!(hellos.len(), 2);
        assert!(hellos.windows(2).all(|pair| pair[0] == pair[1]));
    }
    assert_eq!(*state.ingress_sequences.lock().unwrap(), vec![2, 3]);

    state.fail_next_pull.store(true, Ordering::SeqCst);
    let recovered = link.synchronize_once().await.unwrap();
    assert_eq!(*state.pull_waits_ms.lock().unwrap(), vec![15_000]);
    assert_eq!(link.transport(), NodeTransport::WebSocket);
    assert_eq!(recovered.last_hub_sequence, 4);
    assert_eq!(recovered.acknowledged_node_sequence, 5);
    assert_eq!(recovered.pending_outbound, 0);
    assert_eq!(recovered.pending_inbound, 0);
    assert_eq!(
        link.fallbacks().last(),
        Some(&NodeTransportFallback {
            transport: NodeTransport::LongPoll,
            reason: NodeTransportFailure::HubUnavailable,
        })
    );
    {
        let hellos = state.hellos.lock().unwrap();
        assert_eq!(hellos.len(), 3);
        assert!(hellos.windows(2).all(|pair| pair[0] == pair[1]));
    }
    assert_eq!(*state.ingress_sequences.lock().unwrap(), vec![2, 3, 4, 5]);
    let refreshed = link
        .set_active_runs(&["run-b".to_string(), "run-a".to_string()])
        .await
        .unwrap();
    assert_eq!(refreshed.acknowledged_node_sequence, 6);
    let unchanged = link
        .set_active_runs(&["run-a".to_string(), "run-b".to_string()])
        .await
        .unwrap();
    assert_eq!(unchanged.acknowledged_node_sequence, 6);
    assert_eq!(
        *state.ingress_sequences.lock().unwrap(),
        vec![2, 3, 4, 5, 6]
    );
    let presence = link
        .refresh_presence(&["run-a".to_string(), "run-b".to_string()])
        .await
        .unwrap();
    assert_eq!(presence.acknowledged_node_sequence, 7);
    assert_eq!(
        *state.ingress_sequences.lock().unwrap(),
        vec![2, 3, 4, 5, 6, 7]
    );
    let rendered = format!("{link:?}");
    assert!(!rendered.contains(&"a".repeat(64)));
    assert!(!rendered.contains(&address.to_string()));

    link.close(Some("test_complete")).await.unwrap();
    assert_eq!(*state.closes.lock().unwrap(), 1);
    server.abort();
}
