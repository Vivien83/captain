use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request, StatusCode},
};
use captain_types::config::{DefaultModelConfig, KernelConfig};
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    LogicalWorkspace, NodeTransport, HUB_NODE_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const TEST_API_KEY: &str = "alpha14-test-api-key-0123456789";

fn test_router() -> (tempfile::TempDir, Router<()>, Arc<AppState>) {
    let temp = tempfile::tempdir().unwrap();
    let config = KernelConfig {
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
    assert!(!config.pairing.enabled);
    assert!(config.pairing.hub_enabled);
    let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
    kernel.set_self_handle();
    let state = app_state_from_bridge(kernel, None);
    let auth_state = build_auth_state(&state);
    let security = Arc::clone(&auth_state.security);
    let router = mount_api_routes()
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(axum::Extension(security))
        .with_state(Arc::clone(&state));
    (temp, router, state)
}

fn pairing_claim(role: DeviceRole, raw_credential: &str) -> DevicePairingClaim {
    let mut capabilities = CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "linux-x86_64".to_string(),
        transports: vec![NodeTransport::LongPoll],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main".to_string(),
            read_only: true,
        }],
        supports_streaming_output: true,
    };
    if role == DeviceRole::Client {
        capabilities.tool_families.clear();
        capabilities.workspaces.clear();
    }
    DevicePairingClaim {
        display_name: format!("Router Test {role:?}"),
        role,
        platform: capabilities.platform.clone(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: hex::encode(Sha256::digest(raw_credential.as_bytes())),
        capabilities,
        requested_grants: if role == DeviceRole::Node {
            DeviceGrant {
                workspace_ids: vec!["workspace-main".to_string()],
                tool_families: vec!["file".to_string()],
                allow_mutation: false,
            }
        } else {
            DeviceGrant::default()
        },
    }
}

fn claim_body() -> Vec<u8> {
    serde_json::to_vec(&pairing_claim(DeviceRole::Node, &"b".repeat(64))).unwrap()
}

fn paired_access_token(
    state: &Arc<AppState>,
    role: DeviceRole,
    raw_credential: String,
) -> (String, String) {
    state
        .kernel
        .hub_pairing
        .open_enrollment_window(600)
        .unwrap();
    let challenge = state
        .kernel
        .hub_pairing
        .create_claim(&pairing_claim(role, &raw_credential))
        .unwrap();
    let grant = pairing_claim(role, &raw_credential).requested_grants;
    let device = state
        .kernel
        .hub_pairing
        .approve_request(&challenge.request_id, &grant)
        .unwrap();
    let token = state
        .kernel
        .hub_pairing
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id.clone(),
            credential: raw_credential,
        })
        .unwrap();
    (device.device_id, token.access_token)
}

