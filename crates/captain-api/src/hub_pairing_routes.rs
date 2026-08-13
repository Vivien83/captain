//! Authenticated operator and proof-bound bootstrap handlers for Hub devices.
//!
//! This module does not decide which paths bypass global authentication. Route
//! mounting and the exact public allowlist live in the server and middleware.

use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use captain_kernel::{
    hub_node_service::{HubNodeServiceError, HUB_NODE_LEASE_DURATION_MS},
    hub_pairing_service::PairingServiceError,
};
use captain_memory::{
    devices::{DeviceRecord, PairingRequestSummary},
    hub_node_rail::{HubNodeConnectionRecord, HubNodeConnectionStatus},
};
use captain_runtime::audit::AuditAction;
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    ExecutionTarget, PairingPollRequest, HUB_NODE_PROTOCOL_VERSION,
};
use governor::{clock::DefaultClock, state::keyed::DashMapStateStore, Quota, RateLimiter};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    sync::{Arc, LazyLock},
};

pub(crate) const MAX_HUB_PAIRING_BODY_SIZE: usize = 16 * 1024;
pub(crate) const HUB_PAIRING_REQUESTS_PATH: &str = "/api/hub/pairing/requests";
pub(crate) const HUB_PAIRING_APPROVE_PATH: &str = "/api/hub/pairing/approve";
pub(crate) const HUB_PAIRING_REVIEW_PATH: &str = "/api/hub/pairing/review";
pub(crate) const HUB_PAIRING_ENROLLMENT_PATH: &str = "/api/hub/pairing/enrollment";
pub(crate) const HUB_DEVICES_PATH: &str = "/api/hub/devices";

const CLAIMS_PER_MINUTE: u32 = 12;
const POLLS_PER_MINUTE: u32 = 600;
const EXCHANGES_PER_MINUTE: u32 = 120;
const CLIENT_PRESENCE_WINDOW_MS: i64 = 2 * 60 * 1000;

type HubPairingLimiter = RateLimiter<IpAddr, DashMapStateStore<IpAddr>, DefaultClock>;

static CLAIM_LIMITER: LazyLock<HubPairingLimiter> =
    LazyLock::new(|| pairing_limiter(CLAIMS_PER_MINUTE, 5));
static POLL_LIMITER: LazyLock<HubPairingLimiter> =
    LazyLock::new(|| pairing_limiter(POLLS_PER_MINUTE, 50));
static EXCHANGE_LIMITER: LazyLock<HubPairingLimiter> =
    LazyLock::new(|| pairing_limiter(EXCHANGES_PER_MINUTE, 20));

pub fn is_hub_pairing_bootstrap_route(method: &axum::http::Method, path: &str) -> bool {
    *method == axum::http::Method::POST
        && matches!(
            path,
            captain_wire::PAIRING_CLAIM_PATH
                | captain_wire::PAIRING_POLL_PATH
                | captain_wire::DEVICE_TOKEN_PATH
        )
}

#[derive(Deserialize)]
pub struct HubPairingApprovalRequest {
    display_code: String,
    grant: DeviceGrant,
}

#[derive(Deserialize)]
pub struct HubPairingReviewRequest {
    display_code: String,
}

#[derive(Deserialize)]
pub struct HubEnrollmentWindowRequest {
    #[serde(default = "default_enrollment_window_secs")]
    duration_secs: u64,
}

pub async fn hub_pairing_claim(
    State(state): State<Arc<AppState>>,
    connect: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client_ip = pairing_client_ip(&state, connect, &headers);
    if let Err(response) = enforce_pairing_rate_limit(&CLAIM_LIMITER, client_ip, 5) {
        return response;
    }
    let claim = match parse_pairing_body::<DevicePairingClaim>(&body, "pairing claim") {
        Ok(claim) => claim,
        Err(response) => return response,
    };
    match state.kernel.hub_pairing.create_claim(&claim) {
        Ok(challenge) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::WireConnect,
                "device pairing claim accepted",
                format!(
                    "request_id={} role={}",
                    challenge.request_id,
                    device_role_label(claim.role)
                ),
            );
            (StatusCode::CREATED, Json(challenge)).into_response()
        }
        Err(error) => claim_error_response(error),
    }
}

