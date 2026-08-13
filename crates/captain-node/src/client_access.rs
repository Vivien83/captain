//! Shared authenticated transport for paired lightweight Client surfaces.

use crate::{
    ClientAccessSession, ClientAccessToken, ClientLocalConfig, ClientPairingStore, NodeHttpClient,
    ResolvedProxyPassword,
};
use captain_wire::client_relay_path_is_canonical;
use reqwest::Method;
use reqwest_websocket::RequestBuilderExt;
use std::{fmt, time::Duration};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use url::Url;
use zeroize::Zeroizing;

const MIN_REFRESH_MARGIN_MS: i64 = 5_000;
const MAX_REFRESH_MARGIN_MS: i64 = 60_000;

/// One in-memory copy of a short-lived Client bearer.
pub struct ClientAccessCredential {
    token: Zeroizing<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl ClientAccessCredential {
    pub fn as_str(&self) -> &str {
        self.token.as_str()
    }
}

impl fmt::Debug for ClientAccessCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientAccessCredential")
            .field("token", &"[REDACTED]")
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Process-local broker shared by terminal and Desktop Client surfaces.
///
/// It reuses the exact paired identity, HTTPS/proxy/CA policy and one
/// serialized refresh path. The long-lived device credential remains inside
/// `ClientAccessSession`; only short-lived bearers are copied into requests.
pub struct ClientAccessTransport {
    config: ClientLocalConfig,
    proxy_password: Option<ResolvedProxyPassword>,
    http: NodeHttpClient,
    access: ClientAccessSession,
    token: Mutex<Option<ClientAccessToken>>,
}

impl ClientAccessTransport {
    pub fn open(
        config: ClientLocalConfig,
        proxy_password: Option<ResolvedProxyPassword>,
        pairing_store: ClientPairingStore,
    ) -> Result<Self, ClientAccessError> {
        let http = config
            .network
            .build_client(proxy_password.as_ref())
            .map_err(|_| ClientAccessError::TransportUnavailable)?;
        let access = ClientAccessSession::open(http.clone(), pairing_store)
            .map_err(|_| ClientAccessError::PairingUnavailable)?;
        Ok(Self {
            config,
            proxy_password,
            http,
            access,
            token: Mutex::new(None),
        })
    }

    /// Normalized Hub origin. Callers must treat it as private configuration.
    pub fn base_url(&self) -> &str {
        self.config.network.hub_url.trim_end_matches('/')
    }

    pub async fn credential(&self) -> Result<ClientAccessCredential, ClientAccessError> {
        let now_ms = current_time_ms()?;
        let mut token = self.token.lock().await;
        if token
            .as_ref()
            .is_some_and(|current| token_is_fresh(current, now_ms))
        {
            return credential_from_token(token.as_ref().expect("token checked above"));
        }

        match self.access.issue_access_token().await {
            Ok(new_token) => {
                let replace = token.as_ref().is_none_or(|current| {
                    (new_token.expires_at_ms, new_token.issued_at_ms)
                        >= (current.expires_at_ms, current.issued_at_ms)
                });
                if replace {
                    *token = Some(new_token);
                }
            }
            Err(_)
                if token
                    .as_ref()
                    .is_some_and(|current| current.expires_at_ms > now_ms) =>
            {
                tracing::warn!(
                    "Client token refresh deferred while the current bearer remains valid"
                );
            }
            Err(_) => return Err(ClientAccessError::TokenUnavailable),
        }

        let current = token
            .as_ref()
            .filter(|current| current.expires_at_ms > current_time_ms().unwrap_or(i64::MAX))
            .ok_or(ClientAccessError::TokenUnavailable)?;
        credential_from_token(current)
    }

    /// Build one same-origin authenticated Hub request.
    pub async fn request(
        &self,
        method: Method,
        path_and_query: &str,
    ) -> Result<reqwest::RequestBuilder, ClientAccessError> {
        let url = self.endpoint(path_and_query)?;
        let credential = self.credential().await?;
        Ok(self
            .http
            .client
            .request(method, url)
            .bearer_auth(credential.as_str()))
    }

    /// Send exactly one bounded Hub request. Mutations are never retried here:
    /// a lost response must remain explicit instead of duplicating an effect.
    pub async fn execute(
        &self,
        method: Method,
        path_and_query: &str,
        headers: &reqwest::header::HeaderMap,
        body: bytes::Bytes,
    ) -> Result<reqwest::Response, ClientAccessError> {
        let request = self
            .request(method, path_and_query)
            .await?
            .headers(headers.clone())
            .body(body);
        tokio::time::timeout(
            Duration::from_secs(self.config.network.request_timeout_secs),
            request.send(),
        )
        .await
        .map_err(|_| ClientAccessError::HubUnavailable)?
        .map_err(|_| ClientAccessError::HubUnavailable)
    }

