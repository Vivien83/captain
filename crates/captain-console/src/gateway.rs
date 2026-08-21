//! Ephemeral loopback gateway for one immutable Captain Console profile.
//!
//! The WebView never receives a Hub bearer. It talks to this one-process local
//! origin, which attaches the short-lived paired credential through
//! `ClientAccessTransport` and relays only the reviewed Desktop work surface.

use crate::{secret_support::resolve_proxy_password, ConsoleProfileCatalog, ConsoleProfileError};
use axum::{
    body::{to_bytes, Body},
    extract::{ws::WebSocketUpgrade, State},
    http::{header, HeaderMap, Method, Request, Response, StatusCode},
    routing::any,
    Router,
};
use captain_node::{
    ClientAccessError, ClientAccessTransport, ClientLocalConfigStore, ClientPairingStore,
};
use captain_wire::{client_relay_path_is_canonical, desktop_client_route_allows, ClientHttpMethod};
use futures::{SinkExt, StreamExt};
use rand::{rngs::OsRng, RngCore};
use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use subtle::ConstantTimeEq;
use tokio::sync::watch;
use tracing::{info, warn};
use zeroize::{Zeroize, Zeroizing};

const CLIENT_STATE_DIR: &str = "state";
const LOCAL_SESSION_COOKIE: &str = "captain_desktop_session";
const MAX_LOCAL_REQUEST_BYTES: usize = 12 * 1024 * 1024;
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_URI_BYTES: usize = 8 * 1024;
const LOOPBACK_BIND_ATTEMPTS: u32 = 8;

pub struct GatewayHandle {
    pub port: u16,
    pub paired_profile_loaded: bool,
    pub active_profile_id: Option<String>,
    bootstrap_url: Option<Zeroizing<String>>,
    bootstrap_secret: Arc<Mutex<Option<Zeroizing<String>>>>,
    authority: String,
    shutdown_tx: watch::Sender<bool>,
    server_thread: Option<std::thread::JoinHandle<()>>,
}

impl GatewayHandle {
    pub fn take_bootstrap_url(&mut self) -> Result<String, GatewayError> {
        self.bootstrap_url
            .take()
            .map(|url| url.to_string())
            .ok_or(GatewayError::BootstrapUnavailable)
    }

    pub fn issue_bootstrap_url(&self) -> Result<String, GatewayError> {
        let secret = random_secret()?;
        let url = format!(
            "http://{}/?desktop_ticket={}",
            self.authority,
            secret.as_str()
        );
        let mut slot = self
            .bootstrap_secret
            .lock()
            .map_err(|_| GatewayError::BootstrapUnavailable)?;
        *slot = Some(secret);
        Ok(url)
    }