pub async fn hub_pairing_poll(
    State(state): State<Arc<AppState>>,
    connect: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client_ip = pairing_client_ip(&state, connect, &headers);
    if let Err(response) = enforce_pairing_rate_limit(&POLL_LIMITER, client_ip, 1) {
        return response;
    }
    let request = match parse_pairing_body::<PairingPollRequest>(&body, "pairing poll") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match state.kernel.hub_pairing.poll(&request) {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => poll_error_response(error),
    }
}

pub async fn hub_device_token(
    State(state): State<Arc<AppState>>,
    connect: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let client_ip = pairing_client_ip(&state, connect, &headers);
    if let Err(response) = enforce_pairing_rate_limit(&EXCHANGE_LIMITER, client_ip, 1) {
        return response;
    }
    let request =
        match parse_pairing_body::<DeviceCredentialExchange>(&body, "device credential exchange") {
            Ok(request) => request,
            Err(response) => return response,
        };
    match state
        .kernel
        .hub_pairing
        .exchange_device_credential(&request)
    {
        Ok(token) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::WireConnect,
                "device access token issued",
                format!("device_id={}", request.device_id),
            );
            (StatusCode::OK, Json(token)).into_response()
        }
        Err(error) => credential_error_response(error),
    }
}

pub async fn hub_pairing_requests(State(state): State<Arc<AppState>>) -> Response {
    let requests = match state.kernel.hub_pairing.pending_requests() {
        Ok(requests) => requests,
        Err(error) => return operator_error_response(error),
    };
    let requests = match requests
        .into_iter()
        .map(pairing_request_payload)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(requests) => requests,
        Err(error) => return operator_error_response(error),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({"requests": requests})),
    )
        .into_response()
}

