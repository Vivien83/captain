//! Shared HTTPS stack for the Node WebSocket and HTTP fallback transports.

mod transport;

pub use transport::{NodeHttpStream, NodeWebSocket};

use captain_wire::{
    HUB_NODE_CLOSE_PATH, HUB_NODE_CONNECT_PATH, HUB_NODE_ENVELOPE_PATH, HUB_NODE_PULL_PATH,
    HUB_NODE_STREAM_PATH, HUB_NODE_WEBSOCKET_PATH, MAX_HUB_NODE_FRAME_BYTES,
};
use reqwest::header::HeaderMap;
use reqwest::{Certificate, Client, NoProxy, Proxy};
use reqwest_websocket::RequestBuilderExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, fs, path::PathBuf, time::Duration};
use thiserror::Error;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use url::Url;
use zeroize::Zeroizing;

const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 45;
const MAX_CONNECT_TIMEOUT_SECS: u64 = 120;
const MAX_REQUEST_TIMEOUT_SECS: u64 = 180;
const MAX_ENTERPRISE_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeNetworkConfig {
    pub hub_url: String,
    #[serde(default)]
    pub proxy: NodeProxyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_ca_bundle: Option<PathBuf>,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
}

impl fmt::Debug for NodeNetworkConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeNetworkConfig")
            .field("hub_url", &"[REDACTED]")
            .field("proxy", &self.proxy)
            .field(
                "enterprise_ca_bundle_configured",
                &self.enterprise_ca_bundle.is_some(),
            )
            .field("connect_timeout_secs", &self.connect_timeout_secs)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .finish()
    }
}

impl NodeNetworkConfig {
    pub fn new(hub_url: impl Into<String>) -> Self {
        Self {
            hub_url: hub_url.into(),
            proxy: NodeProxyMode::default(),
            enterprise_ca_bundle: None,
            connect_timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
        }
    }

    pub fn build_client(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
    ) -> Result<NodeHttpClient, NodeNetworkError> {
        self.build_client_with_policy(proxy_password, false)
    }

    /// Build a blocking client with the same Hub, proxy and enterprise CA
    /// policy as the outbound Node transport.
    pub fn build_blocking_client(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
    ) -> Result<reqwest::blocking::Client, NodeNetworkError> {
        self.build_blocking_client_with_policy(
            proxy_password,
            false,
            Some(Duration::from_secs(self.request_timeout_secs)),
        )
    }