    pub fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(thread) = self.server_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for GatewayHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

struct GatewayState {
    transport: Option<Arc<ClientAccessTransport>>,
    unavailable_reason: Option<GatewayUnavailableReason>,
    bootstrap_secret: Arc<Mutex<Option<Zeroizing<String>>>>,
    session_secret: Zeroizing<String>,
    authority: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatewayUnavailableReason {
    Unconfigured,
    ProfileUnavailable,
    PairingIncomplete,
    ProxyCredentialUnavailable,
    ConfigurationUnavailable,
}

pub fn start_gateway() -> Result<GatewayHandle, GatewayError> {
    start_gateway_for_profile(None)
}

/// Start a gateway bound to one profile for its entire lifetime.
///
/// Passing a profile ID does not change the registry's active profile. This
/// lets a Console manager keep multiple independent gateways alive without
/// mutating a transport underneath an in-flight request or WebSocket.
pub fn start_gateway_for_profile(profile_id: Option<&str>) -> Result<GatewayHandle, GatewayError> {
    let home = captain_home().ok_or(GatewayError::HomeUnavailable)?;
    start_gateway_at(&home, profile_id)
}

pub(crate) fn start_gateway_at(
    home: &Path,
    profile_id: Option<&str>,
) -> Result<GatewayHandle, GatewayError> {
    let (transport, unavailable_reason, active_profile_id) = load_transport(home, profile_id);
    let paired_profile_loaded = transport.is_some();
    let listener = bind_loopback()?;
    let port = listener
        .local_addr()
        .map_err(|error| GatewayError::BindFailed { kind: error.kind() })?
        .port();
    let authority = format!("127.0.0.1:{port}");
    let bootstrap_secret = random_secret()?;
    let session_secret = random_secret()?;
    let bootstrap_url = Zeroizing::new(format!(
        "http://{authority}/?desktop_ticket={}",
        bootstrap_secret.as_str()
    ));
    let bootstrap_secret = Arc::new(Mutex::new(Some(bootstrap_secret)));
    let state = Arc::new(GatewayState {
        transport,
        unavailable_reason,
        bootstrap_secret: Arc::clone(&bootstrap_secret),
        session_secret,
        authority: authority.clone(),
    });
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let server_thread = std::thread::Builder::new()
        .name("captain-console-gateway".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    warn!(error = %error, "Desktop gateway runtime unavailable");
                    return;
                }
            };
            runtime.block_on(async move {
                if listener.set_nonblocking(true).is_err() {
                    warn!("Desktop gateway listener could not become nonblocking");
                    return;
                }
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(error) => {
                        warn!(error = %error, "Desktop gateway listener conversion failed");
                        return;
                    }
                };
                let app = Router::new()
                    .route("/", any(gateway_request))
                    .route("/{*path}", any(gateway_request))
                    .with_state(state);
                let server = axum::serve(listener, app).with_graceful_shutdown(async move {
                    let _ = shutdown_rx.wait_for(|shutdown| *shutdown).await;
                });
                if let Err(error) = server.await {
                    warn!(error = %error, "Desktop gateway stopped unexpectedly");
                }
            });
        })
        .map_err(|_| GatewayError::RuntimeUnavailable)?;

    info!(port, paired_profile_loaded, "Desktop Client gateway ready");
    Ok(GatewayHandle {
        port,
        paired_profile_loaded,
        active_profile_id,
        bootstrap_url: Some(bootstrap_url),
        bootstrap_secret,
        authority,
        shutdown_tx,
        server_thread: Some(server_thread),
    })
}

fn bind_loopback() -> Result<TcpListener, GatewayError> {
    for attempt in 1..=LOOPBACK_BIND_ATTEMPTS {
        match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => return Ok(listener),
            Err(error) if attempt < LOOPBACK_BIND_ATTEMPTS => {
                warn!(
                    attempt,
                    error_kind = ?error.kind(),
                    "Console loopback bind will retry"
                );
                std::thread::sleep(std::time::Duration::from_millis(u64::from(attempt) * 10));
            }
            Err(error) => {
                let kind = error.kind();
                warn!(
                    attempts = LOOPBACK_BIND_ATTEMPTS,
                    error_kind = ?kind,
                    "Console loopback bind failed"
                );
                return Err(GatewayError::BindFailed { kind });
            }
        }
    }
    Err(GatewayError::BindFailed {
        kind: std::io::ErrorKind::Other,
    })
}

