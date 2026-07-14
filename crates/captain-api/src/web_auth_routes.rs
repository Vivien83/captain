//! Web authentication route handlers.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

/// POST /api/auth/login - Authenticate with username/password and return a session token.
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> Response {
    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &state.kernel.config.home_dir,
        &state.kernel.config.api_key,
        &state.kernel.config.auth,
    );
    let auth_cfg = &auth_snapshot.auth;
    if !auth_cfg.enabled {
        return json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": "Auth not enabled"}),
        );
    }

    let username = req
        .get("username")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let password = req
        .get("password")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    let username_ok = crate::session_auth::username_matches(username, &auth_cfg.username);
    if !username_ok || !crate::session_auth::verify_password(password, &auth_cfg.password_hash) {
        state.kernel.audit_log.record(
            "system",
            captain_runtime::audit::AuditAction::AuthAttempt,
            "web login failed",
            format!("username: {username}"),
        );
        return json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"error": "Invalid credentials"}),
        );
    }

    let token = crate::session_auth::create_session_token(
        username,
        &auth_snapshot.session_secret(),
        auth_cfg.session_ttl_hours,
    );
    let ttl_secs = auth_cfg.session_ttl_hours * 3600;
    let cookie =
        format!("captain_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}");

    state.kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::AuthAttempt,
        "web login success",
        format!("username: {username}"),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", &cookie)
        .body(Body::from(
            serde_json::json!({
                "status": "ok",
                "token": token,
                "username": username,
            })
            .to_string(),
        ))
        .unwrap()
}

/// POST /api/auth/logout - Clear the session cookie.
pub async fn auth_logout() -> impl IntoResponse {
    let cookie = "captain_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [("content-type", "application/json"), ("set-cookie", cookie)],
        serde_json::json!({"status": "ok"}).to_string(),
    )
}

/// GET /api/auth/check - Check current authentication state.
pub async fn auth_check(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
) -> impl IntoResponse {
    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &state.kernel.config.home_dir,
        &state.kernel.config.api_key,
        &state.kernel.config.auth,
    );
    let auth_cfg = &auth_snapshot.auth;

    if !auth_cfg.enabled && !auth_snapshot.api_key_configured() {
        return Json(serde_json::json!({
            "authenticated": false,
            "mode": "none",
            "api_key_configured": false,
        }));
    }
    if !auth_cfg.enabled && auth_snapshot.api_key_configured() {
        return Json(serde_json::json!({
            "authenticated": false,
            "mode": "apikey",
            "api_key_configured": true,
        }));
    }

    let session_user = request
        .headers()
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_session_cookie)
        .and_then(|token| {
            crate::session_auth::verify_session_token_for_auth(&token, &auth_snapshot)
        });

    if let Some(username) = session_user {
        Json(serde_json::json!({
            "authenticated": true,
            "mode": "session",
            "api_key_configured": auth_snapshot.api_key_configured(),
            "username": username,
        }))
    } else {
        Json(serde_json::json!({
            "authenticated": false,
            "mode": "session",
            "api_key_configured": auth_snapshot.api_key_configured(),
        }))
    }
}

fn extract_session_cookie(cookies: &str) -> Option<String> {
    cookies.split(';').find_map(|cookie| {
        cookie
            .trim()
            .strip_prefix("captain_session=")
            .map(|value| value.to_string())
    })
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