    /// Build a blocking Client transport with an operation-specific timeout.
    /// `None` is reserved for long-lived streams such as SSE; connection
    /// timeout, HTTPS, proxy and enterprise CA policy remain unchanged.
    pub fn build_blocking_client_with_timeout(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
        request_timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Client, NodeNetworkError> {
        self.build_blocking_client_with_policy(proxy_password, false, request_timeout)
    }

    /// Build the same blocking transport with caller-owned default headers.
    /// This is used by lightweight Clients to attach their short-lived bearer
    /// without weakening the shared network policy.
    pub fn build_blocking_client_with_headers(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
        request_timeout: Option<Duration>,
        headers: HeaderMap,
    ) -> Result<reqwest::blocking::Client, NodeNetworkError> {
        self.build_blocking_client_with_policy_and_headers(
            proxy_password,
            false,
            request_timeout,
            Some(headers),
        )
    }

    #[cfg(test)]
    pub(crate) fn build_loopback_client(&self) -> Result<NodeHttpClient, NodeNetworkError> {
        self.build_client_with_policy(None, true)
    }

    fn build_client_with_policy(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
        allow_loopback_http: bool,
    ) -> Result<NodeHttpClient, NodeNetworkError> {
        validate_timeouts(self.connect_timeout_secs, self.request_timeout_secs)?;
        let endpoints = HubNodeEndpoints::parse(&self.hub_url, allow_loopback_http)?;
        let mut builder = Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();

        builder = match &self.proxy {
            NodeProxyMode::Disabled => {
                if proxy_password.is_some() {
                    return Err(NodeNetworkError::UnexpectedProxyPassword);
                }
                builder
            }
            NodeProxyMode::Environment => {
                if proxy_password.is_some() {
                    return Err(NodeNetworkError::UnexpectedProxyPassword);
                }
                match environment_proxy()? {
                    Some(proxy) => builder.proxy(proxy),
                    None => builder,
                }
            }
            NodeProxyMode::Explicit {
                url,
                username,
                password_secret,
            } => {
                let mut proxy = explicit_proxy(url, username, password_secret, proxy_password)?;
                proxy = proxy.no_proxy(NoProxy::from_env());
                builder.proxy(proxy)
            }
        };

        if let Some(path) = &self.enterprise_ca_bundle {
            for certificate in enterprise_ca_certificates(path)? {
                builder = builder.add_root_certificate(certificate);
            }
        }

        let client = builder
            .build()
            .map_err(|_| NodeNetworkError::ClientBuildFailed)?;
        Ok(NodeHttpClient {
            client,
            endpoints,
            request_timeout: Duration::from_secs(self.request_timeout_secs),
        })
    }

    fn build_blocking_client_with_policy(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
        allow_loopback_http: bool,
        request_timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Client, NodeNetworkError> {
        self.build_blocking_client_with_policy_and_headers(
            proxy_password,
            allow_loopback_http,
            request_timeout,
            None,
        )
    }

    fn build_blocking_client_with_policy_and_headers(
        &self,
        proxy_password: Option<&ResolvedProxyPassword>,
        allow_loopback_http: bool,
        request_timeout: Option<Duration>,
        headers: Option<HeaderMap>,
    ) -> Result<reqwest::blocking::Client, NodeNetworkError> {
        validate_timeouts(self.connect_timeout_secs, self.request_timeout_secs)?;
        if request_timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(NodeNetworkError::InvalidTimeout);
        }
        HubNodeEndpoints::parse(&self.hub_url, allow_loopback_http)?;
        let mut builder = reqwest::blocking::Client::builder()
            .http1_only()
            .connect_timeout(Duration::from_secs(self.connect_timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if let Some(timeout) = request_timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(headers) = headers {
            builder = builder.default_headers(headers);
        }

        builder = match &self.proxy {
            NodeProxyMode::Disabled => {
                if proxy_password.is_some() {
                    return Err(NodeNetworkError::UnexpectedProxyPassword);
                }
                builder
            }
            NodeProxyMode::Environment => {
                if proxy_password.is_some() {
                    return Err(NodeNetworkError::UnexpectedProxyPassword);
                }
                match environment_proxy()? {
                    Some(proxy) => builder.proxy(proxy),
                    None => builder,
                }
            }
            NodeProxyMode::Explicit {
                url,
                username,
                password_secret,
            } => {
                let proxy = explicit_proxy(url, username, password_secret, proxy_password)?
                    .no_proxy(NoProxy::from_env());
                builder.proxy(proxy)
            }
        };

        if let Some(path) = &self.enterprise_ca_bundle {
            for certificate in enterprise_ca_certificates(path)? {
                builder = builder.add_root_certificate(certificate);
            }
        }

        builder
            .build()
            .map_err(|_| NodeNetworkError::ClientBuildFailed)
    }
}

fn default_connect_timeout_secs() -> u64 {
    DEFAULT_CONNECT_TIMEOUT_SECS
}

fn default_request_timeout_secs() -> u64 {
    DEFAULT_REQUEST_TIMEOUT_SECS
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NodeProxyMode {
    #[default]
    Environment,
    Disabled,
    Explicit {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_secret: Option<String>,
    },
}

impl fmt::Debug for NodeProxyMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment => formatter.write_str("Environment"),
            Self::Disabled => formatter.write_str("Disabled"),
            Self::Explicit {
                username,
                password_secret,
                ..
            } => formatter
                .debug_struct("Explicit")
                .field("url", &"[REDACTED]")
                .field("username_configured", &username.is_some())
                .field("password_secret_configured", &password_secret.is_some())
                .finish(),
        }
    }
}

pub struct ResolvedProxyPassword {
    secret_name: String,
    password: Zeroizing<String>,
}

impl ResolvedProxyPassword {
    pub fn new(secret_name: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            secret_name: secret_name.into(),
            password: Zeroizing::new(password.into()),
        }
    }
}

impl fmt::Debug for ResolvedProxyPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedProxyPassword")
            .field("secret_name", &self.secret_name)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HubNodeEndpoints {
    pub connect: Url,
    pub envelope: Url,
    pub pull: Url,
    pub stream: Url,
    pub websocket: Url,
    pub close: Url,
}

impl fmt::Debug for HubNodeEndpoints {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeEndpoints")
            .field("connect", &HUB_NODE_CONNECT_PATH)
            .field("envelope", &HUB_NODE_ENVELOPE_PATH)
            .field("pull", &HUB_NODE_PULL_PATH)
            .field("stream", &HUB_NODE_STREAM_PATH)
            .field("websocket", &HUB_NODE_WEBSOCKET_PATH)
            .field("close", &HUB_NODE_CLOSE_PATH)
            .finish()
    }
}

