//! Process-local access broker for one paired lightweight Client.

use crate::cli_captain_home;
use crate::commands::client::{client_root, CLIENT_STATE_DIR};
use crate::commands::node::support::resolve_proxy_password;
use captain_node::{
    ClientAccessError, ClientAccessTransport, ClientLocalConfigStore, ClientPairingStore,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use zeroize::Zeroizing;

const CLIENT_CONFIG_FILE: &str = "config.toml";
const INITIAL_STATUS_TIMEOUT: Duration = Duration::from_secs(15);

static ACTIVE_CLIENT: OnceLock<Arc<RemoteClientBroker>> = OnceLock::new();

pub(crate) fn client_profile_configured() -> bool {
    ACTIVE_CLIENT.get().is_some() || client_root().join(CLIENT_CONFIG_FILE).exists()
}

pub(crate) fn initialize() -> Result<Option<String>, RemoteClientError> {
    if let Some(active) = ACTIVE_CLIENT.get() {
        return Ok(Some(active.base_url.clone()));
    }
    if !client_profile_configured() {
        return Ok(None);
    }

    let broker = Arc::new(RemoteClientBroker::load()?);
    broker.ensure_available()?;
    match ACTIVE_CLIENT.set(Arc::clone(&broker)) {
        Ok(()) => Ok(Some(broker.base_url.clone())),
        Err(_) => ACTIVE_CLIENT
            .get()
            .map(|active| Some(active.base_url.clone()))
            .ok_or(RemoteClientError::StateUnavailable),
    }
}

pub(crate) fn auth_headers() -> Result<Option<HeaderMap>, RemoteClientError> {
    ACTIVE_CLIENT
        .get()
        .map(|active| active.auth_headers().map(Some))
        .unwrap_or(Ok(None))
}

pub(crate) fn blocking_client(
    timeout: Option<Duration>,
) -> Result<Option<reqwest::blocking::Client>, RemoteClientError> {
    ACTIVE_CLIENT
        .get()
        .map(|active| active.blocking_client(timeout).map(Some))
        .unwrap_or(Ok(None))
}

pub(crate) fn fail_closed_headers() -> HeaderMap {
    bearer_headers(&"0".repeat(64)).unwrap_or_default()
}

pub(crate) fn fail_closed_client(timeout: Option<Duration>) -> reqwest::blocking::Client {
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(200))
        .no_proxy()
        .default_headers(fail_closed_headers())
        .proxy(reqwest::Proxy::all("http://127.0.0.1:9").expect("static fail-closed proxy"));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().expect("static fail-closed HTTP client")
}

struct RemoteClientBroker {
    base_url: String,
    access: ClientAccessTransport,
}

impl RemoteClientBroker {
    fn load() -> Result<Self, RemoteClientError> {
        let root = client_root();
        let config_store = ClientLocalConfigStore::open(&root)
            .map_err(|_| RemoteClientError::ConfigurationUnavailable)?;
        let config = config_store
            .load()
            .map_err(|_| RemoteClientError::ConfigurationUnavailable)?
            .ok_or(RemoteClientError::ConfigurationUnavailable)?;
        let proxy_password = resolve_proxy_password(&config.network, &cli_captain_home())
            .map_err(|_| RemoteClientError::ProxyCredentialUnavailable)?;
        let pairing_store = ClientPairingStore::open(config_store.root().join(CLIENT_STATE_DIR))
            .map_err(|_| RemoteClientError::PairingUnavailable)?;
        let base_url = config.network.hub_url.trim_end_matches('/').to_string();
        let access = ClientAccessTransport::open(config, proxy_password, pairing_store)
            .map_err(map_client_access_error)?;
        Ok(Self { base_url, access })
    }

    fn ensure_available(&self) -> Result<(), RemoteClientError> {
        let client = self.blocking_client(Some(INITIAL_STATUS_TIMEOUT))?;
        let response = client
            .get(format!("{}/api/status", self.base_url))
            .send()
            .map_err(|_| RemoteClientError::HubUnavailable)?;
        if response.status().is_success() {
            Ok(())
        } else if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ) {
            Err(RemoteClientError::PairingRejected)
        } else {
            Err(RemoteClientError::HubUnavailable)
        }
    }

    fn blocking_client(
        &self,
        timeout: Option<Duration>,
    ) -> Result<reqwest::blocking::Client, RemoteClientError> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| RemoteClientError::RuntimeUnavailable)?
                        .block_on(self.access.blocking_client(timeout))
                        .map_err(map_client_access_error)
                })
                .join()
                .map_err(|_| RemoteClientError::RuntimeUnavailable)?
        })
    }

    fn auth_headers(&self) -> Result<HeaderMap, RemoteClientError> {
        let credential = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| RemoteClientError::RuntimeUnavailable)?
                        .block_on(self.access.credential())
                        .map_err(map_client_access_error)
                })
                .join()
                .map_err(|_| RemoteClientError::RuntimeUnavailable)?
        })?;
        bearer_headers(credential.as_str())
    }
}

