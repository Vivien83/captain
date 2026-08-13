use super::*;
use axum::{body::to_bytes, extract::State};
use captain_kernel::CaptainKernel;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_wire::{
    CapabilityDescriptor, DeviceAccessToken, LogicalWorkspace, NodeTransport, PairingChallenge,
    PairingPollResponse, PairingState, ProtocolVersion, HUB_NODE_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::{net::Ipv4Addr, time::Instant};

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

fn raw_credential() -> String {
    "a".repeat(64)
}

fn requested_grant() -> DeviceGrant {
    DeviceGrant {
        workspace_ids: vec!["workspace-main".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    }
}

fn claim() -> DevicePairingClaim {
    let credential = raw_credential();
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
    DevicePairingClaim {
        display_name: "Test Node".to_string(),
        role: DeviceRole::Node,
        platform: capabilities.platform.clone(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: hex::encode(Sha256::digest(credential.as_bytes())),
        capabilities,
        requested_grants: requested_grant(),
    }
}

fn projected_device(role: DeviceRole, status: &str, last_seen_ms: i64) -> DeviceRecord {
    let capabilities = match role {
        DeviceRole::Node => claim().capabilities,
        DeviceRole::Client => CapabilityDescriptor {
            captain_version: "0.1.0-alpha.14".to_string(),
            platform: "linux-x86_64".to_string(),
            transports: vec![NodeTransport::WebSocket],
            tool_families: Vec::new(),
            workspaces: Vec::new(),
            supports_streaming_output: true,
        },
    };
    let grants = if role == DeviceRole::Node {
        requested_grant()
    } else {
        DeviceGrant::default()
    };
    DeviceRecord {
        device_id: "device-test".to_string(),
        display_name: "Test device".to_string(),
        role: device_role_label(role).to_string(),
        platform: capabilities.platform.clone(),
        captain_version: capabilities.captain_version.clone(),
        protocol_major: HUB_NODE_PROTOCOL_VERSION.major,
        protocol_minor: HUB_NODE_PROTOCOL_VERSION.minor,
        capabilities_json: serde_json::to_string(&capabilities).unwrap(),
        grants_json: serde_json::to_string(&grants).unwrap(),
        status: status.to_string(),
        paired_at_ms: 1_000,
        last_seen_ms,
        updated_at_ms: last_seen_ms,
        last_transport: None,
        last_error_code: None,
        revoked_at_ms: (status == "revoked").then_some(last_seen_ms),
    }
}

async fn response_json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn bootstrap_and_operator_handlers_complete_then_revoke_pairing() {
    let (_temp, state) = test_state();
    let response = hub_pairing_enrollment_open(
        State(Arc::clone(&state)),
        Json(HubEnrollmentWindowRequest { duration_secs: 600 }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = hub_pairing_claim(
        State(Arc::clone(&state)),
        None,
        HeaderMap::new(),
        Bytes::from(serde_json::to_vec(&claim()).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let challenge: PairingChallenge =
        serde_json::from_value(response_json(response).await).unwrap();

    let response = hub_pairing_review(
        State(Arc::clone(&state)),
        Json(HubPairingReviewRequest {
            display_code: challenge.display_code.clone(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let review = response_json(response).await;
    assert_eq!(review["request_id"], challenge.request_id);
    assert_eq!(review["display_name"], "Test Node");
    assert_eq!(review["requested_grants"]["allow_mutation"], false);

    let response = hub_pairing_approve(
        State(Arc::clone(&state)),
        Json(HubPairingApprovalRequest {
            display_code: challenge.display_code.clone(),
            grant: requested_grant(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let device = response_json(response).await;
    let device_id = device["device_id"].as_str().unwrap().to_string();

    let response = hub_pairing_poll(
        State(Arc::clone(&state)),
        None,
        HeaderMap::new(),
        Bytes::from(
            serde_json::to_vec(&PairingPollRequest {
                request_id: challenge.request_id,
                polling_secret: challenge.polling_secret,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let poll: PairingPollResponse = serde_json::from_value(response_json(response).await).unwrap();
    assert_eq!(poll.status, PairingState::Approved);
    assert_eq!(poll.device_id.as_deref(), Some(device_id.as_str()));
    assert_eq!(poll.approved_grants, Some(requested_grant()));

    let exchange = DeviceCredentialExchange {
        device_id: device_id.clone(),
        credential: raw_credential(),
    };
    let response = hub_device_token(
        State(Arc::clone(&state)),
        None,
        HeaderMap::new(),
        Bytes::from(serde_json::to_vec(&exchange).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let token: DeviceAccessToken = serde_json::from_value(response_json(response).await).unwrap();
    assert_eq!(token.approved_grants, requested_grant());
    assert_eq!(
        token.protocol_version,
        ProtocolVersion {
            major: HUB_NODE_PROTOCOL_VERSION.major,
            minor: HUB_NODE_PROTOCOL_VERSION.minor,
        }
    );

    let response = hub_device_revoke(State(Arc::clone(&state)), Path(device_id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = hub_device_token(
        State(state),
        None,
        HeaderMap::new(),
        Bytes::from(serde_json::to_vec(&exchange).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn identifiers_and_payload_size_fail_closed() {
    assert!(valid_device_id(&format!("node-{}", uuid::Uuid::new_v4())));
    assert!(!valid_device_id("node-private/path"));
    assert!(canonical_uuid(&uuid::Uuid::new_v4().to_string()).is_some());
    assert!(canonical_uuid("not-a-request").is_none());

    let oversized = vec![b'x'; MAX_HUB_PAIRING_BODY_SIZE + 1];
    let response =
        parse_pairing_body::<PairingPollRequest>(&oversized, "pairing poll").unwrap_err();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn public_errors_never_echo_credentials_or_storage_details() {
    let secret = "secret-that-must-not-appear";
    let response =
        credential_error_response(PairingServiceError::DeviceNotActive(secret.to_string()));
    let payload = response_json(response).await.to_string();
    assert!(!payload.contains(secret));

    let response = operator_error_response(PairingServiceError::StorageUnavailable);
    let payload = response_json(response).await.to_string();
    assert!(!payload.to_ascii_lowercase().contains("sqlite"));
}

#[tokio::test]
async fn enrollment_handlers_report_open_and_closed_states() {
    let (_temp, state) = test_state();
    let response = hub_pairing_enrollment_status(State(Arc::clone(&state))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["open"], false);

    let request: HubEnrollmentWindowRequest = serde_json::from_str("{}").unwrap();
    assert_eq!(request.duration_secs, 600);
    let response = hub_pairing_enrollment_open(State(Arc::clone(&state)), Json(request)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["open"], true);

    let response = hub_pairing_enrollment_close(State(Arc::clone(&state))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["open"], false);
    let response = hub_pairing_claim(
        State(state),
        None,
        HeaderMap::new(),
        Bytes::from(serde_json::to_vec(&claim()).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn route_limiter_enforces_its_configured_burst() {
    let limiter = pairing_limiter(60, 2);
    let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10));
    assert!(enforce_pairing_rate_limit(&limiter, ip, 1).is_ok());
    assert!(enforce_pairing_rate_limit(&limiter, ip, 1).is_ok());
    assert_eq!(
        enforce_pairing_rate_limit(&limiter, ip, 1)
            .unwrap_err()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[test]
fn device_projection_requires_a_live_node_connection_before_selection() {
    let now_ms = 100_000;
    let device = projected_device(DeviceRole::Node, "active", now_ms);
    let offline = device_payload_at(device.clone(), None, now_ms).unwrap();
    assert_eq!(offline["status"], "offline");
    assert_eq!(offline["presence"]["reason_code"], "node_offline");
    assert_eq!(offline["presence"]["selectable"], false);

    let live = HubNodeConnectionRecord {
        device_id: device.device_id.clone(),
        connection_id: "connection-test".to_string(),
        transport: NodeTransport::WebSocket,
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        status: HubNodeConnectionStatus::Active,
        connected_at_ms: now_ms - 10_000,
        last_seen_ms: now_ms - 1_000,
        updated_at_ms: now_ms - 1_000,
        disconnected_at_ms: None,
        last_error_code: None,
    };
    let online = device_payload_at(device.clone(), Some(&live), now_ms).unwrap();
    assert_eq!(online["status"], "online");
    assert_eq!(online["presence"]["selectable"], true);
    assert!(online["presence"]["reason_code"].is_null());

    let stale = HubNodeConnectionRecord {
        last_seen_ms: now_ms - i64::try_from(HUB_NODE_LEASE_DURATION_MS).unwrap() - 1,
        ..live
    };
    let offline = device_payload_at(device, Some(&stale), now_ms).unwrap();
    assert_eq!(offline["status"], "offline");
    assert_eq!(offline["presence"]["selectable"], false);
}

#[test]
fn device_projection_distinguishes_clients_revocation_and_protocol_drift() {
    let now_ms = 100_000;
    let client = projected_device(DeviceRole::Client, "active", now_ms - 1_000);
    let online = device_payload_at(client.clone(), None, now_ms).unwrap();
    assert_eq!(online["status"], "online");
    assert_eq!(online["presence"]["selectable"], false);

    let offline = device_payload_at(
        projected_device(
            DeviceRole::Client,
            "active",
            now_ms - CLIENT_PRESENCE_WINDOW_MS - 1,
        ),
        None,
        now_ms,
    )
    .unwrap();
    assert_eq!(offline["presence"]["reason_code"], "client_offline");

    let revoked = device_payload_at(
        projected_device(DeviceRole::Client, "revoked", now_ms),
        None,
        now_ms,
    )
    .unwrap();
    assert_eq!(revoked["status"], "revoked");

    let mut incompatible = client;
    incompatible.protocol_major = HUB_NODE_PROTOCOL_VERSION.major + 1;
    let incompatible = device_payload_at(incompatible, None, now_ms).unwrap();
    assert_eq!(incompatible["status"], "incompatible");
    assert_eq!(incompatible["presence"]["compatible"], false);
}