async fn gateway_request(
    State(state): State<Arc<GatewayState>>,
    websocket: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
    request: Request<Body>,
) -> Response<Body> {
    if !valid_host(request.headers(), &state.authority) {
        return local_error(StatusCode::MISDIRECTED_REQUEST, "invalid_local_origin");
    }
    if is_bootstrap_request(&request) {
        return bootstrap_response(&state, request.uri().query());
    }
    if !session_cookie_matches(request.headers(), state.session_secret.as_str()) {
        return local_error(StatusCode::UNAUTHORIZED, "desktop_session_required");
    }
    if !valid_browser_provenance(&request, &state.authority) {
        return local_error(StatusCode::FORBIDDEN, "desktop_origin_rejected");
    }

    let method = request.method().clone();
    let path = request.uri().path().to_string();
    if request.uri().to_string().len() > MAX_URI_BYTES {
        return local_error(StatusCode::URI_TOO_LONG, "desktop_uri_too_long");
    }
    let Some(transport) = state.transport.as_ref().cloned() else {
        return unavailable_response(state.unavailable_reason, path == "/");
    };

    if is_static_get(&method, &path) {
        return proxy_http(transport, request).await;
    }
    let Some(client_method) = client_method(&method) else {
        return local_error(StatusCode::METHOD_NOT_ALLOWED, "desktop_method_forbidden");
    };
    if !desktop_client_route_allows(client_method, &path) {
        return local_error(StatusCode::FORBIDDEN, "desktop_route_forbidden");
    }
    if is_websocket_request(request.headers()) {
        let Ok(websocket) = websocket else {
            return local_error(StatusCode::BAD_REQUEST, "desktop_websocket_invalid");
        };
        return proxy_websocket(transport, request, websocket).await;
    }
    proxy_http(transport, request).await
}

async fn proxy_http(
    transport: Arc<ClientAccessTransport>,
    request: Request<Body>,
) -> Response<Body> {
    let method = request.method().clone();
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let headers = upstream_request_headers(request.headers());
    let body = match to_bytes(request.into_body(), MAX_LOCAL_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => return local_error(StatusCode::PAYLOAD_TOO_LARGE, "desktop_body_too_large"),
    };
    let response = match transport
        .execute(method, &path_and_query, &headers, body)
        .await
    {
        Ok(response) => response,
        Err(error) if path_and_query == "/" => return hub_unavailable_page(error),
        Err(error) if path_and_query == "/api/auth/check" => {
            return client_auth_unavailable_response(error);
        }
        Err(error) => return transport_error(error),
    };
    if path_and_query == "/api/auth/check" && response.status() == StatusCode::UNAUTHORIZED {
        return client_auth_unavailable_response(ClientAccessError::PairingRejected);
    }
    if response.status().is_redirection() {
        return local_error(StatusCode::BAD_GATEWAY, "hub_redirect_refused");
    }
    upstream_response(response)
}

async fn proxy_websocket(
    transport: Arc<ClientAccessTransport>,
    request: Request<Body>,
    websocket: WebSocketUpgrade,
) -> Response<Body> {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let upstream = match transport
        .open_websocket(&path_and_query, MAX_WEBSOCKET_MESSAGE_BYTES)
        .await
    {
        Ok(upstream) => upstream,
        Err(error) => return transport_error(error),
    };
    websocket
        .max_message_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .max_frame_size(MAX_WEBSOCKET_MESSAGE_BYTES)
        .on_upgrade(move |local| bridge_websocket(local, upstream))
}

async fn bridge_websocket(
    local: axum::extract::ws::WebSocket,
    upstream: reqwest_websocket::WebSocket,
) {
    let (mut local_sink, mut local_stream) = local.split();
    let (mut upstream_sink, mut upstream_stream) = upstream.split();
    loop {
        tokio::select! {
            local = local_stream.next() => match local {
                Some(Ok(message)) => {
                    let Some(message) = local_to_upstream(message) else { break; };
                    if upstream_sink.send(message).await.is_err() { break; }
                }
                _ => break,
            },
            upstream = upstream_stream.next() => match upstream {
                Some(Ok(message)) => {
                    let Some(message) = upstream_to_local(message) else { break; };
                    if local_sink.send(message).await.is_err() { break; }
                }
                _ => break,
            }
        }
    }
}

