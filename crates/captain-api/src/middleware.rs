//! Production middleware for the Captain API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - In-memory rate limiting (per IP)

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use axum::middleware::Next;
use std::time::Instant;
use tracing::info;

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Middleware: inject a unique request ID and log the request/response.
pub async fn request_logging(request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    let mut response = next.run(request).await;

    let elapsed = start.elapsed();
    let status = response.status().as_u16();

    info!(
        request_id = %request_id,
        method = %method,
        path = %uri,
        status = status,
        latency_ms = elapsed.as_millis() as u64,
        "API request"
    );

    // Inject the request ID into the response
    if let Ok(header_val) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, header_val);
    }

    response
}

/// Authentication state passed to the auth middleware.
#[derive(Clone)]
pub struct AuthState {
    pub api_key: String,
    pub home_dir: std::path::PathBuf,
    pub fallback_auth: captain_types::config::AuthConfig,
}

/// Bearer token authentication middleware.
///
/// When `api_key` is non-empty (after trimming), requests to non-public
/// endpoints must include `Authorization: Bearer <api_key>`.
/// If the key is empty or whitespace-only, auth is disabled entirely
/// (public/local development mode).
///
/// When web auth is enabled, session cookies are also accepted.
pub async fn auth(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path();

    if is_loopback_shutdown_request(&request, path) || is_public_endpoint(&method, path) {
        return next.run(request).await;
    }

    let auth_snapshot = crate::session_auth::load_web_auth_snapshot(
        &auth_state.home_dir,
        &auth_state.api_key,
        &auth_state.fallback_auth,
    );

    match authorize_request(&request, &auth_snapshot) {
        AuthDecision::Allow => next.run(request).await,
        AuthDecision::Deny(error_msg) => unauthorized_response(error_msg),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AuthDecision {
    Allow,
    Deny(&'static str),
}

#[derive(Debug)]
struct RequestCredentials<'a> {
    bearer_token: Option<&'a str>,
    header_token: Option<&'a str>,
    query_token: Option<&'a str>,
    session_cookie: Option<String>,
}

impl<'a> RequestCredentials<'a> {
    fn web_session_token_candidate(&self) -> Option<&'a str> {
        self.bearer_token.or(self.query_token)
    }
}

fn is_loopback_shutdown_request(request: &Request<Body>, path: &str) -> bool {
    if path != "/api/shutdown" {
        return false;
    }
    request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false)
}

fn is_public_endpoint(method: &Method, path: &str) -> bool {
    let is_get = *method == Method::GET;
    path == "/"
        || public_static_or_boot_endpoint(path, is_get)
        || public_read_api_endpoint(path, is_get)
        || public_protocol_endpoint(method, path, is_get)
}

fn public_static_or_boot_endpoint(path: &str, is_get: bool) -> bool {
    (path == "/" || path.starts_with("/assets/") || path == "/terminal" || path == "/config")
        && is_get
        || path == "/logo.svg"
        || path == "/favicon.ico"
        || path == "/manifest.json"
        || path == "/sw.js"
        || (path == "/.well-known/agent.json" && is_get)
        || (path.starts_with("/a2a/") && is_get)
}

fn public_read_api_endpoint(path: &str, is_get: bool) -> bool {
    path == "/api/health"
        || path == "/api/health/detail"
        || path == "/api/status"
        || path == "/api/version"
        || (path == "/api/agents" && is_get)
        || (path == "/api/profiles" && is_get)
        || (path.starts_with("/api/uploads/") && is_get)
        || (path == "/api/models" && is_get)
        || (path == "/api/models/aliases" && is_get)
        || (path == "/api/providers" && is_get)
        || (path == "/api/budget" && is_get)
        || (path == "/api/budget/agents" && is_get)
        || (path.starts_with("/api/budget/agents/") && is_get)
        || (path == "/api/network/status" && is_get)
        || (path == "/api/a2a/agents" && is_get)
        || (path == "/api/approvals" && is_get)
        || (path.starts_with("/api/approvals/") && is_get)
        || (path == "/api/channels" && is_get)
        || (path == "/api/hands" && is_get)
        || (path == "/api/hands/active" && is_get)
        || (path.starts_with("/api/hands/") && is_get)
        || (path == "/api/skills" && is_get)
        || (path == "/api/sessions" && is_get)
        || (path == "/api/integrations" && is_get)
        || (path == "/api/integrations/available" && is_get)
        || (path == "/api/integrations/health" && is_get)
        || (path == "/api/workflows" && is_get)
        || path == "/api/logs/stream"
        || (path.starts_with("/api/cron/") && is_get)
}

