use super::*;
use axum::{body::to_bytes, extract::State};
use captain_kernel::CaptainKernel;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    HubNodeEnvelope, HubNodeMessage, LogicalWorkspace, HUB_NODE_PROTOCOL_VERSION,
    HUB_NODE_WEBSOCKET_PATH,
};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

struct PairedNode {
    device_id: String,
    access_token: String,
    capabilities: CapabilityDescriptor,
}

fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = KernelConfig {
        home_dir: temp.path().to_path_buf(),
        data_dir: temp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    config.pairing.hub_enabled = true;
    let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
    kernel.set_self_handle();
    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    });
    (temp, state)
}

fn node_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "linux-x86_64".to_string(),
        transports: vec![NodeTransport::HttpStream, NodeTransport::LongPoll],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main".to_string(),
            read_only: true,
        }],
        supports_streaming_output: true,
    }
}

fn grant() -> DeviceGrant {
    DeviceGrant {
        workspace_ids: vec!["workspace-main".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    }
}

fn pair_node(state: &AppState) -> PairedNode {
    let credential = "a".repeat(64);
    let capabilities = node_capabilities();
    state
        .kernel
        .hub_pairing
        .open_enrollment_window(300)
        .unwrap();
    let challenge = state
        .kernel
        .hub_pairing
        .create_claim(&DevicePairingClaim {
            display_name: "HTTP Test Node".to_string(),
            role: DeviceRole::Node,
            platform: capabilities.platform.clone(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            credential_sha256: hex::encode(Sha256::digest(credential.as_bytes())),
            capabilities: capabilities.clone(),
            requested_grants: grant(),
        })
        .unwrap();
    let device = state
        .kernel
        .hub_pairing
        .approve_request(&challenge.request_id, &grant())
        .unwrap();
    let access = state
        .kernel
        .hub_pairing
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id.clone(),
            credential,
        })
        .unwrap();
    PairedNode {
        device_id: device.device_id,
        access_token: access.access_token,
        capabilities,
    }
}

fn authorization(node: &PairedNode) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", node.access_token)).unwrap(),
    );
    headers
}

fn hello(node: &PairedNode, connection_id: &str) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: node.device_id.clone(),
        connection_id: connection_id.to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: chrono::Utc::now().timestamp_millis(),
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: node.capabilities.clone(),
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    }
}

fn envelope(
    node: &PairedNode,
    connection_id: &str,
    sequence: u64,
    ack_sequence: Option<u64>,
    message: HubNodeMessage,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: node.device_id.clone(),
        connection_id: connection_id.to_string(),
        sequence,
        ack_sequence,
        sent_at_ms: chrono::Utc::now().timestamp_millis(),
        message,
    }
}

fn json_bytes<T: serde::Serialize>(value: &T) -> Bytes {
    Bytes::from(serde_json::to_vec(value).unwrap())
}

async fn response_json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 2 * MAX_HUB_NODE_BODY_SIZE)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn http_transport_route_matcher_is_exact_and_excludes_websocket_route() {
    for path in [
        HUB_NODE_CONNECT_PATH,
        HUB_NODE_ENVELOPE_PATH,
        HUB_NODE_PULL_PATH,
        HUB_NODE_CLOSE_PATH,
    ] {
        assert!(is_hub_node_http_transport_route(&Method::POST, path));
        assert!(!is_hub_node_http_transport_route(&Method::GET, path));
        assert!(!is_hub_node_http_transport_route(
            &Method::POST,
            &format!("{path}/extra")
        ));
    }
    assert!(is_hub_node_http_transport_route(
        &Method::GET,
        HUB_NODE_STREAM_PATH
    ));
    assert!(!is_hub_node_http_transport_route(
        &Method::POST,
        HUB_NODE_STREAM_PATH
    ));
    assert!(!is_hub_node_http_transport_route(
        &Method::GET,
        HUB_NODE_WEBSOCKET_PATH
    ));
}