fn local_to_upstream(message: axum::extract::ws::Message) -> Option<reqwest_websocket::Message> {
    match message {
        axum::extract::ws::Message::Text(text) => {
            Some(reqwest_websocket::Message::Text(text.to_string()))
        }
        axum::extract::ws::Message::Binary(bytes) => {
            Some(reqwest_websocket::Message::Binary(bytes.to_vec()))
        }
        axum::extract::ws::Message::Ping(bytes) => {
            Some(reqwest_websocket::Message::Ping(bytes.to_vec()))
        }
        axum::extract::ws::Message::Pong(bytes) => {
            Some(reqwest_websocket::Message::Pong(bytes.to_vec()))
        }
        axum::extract::ws::Message::Close(_) => None,
    }
}

fn upstream_to_local(message: reqwest_websocket::Message) -> Option<axum::extract::ws::Message> {
    match message {
        reqwest_websocket::Message::Text(text) => {
            Some(axum::extract::ws::Message::Text(text.into()))
        }
        reqwest_websocket::Message::Binary(bytes) => {
            Some(axum::extract::ws::Message::Binary(bytes.into()))
        }
        reqwest_websocket::Message::Ping(bytes) => {
            Some(axum::extract::ws::Message::Ping(bytes.into()))
        }
        reqwest_websocket::Message::Pong(bytes) => {
            Some(axum::extract::ws::Message::Pong(bytes.into()))
        }
        reqwest_websocket::Message::Close { .. } => None,
    }
}

fn upstream_response(response: reqwest::Response) -> Response<Body> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_DISPOSITION,
        header::CACHE_CONTROL,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::CONTENT_SECURITY_POLICY,
        header::X_CONTENT_TYPE_OPTIONS,
        header::REFERRER_POLICY,
    ] {
        if let Some(value) = headers.get(&name) {
            builder = builder.header(name, value);
        }
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|_| local_error(StatusCode::BAD_GATEWAY, "hub_response_invalid"))
}

fn upstream_request_headers(source: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::ACCEPT,
        header::ACCEPT_LANGUAGE,
        header::RANGE,
        header::IF_NONE_MATCH,
        header::IF_MODIFIED_SINCE,
    ] {
        if let Some(value) = source.get(&name) {
            headers.insert(name, value.clone());
        }
    }
    if let Some(value) = source.get("x-filename") {
        headers.insert("x-filename", value.clone());
    }
    headers
}

fn is_bootstrap_request(request: &Request<Body>) -> bool {
    request.method() == Method::GET
        && request.uri().path() == "/"
        && request
            .uri()
            .query()
            .is_some_and(|query| query.starts_with("desktop_ticket=") && !query.contains('&'))
}

fn bootstrap_response(state: &GatewayState, query: Option<&str>) -> Response<Body> {
    let candidate = query
        .and_then(|query| query.strip_prefix("desktop_ticket="))
        .unwrap_or_default();
    let mut bootstrap = state
        .bootstrap_secret
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let accepted = bootstrap
        .as_ref()
        .is_some_and(|secret| constant_time_eq(secret.as_str(), candidate));
    if !accepted {
        return local_error(StatusCode::UNAUTHORIZED, "desktop_bootstrap_rejected");
    }
    bootstrap.take();
    let cookie = format!(
        "{LOCAL_SESSION_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
        state.session_secret.as_str()
    );
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .header(header::SET_COOKIE, cookie)
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::REFERRER_POLICY, "no-referrer")
        .body(Body::empty())
        .unwrap_or_else(|_| {
            local_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "desktop_bootstrap_failed",
            )
        })
}

fn valid_host(headers: &HeaderMap, authority: &str) -> bool {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host.eq_ignore_ascii_case(authority))
}

fn session_cookie_matches(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (name, value) = cookie.trim().split_once('=')?;
                (name == LOCAL_SESSION_COOKIE).then_some(value)
            })
        })
        .is_some_and(|candidate| constant_time_eq(expected, candidate))
}

fn valid_browser_provenance(request: &Request<Body>, authority: &str) -> bool {
    if let Some(site) = request
        .headers()
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
    {
        if site != "same-origin" && site != "none" {
            return false;
        }
    }
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        return origin == format!("http://{authority}");
    }
    if matches!(*request.method(), Method::GET | Method::HEAD) {
        return true;
    }
    false
}

