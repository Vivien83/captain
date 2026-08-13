use super::*;
use crate::network::{NodeNetworkConfig, NodeProxyMode};
use axum::http::StatusCode;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use captain_wire::{
    DeviceAccessToken, LogicalWorkspace, NodeTransport, PairingChallenge, PairingPollResponse,
    DEVICE_TOKEN_PATH, PAIRING_CLAIM_PATH, PAIRING_POLL_PATH,
};
use std::{
    fs,
    sync::{
        atomic::{AtomicI64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
struct FakeHub {
    claims: Mutex<Vec<DevicePairingClaim>>,
    fail_first_claim: bool,
    oversized_claim_response: bool,
    pending_before_approval: bool,
    polls: AtomicUsize,
    tokens: AtomicUsize,
    expires_at_ms: AtomicI64,
}

fn profile() -> NodePairingProfile {
    let capabilities = CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![NodeTransport::WebSocket, NodeTransport::LongPoll],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            read_only: true,
        }],
        supports_streaming_output: true,
    };
    NodePairingProfile::new(
        "Office Mac",
        capabilities.platform.clone(),
        capabilities,
        DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation: false,
        },
    )
}

fn client_profile() -> ClientPairingProfile {
    ClientPairingProfile::new(
        "Office Client",
        "macos-arm64",
        CapabilityDescriptor {
            captain_version: "0.1.0-alpha.14".to_string(),
            platform: "macos-arm64".to_string(),
            transports: vec![NodeTransport::HttpStream],
            tool_families: Vec::new(),
            workspaces: Vec::new(),
            supports_streaming_output: true,
        },
    )
}

async fn claim_handler(
    State(state): State<Arc<FakeHub>>,
    Json(claim): Json<DevicePairingClaim>,
) -> Response {
    let claim_number = {
        let mut claims = state.claims.lock().unwrap();
        claims.push(claim);
        claims.len()
    };
    if state.fail_first_claim && claim_number == 1 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": {"code": "pairing_storage_unavailable", "message": "hidden"}
            })),
        )
            .into_response();
    }
    if state.oversized_claim_response {
        return (StatusCode::CREATED, vec![b'x'; MAX_HUB_RESPONSE_BYTES + 1]).into_response();
    }
    let expires_at_ms = current_time_ms().unwrap() + 120_000;
    state.expires_at_ms.store(expires_at_ms, Ordering::Release);
    (
        StatusCode::CREATED,
        Json(PairingChallenge {
            request_id: "00000000-0000-4000-8000-000000000014".to_string(),
            display_code: "2345-6789".to_string(),
            polling_secret: "b".repeat(64),
            expires_at_ms,
            approval_path: "/devices/pair?code=2345-6789".to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        }),
    )
        .into_response()
}

async fn poll_handler(
    State(state): State<Arc<FakeHub>>,
    Json(request): Json<PairingPollRequest>,
) -> Response {
    assert_eq!(request.polling_secret, "b".repeat(64));
    let poll_number = state.polls.fetch_add(1, Ordering::AcqRel) + 1;
    let pending = state.pending_before_approval && poll_number == 1;
    let (device_id, approved_grants) = state
        .claims
        .lock()
        .unwrap()
        .last()
        .map(|claim| {
            (
                match claim.role {
                    DeviceRole::Client => "client-office",
                    DeviceRole::Node => "node-office",
                }
                .to_string(),
                claim.requested_grants.clone(),
            )
        })
        .unwrap();
    Json(PairingPollResponse {
        status: if pending {
            PairingState::Pending
        } else {
            PairingState::Approved
        },
        device_id: (!pending).then_some(device_id),
        approved_grants: (!pending).then_some(approved_grants),
        expires_at_ms: state.expires_at_ms.load(Ordering::Acquire),
    })
    .into_response()
}

async fn token_handler(
    State(state): State<Arc<FakeHub>>,
    Json(request): Json<DeviceCredentialExchange>,
) -> Response {
    let approved_grants = state
        .claims
        .lock()
        .unwrap()
        .last()
        .map(|claim| claim.requested_grants.clone())
        .unwrap();
    assert!(matches!(
        request.device_id.as_str(),
        "node-office" | "client-office"
    ));
    let now_ms = current_time_ms().unwrap();
    let token_character = match state.tokens.fetch_add(1, Ordering::AcqRel) {
        0 => 'c',
        1 => 'd',
        _ => 'e',
    };
    Json(DeviceAccessToken {
        access_token: token_character.to_string().repeat(64),
        token_type: "Bearer".to_string(),
        issued_at_ms: now_ms,
        expires_at_ms: now_ms + 120_000,
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        approved_grants,
    })
    .into_response()
}

