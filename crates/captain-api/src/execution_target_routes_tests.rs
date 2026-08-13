use super::*;
use axum::{body::to_bytes, extract::State};
use captain_kernel::CaptainKernel;
use captain_memory::project::NewProject;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    HubNodeEnvelope, HubNodeMessage, LogicalWorkspace, NodeTransport, HUB_NODE_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::time::Instant;

fn test_state(pairing_enabled: bool) -> (tempfile::TempDir, Arc<AppState>) {
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
    config.pairing.hub_enabled = pairing_enabled;
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

async fn response_json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn node_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "linux-x86_64".to_string(),
        transports: vec![NodeTransport::LongPoll],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn node_grant() -> DeviceGrant {
    DeviceGrant {
        workspace_ids: vec!["workspace-main".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: true,
    }
}

fn pair_node(state: &Arc<AppState>) -> (ExecutionTarget, String, CapabilityDescriptor) {
    let raw_credential = "e".repeat(64);
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
            display_name: "Test Node".to_string(),
            role: DeviceRole::Node,
            platform: capabilities.platform.clone(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            credential_sha256: hex::encode(Sha256::digest(raw_credential.as_bytes())),
            capabilities: capabilities.clone(),
            requested_grants: node_grant(),
        })
        .unwrap();
    let device = state
        .kernel
        .hub_pairing
        .approve_request(&challenge.request_id, &node_grant())
        .unwrap();
    let token = state
        .kernel
        .hub_pairing
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id.clone(),
            credential: raw_credential,
        })
        .unwrap();
    (
        ExecutionTarget::Node {
            device_id: device.device_id,
            workspace_id: "workspace-main".to_string(),
        },
        token.access_token,
        capabilities,
    )
}

#[tokio::test]
async fn catalog_always_contains_auto_and_hub_without_pairing() {
    let (_temp, state) = test_state(false);
    let response = list_execution_targets(State(state)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["policy_version"], EXECUTION_TARGET_POLICY_VERSION);
    assert_eq!(payload["targets"].as_array().unwrap().len(), 2);
    assert_eq!(payload["targets"][0]["target"]["kind"], "auto");
    assert_eq!(payload["targets"][1]["target"]["kind"], "hub");
    assert!(!payload.to_string().contains("path"));
}

#[tokio::test]
async fn session_and_project_bindings_are_canonical_and_durable() {
    let (_temp, state) = test_state(false);
    let captain = state.kernel.registry.find_by_name("captain").unwrap();
    let session = state.kernel.memory.create_session(captain.id).unwrap();
    let project = state
        .kernel
        .memory
        .project_create(NewProject {
            name: "Routing".to_string(),
            slug: "routing".to_string(),
            goal: "Test target routing".to_string(),
            deadline: None,
        })
        .unwrap();

    let response =
        get_session_execution_target(State(Arc::clone(&state)), Path(session.id.to_string())).await;
    let payload = response_json(response).await;
    assert_eq!(payload["source"], "default");
    assert_eq!(payload["target"]["kind"], "auto");

    let response = set_session_execution_target(
        State(Arc::clone(&state)),
        Path(session.id.to_string()),
        Json(SetExecutionTargetRequest {
            target: ExecutionTarget::Hub,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["source"], "pinned");
    assert_eq!(payload["target"]["kind"], "hub");

    let response = set_project_execution_target(
        State(Arc::clone(&state)),
        Path(project.slug),
        Json(SetExecutionTargetRequest {
            target: ExecutionTarget::Auto,
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let stored = state
        .kernel
        .memory
        .execution_targets()
        .get(ExecutionTargetScope::Project, &project.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.target, ExecutionTarget::Auto);
}

#[tokio::test]
async fn offline_node_is_visible_but_cannot_be_pinned_then_online_node_can() {
    let (_temp, state) = test_state(true);
    let captain = state.kernel.registry.find_by_name("captain").unwrap();
    let session = state.kernel.memory.create_session(captain.id).unwrap();
    let (target, access_token, capabilities) = pair_node(&state);

    let response = list_execution_targets(State(Arc::clone(&state))).await;
    let payload = response_json(response).await;
    assert_eq!(payload["targets"].as_array().unwrap().len(), 3);
    assert_eq!(payload["targets"][2]["status"], "offline");
    assert_eq!(payload["targets"][2]["selectable"], false);
    assert!(!payload.to_string().contains("/Users/"));

    let response = set_session_execution_target(
        State(Arc::clone(&state)),
        Path(session.id.to_string()),
        Json(SetExecutionTargetRequest {
            target: target.clone(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "node_offline"
    );

    let ExecutionTarget::Node { device_id, .. } = &target else {
        unreachable!();
    };
    state
        .kernel
        .hub_nodes
        .open_connection(
            &access_token,
            &HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: device_id.clone(),
                connection_id: "connection-routing-test".to_string(),
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
            NodeTransport::LongPoll,
        )
        .unwrap();

    let response = set_session_execution_target(
        State(state),
        Path(session.id.to_string()),
        Json(SetExecutionTargetRequest { target }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["target"]["kind"], "node");
    assert_eq!(payload["availability"]["selectable"], true);
}

#[tokio::test]
async fn malformed_or_unknown_targets_fail_without_becoming_bindings() {
    let (_temp, state) = test_state(false);
    let missing = get_session_execution_target(
        State(Arc::clone(&state)),
        Path(uuid::Uuid::new_v4().to_string()),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let target = ExecutionTarget::Node {
        device_id: "node-00000000-0000-0000-0000-000000000001".to_string(),
        workspace_id: "unknown".to_string(),
    };
    let captain = state.kernel.registry.find_by_name("captain").unwrap();
    let session = state.kernel.memory.create_session(captain.id).unwrap();
    let response = set_session_execution_target(
        State(state),
        Path(session.id.to_string()),
        Json(SetExecutionTargetRequest { target }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "execution_target_not_authorized"
    );
}
