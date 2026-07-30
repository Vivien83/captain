//! Web authentication route handlers.

use crate::state::AppState;
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use captain_types::config::WebPasswordVerification;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_LOGIN_USERNAME_BYTES: usize = 256;
const MAX_LOGIN_PASSWORD_BYTES: usize = 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RealtimeTicketRequest {
    path: String,
}

#[derive(Debug, Serialize)]
pub struct RealtimeTicketResponse {
    ticket: String,
    expires_at_unix: u64,
    expires_in_seconds: u64,
}

/// POST /api/auth/login - Authenticate with username/password and return a session token.
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    Extension(security): Extension<Arc<crate::web_auth_security::WebAuthSecurity>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Response {
    let mut auth_snapshot = crate::session_auth::load_web_auth_snapshot(
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

    let peer = peer.map(|Extension(ConnectInfo(address))| address);
    let client_ip = crate::web_auth_security::request_client_ip(
        peer,
        &headers,
        &state.kernel.config.deployment,
    );
    let rate_limit_username = if req.username.len() <= MAX_LOGIN_USERNAME_BYTES {
        req.username.as_str()
    } else {
        "<oversized>"
    };
    if let Some(retry_after) =
        security.login_retry_after(client_ip, rate_limit_username, Instant::now())
    {
        state.kernel.audit_log.record_or_alert(
            "system",
            captain_runtime::audit::AuditAction::AuthAttempt,
            "web login rate limited",
            format!(
                "client_ip: {client_ip}; retry_after_seconds: {}",
                retry_after.as_secs().max(1)
            ),
        );
        return rate_limited_response(retry_after);
    }

    let input_in_bounds = req.username.len() <= MAX_LOGIN_USERNAME_BYTES
        && req.password.len() <= MAX_LOGIN_PASSWORD_BYTES;
    let username_ok = crate::session_auth::username_matches(&req.username, &auth_cfg.username);
    let verification = if req.password.len() <= MAX_LOGIN_PASSWORD_BYTES {
        captain_types::config::verify_web_password(&req.password, &auth_cfg.password_hash)
    } else {
        WebPasswordVerification::Invalid
    };
    if !input_in_bounds || !username_ok || !verification.is_valid() {
        security.record_login_failure(client_ip, rate_limit_username, Instant::now());
        state.kernel.audit_log.record_or_alert(
            "system",
            captain_runtime::audit::AuditAction::AuthAttempt,
            "web login failed",
            format!("client_ip: {client_ip}"),
        );
        return json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"error": "Invalid credentials"}),
        );
    }

    if verification == WebPasswordVerification::LegacySha256 {
        match migrate_legacy_password_hash(
            &state.kernel.config.home_dir.join("config.toml"),
            &auth_cfg.password_hash,
            &req.password,
            &security,
        ) {
            Ok(argon2_hash) => auth_snapshot.auth.password_hash = argon2_hash,
            Err(error) => {
                tracing::error!(%error, "Legacy web password migration failed");
                return json_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    serde_json::json!({
                        "error": "Web credentials could not be migrated safely"
                    }),
                );
            }
        }
    }
    security.record_login_success(client_ip, &req.username);

    let auth_cfg = &auth_snapshot.auth;
    let Some(session_secret) = auth_snapshot.session_secret() else {
        tracing::error!("Web session signing state is invalid");
        return json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "Web session signing is unavailable"}),
        );
    };
    let ttl_hours = auth_cfg.session_ttl_hours.clamp(1, 8760);
    let token = crate::session_auth::create_session_token(
        &req.username,
        &session_secret,
        ttl_hours,
        auth_cfg.session_epoch,
    );
    let ttl_secs = ttl_hours * 3600;
    let secure = crate::web_auth_security::session_cookie_is_secure(
        auth_cfg.session_cookie_secure,
        peer,
        &headers,
        &state.kernel.config.deployment,
    );
    let cookie = session_cookie(&token, ttl_secs, secure);

    state.kernel.audit_log.record_or_alert(
        "system",
        captain_runtime::audit::AuditAction::AuthAttempt,
        "web login success",
        format!("client_ip: {client_ip}"),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", &cookie)
        .body(Body::from(
            serde_json::json!({
                "status": "ok",
                "token": token,
                "username": req.username,
            })
            .to_string(),
        ))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn legacy_hash_migration_is_atomic_and_preserves_the_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let legacy = format!("{:x}", Sha256::digest(b"legacy-password"));
        std::fs::write(
            &config_path,
            format!(
                "log_level = \"debug\"\n\n[auth]\npassword_hash = \"{legacy}\"\nsession_epoch = 9\n"
            ),
        )
        .unwrap();

        let migrated = migrate_legacy_password_hash(
            &config_path,
            &legacy,
            "legacy-password",
            &crate::web_auth_security::WebAuthSecurity::default(),
        )
        .unwrap();
        let updated = std::fs::read_to_string(&config_path).unwrap();

        assert!(migrated.starts_with("$argon2id$"));
        assert!(updated.contains("log_level = \"debug\""));
        assert!(updated.contains("session_epoch = 9"));
        assert!(!updated.contains(&legacy));
        assert_eq!(
            captain_types::config::verify_web_password("legacy-password", &migrated),
            WebPasswordVerification::Argon2id
        );
    }

    #[test]
    fn secure_session_cookie_is_explicit_and_bounded() {
        let cookie = session_cookie("opaque", u64::MAX, true);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("; Secure"));
        assert!(cookie.contains("Max-Age=31536000"));
    }
}