fn request(method: Method, path: &str, body: Vec<u8>, authorized: bool) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if authorized {
        builder = builder.header("authorization", format!("Bearer {TEST_API_KEY}"));
    }
    builder
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn bootstrap_routes_are_exact_bounded_and_operator_routes_stay_private() {
    let (_temp, router, _state) = test_router();

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            captain_wire::PAIRING_CLAIM_PATH,
            claim_body(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            crate::hub_pairing_routes::HUB_PAIRING_ENROLLMENT_PATH,
            br#"{"duration_secs":600}"#.to_vec(),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            captain_wire::PAIRING_CLAIM_PATH,
            claim_body(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            captain_wire::PAIRING_CLAIM_PATH,
            vec![b'x'; crate::hub_pairing_routes::MAX_HUB_PAIRING_BODY_SIZE + 1],
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    for (method, path) in [
        (Method::GET, captain_wire::PAIRING_CLAIM_PATH),
        (Method::POST, "/api/hub/pairing/claim/extra"),
    ] {
        let response = router
            .clone()
            .oneshot(request(method, path, Vec::new(), false))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    let response = router
        .clone()
        .oneshot(request(
            Method::GET,
            crate::hub_pairing_routes::HUB_DEVICES_PATH,
            Vec::new(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = router
        .clone()
        .oneshot(request(
            Method::POST,
            crate::hub_pairing_routes::HUB_PAIRING_REVIEW_PATH,
            br#"{"display_code":"ABCD-EFGH"}"#.to_vec(),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = router
        .oneshot(request(
            Method::GET,
            crate::hub_pairing_routes::HUB_DEVICES_PATH,
            Vec::new(),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn paired_client_access_is_scoped_role_checked_and_immediately_revocable() {
    let (_temp, router, state) = test_router();
    let (client_id, client_token) = paired_access_token(&state, DeviceRole::Client, "c".repeat(64));

    let response = router
        .clone()
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/status",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/execution-targets",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let captain = state.kernel.registry.find_by_name("captain").unwrap();
    let session = state.kernel.memory.create_session(captain.id).unwrap();
    let response = router
        .clone()
        .oneshot(request_with_bearer_json(
            Method::PUT,
            &format!("/api/sessions/{}/execution-target", session.id),
            &client_token,
            br#"{"target":{"kind":"hub"}}"#.to_vec(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = router
        .clone()
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/config",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = router
        .clone()
        .oneshot(request_with_bearer(
            Method::POST,
            "/api/learning/review/review-1/decide",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let (_node_id, node_token) = paired_access_token(&state, DeviceRole::Node, "d".repeat(64));
    let response = router
        .clone()
        .oneshot(request_with_bearer(Method::GET, "/api/status", &node_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    state.kernel.hub_pairing.revoke_device(&client_id).unwrap();
    let response = router
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/status",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn paired_client_access_token_is_bound_to_one_hub_instance() {
    let (_first_temp, first_router, first_state) = test_router();
    let (_second_temp, second_router, _second_state) = test_router();
    let (_client_id, first_token) =
        paired_access_token(&first_state, DeviceRole::Client, "e".repeat(64));

    let accepted = first_router
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/status",
            &first_token,
        ))
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);

    let rejected = second_router
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/status",
            &first_token,
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn paired_client_web_identity_and_realtime_tickets_remain_scoped() {
    let (_temp, router, state) = test_router();
    let (_client_id, client_token) =
        paired_access_token(&state, DeviceRole::Client, "f".repeat(64));

    let response = router
        .clone()
        .oneshot(request_with_bearer(
            Method::GET,
            "/api/auth/check",
            &client_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["authenticated"], true);
    assert_eq!(payload["mode"], "client");
    assert_eq!(payload["client_policy_version"], 5);

    let allowed = serde_json::to_vec(&serde_json::json!({
        "path": "/api/memory/events"
    }))
    .unwrap();
    let response = router
        .clone()
        .oneshot(request_with_bearer_json(
            Method::POST,
            "/api/auth/realtime-ticket",
            &client_token,
            allowed,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let forbidden = serde_json::to_vec(&serde_json::json!({
        "path": "/api/sessions/captain/terminal"
    }))
    .unwrap();
    let response = router
        .oneshot(request_with_bearer_json(
            Method::POST,
            "/api/auth/realtime-ticket",
            &client_token,
            forbidden,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["code"], "client_route_forbidden");
}

#[tokio::test]
async fn paired_client_message_cannot_forge_operator_provenance_or_shutdown_hub() {
    let (_temp, router, state) = test_router();
    let (_client_id, client_token) =
        paired_access_token(&state, DeviceRole::Client, "e".repeat(64));
    let captain = state
        .kernel
        .registry
        .find_by_name("captain")
        .expect("principal Captain agent");
    let body = serde_json::to_vec(&serde_json::json!({
        "message": "/shutdown confirm",
        "sender_id": "operator",
        "sender_name": "Administrator",
        "channel_type": "telegram"
    }))
    .unwrap();

    let response = router
        .oneshot(request_with_bearer_json(
            Method::POST,
            &format!("/api/agents/{}/message", captain.id),
            &client_token,
            body,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["code"], "paired_client_authority_denied");
    assert_eq!(payload["command"], "shutdown");
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(25),
            state.shutdown_notify.notified(),
        )
        .await
        .is_err(),
        "denied Client command must not schedule Hub shutdown"
    );
}

fn request_with_bearer(method: Method, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn request_with_bearer_json(
    method: Method,
    path: &str,
    token: &str,
    body: Vec<u8>,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