impl fmt::Debug for RemoteClientBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteClientBroker")
            .field("base_url", &"[REDACTED]")
            .field("access", &self.access)
            .finish()
    }
}

fn map_client_access_error(error: ClientAccessError) -> RemoteClientError {
    match error {
        ClientAccessError::PairingUnavailable => RemoteClientError::PairingUnavailable,
        ClientAccessError::TransportUnavailable | ClientAccessError::InvalidPath => {
            RemoteClientError::TransportUnavailable
        }
        ClientAccessError::HubUnavailable => RemoteClientError::HubUnavailable,
        ClientAccessError::PairingRejected => RemoteClientError::PairingRejected,
        ClientAccessError::TokenUnavailable => RemoteClientError::TokenUnavailable,
        ClientAccessError::ClockUnavailable => RemoteClientError::ClockUnavailable,
    }
}

fn bearer_headers(raw_token: &str) -> Result<HeaderMap, RemoteClientError> {
    let bearer = Zeroizing::new(format!("Bearer {raw_token}"));
    let mut value =
        HeaderValue::from_str(&bearer).map_err(|_| RemoteClientError::TokenUnavailable)?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, value);
    Ok(headers)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RemoteClientError {
    #[error("the lightweight Client configuration is unavailable")]
    ConfigurationUnavailable,
    #[error("the lightweight Client pairing is unavailable or incomplete")]
    PairingUnavailable,
    #[error("the configured proxy credential is unavailable")]
    ProxyCredentialUnavailable,
    #[error("the secure Client transport is unavailable")]
    TransportUnavailable,
    #[error("the lightweight Client runtime is unavailable")]
    RuntimeUnavailable,
    #[error("the Hub is unavailable through the configured Client transport")]
    HubUnavailable,
    #[error("the Hub rejected this paired Client; revoke or pair it again")]
    PairingRejected,
    #[error("the short-lived Client credential is unavailable")]
    TokenUnavailable,
    #[error("the local Client state is unavailable")]
    StateUnavailable,
    #[error("the system clock is unavailable")]
    ClockUnavailable,
}

pub(crate) fn restricted_action_message(action: &str) -> String {
    format!(
        "{action} is not available to a lightweight Client. Use an authenticated Hub operator surface."
    )
}

pub(crate) fn daemon_command_is_restricted(command: &str) -> bool {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        first.as_str(),
        "/config" | "/restart" | "/shutdown" | "/reload"
    )
}

pub(crate) fn request_error(action: &str, error: &reqwest::Error) -> String {
    if client_profile_configured() {
        let class = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connection"
        } else if error.is_decode() {
            "response"
        } else {
            "transport"
        };
        format!("{action}: remote Hub {class} failure")
    } else {
        format!("{action}: {error}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fail_closed_headers_are_sensitive_and_never_empty() {
        let headers = fail_closed_headers();
        let value = headers.get(AUTHORIZATION).unwrap();
        assert!(value.is_sensitive());
        assert_eq!(value.to_str().unwrap().len(), "Bearer ".len() + 64);
    }

    #[test]
    fn broker_debug_and_errors_do_not_expose_an_origin_or_token() {
        let error = RemoteClientError::HubUnavailable.to_string();
        assert!(!error.contains("https://"));
        assert!(!error.contains("Bearer"));
    }

    #[test]
    fn restricted_actions_are_bounded_and_non_secret() {
        let message = restricted_action_message("Configuration");
        assert!(message.contains("lightweight Client"));
        assert!(!message.contains("https://"));
        assert!(!message.contains("Bearer"));
    }

    #[test]
    fn client_daemon_command_boundary_covers_reload_aliases() {
        for command in [
            "/config",
            "/CONFIG",
            "/restart",
            "/restart status",
            "/shutdown confirm",
            "/reload config",
        ] {
            assert!(daemon_command_is_restricted(command), "{command}");
        }
        for command in ["/health", "/version", "/status"] {
            assert!(!daemon_command_is_restricted(command), "{command}");
        }
    }
}