#[tokio::test]
async fn long_poll_flow_reauthenticates_binds_transport_and_closes_cleanly() {
    let (_temp, state) = test_state();
    let node = pair_node(&state);
    let connect = HubNodeConnectRequest {
        transport: NodeTransport::LongPoll,
        hello: hello(&node, "connection-1"),
    };
    let response = hub_node_connect(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&connect),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let opened: HubNodeDeliveryBatch =
        serde_json::from_value(response_json(response).await).unwrap();
    assert!(matches!(
        opened.messages[0].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::LongPoll,
            ..
        }
    ));

    let ingress = HubNodeIngressRequest {
        transport: NodeTransport::HttpStream,
        envelope: envelope(&node, "connection-1", 2, Some(1), HubNodeMessage::AckOnly),
    };
    let response = hub_node_envelope(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&ingress),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let ingress = HubNodeIngressRequest {
        transport: NodeTransport::LongPoll,
        ..ingress
    };
    let response = hub_node_envelope(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&ingress),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let batch: HubNodeDeliveryBatch =
        serde_json::from_value(response_json(response).await).unwrap();
    assert_eq!(batch.acknowledged_node_sequence, 2);
    assert!(batch.messages.is_empty());

    let pull = HubNodePullRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: node.device_id.clone(),
        connection_id: "connection-1".to_string(),
        max_messages: 64,
        wait_ms: 1,
    };
    let permit = state
        .kernel
        .hub_nodes
        .acquire_transport_permit(&node.access_token, &node.device_id, NodeTransport::LongPoll)
        .unwrap();
    let response = hub_node_pull(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&pull),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    drop(permit);
    let response = hub_node_pull(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&pull),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let batch: HubNodeDeliveryBatch =
        serde_json::from_value(response_json(response).await).unwrap();
    assert_eq!(batch.retry_after_ms, Some(1_000));

    let close = HubNodeCloseRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: node.device_id.clone(),
        connection_id: "connection-1".to_string(),
        error_code: Some("client_shutdown".to_string()),
    };
    let response = hub_node_close(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&close),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = hub_node_pull(State(state), authorization(&node), json_bytes(&pull)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn https_stream_starts_with_a_validated_delivery_event() {
    let (_temp, state) = test_state();
    let node = pair_node(&state);
    let connect = HubNodeConnectRequest {
        transport: NodeTransport::HttpStream,
        hello: hello(&node, "connection-stream"),
    };
    assert_eq!(
        hub_node_connect(
            State(Arc::clone(&state)),
            authorization(&node),
            json_bytes(&connect),
        )
        .await
        .status(),
        StatusCode::OK
    );

    let response = hub_node_stream(
        State(state),
        authorization(&node),
        Query(HubNodeStreamRequest {
            protocol_major: HUB_NODE_PROTOCOL_VERSION.major,
            protocol_minor: HUB_NODE_PROTOCOL_VERSION.minor,
            device_id: node.device_id.clone(),
            connection_id: "connection-stream".to_string(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut body = response.into_body().into_data_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(1), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = String::from_utf8(chunk.to_vec()).unwrap();
    assert!(event.contains("event: delivery"));
    assert!(event.contains("welcome"));
    assert!(!event.contains(&node.access_token));
}

#[tokio::test]
async fn malformed_missing_or_non_device_credentials_fail_without_detail_leaks() {
    let (_temp, state) = test_state();
    let node = pair_node(&state);
    let connect = HubNodeConnectRequest {
        transport: NodeTransport::LongPoll,
        hello: hello(&node, "connection-1"),
    };

    let response = hub_node_connect(
        State(Arc::clone(&state)),
        HeaderMap::new(),
        json_bytes(&connect),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().contains_key(header::WWW_AUTHENTICATE));

    let mut wrong = HeaderMap::new();
    wrong.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer operator-api-key"),
    );
    let response = hub_node_connect(State(Arc::clone(&state)), wrong, json_bytes(&connect)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload = response_json(response).await.to_string();
    assert!(!payload.contains("operator-api-key"));
    assert!(!payload.contains(&node.device_id));

    let response = hub_node_connect(
        State(Arc::clone(&state)),
        authorization(&node),
        Bytes::from_static(b"not-json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let websocket_over_http = HubNodeConnectRequest {
        transport: NodeTransport::WebSocket,
        ..connect
    };
    let response = hub_node_connect(
        State(Arc::clone(&state)),
        authorization(&node),
        json_bytes(&websocket_over_http),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = hub_node_connect(
        State(state),
        authorization(&node),
        Bytes::from(vec![b'x'; MAX_HUB_NODE_BODY_SIZE + 1]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