fn public_protocol_endpoint(method: &Method, path: &str, is_get: bool) -> bool {
    path.starts_with("/api/providers/github-copilot/oauth/")
        || crate::agent_api_routes::is_agent_api_ingress_route(method, path)
        || path == "/api/auth/login"
        || path == "/api/auth/logout"
        || (path == "/api/auth/check" && is_get)
}

fn authorize_request(
    request: &Request<Body>,
    auth_snapshot: &crate::session_auth::WebAuthSnapshot,
) -> AuthDecision {
    let auth_enabled = auth_snapshot.auth.enabled;
    let api_key = auth_snapshot.api_key.trim();
    if api_key.is_empty() && !auth_enabled {
        return AuthDecision::Allow;
    }

    let credentials = request_credentials(request);
    let header_auth = credentials
        .header_token
        .map(|token| api_key_matches(token, api_key));
    let query_auth = credentials
        .query_token
        .map(|token| api_key_matches(token, api_key));

    if header_auth == Some(true) || query_auth == Some(true) {
        return AuthDecision::Allow;
    }
    if auth_enabled && web_session_matches(&credentials, auth_snapshot) {
        return AuthDecision::Allow;
    }

    AuthDecision::Deny(auth_error_message(
        header_auth.is_some() || query_auth.is_some(),
        auth_enabled,
        api_key.is_empty(),
    ))
}

fn request_credentials(request: &Request<Body>) -> RequestCredentials<'_> {
    let bearer_token =
        header_value(request, "authorization").and_then(|v| v.strip_prefix("Bearer "));
    let x_api_key = header_value(request, "x-api-key");
    let header_token = bearer_token.or(x_api_key);
    let query_token = request
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")));

    RequestCredentials {
        bearer_token,
        header_token,
        query_token,
        session_cookie: extract_session_cookie(request),
    }
}

fn header_value<'a>(request: &'a Request<Body>, name: &str) -> Option<&'a str> {
    request.headers().get(name).and_then(|v| v.to_str().ok())
}

fn api_key_matches(token: &str, api_key: &str) -> bool {
    if api_key.is_empty() || token.len() != api_key.len() {
        return false;
    }
    use subtle::ConstantTimeEq;
    token.as_bytes().ct_eq(api_key.as_bytes()).into()
}

fn web_session_matches(
    credentials: &RequestCredentials<'_>,
    auth_snapshot: &crate::session_auth::WebAuthSnapshot,
) -> bool {
    if let Some(token) = credentials.web_session_token_candidate() {
        if crate::session_auth::verify_session_token_for_auth(token, auth_snapshot).is_some() {
            return true;
        }
    }
    credentials
        .session_cookie
        .as_deref()
        .and_then(|token| crate::session_auth::verify_session_token_for_auth(token, auth_snapshot))
        .is_some()
}

fn auth_error_message(
    credential_provided: bool,
    auth_enabled: bool,
    api_key_empty: bool,
) -> &'static str {
    if credential_provided {
        "Invalid API key"
    } else if auth_enabled && api_key_empty {
        "Missing or invalid web session credentials"
    } else if auth_enabled {
        "Missing Authorization: Bearer <api_key> header or web session credentials"
    } else {
        "Missing Authorization: Bearer <api_key> header"
    }
}

