use crate::{cli_captain_home, production_credential_resolver_at};
use captain_node::{
    NodeLinkError, NodeNetworkConfig, NodeNetworkError, NodePairingError, NodeProxyMode,
    ResolvedProxyPassword,
};
use std::{path::PathBuf, time::Duration};

pub(super) const NODE_STATE_DIR: &str = "state";
pub(crate) const PAIRING_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

pub(super) fn node_root() -> PathBuf {
    cli_captain_home().join("node")
}

pub(crate) fn proxy_mode(
    proxy: Option<String>,
    username: Option<String>,
    password_secret: Option<String>,
    no_proxy: bool,
) -> Result<NodeProxyMode, String> {
    if no_proxy {
        if username.is_some() || password_secret.is_some() {
            return Err("Proxy credentials require an explicit proxy URL".to_string());
        }
        return Ok(NodeProxyMode::Disabled);
    }
    match proxy {
        Some(url) => {
            if username.is_some() != password_secret.is_some() {
                return Err(
                    "An authenticated proxy requires both username and password secret".to_string(),
                );
            }
            Ok(NodeProxyMode::Explicit {
                url,
                username,
                password_secret,
            })
        }
        None if username.is_none() && password_secret.is_none() => Ok(NodeProxyMode::Environment),
        None => Err("Proxy credentials require an explicit proxy URL".to_string()),
    }
}

pub(crate) fn resolve_proxy_password(
    network: &NodeNetworkConfig,
    home: &std::path::Path,
) -> Result<Option<ResolvedProxyPassword>, String> {
    let NodeProxyMode::Explicit {
        password_secret: Some(secret_name),
        ..
    } = &network.proxy
    else {
        return Ok(None);
    };
    let resolver = production_credential_resolver_at(home)
        .map_err(|_| "Captain's credential resolver is unavailable".to_string())?;
    let password = resolver.resolve(secret_name).ok_or_else(|| {
        "The configured proxy password secret is unavailable in Captain's secret store".to_string()
    })?;
    Ok(Some(ResolvedProxyPassword::new(
        secret_name,
        password.as_str(),
    )))
}

pub(crate) fn pairing_retry_delay(error: &NodePairingError) -> Option<Duration> {
    match error {
        NodePairingError::RateLimited { retry_after_secs } => Some(Duration::from_secs(
            (*retry_after_secs).clamp(1, MAX_RETRY_DELAY.as_secs()),
        )),
        NodePairingError::NetworkUnavailable
        | NodePairingError::RequestTimedOut
        | NodePairingError::HubUnavailable => Some(PAIRING_POLL_INTERVAL),
        _ => None,
    }
}

pub(super) fn retryable_link_error(error: &NodeLinkError) -> bool {
    match error {
        NodeLinkError::InvalidAccessToken | NodeLinkError::TransportsUnavailable { .. } => true,
        NodeLinkError::Network(network) => matches!(
            network,
            NodeNetworkError::RequestTimedOut
                | NodeNetworkError::NetworkUnavailable
                | NodeNetworkError::TransportClosed
                | NodeNetworkError::HubUnavailable
                | NodeNetworkError::HubAuthenticationFailed
                | NodeNetworkError::HubTransportBusy { .. }
        ),
        _ => false,
    }
}

pub(super) fn next_retry(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

pub(super) async fn wait_or_stop(delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

pub(super) fn current_time_ms() -> Result<i64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "The system clock is invalid".to_string())?;
    i64::try_from(duration.as_millis()).map_err(|_| "The system clock is invalid".to_string())
}

pub(crate) fn safe_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn load_kernel_config(
    config_path: Option<PathBuf>,
    home: &std::path::Path,
) -> Result<captain_types::config::KernelConfig, String> {
    let path = config_path.unwrap_or_else(|| home.join("config.toml"));
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .map_err(|_| "Captain configuration is unreadable".to_string())?;
        raw.parse::<toml::Value>()
            .map_err(|_| "Captain configuration is invalid TOML".to_string())?;
    }
    let config = captain_kernel::config::load_config(Some(&path));
    if config.validate().is_empty() {
        Ok(config)
    } else {
        Err("Captain execution policy is invalid for the local Node".to_string())
    }
}

pub(crate) fn proxy_name(proxy: &NodeProxyMode) -> &'static str {
    match proxy {
        NodeProxyMode::Environment => "environment",
        NodeProxyMode::Disabled => "disabled",
        NodeProxyMode::Explicit { .. } => "explicit",
    }
}

pub(super) fn transport_name(transport: captain_wire::NodeTransport) -> &'static str {
    match transport {
        captain_wire::NodeTransport::WebSocket => "WebSocket",
        captain_wire::NodeTransport::HttpStream => "HTTPS stream",
        captain_wire::NodeTransport::LongPoll => "HTTPS long-poll",
    }
}
