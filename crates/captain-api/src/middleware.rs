//! Production middleware for the Captain API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - In-memory rate limiting (per IP)

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use axum::middleware::Next;
use std::sync::Arc;
use std::time::Instant;
use tracing::info;

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

pub(crate) const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; script-src-attr 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:*; font-src 'self'; media-src 'self' blob:; frame-src 'self' blob:; worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

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
    pub deployment: captain_types::config::DeploymentConfig,
    pub security: Arc<crate::web_auth_security::WebAuthSecurity>,
}

/// Bearer token authentication middleware.
///
/// When `api_key` is non-empty (after trimming), requests to non-public
/// endpoints must include `Authorization: Bearer <api_key>`.
///
/// When web auth is enabled, session cookies are also accepted.
/// Credentialless access is fail-closed unless the operator explicitly enables
/// the direct-loopback-only development escape hatch.
pub async fn auth(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    mut request: Request<Body>,
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

    if let Some(ticket) = crate::web_auth_security::realtime_ticket_from_uri(request.uri()) {
        let peer = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|connect| connect.0);
        let ip = crate::web_auth_security::request_client_ip(
            peer,
            request.headers(),
            &auth_state.deployment,
        );
        if auth_state.security.consume_realtime_ticket(
            ticket,
            path,
            ip,
            auth_snapshot.auth.session_epoch,
            Instant::now(),
        ) {
            request
                .extensions_mut()
                .insert(crate::web_auth_security::RealtimeTicketAuthorization);
            return next.run(request).await;
        }
        return unauthorized_response("Invalid or expired realtime ticket");
    }

    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|connect| connect.0);
    let client_is_loopback = crate::web_auth_security::request_client_is_loopback(
        peer,
        request.headers(),
        &auth_state.deployment,
    );
    match authorize_request(&request, &auth_snapshot, client_is_loopback) {
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
    session_cookie: Option<String>,
}