    /// Open a same-origin authenticated WebSocket through the configured
    /// enterprise transport. Used by the Desktop's local WebView gateway.
    pub async fn open_websocket(
        &self,
        path_and_query: &str,
        max_message_bytes: usize,
    ) -> Result<reqwest_websocket::WebSocket, ClientAccessError> {
        let url = self.endpoint(path_and_query)?;
        let credential = self.credential().await?;
        let config = WebSocketConfig {
            max_message_size: Some(max_message_bytes),
            max_frame_size: Some(max_message_bytes),
            max_write_buffer_size: max_message_bytes.saturating_mul(2),
            ..WebSocketConfig::default()
        };
        let response = tokio::time::timeout(
            Duration::from_secs(self.config.network.request_timeout_secs),
            self.http
                .client
                .get(url)
                .bearer_auth(credential.as_str())
                .upgrade()
                .web_socket_config(config)
                .send(),
        )
        .await
        .map_err(|_| ClientAccessError::HubUnavailable)?
        .map_err(|_| ClientAccessError::HubUnavailable)?;
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            return Err(ClientAccessError::PairingRejected);
        }
        response
            .into_websocket()
            .await
            .map_err(|_| ClientAccessError::HubUnavailable)
    }

    pub async fn blocking_client(
        &self,
        timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Client, ClientAccessError> {
        let credential = self.credential().await?;
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", credential.as_str()))
                .map_err(|_| ClientAccessError::TokenUnavailable)?;
        value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, value);
        self.config
            .network
            .build_blocking_client_with_headers(self.proxy_password.as_ref(), timeout, headers)
            .map_err(|_| ClientAccessError::TransportUnavailable)
    }

    fn endpoint(&self, path_and_query: &str) -> Result<Url, ClientAccessError> {
        if !path_and_query_is_valid(path_and_query) {
            return Err(ClientAccessError::InvalidPath);
        }
        let mut origin = self.http.endpoints.connect.clone();
        origin.set_path("/");
        origin.set_query(None);
        let url = origin
            .join(path_and_query.trim_start_matches('/'))
            .map_err(|_| ClientAccessError::InvalidPath)?;
        if url.origin() != origin.origin() {
            return Err(ClientAccessError::InvalidPath);
        }
        Ok(url)
    }
}

impl fmt::Debug for ClientAccessTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientAccessTransport")
            .field("config", &"[REDACTED NETWORK PROFILE]")
            .field("proxy_password", &"[REDACTED]")
            .field("http", &self.http)
            .field("access", &self.access)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

fn credential_from_token(
    token: &ClientAccessToken,
) -> Result<ClientAccessCredential, ClientAccessError> {
    if token.as_str().len() != 64 {
        return Err(ClientAccessError::TokenUnavailable);
    }
    Ok(ClientAccessCredential {
        token: Zeroizing::new(token.as_str().to_string()),
        issued_at_ms: token.issued_at_ms,
        expires_at_ms: token.expires_at_ms,
    })
}

fn path_and_query_is_valid(path_and_query: &str) -> bool {
    !path_and_query.contains('#')
        && !path_and_query.chars().any(char::is_control)
        && client_relay_path_is_canonical(
            path_and_query
                .split_once('?')
                .map_or(path_and_query, |(path, _)| path),
        )
}

fn token_is_fresh(token: &ClientAccessToken, now_ms: i64) -> bool {
    token_refresh_deadline_ms(token.issued_at_ms, token.expires_at_ms) > now_ms
}

fn token_refresh_deadline_ms(issued_at_ms: i64, expires_at_ms: i64) -> i64 {
    let ttl = expires_at_ms.saturating_sub(issued_at_ms).max(1);
    let margin = (ttl / 5)
        .clamp(MIN_REFRESH_MARGIN_MS, MAX_REFRESH_MARGIN_MS)
        .min((ttl / 2).max(1));
    expires_at_ms.saturating_sub(margin)
}

fn current_time_ms() -> Result<i64, ClientAccessError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ClientAccessError::ClockUnavailable)?;
    i64::try_from(duration.as_millis()).map_err(|_| ClientAccessError::ClockUnavailable)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClientAccessError {
    #[error("the lightweight Client pairing is unavailable or incomplete")]
    PairingUnavailable,
    #[error("the secure Client transport is unavailable")]
    TransportUnavailable,
    #[error("the Hub is unavailable through the configured Client transport")]
    HubUnavailable,
    #[error("the Hub rejected this paired Client; revoke or pair it again")]
    PairingRejected,
    #[error("the short-lived Client credential is unavailable")]
    TokenUnavailable,
    #[error("the requested Hub path is invalid")]
    InvalidPath,
    #[error("the system clock is unavailable")]
    ClockUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_tokens_refresh_after_four_fifths_of_their_lifetime() {
        assert_eq!(token_refresh_deadline_ms(1_000, 101_000), 81_000);
    }

    #[test]
    fn short_tokens_refresh_halfway_without_crossing_expiry() {
        let refresh_at = token_refresh_deadline_ms(10_000, 18_000);
        assert_eq!(refresh_at, 14_000);
        assert!(refresh_at < 18_000);
    }

    #[test]
    fn endpoint_path_validation_rejects_normalization_bypasses() {
        for path in [
            "api/status",
            "//evil.example/api/status",
            "/assets/app/../api/status",
            "/assets/app/%2E%2E/api/status",
            "/assets/app/%252e%252e/api/status",
            "/api//status",
            "/api/status#fragment",
            "/api/status\n",
        ] {
            assert!(!path_and_query_is_valid(path), "{path}");
        }
        assert!(path_and_query_is_valid(
            "/api/sessions/session%201?limit=20"
        ));
    }
}