async fn fake_client(state: Arc<FakeHub>) -> (NodeHttpClient, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route(PAIRING_CLAIM_PATH, post(claim_handler))
        .route(PAIRING_POLL_PATH, post(poll_handler))
        .route(DEVICE_TOKEN_PATH, post(token_handler))
        .with_state(state);
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = NodeNetworkConfig::new(format!("http://{address}"));
    config.proxy = NodeProxyMode::Disabled;
    config.connect_timeout_secs = 1;
    config.request_timeout_secs = 2;
    (config.build_loopback_client().unwrap(), server)
}

#[test]
fn state_store_is_private_exclusive_and_fails_closed_on_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let store = NodePairingStore::open(&root).unwrap();
    let credential = Zeroizing::new("a".repeat(64));
    let claim = profile().claim(sha256_hex(credential.as_bytes()));
    let state = PersistedPairingState::new(
        "b".repeat(64),
        PersistedPairingPhase::Prepared { credential, claim },
    );
    let rendered = format!("{state:?}");
    assert!(!rendered.contains(&"a".repeat(64)));
    assert!(!rendered.contains(&"b".repeat(64)));
    store.save(&state).unwrap();
    assert_eq!(
        store.status().unwrap(),
        Some(NodePairingProgress::ReadyToClaim)
    );
    assert_eq!(
        NodePairingStore::open(&root).unwrap_err(),
        NodePairingError::NodeAlreadyRunning
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join(PAIRING_LOCK_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    drop(store);
    let state_path = root.join(PAIRING_STATE_FILE);
    let mut unsupported: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    unsupported["schema_version"] = serde_json::json!(2);
    fs::write(&state_path, serde_json::to_vec(&unsupported).unwrap()).unwrap();
    let reopened = NodePairingStore::open(&root).unwrap();
    assert_eq!(
        reopened.status(),
        Err(NodePairingError::StateVersionUnsupported)
    );
    drop(reopened);

    fs::write(root.join(PAIRING_STATE_FILE), b"not-json").unwrap();
    let reopened = NodePairingStore::open(&root).unwrap();
    assert_eq!(reopened.status(), Err(NodePairingError::StateCorrupt));
}

#[test]
fn shared_state_root_keeps_the_process_lock_until_its_last_owner_drops() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let store = NodePairingStore::open(&root).unwrap();
    let shared_root = store.state_root_handle();

    drop(store);
    assert_eq!(
        NodePairingStore::open(&root).unwrap_err(),
        NodePairingError::NodeAlreadyRunning
    );

    drop(shared_root);
    NodePairingStore::open(&root).unwrap();
}

#[test]
fn client_and_node_identity_stores_are_role_sealed() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("device");
    let client_store = ClientPairingStore::open(&root).unwrap();
    let profile = ClientPairingProfile::new(
        "Office Client",
        "macos-arm64",
        CapabilityDescriptor {
            captain_version: "0.1.0-alpha.14".to_string(),
            platform: "macos-arm64".to_string(),
            transports: vec![NodeTransport::HttpStream],
            tool_families: Vec::new(),
            workspaces: Vec::new(),
            supports_streaming_output: true,
        },
    );
    let credential = Zeroizing::new("a".repeat(64));
    client_store
        .save_for_test(&PersistedPairingState::new(
            "b".repeat(64),
            PersistedPairingPhase::Prepared {
                claim: profile.claim_for_test(sha256_hex(credential.as_bytes())),
                credential,
            },
        ))
        .unwrap();
    drop(client_store);

    let node_store = NodePairingStore::open(&root).unwrap();
    assert_eq!(node_store.status(), Err(NodePairingError::RoleMismatch));
    drop(node_store);
    let client_store = ClientPairingStore::open(&root).unwrap();
    assert_eq!(
        client_store.status().unwrap(),
        Some(NodePairingProgress::ReadyToClaim)
    );
}

#[test]
fn paired_client_access_sessions_release_the_store_lock_for_other_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("client");
    let mut network = NodeNetworkConfig::new("http://127.0.0.1:9");
    network.proxy = NodeProxyMode::Disabled;
    let http = network.build_loopback_client().unwrap();
    let store = ClientPairingStore::open(&root).unwrap();
    store
        .save_for_test(&PersistedPairingState::new(
            http.hub_sha256(),
            PersistedPairingPhase::Paired {
                credential: Zeroizing::new("a".repeat(64)),
                device_id: "client-office".to_string(),
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                approved_grants: DeviceGrant::default(),
                role: DeviceRole::Client,
            },
        ))
        .unwrap();
    drop(store);

    let first =
        ClientAccessSession::open(http.clone(), ClientPairingStore::open(&root).unwrap()).unwrap();
    let second = ClientAccessSession::open(http, ClientPairingStore::open(&root).unwrap()).unwrap();

    let first_debug = format!("{first:?}");
    let second_debug = format!("{second:?}");
    assert!(!first_debug.contains(&"a".repeat(64)));
    assert!(!first_debug.contains("127.0.0.1"));
    assert!(!second_debug.contains(&"a".repeat(64)));
    assert!(ClientPairingStore::open(&root).is_ok());
}