impl<'a> RequestCredentials<'a> {
    fn web_session_token_candidate(&self) -> Option<&'a str> {
        self.bearer_token
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
    PUBLIC_ALLOWLIST
        .iter()
        .any(|rule| rule.matches(method, path))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicMethod {
    Get,
    Post,
}

impl PublicMethod {
    fn matches(self, method: &Method) -> bool {
        match self {
            Self::Get => *method == Method::GET,
            Self::Post => *method == Method::POST,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicPath {
    Exact(&'static str),
    Prefix(&'static str),
    AgentApiIngress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublicEndpoint {
    method: PublicMethod,
    path: PublicPath,
}

impl PublicEndpoint {
    fn matches(self, method: &Method, path: &str) -> bool {
        if !self.method.matches(method) {
            return false;
        }
        match self.path {
            PublicPath::Exact(expected) => path == expected,
            PublicPath::Prefix(prefix) => path.starts_with(prefix),
            PublicPath::AgentApiIngress => {
                crate::agent_api_routes::is_agent_api_ingress_route(method, path)
            }
        }
    }
}

/// Explicitly reviewed global-auth bypasses. Every other route is private.
///
/// The per-agent ingress rule is present only because that exact route applies
/// its own rate limit and per-agent Bearer token before executing a turn.
const PUBLIC_ALLOWLIST: &[PublicEndpoint] = &[
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Prefix("/assets/"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/logo.svg"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/favicon.ico"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/manifest.json"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/sw.js"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/health"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/version"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::Exact("/api/auth/login"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::Exact("/api/auth/logout"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/auth/check"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::AgentApiIngress,
    },
];

fn authorize_request(
    request: &Request<Body>,
    auth_snapshot: &crate::session_auth::WebAuthSnapshot,
    client_is_loopback: bool,
) -> AuthDecision {
    let auth_enabled = auth_snapshot.auth.enabled;
    let api_key = auth_snapshot.api_key.trim();
    if api_key.is_empty()
        && !auth_enabled
        && auth_snapshot.auth.allow_unauthenticated_loopback
        && client_is_loopback
    {
        return AuthDecision::Allow;
    }

    let credentials = request_credentials(request);
    let header_auth = credentials
        .header_token
        .map(|token| api_key_matches(token, api_key));
    if header_auth == Some(true) {
        return AuthDecision::Allow;
    }
    if auth_enabled && web_session_matches(&credentials, auth_snapshot) {
        return AuthDecision::Allow;
    }

    AuthDecision::Deny(auth_error_message(
        header_auth.is_some(),
        auth_enabled,
        api_key.is_empty(),
        auth_snapshot.auth.allow_unauthenticated_loopback,
        client_is_loopback,
    ))
}

fn request_credentials(request: &Request<Body>) -> RequestCredentials<'_> {
    let bearer_token =
        header_value(request, "authorization").and_then(|v| v.strip_prefix("Bearer "));
    let x_api_key = header_value(request, "x-api-key");
    let header_token = bearer_token.or(x_api_key);
    RequestCredentials {
        bearer_token,
        header_token,
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
    loopback_opt_out: bool,
    client_is_loopback: bool,
) -> &'static str {
    if credential_provided {
        "Invalid API key"
    } else if !auth_enabled && api_key_empty && loopback_opt_out && !client_is_loopback {
        "Unauthenticated access is restricted to direct loopback clients"
    } else if !auth_enabled && api_key_empty {
        "Authentication is not configured; run `captain setup`"
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
    // Legacy browser XSS filters can mutate otherwise safe markup. Captain
    // relies on its strict CSP and explicit output sanitization instead.
    headers.insert("x-xss-protection", "0".parse().unwrap());
    // Browser scripts are immutable same-origin assets. Inline style remains
    // necessary for bounded dynamic layout values emitted by the UI runtime.
    headers.insert(
        "content-security-policy",
        CONTENT_SECURITY_POLICY.parse().unwrap(),
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
    use tower::ServiceExt;

    #[test]
    fn test_request_id_header_constant() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }

    #[test]
    fn browser_csp_forbids_dynamic_or_inline_script_authority() {
        let script_src = CONTENT_SECURITY_POLICY
            .split(';')
            .map(str::trim)
            .find(|directive| directive.starts_with("script-src "))
            .expect("script-src directive must exist");

        assert_eq!(script_src, "script-src 'self'");
        assert!(!CONTENT_SECURITY_POLICY.contains("'unsafe-eval'"));
        assert!(!script_src.contains("'unsafe-inline'"));
        assert!(CONTENT_SECURITY_POLICY.contains("script-src-attr 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("object-src 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("base-uri 'none'"));
        assert!(CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"));
    }

    #[tokio::test]
    async fn security_header_middleware_emits_the_reviewed_csp() {
        let app = axum::Router::new()
            .route("/", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(security_headers));
        let response = app
            .oneshot(
                axum::http::Request::get("/")
                    .body(axum::body::Body::empty())
                    .expect("request should be valid"),
            )
            .await
            .expect("middleware response should succeed");

        assert_eq!(
            response.headers()["content-security-policy"],
            CONTENT_SECURITY_POLICY
        );
        assert_eq!(response.headers()["x-xss-protection"], "0");
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
    fn request_credentials_never_treat_query_strings_as_credentials() {
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
                session_secret: captain_types::config::generate_session_secret().unwrap(),
                session_epoch: 0,
                session_ttl_hours: 1,
                ..Default::default()
            },
        };
        let request = Request::builder()
            .uri("/api/commands")
            .header("authorization", "Bearer ")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot, false),
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
                session_secret: captain_types::config::generate_session_secret().unwrap(),
                session_epoch: 0,
                session_ttl_hours: 1,
                ..Default::default()
            },
        };
        let token = crate::session_auth::create_session_token(
            "admin",
            &snapshot.session_secret().unwrap(),
            1,
            snapshot.auth.session_epoch,
        );
        let request = Request::builder()
            .uri("/api/commands")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot, false),
            AuthDecision::Allow
        );
    }

    #[test]
    fn query_string_token_never_authorizes_a_protected_route() {
        let snapshot = crate::session_auth::WebAuthSnapshot {
            api_key: "query-token".to_string(),
            auth: captain_types::config::AuthConfig::default(),
        };
        let request = Request::builder()
            .uri("/api/commands?token=query-token")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot, false),
            AuthDecision::Deny("Missing Authorization: Bearer <api_key> header")
        );
    }

    #[test]
    fn unconfigured_auth_fails_closed_by_default() {
        let snapshot = crate::session_auth::WebAuthSnapshot {
            api_key: String::new(),
            auth: captain_types::config::AuthConfig::default(),
        };
        let request = Request::builder()
            .uri("/api/commands")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot, true),
            AuthDecision::Deny("Authentication is not configured; run `captain setup`")
        );
    }

    #[test]
    fn explicit_unauthenticated_mode_is_loopback_only() {
        let snapshot = crate::session_auth::WebAuthSnapshot {
            api_key: String::new(),
            auth: captain_types::config::AuthConfig {
                allow_unauthenticated_loopback: true,
                ..Default::default()
            },
        };
        let request = Request::builder()
            .uri("/api/commands")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            authorize_request(&request, &snapshot, true),
            AuthDecision::Allow
        );
        assert_eq!(
            authorize_request(&request, &snapshot, false),
            AuthDecision::Deny("Unauthenticated access is restricted to direct loopback clients")
        );
    }
}

#[cfg(test)]
#[path = "middleware_auth_matrix_tests.rs"]
mod middleware_auth_matrix_tests;
