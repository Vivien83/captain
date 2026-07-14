use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

/// POST /api/pairing/request - Create a new pairing request.
pub async fn pairing_request(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return pairing_disabled();
    }
    match state.kernel.pairing.create_pairing_request() {
        Ok(request) => Json(serde_json::json!({
            "token": request.token,
            "qr_uri": format!("captain://pair?token={}", request.token),
            "expires_at": request.expires_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// POST /api/pairing/complete - Complete pairing with token and device info.
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return pairing_disabled();
    }
    let token = body
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let device_info = captain_kernel::pairing::PairedDevice {
        device_id: uuid::Uuid::new_v4().to_string(),
        display_name: body
            .get("display_name")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        platform: body
            .get("platform")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token: body
            .get("push_token")
            .and_then(|value| value.as_str())
            .map(String::from),
    };
    match state.kernel.pairing.complete_pairing(token, device_info) {
        Ok(device) => Json(serde_json::json!({
            "device_id": device.device_id,
            "display_name": device.display_name,
            "platform": device.platform,
            "paired_at": device.paired_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// GET /api/pairing/devices - List paired devices.
pub async fn pairing_devices(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return pairing_disabled();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing
        .list_devices()
        .into_iter()
        .map(|device| {
            serde_json::json!({
                "device_id": device.device_id,
                "display_name": device.display_name,
                "platform": device.platform,
                "paired_at": device.paired_at.to_rfc3339(),
                "last_seen": device.last_seen.to_rfc3339(),
            })
        })
        .collect();
    Json(serde_json::json!({"devices": devices})).into_response()
}

/// DELETE /api/pairing/devices/{id} - Remove a paired device.
pub async fn pairing_remove_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return pairing_disabled();
    }
    match state.kernel.pairing.remove_device(&device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e}))).into_response(),
    }
}

/// POST /api/pairing/notify - Push a notification to all paired devices.
pub async fn pairing_notify(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !state.kernel.config.pairing.enabled {
        return pairing_disabled();
    }
    let title = body
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("Captain");
    let message = body
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "message is required"})),
        )
            .into_response();
    }
    state.kernel.pairing.notify_devices(title, message).await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing.list_devices().len()}))
        .into_response()
}

fn pairing_disabled() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "Pairing not enabled"})),
    )
        .into_response()
}
