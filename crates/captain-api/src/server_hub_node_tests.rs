use super::*;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode},
};
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    HubNodeConnectRequest, HubNodeEnvelope, HubNodeMessage, LogicalWorkspace, NodeTransport,
    HUB_NODE_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const TEST_API_KEY: &str = "alpha14-node-router-api-key-0123456789";

struct PairedNode {
    access_token: String,
    connect: HubNodeConnectRequest,
}

fn test_router() -> (tempfile::TempDir, Router<()>, Arc<CaptainKernel>) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = KernelConfig {
        home_dir: temp.path().to_path_buf(),
        data_dir: temp.path().join("data"),
        api_key: TEST_API_KEY.to_string(),
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
    let state = app_state_from_bridge(Arc::clone(&kernel), None);
    let router = mount_api_routes()
        .layer(axum::middleware::from_fn_with_state(
            build_auth_state(&state),
            middleware::auth,
        ))
        .with_state(state);
    (temp, router, kernel)
}

fn pair_node(kernel: &CaptainKernel) -> PairedNode {
    let raw_credential = "c".repeat(64);
    let capabilities = CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "linux-x86_64".to_string(),
        transports: vec![NodeTransport::WebSocket, NodeTransport::LongPoll],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main".to_string(),
            read_only: true,
        }],
        supports_streaming_output: true,
    };
    let grant = DeviceGrant {
        workspace_ids: vec!["workspace-main".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    kernel.hub_pairing.open_enrollment_window(300).unwrap();
    let challenge = kernel
        .hub_pairing
        .create_claim(&DevicePairingClaim {
            display_name: "Router Node".to_string(),
            role: DeviceRole::Node,
            platform: capabilities.platform.clone(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            credential_sha256: hex::encode(Sha256::digest(raw_credential.as_bytes())),
            capabilities: capabilities.clone(),
            requested_grants: grant.clone(),
        })
        .unwrap();
    let device = kernel
        .hub_pairing
        .approve_request(&challenge.request_id, &grant)
        .unwrap();
    let access = kernel
        .hub_pairing
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id.clone(),
            credential: raw_credential,
        })
        .unwrap();
    PairedNode {
        access_token: access.access_token,
        connect: HubNodeConnectRequest {
            transport: NodeTransport::LongPoll,
            hello: HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: device.device_id,
                connection_id: "router-connection".to_string(),
                sequence: 1,
                ack_sequence: None,
                sent_at_ms: chrono::Utc::now().timestamp_millis(),
                message: HubNodeMessage::Hello {
                    role: DeviceRole::Node,
                    capabilities,
                    resume_after_sequence: 0,
                    active_run_ids: Vec::new(),
                },
            },
        },
    }
}

fn request(method: Method, path: &str, body: Vec<u8>, bearer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(bearer) = bearer {
        builder = builder.header("authorization", format!("Bearer {bearer}"));
    }
    let mut request = builder
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 41_414))));
    request
}

fn websocket_request(path: &str, bearer: Option<&str>) -> Request<Body> {
    let mut request = request(Method::GET, path, Vec::new(), bearer);
    let headers = request.headers_mut();
    headers.insert("connection", "upgrade".parse().unwrap());
    headers.insert("upgrade", "websocket".parse().unwrap());
    headers.insert("sec-websocket-version", "13".parse().unwrap());
    headers.insert(
        "sec-websocket-key",
        "dGhlIHNhbXBsZSBub25jZQ==".parse().unwrap(),
    );
    request
}

#[tokio::test]
async fn device_bearer_is_accepted_only_by_exact_bounded_node_routes() {
    let (_temp, router, kernel) = test_router();
    let node = pair_node(&kernel);
    let body = serde_json::to_vec(&node.connect).unwrap();

    for bearer in [None, Some(TEST_API_KEY)] {
        let response = router
            .clone()
            .oneshot(request(
                Method::POST,
                captain_wire::HUB_NODE_CONNECT_PATH,
                body.clone(),
                bearer,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            captain_wire::HUB_NODE_CONNECT_PATH,
            body,
            Some(&node.access_token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            "/api/hub/nodes/connect/extra",
            Vec::new(),
            Some(&node.access_token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    for bearer in [None, Some(TEST_API_KEY)] {
        let response = router
            .clone()
            .oneshot(websocket_request(
                captain_wire::HUB_NODE_WEBSOCKET_PATH,
                bearer,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = router
        .clone()
        .oneshot(websocket_request(
            captain_wire::HUB_NODE_WEBSOCKET_PATH,
            Some(&node.access_token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);

    let response = router
        .clone()
        .oneshot(websocket_request(
            "/api/hub/nodes/ws/extra",
            Some(&node.access_token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = router
        .oneshot(request(
            Method::POST,
            captain_wire::HUB_NODE_CONNECT_PATH,
            vec![b'x'; crate::hub_node_routes::MAX_HUB_NODE_BODY_SIZE + 1],
            Some(&node.access_token),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn real_websocket_upgrade_delivers_welcome_and_durable_node_ack() {
    use futures::{SinkExt, StreamExt};

    let (_temp, router, kernel) = test_router();
    let node = pair_node(&kernel);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let url = format!("ws://{address}{}", captain_wire::HUB_NODE_WEBSOCKET_PATH);
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", node.access_token).parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    let mut hello = node.connect.hello;
    hello.connection_id = "router-websocket".to_string();
    let frame = captain_wire::HubNodeWebSocketFrame::NodeEnvelope { envelope: hello };
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&frame).unwrap().into(),
        ))
        .await
        .unwrap();
    let welcome = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let tokio_tungstenite::tungstenite::Message::Text(welcome) = welcome else {
        panic!("expected a text delivery frame")
    };
    let captain_wire::HubNodeWebSocketFrame::HubDelivery { batch } =
        serde_json::from_str(&welcome).unwrap()
    else {
        panic!("expected a Hub delivery frame")
    };
    assert!(matches!(
        batch.messages[0].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::WebSocket,
            ..
        }
    ));

    let ack = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: batch.device_id.clone(),
        connection_id: batch.connection_id.clone(),
        sequence: 2,
        ack_sequence: Some(batch.messages[0].sequence),
        sent_at_ms: chrono::Utc::now().timestamp_millis(),
        message: HubNodeMessage::AckOnly,
    };
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&captain_wire::HubNodeWebSocketFrame::NodeEnvelope {
                envelope: ack,
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    let acknowledged = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let tokio_tungstenite::tungstenite::Message::Text(acknowledged) = acknowledged else {
        panic!("expected a text acknowledgement frame")
    };
    let captain_wire::HubNodeWebSocketFrame::HubDelivery { batch } =
        serde_json::from_str(&acknowledged).unwrap()
    else {
        panic!("expected a Hub acknowledgement frame")
    };
    assert_eq!(batch.acknowledged_node_sequence, 2);
    assert!(batch.messages.is_empty());

    socket.close(None).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let connection = kernel
                .memory
                .hub_node_rail()
                .connection(&batch.device_id)
                .unwrap()
                .unwrap();
            if connection.status == captain_memory::hub_node_rail::HubNodeConnectionStatus::Offline
            {
                break connection;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(closed.last_error_code.as_deref(), Some("client_closed"));
    server.abort();
}