pub async fn hub_pairing_review(
    State(state): State<Arc<AppState>>,
    Json(request): Json<HubPairingReviewRequest>,
) -> Response {
    match state
        .kernel
        .hub_pairing
        .review_display_code(&request.display_code)
    {
        Ok(request) => match pairing_request_payload(request) {
            Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
            Err(error) => operator_error_response(error),
        },
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_pairing_enrollment_status(State(state): State<Arc<AppState>>) -> Response {
    match state.kernel.hub_pairing.enrollment_status() {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_pairing_enrollment_open(
    State(state): State<Arc<AppState>>,
    Json(request): Json<HubEnrollmentWindowRequest>,
) -> Response {
    match state
        .kernel
        .hub_pairing
        .open_enrollment_window(request.duration_secs)
    {
        Ok(status) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::ApprovalDecision,
                "device enrollment window opened by operator",
                format!("expires_at_ms={}", status.expires_at_ms.unwrap_or_default()),
            );
            (StatusCode::OK, Json(status)).into_response()
        }
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_pairing_enrollment_close(State(state): State<Arc<AppState>>) -> Response {
    match state.kernel.hub_pairing.close_enrollment_window() {
        Ok(status) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::ApprovalDecision,
                "device enrollment window closed by operator",
                "state=closed",
            );
            (StatusCode::OK, Json(status)).into_response()
        }
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_pairing_approve(
    State(state): State<Arc<AppState>>,
    Json(request): Json<HubPairingApprovalRequest>,
) -> Response {
    match state
        .kernel
        .hub_pairing
        .approve_display_code(&request.display_code, &request.grant)
    {
        Ok(device) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::ApprovalDecision,
                "device pairing approved by operator",
                format!("device_id={} role={}", device.device_id, device.role),
            );
            match device_payload(device) {
                Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
                Err(error) => operator_error_response(error),
            }
        }
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_pairing_deny(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Response {
    let request_id = match canonical_uuid(&request_id) {
        Some(request_id) => request_id,
        None => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_id",
                "Invalid request ID",
            )
        }
    };
    match state.kernel.hub_pairing.deny_request(&request_id) {
        Ok(()) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::ApprovalDecision,
                "device pairing denied by operator",
                format!("request_id={request_id}"),
            );
            (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
        }
        Err(error) => operator_error_response(error),
    }
}

pub async fn hub_devices(State(state): State<Arc<AppState>>) -> Response {
    let devices = match state.kernel.hub_pairing.list_devices() {
        Ok(devices) => devices,
        Err(error) => return operator_error_response(error),
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    let devices = match devices
        .into_iter()
        .map(|device| {
            let connection = if device.role == "node" {
                state
                    .kernel
                    .hub_nodes
                    .device_connection(&device.device_id)
                    .map_err(map_node_projection_error)?
            } else {
                None
            };
            device_payload_at(device, connection.as_ref(), now_ms)
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(devices) => devices,
        Err(error) => return operator_error_response(error),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({"devices": devices})),
    )
        .into_response()
}

pub async fn hub_device_revoke(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> Response {
    if !valid_device_id(&device_id) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_device_id",
            "Invalid device ID",
        );
    }
    match state.kernel.hub_pairing.revoke_device(&device_id) {
        Ok(()) => {
            state.kernel.audit_log.record_or_alert(
                "hub_pairing",
                AuditAction::ApprovalDecision,
                "device access revoked by operator",
                format!("device_id={device_id}"),
            );
            (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
        }
        Err(error) => operator_error_response(error),
    }
}

fn pairing_limiter(per_minute: u32, burst: u32) -> HubPairingLimiter {
    let quota = Quota::per_minute(NonZeroU32::new(per_minute).expect("non-zero pairing rate"))
        .allow_burst(NonZeroU32::new(burst).expect("non-zero pairing burst"));
    RateLimiter::keyed(quota)
}

fn pairing_client_ip(
    state: &AppState,
    connect: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: &HeaderMap,
) -> IpAddr {
    crate::web_auth_security::request_client_ip(
        connect.map(|Extension(ConnectInfo(address))| address),
        headers,
        &state.kernel.config.deployment,
    )
}

#[allow(clippy::result_large_err)]
fn enforce_pairing_rate_limit(
    limiter: &HubPairingLimiter,
    client_ip: IpAddr,
    retry_after_secs: u64,
) -> Result<(), Response> {
    if limiter.check_key(&client_ip).is_ok() {
        return Ok(());
    }
    tracing::warn!(client_ip = %client_ip, "Hub pairing route rate limit exceeded");
    Err(api_error_with_retry(
        StatusCode::TOO_MANY_REQUESTS,
        "pairing_rate_limited",
        "Pairing rate limit exceeded",
        retry_after_secs,
    ))
}

#[allow(clippy::result_large_err)]
fn parse_pairing_body<T: DeserializeOwned>(body: &[u8], label: &str) -> Result<T, Response> {
    if body.len() > MAX_HUB_PAIRING_BODY_SIZE {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "pairing_body_too_large",
            "Pairing request is too large",
        ));
    }
    serde_json::from_slice(body).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_pairing_payload",
            &format!("Invalid {label}"),
        )
    })
}

fn pairing_request_payload(
    request: PairingRequestSummary,
) -> Result<serde_json::Value, PairingServiceError> {
    let capabilities: serde_json::Value = serde_json::from_str(&request.capabilities_json)
        .map_err(|_| PairingServiceError::StorageUnavailable)?;
    let requested_grants: serde_json::Value = serde_json::from_str(&request.requested_grants_json)
        .map_err(|_| PairingServiceError::StorageUnavailable)?;
    Ok(serde_json::json!({
        "request_id": request.request_id,
        "display_name": request.display_name,
        "role": request.role,
        "platform": request.platform,
        "captain_version": request.captain_version,
        "protocol_version": {
            "major": request.protocol_major,
            "minor": request.protocol_minor,
        },
        "capabilities": capabilities,
        "requested_grants": requested_grants,
        "status": request.status,
        "created_at_ms": request.created_at_ms,
        "expires_at_ms": request.expires_at_ms,
    }))
}