/// POST /api/auth/logout - Clear the session cookie.
pub async fn auth_logout(
    State(state): State<Arc<AppState>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
) -> Response {
    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &state.kernel.config.home_dir,
        &state.kernel.config.api_key,
        &state.kernel.config.auth,
    );
    let secure = crate::web_auth_security::session_cookie_is_secure(
        auth_snapshot.auth.session_cookie_secure,
        peer.map(|Extension(ConnectInfo(address))| address),
        &headers,
        &state.kernel.config.deployment,
    );
    let cookie = session_cookie("", 0, secure);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", cookie)
        .body(Body::from(serde_json::json!({"status": "ok"}).to_string()))
        .unwrap_or_default()
}

/// POST /api/auth/realtime-ticket - Mint a short-lived, single-use WS/SSE ticket.
pub async fn auth_realtime_ticket(
    State(state): State<Arc<AppState>>,
    Extension(security): Extension<Arc<crate::web_auth_security::WebAuthSecurity>>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(request): Json<RealtimeTicketRequest>,
) -> Response {
    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &state.kernel.config.home_dir,
        &state.kernel.config.api_key,
        &state.kernel.config.auth,
    );
    let client_ip = crate::web_auth_security::request_client_ip(
        peer.map(|Extension(ConnectInfo(address))| address),
        &headers,
        &state.kernel.config.deployment,
    );
    match security.issue_realtime_ticket(
        &request.path,
        client_ip,
        auth_snapshot.auth.session_epoch,
        Instant::now(),
    ) {
        Ok(grant) => json_response(
            StatusCode::OK,
            serde_json::to_value(RealtimeTicketResponse {
                ticket: grant.ticket,
                expires_at_unix: grant.expires_at_unix,
                expires_in_seconds: crate::web_auth_security::REALTIME_TICKET_TTL.as_secs(),
            })
            .unwrap_or_default(),
        ),
        Err(error) if error == "Unsupported realtime path" => {
            json_response(StatusCode::BAD_REQUEST, serde_json::json!({"error": error}))
        }
        Err(error) => {
            tracing::warn!(%error, "Realtime ticket issuance refused");
            json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({"error": "Realtime ticket service is temporarily unavailable"}),
            )
        }
    }
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

fn session_cookie(token: &str, max_age_seconds: u64, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let max_age_seconds = max_age_seconds.min(8760 * 3600);
    format!(
        "captain_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{secure_attribute}"
    )
}

fn rate_limited_response(retry_after: Duration) -> Response {
    let retry_after_seconds = retry_after.as_secs().max(1);
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("content-type", "application/json")
        .header("retry-after", retry_after_seconds.to_string())
        .body(Body::from(
            serde_json::json!({
                "error": "Too many login attempts",
                "retry_after_seconds": retry_after_seconds,
            })
            .to_string(),
        ))
        .unwrap_or_default()
}

fn migrate_legacy_password_hash(
    config_path: &std::path::Path,
    expected_legacy_hash: &str,
    password: &str,
    security: &crate::web_auth_security::WebAuthSecurity,
) -> Result<String, String> {
    security.with_password_migration(|| {
        let raw = std::fs::read_to_string(config_path)
            .map_err(|error| format!("read {}: {error}", config_path.display()))?;
        let mut document = raw
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| format!("parse {}: {error}", config_path.display()))?;
        let auth = document
            .get_mut("auth")
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or_else(|| "[auth] is missing or invalid".to_string())?;
        let current_hash = auth
            .get("password_hash")
            .and_then(toml_edit::Item::as_str)
            .ok_or_else(|| "auth.password_hash is missing or invalid".to_string())?
            .to_string();

        if current_hash != expected_legacy_hash {
            return match captain_types::config::verify_web_password(password, &current_hash) {
                WebPasswordVerification::Argon2id => Ok(current_hash),
                _ => Err("auth.password_hash changed during login".to_string()),
            };
        }
        if captain_types::config::verify_web_password(password, &current_hash)
            != WebPasswordVerification::LegacySha256
        {
            return Err("legacy auth.password_hash no longer verifies".to_string());
        }

        let argon2_hash = captain_types::config::hash_web_password(password)
            .map_err(|error| format!("hash password: {error}"))?;
        auth.insert("password_hash", toml_edit::value(argon2_hash.as_str()));
        captain_types::durable_fs::atomic_write(config_path, document.to_string().as_bytes())
            .map_err(|error| format!("write {}: {error}", config_path.display()))?;
        Ok(argon2_hash)
    })
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