#[test]
fn legacy_paired_state_without_grants_reopens_with_no_authority() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let store = NodePairingStore::open(&root).unwrap();
    store
        .save(&PersistedPairingState::new(
            "b".repeat(64),
            PersistedPairingPhase::Paired {
                credential: Zeroizing::new("a".repeat(64)),
                device_id: "node-office".to_string(),
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                approved_grants: profile().0.requested_grants,
                role: DeviceRole::Node,
            },
        ))
        .unwrap();
    drop(store);

    let state_path = root.join(PAIRING_STATE_FILE);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    legacy["phase"]
        .as_object_mut()
        .unwrap()
        .remove("approved_grants");
    legacy["phase"].as_object_mut().unwrap().remove("role");
    fs::write(&state_path, serde_json::to_vec(&legacy).unwrap()).unwrap();

    let reopened = NodePairingStore::open(&root).unwrap();
    assert_eq!(reopened.approved_grants().unwrap(), DeviceGrant::default());
}

#[cfg(unix)]
#[test]
fn state_store_rejects_a_symlink_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let linked = temp.path().join("linked");
    symlink(&target, &linked).unwrap();
    assert_eq!(
        NodePairingStore::open(&linked).unwrap_err(),
        NodePairingError::UnsafeStatePath
    );
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn ambiguous_claim_failure_reuses_the_durable_credential() {
    let hub = Arc::new(FakeHub {
        fail_first_claim: true,
        ..FakeHub::default()
    });
    let (http, server) = fake_client(Arc::clone(&hub)).await;
    let temp = tempfile::tempdir().unwrap();
    let client = NodePairingClient::new(http, NodePairingStore::open(temp.path()).unwrap());

    assert_eq!(
        client.start_or_resume(&profile()).await,
        Err(NodePairingError::HubUnavailable)
    );
    assert_eq!(
        client.status().unwrap(),
        Some(NodePairingProgress::ReadyToClaim)
    );
    let mut changed_profile = profile();
    changed_profile.set_display_name("Changed Mac");
    assert_eq!(
        client.start_or_resume(&changed_profile).await,
        Err(NodePairingError::ProfileChangedDuringPairing)
    );
    assert_eq!(hub.claims.lock().unwrap().len(), 1);
    let progress = client.start_or_resume(&profile()).await.unwrap();
    assert!(matches!(
        progress,
        NodePairingProgress::AwaitingApproval { .. }
    ));

    let claims = hub.claims.lock().unwrap();
    assert_eq!(claims.len(), 2);
    assert!(claims[0] == claims[1]);
    assert_eq!(claims[0].credential_sha256.len(), 64);
    drop(claims);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join(PAIRING_STATE_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let rendered = format!("{client:?}");
    assert!(!rendered.contains("Office Mac"));
    assert!(!rendered.contains(&claims_secret_for_test(&client)));
    server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn durable_identity_cannot_be_reused_against_another_hub() {
    let first_hub = Arc::new(FakeHub {
        fail_first_claim: true,
        ..FakeHub::default()
    });
    let (first_http, first_server) = fake_client(first_hub).await;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let first = NodePairingClient::new(first_http, NodePairingStore::open(&root).unwrap());
    assert_eq!(
        first.start_or_resume(&profile()).await,
        Err(NodePairingError::HubUnavailable)
    );
    drop(first);

    let (other_http, other_server) = fake_client(Arc::new(FakeHub::default())).await;
    let other = NodePairingClient::new(other_http, NodePairingStore::open(&root).unwrap());
    assert_eq!(other.status(), Err(NodePairingError::HubIdentityMismatch));
    assert_eq!(
        other.start_or_resume(&profile()).await,
        Err(NodePairingError::HubIdentityMismatch)
    );
    first_server.abort();
    other_server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn pairing_poll_and_token_exchange_survive_client_restart() {
    let hub = Arc::new(FakeHub {
        pending_before_approval: true,
        ..FakeHub::default()
    });
    let (http, server) = fake_client(Arc::clone(&hub)).await;
    let restart_http = http.clone();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let client = NodePairingClient::new(http, NodePairingStore::open(&root).unwrap());
    assert!(matches!(
        client.start_or_resume(&profile()).await.unwrap(),
        NodePairingProgress::AwaitingApproval { .. }
    ));
    assert!(matches!(
        client.poll().await.unwrap(),
        NodePairingProgress::AwaitingApproval { .. }
    ));
    assert_eq!(
        client.poll().await.unwrap(),
        NodePairingProgress::Paired {
            device_id: "node-office".to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        }
    );
    let access = client.issue_access_token().await.unwrap();
    assert_eq!(access.as_str(), "c".repeat(64));
    assert_eq!(access.approved_grants(), &profile().0.requested_grants);
    assert_eq!(
        client.store.approved_grants().unwrap(),
        profile().0.requested_grants
    );
    assert!(!format!("{access:?}").contains(access.as_str()));

    drop(client);
    let restarted = NodePairingClient::new(restart_http, NodePairingStore::open(&root).unwrap());
    assert!(matches!(
        restarted.status().unwrap(),
        Some(NodePairingProgress::Paired { .. })
    ));
    let restarted_access = restarted.issue_access_token().await.unwrap();
    assert_eq!(restarted_access.as_str(), "d".repeat(64));
    assert_eq!(
        restarted_access.approved_grants(),
        &profile().0.requested_grants
    );
    server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn client_pairing_survives_restart_and_rotates_scoped_tokens() {
    let hub = Arc::new(FakeHub::default());
    let (http, server) = fake_client(Arc::clone(&hub)).await;
    let restart_http = http.clone();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("client");
    let client = ClientPairingClient::new(http, ClientPairingStore::open(&root).unwrap());
    assert!(matches!(
        client.start_or_resume(&client_profile()).await.unwrap(),
        ClientPairingProgress::AwaitingApproval { .. }
    ));
    assert_eq!(
        client.poll().await.unwrap(),
        ClientPairingProgress::Paired {
            device_id: "client-office".to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        }
    );
    let first = client.issue_access_token().await.unwrap();
    assert_eq!(first.as_str(), "c".repeat(64));
    assert_eq!(first.approved_grants(), &DeviceGrant::default());
    drop(client);

    let first_surface = ClientAccessSession::open(
        restart_http.clone(),
        ClientPairingStore::open(&root).unwrap(),
    )
    .unwrap();
    let second_surface =
        ClientAccessSession::open(restart_http, ClientPairingStore::open(&root).unwrap()).unwrap();
    let rotated = first_surface.issue_access_token().await.unwrap();
    assert_eq!(rotated.as_str(), "d".repeat(64));
    assert_ne!(rotated.as_str(), first.as_str());
    assert_eq!(rotated.approved_grants(), &DeviceGrant::default());
    assert!(!format!("{rotated:?}").contains(rotated.as_str()));
    let concurrent = second_surface.issue_access_token().await.unwrap();
    assert_eq!(concurrent.as_str(), "e".repeat(64));
    assert_ne!(concurrent.as_str(), rotated.as_str());

    let claims = hub.claims.lock().unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].role, DeviceRole::Client);
    assert!(claims[0].capabilities.workspaces.is_empty());
    assert!(claims[0].capabilities.tool_families.is_empty());
    assert_eq!(claims[0].requested_grants, DeviceGrant::default());
    server.abort();
}

#[tokio::test]
#[ignore = "requires a loopback socket unavailable in the tranche sandbox"]
async fn oversized_hub_response_preserves_the_prepared_state() {
    let hub = Arc::new(FakeHub {
        oversized_claim_response: true,
        ..FakeHub::default()
    });
    let (http, server) = fake_client(hub).await;
    let temp = tempfile::tempdir().unwrap();
    let client = NodePairingClient::new(http, NodePairingStore::open(temp.path()).unwrap());
    assert_eq!(
        client.start_or_resume(&profile()).await,
        Err(NodePairingError::HubResponseTooLarge)
    );
    assert_eq!(
        client.status().unwrap(),
        Some(NodePairingProgress::ReadyToClaim)
    );
    server.abort();
}

fn claims_secret_for_test(client: &NodePairingClient) -> String {
    let state = client.store.load().unwrap().unwrap();
    match state.phase {
        PersistedPairingPhase::Prepared { credential, .. }
        | PersistedPairingPhase::AwaitingApproval { credential, .. }
        | PersistedPairingPhase::Paired { credential, .. } => credential.to_string(),
        PersistedPairingPhase::Terminal { .. } => String::new(),
    }
}