impl HubNodeEndpoints {
    fn parse(raw_hub_url: &str, allow_loopback_http: bool) -> Result<Self, NodeNetworkError> {
        let mut origin = Url::parse(raw_hub_url).map_err(|_| NodeNetworkError::InvalidHubUrl)?;
        if !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.cannot_be_a_base()
        {
            return Err(NodeNetworkError::InvalidHubUrl);
        }
        let host = origin.host_str().ok_or(NodeNetworkError::InvalidHubUrl)?;
        let loopback_http = allow_loopback_http
            && origin.scheme() == "http"
            && (host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback()));
        if !loopback_http {
            if origin.scheme() != "https" {
                return Err(NodeNetworkError::HttpsRequired);
            }
            if origin.port_or_known_default() != Some(443) {
                return Err(NodeNetworkError::HttpsPortRequired);
            }
        }
        if origin.path() != "/" && !origin.path().is_empty() {
            return Err(NodeNetworkError::HubBasePathUnsupported);
        }
        origin.set_path("/");

        let connect = endpoint(&origin, HUB_NODE_CONNECT_PATH)?;
        let envelope = endpoint(&origin, HUB_NODE_ENVELOPE_PATH)?;
        let pull = endpoint(&origin, HUB_NODE_PULL_PATH)?;
        let stream = endpoint(&origin, HUB_NODE_STREAM_PATH)?;
        let mut websocket = endpoint(&origin, HUB_NODE_WEBSOCKET_PATH)?;
        let websocket_scheme = if origin.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        websocket
            .set_scheme(websocket_scheme)
            .map_err(|_| NodeNetworkError::InvalidHubUrl)?;
        let close = endpoint(&origin, HUB_NODE_CLOSE_PATH)?;
        Ok(Self {
            connect,
            envelope,
            pull,
            stream,
            websocket,
            close,
        })
    }
}

#[derive(Clone)]
pub struct NodeHttpClient {
    pub(crate) client: Client,
    pub(crate) endpoints: HubNodeEndpoints,
    pub(crate) request_timeout: Duration,
}

impl NodeHttpClient {
    pub fn endpoints(&self) -> &HubNodeEndpoints {
        &self.endpoints
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub(crate) fn hub_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(
            self.endpoints
                .connect
                .origin()
                .ascii_serialization()
                .as_bytes(),
        );
        hex::encode(hasher.finalize())
    }

    pub async fn open_websocket(
        &self,
        access_token: &str,
    ) -> Result<reqwest_websocket::WebSocket, NodeNetworkError> {
        if !valid_access_token_shape(access_token) {
            return Err(NodeNetworkError::InvalidAccessToken);
        }
        let websocket_config = WebSocketConfig {
            max_message_size: Some(MAX_HUB_NODE_FRAME_BYTES),
            max_frame_size: Some(MAX_HUB_NODE_FRAME_BYTES),
            max_write_buffer_size: MAX_HUB_NODE_FRAME_BYTES * 2,
            ..WebSocketConfig::default()
        };
        let response = tokio::time::timeout(
            self.request_timeout,
            self.client
                .get(self.endpoints.websocket.clone())
                .bearer_auth(access_token)
                .upgrade()
                .web_socket_config(websocket_config)
                .send(),
        )
        .await
        .map_err(|_| NodeNetworkError::WebSocketUpgradeFailed)?
        .map_err(|_| NodeNetworkError::WebSocketUpgradeFailed)?;
        response
            .into_websocket()
            .await
            .map_err(|_| NodeNetworkError::WebSocketUpgradeFailed)
    }
}