fn device_payload(device: DeviceRecord) -> Result<serde_json::Value, PairingServiceError> {
    device_payload_at(device, None, chrono::Utc::now().timestamp_millis())
}

#[derive(Debug, Serialize)]
struct DevicePresenceProjection {
    state: &'static str,
    online: bool,
    compatible: bool,
    selectable: bool,
    reason_code: Option<&'static str>,
    action: Option<&'static str>,
}

/// Sanitized Node/workspace projection used by the execution-target selector.
/// It deliberately contains no local path, credential or connection ID.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecutionNodeTargetOption {
    pub(crate) target: ExecutionTarget,
    pub(crate) label: String,
    pub(crate) device_name: String,
    pub(crate) workspace_label: String,
    pub(crate) platform: String,
    pub(crate) captain_version: String,
    pub(crate) status: &'static str,
    pub(crate) online: bool,
    pub(crate) compatible: bool,
    pub(crate) selectable: bool,
    pub(crate) reason_code: Option<&'static str>,
    pub(crate) action: Option<&'static str>,
    pub(crate) read_only: bool,
    pub(crate) mutations_allowed: bool,
    pub(crate) tool_families: Vec<String>,
    pub(crate) last_seen_ms: i64,
}

pub(crate) fn execution_node_target_options(
    state: &AppState,
) -> Result<Vec<ExecutionNodeTargetOption>, PairingServiceError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut options = Vec::new();
    let devices = match state.kernel.hub_pairing.list_devices() {
        Ok(devices) => devices,
        Err(PairingServiceError::Disabled) => return Ok(options),
        Err(error) => return Err(error),
    };
    for device in devices {
        if device.role != "node" {
            continue;
        }
        let capabilities: CapabilityDescriptor = serde_json::from_str(&device.capabilities_json)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        let grants: DeviceGrant = serde_json::from_str(&device.grants_json)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        grants
            .validate_against(&capabilities)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        if grants.tool_families.is_empty() {
            continue;
        }
        let connection = state
            .kernel
            .hub_nodes
            .device_connection(&device.device_id)
            .map_err(map_node_projection_error)?;
        let presence = project_device_presence(
            &device,
            DeviceRole::Node,
            &grants,
            connection.as_ref(),
            now_ms,
        );
        for workspace in capabilities.workspaces.iter().filter(|workspace| {
            grants
                .workspace_ids
                .iter()
                .any(|granted| granted == &workspace.workspace_id)
        }) {
            options.push(ExecutionNodeTargetOption {
                target: ExecutionTarget::Node {
                    device_id: device.device_id.clone(),
                    workspace_id: workspace.workspace_id.clone(),
                },
                label: format!("{} · {}", device.display_name, workspace.label),
                device_name: device.display_name.clone(),
                workspace_label: workspace.label.clone(),
                platform: device.platform.clone(),
                captain_version: device.captain_version.clone(),
                status: presence.state,
                online: presence.online,
                compatible: presence.compatible,
                selectable: presence.selectable,
                reason_code: presence.reason_code,
                action: presence.action,
                read_only: workspace.read_only,
                mutations_allowed: grants.allow_mutation && !workspace.read_only,
                tool_families: grants.tool_families.clone(),
                last_seen_ms: device.last_seen_ms,
            });
        }
    }
    Ok(options)
}

fn device_payload_at(
    device: DeviceRecord,
    connection: Option<&HubNodeConnectionRecord>,
    now_ms: i64,
) -> Result<serde_json::Value, PairingServiceError> {
    let capabilities: CapabilityDescriptor = serde_json::from_str(&device.capabilities_json)
        .map_err(|_| PairingServiceError::StorageUnavailable)?;
    let grants: DeviceGrant = serde_json::from_str(&device.grants_json)
        .map_err(|_| PairingServiceError::StorageUnavailable)?;
    let role = parse_device_role(&device.role)?;
    let presence = project_device_presence(&device, role, &grants, connection, now_ms);
    Ok(serde_json::json!({
        "device_id": device.device_id,
        "display_name": device.display_name,
        "role": device.role,
        "platform": device.platform,
        "captain_version": device.captain_version,
        "protocol_version": {
            "major": device.protocol_major,
            "minor": device.protocol_minor,
        },
        "capabilities": capabilities,
        "grants": grants,
        "status": presence.state,
        "registry_status": device.status,
        "presence": presence,
        "paired_at_ms": device.paired_at_ms,
        "last_seen_ms": device.last_seen_ms,
        "updated_at_ms": device.updated_at_ms,
        "last_transport": device.last_transport,
        "last_error_code": device.last_error_code,
        "revoked_at_ms": device.revoked_at_ms,
    }))
}