fn unauthorized_response(error_msg: &'static str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", "Bearer")
        .body(Body::from(
            serde_json::json!({"error": error_msg}).to_string(),
        ))
        .unwrap_or_default()
}

/// Extract the `captain_session` cookie value from a request.
fn extract_session_cookie(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("captain_session=")
                    .map(|v| v.to_string())
            })
        })
}

/// Security headers middleware — applied to ALL API responses.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "1; mode=block".parse().unwrap());
    // All JS/CSS is bundled inline — only external resource is Google Fonts.
    headers.insert(
        "content-security-policy",
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://fonts.gstatic.com; img-src 'self' data: blob:; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:*; font-src 'self' https://fonts.gstatic.com; media-src 'self' blob:; frame-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'"
            .parse()
            .unwrap(),
    );
    headers.insert(
        "referrer-policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert(
        "cache-control",
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_header_constant() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn public_endpoint_policy_keeps_mutations_private() {
        assert!(is_public_endpoint(&Method::GET, "/api/agents"));
        assert!(!is_public_endpoint(&Method::POST, "/api/agents"));
        assert!(is_public_endpoint(&Method::GET, "/config"));
        assert!(!is_public_endpoint(&Method::GET, "/api/config"));
        assert!(is_public_endpoint(&Method::POST, "/api/auth/logout"));
    }

    #[test]
    fn agent_api_ingress_route_uses_route_specific_auth() {
        let path = "/hooks/agents/00000000-0000-0000-0000-000000000000/ingress";
        assert!(is_public_endpoint(&Method::POST, path));
        assert!(!is_public_endpoint(&Method::GET, path));
    }

    #[test]
    fn shutdown_auth_bypass_is_loopback_only() {
        let mut request = Request::builder()
            .uri("/api/shutdown")
            .body(Body::empty())
            .unwrap();
        assert!(!is_loopback_shutdown_request(&request, "/api/shutdown"));

        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:4200".parse::<std::net::SocketAddr>().unwrap(),
        ));
        assert!(is_loopback_shutdown_request(&request, "/api/shutdown"));
        assert!(!is_loopback_shutdown_request(&request, "/api/status"));
    }

    #[test]
    fn request_credentials_prefer_bearer_and_extract_cookie() {
        let request = Request::builder()
            .uri("/api/logs/stream?token=query-token")
            .header("authorization", "Bearer bearer-token")
            .header("x-api-key", "api-key")
            .header(
                "cookie",
                "theme=dark; captain_session=session-token; other=x",
            )
            .body(Body::empty())
            .unwrap();

        let credentials = request_credentials(&request);
        assert_eq!(credentials.bearer_token, Some("bearer-token"));
        assert_eq!(credentials.header_token, Some("bearer-token"));
        assert_eq!(credentials.query_token, Some("query-token"));
        assert_eq!(credentials.session_cookie.as_deref(), Some("session-token"));
    }

    #[test]
    fn web_auth_without_api_key_rejects_blank_bearer_token() {
        let snapshot = crate::session_auth::WebAuthSnapshot {
            api_key: String::new(),
            auth: captain_types::config::AuthConfig {
                enabled: true,
                username: "admin".to_string(),
                password_hash: "hash".to_string(),
                session_ttl_hours: 1,
            },
        };
        let request = Request::builder()
            .uri("/api/commands")
            .header("authorization", "Bearer ")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot),
            AuthDecision::Deny("Invalid API key")
        );
    }

    #[test]
    fn web_auth_without_api_key_accepts_valid_session_bearer() {
        let snapshot = crate::session_auth::WebAuthSnapshot {
            api_key: String::new(),
            auth: captain_types::config::AuthConfig {
                enabled: true,
                username: "admin".to_string(),
                password_hash: "hash".to_string(),
                session_ttl_hours: 1,
            },
        };
        let token =
            crate::session_auth::create_session_token("admin", &snapshot.session_secret(), 1);
        let request = Request::builder()
            .uri("/api/commands")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(authorize_request(&request, &snapshot), AuthDecision::Allow);
    }
}