fn is_static_get(method: &Method, path: &str) -> bool {
    method == Method::GET
        && client_relay_path_is_canonical(path)
        && (matches!(path, "/" | "/favicon.ico" | "/manifest.json" | "/logo.svg")
            || path == "/assets/logo.png"
            || path.starts_with("/assets/app/"))
}

fn client_method(method: &Method) -> Option<ClientHttpMethod> {
    match *method {
        Method::GET => Some(ClientHttpMethod::Get),
        Method::POST => Some(ClientHttpMethod::Post),
        Method::PUT => Some(ClientHttpMethod::Put),
        Method::PATCH => Some(ClientHttpMethod::Patch),
        Method::DELETE => Some(ClientHttpMethod::Delete),
        _ => None,
    }
}

fn is_websocket_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

fn unavailable_response(reason: Option<GatewayUnavailableReason>, html: bool) -> Response<Body> {
    if !html {
        return local_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "desktop_client_unavailable",
        );
    }
    let message = match reason.unwrap_or(GatewayUnavailableReason::ConfigurationUnavailable) {
        GatewayUnavailableReason::Unconfigured => {
            "No paired Client profile was found. Run `captain client pair --hub https://your-hub.example` first."
        }
        GatewayUnavailableReason::ProfileUnavailable => {
            "The requested Captain profile is unavailable. Select an existing profile before opening it."
        }
        GatewayUnavailableReason::PairingIncomplete => {
            "Client pairing is incomplete. Run `captain client status`, then resume the pairing command."
        }
        GatewayUnavailableReason::ProxyCredentialUnavailable => {
            "The configured proxy credential is unavailable. Restore it in Captain's secret store."
        }
        GatewayUnavailableReason::ConfigurationUnavailable => {
            "The local Client profile is unreadable. Run `captain client status` for a safe diagnosis."
        }
    };
    local_html(StatusCode::SERVICE_UNAVAILABLE, message)
}

fn hub_unavailable_page(error: ClientAccessError) -> Response<Body> {
    let message = match error {
        ClientAccessError::PairingRejected
        | ClientAccessError::PairingUnavailable
        | ClientAccessError::TokenUnavailable => {
            "This Client pairing is no longer accepted. Run `captain client status`, then pair again if it was revoked."
        }
        _ => {
            "The paired Hub cannot be reached through the configured network path. Check connectivity, proxy and enterprise CA settings with `captain client status`."
        }
    };
    local_html(StatusCode::SERVICE_UNAVAILABLE, message)
}

fn client_auth_unavailable_response(error: ClientAccessError) -> Response<Body> {
    let code = match error {
        ClientAccessError::PairingRejected
        | ClientAccessError::PairingUnavailable
        | ClientAccessError::TokenUnavailable => "desktop_pairing_required",
        _ => "desktop_hub_unavailable",
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(
            serde_json::json!({
                "mode": "client",
                "authenticated": false,
                "error": code,
            })
            .to_string(),
        ))
        .unwrap_or_default()
}

fn local_html(status: StatusCode, message: &str) -> Response<Body> {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Captain Client</title></head><body><main><h1>Captain Client</h1><p>{message}</p><p><a href=\"/\">Retry</a></p></main></body></html>"
    );
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'",
        )
        .body(Body::from(body))
        .unwrap_or_default()
}

fn transport_error(error: ClientAccessError) -> Response<Body> {
    let (status, code) = match error {
        ClientAccessError::PairingRejected => {
            (StatusCode::UNAUTHORIZED, "desktop_pairing_rejected")
        }
        ClientAccessError::PairingUnavailable | ClientAccessError::TokenUnavailable => {
            (StatusCode::UNAUTHORIZED, "desktop_pairing_required")
        }
        ClientAccessError::InvalidPath => (StatusCode::BAD_REQUEST, "desktop_path_invalid"),
        ClientAccessError::TransportUnavailable
        | ClientAccessError::HubUnavailable
        | ClientAccessError::ClockUnavailable => {
            (StatusCode::BAD_GATEWAY, "desktop_hub_unavailable")
        }
    };
    local_error(status, code)
}