fn project_device_presence(
    device: &DeviceRecord,
    role: DeviceRole,
    grants: &DeviceGrant,
    connection: Option<&HubNodeConnectionRecord>,
    now_ms: i64,
) -> DevicePresenceProjection {
    let compatible = HUB_NODE_PROTOCOL_VERSION
        .negotiate(captain_wire::ProtocolVersion {
            major: device.protocol_major,
            minor: device.protocol_minor,
        })
        .is_ok();
    if device.status == "revoked" || device.revoked_at_ms.is_some() {
        return DevicePresenceProjection {
            state: "revoked",
            online: false,
            compatible,
            selectable: false,
            reason_code: Some("device_revoked"),
            action: Some("Pair this device again to restore access."),
        };
    }
    if !compatible {
        return DevicePresenceProjection {
            state: "incompatible",
            online: false,
            compatible: false,
            selectable: false,
            reason_code: Some("protocol_incompatible"),
            action: Some("Update Captain on this device, then reconnect it."),
        };
    }

    let online = match role {
        DeviceRole::Node => connection.is_some_and(|connection| {
            connection.status == HubNodeConnectionStatus::Active
                && now_ms.saturating_sub(connection.last_seen_ms)
                    <= i64::try_from(HUB_NODE_LEASE_DURATION_MS).unwrap_or(i64::MAX)
        }),
        DeviceRole::Client => {
            device.status == "active"
                && now_ms.saturating_sub(device.last_seen_ms) <= CLIENT_PRESENCE_WINDOW_MS
        }
    };
    if !online {
        let (reason_code, action) = match role {
            DeviceRole::Node => (
                "node_offline",
                "Start `captain node run` on this device and check its network access.",
            ),
            DeviceRole::Client => (
                "client_offline",
                "Open Captain on this device to reconnect it.",
            ),
        };
        return DevicePresenceProjection {
            state: "offline",
            online: false,
            compatible: true,
            selectable: false,
            reason_code: Some(reason_code),
            action: Some(action),
        };
    }

    let selectable = role == DeviceRole::Node
        && !grants.workspace_ids.is_empty()
        && !grants.tool_families.is_empty();
    DevicePresenceProjection {
        state: "online",
        online: true,
        compatible: true,
        selectable,
        reason_code: (!selectable && role == DeviceRole::Node).then_some("no_execution_grant"),
        action: (!selectable && role == DeviceRole::Node)
            .then_some("Approve at least one workspace and tool family for this Node."),
    }
}

fn parse_device_role(value: &str) -> Result<DeviceRole, PairingServiceError> {
    match value {
        "client" => Ok(DeviceRole::Client),
        "node" => Ok(DeviceRole::Node),
        _ => Err(PairingServiceError::StorageUnavailable),
    }
}

fn map_node_projection_error(_error: HubNodeServiceError) -> PairingServiceError {
    PairingServiceError::StorageUnavailable
}

fn claim_error_response(error: PairingServiceError) -> Response {
    match error {
        PairingServiceError::Disabled => pairing_disabled(),
        PairingServiceError::EnrollmentClosed => api_error(
            StatusCode::FORBIDDEN,
            "pairing_enrollment_closed",
            "Device enrollment is closed; ask an operator to open Add device",
        ),
        PairingServiceError::TooManyPending => api_error_with_retry(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_pairing_requests",
            "Too many pending pairing requests",
            60,
        ),
        PairingServiceError::MaximumDevices { .. } => api_error(
            StatusCode::CONFLICT,
            "maximum_devices_reached",
            "Maximum paired devices reached",
        ),
        PairingServiceError::CredentialAlreadyClaimed => api_error(
            StatusCode::CONFLICT,
            "credential_already_claimed",
            "This device credential already has a pairing claim",
        ),
        PairingServiceError::StorageUnavailable => pairing_storage_unavailable(),
        _ => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_pairing_claim",
            "Invalid pairing claim",
        ),
    }
}