impl fmt::Debug for NodeHttpClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeHttpClient")
            .field("endpoints", &self.endpoints)
            .field("request_timeout", &self.request_timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NodeNetworkError {
    #[error("Hub URL is invalid")]
    InvalidHubUrl,
    #[error("Hub URL must use HTTPS")]
    HttpsRequired,
    #[error("Hub URL must use HTTPS port 443")]
    HttpsPortRequired,
    #[error("Hub URL cannot contain a base path")]
    HubBasePathUnsupported,
    #[error("Node network timeout is invalid")]
    InvalidTimeout,
    #[error("proxy URL is invalid")]
    InvalidProxyUrl,
    #[error("proxy credentials must come from Captain's secret store")]
    ProxyCredentialsInUrl,
    #[error("proxy username and password secret must be configured together")]
    IncompleteProxyCredentials,
    #[error("the configured proxy password secret was not resolved")]
    ProxyPasswordRequired,
    #[error("a proxy password was supplied but the selected proxy mode does not use it")]
    UnexpectedProxyPassword,
    #[error("enterprise CA bundle is unreadable")]
    CaBundleUnreadable,
    #[error("enterprise CA bundle size is invalid")]
    CaBundleInvalidSize,
    #[error("enterprise CA bundle is not valid PEM")]
    CaBundleInvalid,
    #[error("Node HTTP client could not be built")]
    ClientBuildFailed,
    #[error("device access token is invalid")]
    InvalidAccessToken,
    #[error("Hub WebSocket upgrade failed")]
    WebSocketUpgradeFailed,
    #[error("Hub Node request timed out")]
    RequestTimedOut,
    #[error("Hub Node network is unavailable")]
    NetworkUnavailable,
    #[error("Hub Node transport closed")]
    TransportClosed,
    #[error("Hub Node response is too large")]
    HubResponseTooLarge,
    #[error("Hub Node response is invalid")]
    InvalidHubResponse,
    #[error("Hub rejected the device access token")]
    HubAuthenticationFailed,
    #[error("Hub Node transport is disabled")]
    HubTransportDisabled,
    #[error("Hub Node durable state conflicts with the request")]
    HubStateConflict,
    #[error("Hub Node receive transport is busy; retry after {retry_after_secs}s")]
    HubTransportBusy { retry_after_secs: u64 },
    #[error("Hub Node transport is temporarily unavailable")]
    HubUnavailable,
    #[error("Hub rejected the Node request with HTTP {status} ({code})")]
    HubRejected { status: u16, code: String },
}

fn validate_timeouts(connect: u64, request: u64) -> Result<(), NodeNetworkError> {
    if connect == 0
        || connect > MAX_CONNECT_TIMEOUT_SECS
        || request < connect
        || request > MAX_REQUEST_TIMEOUT_SECS
    {
        return Err(NodeNetworkError::InvalidTimeout);
    }
    Ok(())
}

fn endpoint(origin: &Url, path: &str) -> Result<Url, NodeNetworkError> {
    origin
        .join(path.trim_start_matches('/'))
        .map_err(|_| NodeNetworkError::InvalidHubUrl)
}

fn environment_proxy() -> Result<Option<Proxy>, NodeNetworkError> {
    let raw = environment_proxy_url(|key| std::env::var(key).ok());
    raw.map(|url| {
        Proxy::all(url)
            .map(|proxy| proxy.no_proxy(NoProxy::from_env()))
            .map_err(|_| NodeNetworkError::InvalidProxyUrl)
    })
    .transpose()
}

fn environment_proxy_url(mut read: impl FnMut(&str) -> Option<String>) -> Option<String> {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .into_iter()
        .find_map(|key| read(key).filter(|value| !value.trim().is_empty()))
}

fn explicit_proxy(
    raw_url: &str,
    username: &Option<String>,
    password_secret: &Option<String>,
    proxy_password: Option<&ResolvedProxyPassword>,
) -> Result<Proxy, NodeNetworkError> {
    let url = Url::parse(raw_url).map_err(|_| NodeNetworkError::InvalidProxyUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(NodeNetworkError::InvalidProxyUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NodeNetworkError::ProxyCredentialsInUrl);
    }
    if username.is_some() != password_secret.is_some() {
        return Err(NodeNetworkError::IncompleteProxyCredentials);
    }
    let mut proxy = Proxy::all(url).map_err(|_| NodeNetworkError::InvalidProxyUrl)?;
    match (username, password_secret, proxy_password) {
        (None, None, None) => {}
        (Some(username), Some(secret_name), Some(resolved))
            if resolved.secret_name == *secret_name =>
        {
            proxy = proxy.basic_auth(username, resolved.password.as_str());
        }
        (Some(_), Some(_), None) => return Err(NodeNetworkError::ProxyPasswordRequired),
        (Some(_), Some(_), Some(_)) => return Err(NodeNetworkError::ProxyPasswordRequired),
        (None, None, Some(_)) => return Err(NodeNetworkError::UnexpectedProxyPassword),
        _ => return Err(NodeNetworkError::IncompleteProxyCredentials),
    }
    Ok(proxy)
}

fn enterprise_ca_certificates(
    path: &std::path::Path,
) -> Result<Vec<Certificate>, NodeNetworkError> {
    let metadata = fs::metadata(path).map_err(|_| NodeNetworkError::CaBundleUnreadable)?;
    if !metadata.is_file() || metadata.len() > MAX_ENTERPRISE_CA_BUNDLE_BYTES {
        return Err(NodeNetworkError::CaBundleInvalidSize);
    }
    let pem = fs::read(path).map_err(|_| NodeNetworkError::CaBundleUnreadable)?;
    let certificates =
        Certificate::from_pem_bundle(&pem).map_err(|_| NodeNetworkError::CaBundleInvalid)?;
    if certificates.is_empty() {
        Err(NodeNetworkError::CaBundleInvalid)
    } else {
        Ok(certificates)
    }
}

fn valid_access_token_shape(access_token: &str) -> bool {
    access_token.len() == 64
        && access_token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