fn local_error(status: StatusCode, code: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(
            serde_json::json!({ "error": code, "code": code }).to_string(),
        ))
        .unwrap_or_default()
}

pub(crate) fn load_transport(
    home: &Path,
    requested_profile_id: Option<&str>,
) -> (
    Option<Arc<ClientAccessTransport>>,
    Option<GatewayUnavailableReason>,
    Option<String>,
) {
    let selected = match select_profile_root(home, requested_profile_id) {
        Ok(selected) => selected,
        Err(reason) => return (None, Some(reason), None),
    };
    let Some((selected_profile_id, root)) = selected else {
        return (None, Some(GatewayUnavailableReason::Unconfigured), None);
    };
    let credential_reference = selected_profile_id.clone();
    let profile_id = Some(selected_profile_id);
    if !root.join("config.toml").is_file() {
        return (
            None,
            Some(GatewayUnavailableReason::ConfigurationUnavailable),
            profile_id,
        );
    }
    let store = match ClientLocalConfigStore::open(&root) {
        Ok(store) => store,
        Err(_) => {
            return (
                None,
                Some(GatewayUnavailableReason::ConfigurationUnavailable),
                profile_id,
            )
        }
    };
    let config = match store.load() {
        Ok(Some(config)) => config,
        _ => {
            return (
                None,
                Some(GatewayUnavailableReason::ConfigurationUnavailable),
                profile_id,
            )
        }
    };
    let proxy_password = match resolve_proxy_password(&config.network.proxy, home) {
        Ok(password) => password,
        Err(_) => {
            return (
                None,
                Some(GatewayUnavailableReason::ProxyCredentialUnavailable),
                profile_id,
            )
        }
    };
    let pairing =
        match ClientPairingStore::open(store.root().join(CLIENT_STATE_DIR), credential_reference) {
            Ok(pairing) => pairing,
            Err(_) => {
                return (
                    None,
                    Some(GatewayUnavailableReason::PairingIncomplete),
                    profile_id,
                )
            }
        };
    match ClientAccessTransport::open(config, proxy_password, pairing) {
        Ok(transport) => (Some(Arc::new(transport)), None, profile_id),
        Err(_) => (
            None,
            Some(GatewayUnavailableReason::PairingIncomplete),
            profile_id,
        ),
    }
}

fn select_profile_root(
    home: &Path,
    requested_profile_id: Option<&str>,
) -> Result<Option<(String, PathBuf)>, GatewayUnavailableReason> {
    ConsoleProfileCatalog::open(home)
        .map_err(|_| GatewayUnavailableReason::ConfigurationUnavailable)?
        .exact_profile_root(requested_profile_id)
        .map_err(|error| match error {
            ConsoleProfileError::ProfileNotFound => GatewayUnavailableReason::ProfileUnavailable,
            _ => GatewayUnavailableReason::ConfigurationUnavailable,
        })
}

fn captain_home() -> Option<PathBuf> {
    std::env::var_os("CAPTAIN_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".captain")))
}

fn random_secret() -> Result<Zeroizing<String>, GatewayError> {
    let mut raw = [0_u8; 32];
    OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| GatewayError::RandomUnavailable)?;
    let secret = Zeroizing::new(hex::encode(raw));
    raw.zeroize();
    Ok(secret)
}