fn poll_error_response(error: PairingServiceError) -> Response {
    match error {
        PairingServiceError::Disabled => pairing_disabled(),
        PairingServiceError::InvalidPollingCredential | PairingServiceError::PairingNotFound => {
            api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_polling_credential",
                "Invalid pairing polling credential",
            )
        }
        PairingServiceError::PairingExpired => api_error(
            StatusCode::GONE,
            "pairing_expired",
            "Pairing request expired",
        ),
        PairingServiceError::StorageUnavailable => pairing_storage_unavailable(),
        _ => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_pairing_poll",
            "Invalid pairing poll",
        ),
    }
}

fn credential_error_response(error: PairingServiceError) -> Response {
    match error {
        PairingServiceError::Disabled => pairing_disabled(),
        PairingServiceError::InvalidDeviceCredential
        | PairingServiceError::DeviceNotFound
        | PairingServiceError::DeviceNotActive(_) => api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_device_credential",
            "Invalid or revoked device credential",
        ),
        PairingServiceError::StorageUnavailable => pairing_storage_unavailable(),
        _ => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_credential_exchange",
            "Invalid device credential exchange",
        ),
    }
}

fn operator_error_response(error: PairingServiceError) -> Response {
    match error {
        PairingServiceError::Disabled => pairing_disabled(),
        PairingServiceError::PairingNotFound | PairingServiceError::DeviceNotFound => api_error(
            StatusCode::NOT_FOUND,
            "pairing_resource_not_found",
            "Pairing resource not found",
        ),
        PairingServiceError::PairingExpired => api_error(
            StatusCode::GONE,
            "pairing_expired",
            "Pairing request expired",
        ),
        PairingServiceError::MaximumDevices { .. }
        | PairingServiceError::PairingNotPending(_)
        | PairingServiceError::CredentialAlreadyClaimed
        | PairingServiceError::DeviceNotActive(_) => api_error(
            StatusCode::CONFLICT,
            "pairing_state_conflict",
            "Pairing state conflict",
        ),
        PairingServiceError::TooManyPending => api_error_with_retry(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_pairing_requests",
            "Too many pending pairing requests",
            60,
        ),
        PairingServiceError::StorageUnavailable => pairing_storage_unavailable(),
        _ => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_pairing_operation",
            "Invalid pairing operation",
        ),
    }
}

fn pairing_disabled() -> Response {
    api_error(
        StatusCode::NOT_FOUND,
        "pairing_disabled",
        "Device pairing is not enabled",
    )
}

fn pairing_storage_unavailable() -> Response {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "pairing_storage_unavailable",
        "Device pairing is temporarily unavailable",
    )
}

fn api_error(status: StatusCode, code: &'static str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

fn api_error_with_retry(
    status: StatusCode,
    code: &'static str,
    message: &str,
    retry_after_secs: u64,
) -> Response {
    (
        status,
        [("retry-after", retry_after_secs.to_string())],
        Json(serde_json::json!({
            "error": {
                "code": code,
                "message": message,
            }
        })),
    )
        .into_response()
}

fn canonical_uuid(value: &str) -> Option<String> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(|uuid| uuid.to_string())
}

fn default_enrollment_window_secs() -> u64 {
    10 * 60
}

fn valid_device_id(value: &str) -> bool {
    ["client-", "node-"]
        .iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .and_then(|suffix| uuid::Uuid::parse_str(suffix).ok())
        .is_some()
}

fn device_role_label(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Client => "client",
        DeviceRole::Node => "node",
    }
}

#[cfg(test)]
#[path = "hub_pairing_routes_tests.rs"]
mod tests;
