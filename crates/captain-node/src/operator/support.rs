use crate::{
    NodeLinkError, NodeNetworkConfig, NodeNetworkError, NodePairingError, NodeProxyMode,
    ResolvedProxyPassword,
};
use std::{fmt, path::PathBuf, time::Duration};

pub(super) const NODE_STATE_DIR: &str = "state";
pub(super) const PAIRING_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

pub trait NodeProxyPasswordResolver: Send + Sync {
    fn resolve(&self, network: &NodeNetworkConfig)
        -> Result<Option<ResolvedProxyPassword>, String>;
}

pub trait NodeEventSink: Send + Sync {
    fn emit(&self, event: NodeOperatorEvent);
}

#[derive(Clone, PartialEq, Eq)]
pub enum NodeOperatorEvent {
    Pairing {
        display_code: String,
        approval_url: String,
    },
    PairingResumable,
    Paired {
        device_id: String,
    },
    Connected {
        transport: String,
        allow_mutation: bool,
    },
    Stopped,
}

impl fmt::Debug for NodeOperatorEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pairing { .. } => formatter
                .debug_struct("Pairing")
                .field("display_code", &"[REDACTED]")
                .field("approval_url", &"[REDACTED]")
                .finish(),
            Self::PairingResumable => formatter.write_str("PairingResumable"),
            Self::Paired { device_id } => formatter
                .debug_struct("Paired")
                .field("device_id", device_id)
                .finish(),
            Self::Connected {
                transport,
                allow_mutation,
            } => formatter
                .debug_struct("Connected")
                .field("transport", transport)
                .field("allow_mutation", allow_mutation)
                .finish(),
            Self::Stopped => formatter.write_str("Stopped"),
        }
    }
}

pub(super) fn node_root(home: &std::path::Path) -> PathBuf {
    home.join("node")
}

pub(super) fn proxy_mode(
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

pub(super) fn pairing_retry_delay(error: &NodePairingError) -> Option<Duration> {
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

pub(super) async fn wait_or_stop(delay: Duration, shutdown: &mut crate::NodeShutdown) -> bool {
    tokio::select! {
        _ = shutdown.wait() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

pub(super) fn current_time_ms() -> Result<i64, String> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "The system clock is invalid".to_string())?;
    i64::try_from(duration.as_millis()).map_err(|_| "The system clock is invalid".to_string())
}

pub(super) fn safe_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn proxy_name(proxy: &NodeProxyMode) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_event_debug_never_exposes_code_or_url() {
        let event = NodeOperatorEvent::Pairing {
            display_code: "SECRET-CODE".to_string(),
            approval_url: "https://hub.example/pair?code=SECRET-CODE".to_string(),
        };
        let rendered = format!("{event:?}");
        assert!(!rendered.contains("SECRET-CODE"));
        assert!(!rendered.contains("hub.example"));
        assert_eq!(
            rendered,
            "Pairing { display_code: \"[REDACTED]\", approval_url: \"[REDACTED]\" }"
        );
    }
}