fn constant_time_eq(expected: &str, candidate: &str) -> bool {
    expected.len() == candidate.len() && bool::from(expected.as_bytes().ct_eq(candidate.as_bytes()))
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum GatewayError {
    #[error("the Captain Console home directory is unavailable")]
    HomeUnavailable,
    #[error("the Console loopback gateway could not bind ({kind:?})")]
    BindFailed { kind: std::io::ErrorKind },
    #[error("the Console loopback gateway runtime is unavailable")]
    RuntimeUnavailable,
    #[error("the Console bootstrap secret is unavailable")]
    BootstrapUnavailable,
    #[error("the OS random source is unavailable")]
    RandomUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_node::ClientProfileRegistry;

    fn request(method: Method, authority: &str, origin: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri("/api/status")
            .header(header::HOST, authority)
            .header(header::COOKIE, format!("{LOCAL_SESSION_COOKIE}=session"));
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn unsafe_requests_require_the_exact_loopback_origin() {
        let authority = "127.0.0.1:49152";
        assert!(valid_browser_provenance(
            &request(Method::POST, authority, Some("http://127.0.0.1:49152")),
            authority,
        ));
        assert!(!valid_browser_provenance(
            &request(Method::POST, authority, Some("https://evil.example")),
            authority,
        ));
        assert!(!valid_browser_provenance(
            &request(Method::POST, authority, None),
            authority,
        ));
    }

    #[test]
    fn cross_site_fetch_metadata_is_rejected_even_for_safe_methods() {
        let authority = "127.0.0.1:49152";
        let mut request = request(Method::GET, authority, None);
        request.headers_mut().insert(
            "sec-fetch-site",
            axum::http::HeaderValue::from_static("cross-site"),
        );
        assert!(!valid_browser_provenance(&request, authority));
    }

    #[test]
    fn websocket_style_get_with_an_explicit_foreign_origin_is_rejected() {
        let authority = "127.0.0.1:49152";
        assert!(!valid_browser_provenance(
            &request(Method::GET, authority, Some("https://evil.example")),
            authority,
        ));
        assert!(valid_browser_provenance(
            &request(Method::GET, authority, Some("http://127.0.0.1:49152")),
            authority,
        ));
    }

    #[test]
    fn local_session_cookie_is_exact_and_constant_time_compared() {
        let request = request(Method::GET, "127.0.0.1:49152", None);
        assert!(session_cookie_matches(request.headers(), "session"));
        assert!(!session_cookie_matches(request.headers(), "other"));
    }

    #[test]
    fn static_relay_is_read_only_and_bounded() {
        assert!(is_static_get(&Method::GET, "/"));
        assert!(is_static_get(&Method::GET, "/assets/app/main.js"));
        assert!(!is_static_get(&Method::GET, "/sw.js"));
        assert!(!is_static_get(&Method::POST, "/"));
        assert!(!is_static_get(&Method::GET, "/terminal"));
        assert!(!is_static_get(&Method::GET, "/config"));
        assert!(!is_static_get(&Method::GET, "/assets/app/../api/status"));
        assert!(!is_static_get(
            &Method::GET,
            "/assets/app/%2e%2e/api/status"
        ));
        assert!(!is_static_get(&Method::GET, "/assets/app//main.js"));
    }

    #[test]
    fn an_explicit_gateway_profile_never_mutates_the_active_authority() {
        let home = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(home.path().join("console")).unwrap();
        let active = registry.create_profile(10).unwrap();
        let second = registry.create_profile(20).unwrap();

        let (selected_id, selected_root) = select_profile_root(home.path(), Some(&second.id))
            .unwrap()
            .unwrap();

        assert_eq!(selected_id, second.id);
        assert_eq!(selected_root, registry.profile_root(&second.id).unwrap());
        assert_eq!(registry.active_profile().unwrap().unwrap().id, active.id);
    }

    #[test]
    fn a_missing_gateway_profile_fails_closed_without_changing_authority() {
        let home = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(home.path().join("console")).unwrap();
        let active = registry.create_profile(10).unwrap();

        assert_eq!(
            select_profile_root(home.path(), Some("00000000-0000-0000-0000-000000000000")),
            Err(GatewayUnavailableReason::ProfileUnavailable)
        );
        assert_eq!(registry.active_profile().unwrap().unwrap().id, active.id);
    }
}
