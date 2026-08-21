//! Native first-use pairing for a standalone Captain Console installation.

use crate::{
    profiles::profile_label, secret_support::resolve_proxy_password, ConsoleProfileCatalog,
    ConsoleProfileError, ConsoleProfileSummary,
};
use captain_node::{
    ClientLocalConfig, ClientLocalConfigError, ClientLocalConfigStore, ClientPairingClient,
    ClientPairingError, ClientPairingProgress, ClientPairingStore, ClientProfileEntry,
    ClientProfileRegistryError, NodeNetworkConfig, NodeNetworkError,
};
use std::{path::PathBuf, time::Duration};

const CLIENT_STATE_DIR: &str = "state";
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
pub const PAIRING_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct ConsolePairingOptions {
    pub home: PathBuf,
    pub profile_selector: Option<String>,
    pub label: Option<String>,
    pub client_name: String,
    pub platform: String,
    pub network: NodeNetworkConfig,
    pub captain_version: String,
}

pub struct ConsolePairingSession {
    client: ClientPairingClient,
    config: ClientLocalConfig,
    profile: ConsoleProfileSummary,
}

impl ConsolePairingSession {
    pub async fn start(
        options: ConsolePairingOptions,
    ) -> Result<(Self, ClientPairingProgress), ConsolePairingError> {
        let requested_label = options
            .label
            .as_deref()
            .map(validated_profile_label)
            .transpose()?;
        let proxy_password = resolve_proxy_password(&options.network.proxy, &options.home)
            .map_err(|_| ConsolePairingError::ProxyCredentialUnavailable)?;
        let http = options.network.build_client(proxy_password.as_ref())?;
        let config =
            ClientLocalConfig::new(options.client_name, options.platform, options.network)?;
        let catalog = ConsoleProfileCatalog::open(&options.home)?;
        let profile = match options.profile_selector.as_deref() {
            Some(selector) => catalog.resolve(selector)?,
            None => matching_hub_profile(&catalog, &config.network.hub_url)?
                .unwrap_or(catalog.registry.create_profile(current_time_ms()?)?),
        };
        let profile_root = catalog.registry.profile_root(&profile.id)?;
        let config_store = ClientLocalConfigStore::open(profile_root)?;
        if let Some(existing) = config_store.load()? {
            if !same_hub_authority(&existing.network.hub_url, &config.network.hub_url)? {
                return Err(ConsolePairingError::ProfileAuthorityConflict);
            }
        }
        let label = requested_label
            .or_else(|| profile.label.clone())
            .unwrap_or_else(|| profile_label(&profile));
        catalog.registry.set_label(&profile.id, &label)?;
        config_store.save(&config)?;
        catalog.registry.set_active(&profile.id)?;

        let state_root = config_store.root().join(CLIENT_STATE_DIR);
        let mut pairing_store = ClientPairingStore::open(&state_root, &profile.id)?;
        if matches!(
            pairing_store.status()?,
            Some(ClientPairingProgress::Denied { .. } | ClientPairingProgress::Expired { .. })
        ) {
            pairing_store.reset()?;
            pairing_store = ClientPairingStore::open(&state_root, &profile.id)?;
        }
        let client = ClientPairingClient::new(http, pairing_store);
        let progress = client
            .start_or_resume(&config.pairing_profile(&options.captain_version))
            .await?;
        let profile = catalog
            .list()?
            .into_iter()
            .find(|candidate| candidate.id == profile.id)
            .ok_or(ConsolePairingError::ProfileStateUnavailable)?;
        Ok((
            Self {
                client,
                config,
                profile,
            },
            progress,
        ))
    }

    pub fn profile(&self) -> &ConsoleProfileSummary {
        &self.profile
    }

    pub async fn poll(&self) -> Result<ClientPairingProgress, ConsolePairingError> {
        self.client.poll().await.map_err(Into::into)
    }

    pub fn approval_url(
        &self,
        progress: &ClientPairingProgress,
    ) -> Result<Option<String>, ConsolePairingError> {
        let ClientPairingProgress::AwaitingApproval { approval_path, .. } = progress else {
            return Ok(None);
        };
        if !approval_path.starts_with('/') || approval_path.starts_with("//") {
            return Err(ConsolePairingError::InvalidApprovalPath);
        }
        approval_url_for_hub(&self.config.network.hub_url, approval_path).map(Some)
    }
}

impl std::fmt::Debug for ConsolePairingSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConsolePairingSession")
            .field("client", &self.client)
            .field("config", &"[REDACTED]")
            .field("profile", &self.profile)
            .finish()
    }
}

impl ConsolePairingError {
    pub fn retry_delay(&self) -> Option<Duration> {
        match self {
            Self::Pairing(ClientPairingError::RateLimited { retry_after_secs }) => Some(
                Duration::from_secs((*retry_after_secs).clamp(1, MAX_RETRY_DELAY.as_secs())),
            ),
            Self::Pairing(
                ClientPairingError::NetworkUnavailable
                | ClientPairingError::RequestTimedOut
                | ClientPairingError::HubUnavailable,
            ) => Some(PAIRING_POLL_INTERVAL),
            _ => None,
        }
    }
}

fn matching_hub_profile(
    catalog: &ConsoleProfileCatalog,
    hub_url: &str,
) -> Result<Option<ClientProfileEntry>, ConsolePairingError> {
    let mut matches = Vec::new();
    for profile in catalog.registry.list()? {
        let root = catalog.registry.profile_root(&profile.id)?;
        let config = ClientLocalConfigStore::open(root)?.load()?;
        if config
            .map(|config| same_hub_authority(&config.network.hub_url, hub_url))
            .transpose()?
            .unwrap_or(false)
        {
            matches.push(profile);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(ConsolePairingError::AmbiguousAuthority),
    }
}

fn same_hub_authority(left: &str, right: &str) -> Result<bool, ConsolePairingError> {
    let left = url::Url::parse(left).map_err(|_| ConsolePairingError::InvalidHubAuthority)?;
    let right = url::Url::parse(right).map_err(|_| ConsolePairingError::InvalidHubAuthority)?;
    Ok(left.origin() == right.origin())
}

fn approval_url_for_hub(hub_url: &str, approval_path: &str) -> Result<String, ConsolePairingError> {
    if !approval_path.starts_with('/') || approval_path.starts_with("//") {
        return Err(ConsolePairingError::InvalidApprovalPath);
    }
    let mut origin =
        url::Url::parse(hub_url).map_err(|_| ConsolePairingError::InvalidApprovalPath)?;
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let approval = origin
        .join(approval_path.trim_start_matches('/'))
        .map_err(|_| ConsolePairingError::InvalidApprovalPath)?;
    if approval.origin() != origin.origin() {
        return Err(ConsolePairingError::InvalidApprovalPath);
    }
    Ok(approval.to_string())
}

fn validated_profile_label(label: &str) -> Result<String, ConsolePairingError> {
    let label = label.trim();
    if label.is_empty() || label.len() > 80 || label.chars().any(char::is_control) {
        return Err(ConsolePairingError::InvalidProfileLabel);
    }
    Ok(label.to_string())
}

fn current_time_ms() -> Result<i64, ConsolePairingError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ConsolePairingError::ClockUnavailable)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ConsolePairingError::ClockUnavailable)
}

#[derive(Debug, thiserror::Error)]
pub enum ConsolePairingError {
    #[error("the configured network policy is invalid")]
    Network(#[from] NodeNetworkError),
    #[error("the Client profile configuration is unavailable")]
    Config(#[from] ClientLocalConfigError),
    #[error("the Client profile registry is unavailable")]
    Registry(#[from] ClientProfileRegistryError),
    #[error("the Captain profile is unavailable")]
    Profile(#[from] ConsoleProfileError),
    #[error("Client pairing failed: {0}")]
    Pairing(#[from] ClientPairingError),
    #[error("the configured proxy credential is unavailable")]
    ProxyCredentialUnavailable,
    #[error("the selected profile belongs to a different Captain authority")]
    ProfileAuthorityConflict,
    #[error("several profiles target this Captain; select one explicitly")]
    AmbiguousAuthority,
    #[error("the local Captain profile label is invalid")]
    InvalidProfileLabel,
    #[error("the Hub returned an invalid approval path")]
    InvalidApprovalPath,
    #[error("the Captain authority stored in this profile is invalid")]
    InvalidHubAuthority,
    #[error("the Client profile state is unavailable")]
    ProfileStateUnavailable,
    #[error("the system clock is unavailable")]
    ClockUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_url_is_exact_origin_and_pairing_errors_are_bounded() {
        assert_eq!(
            approval_url_for_hub("https://hub.example", "/devices/pair?code=ABCD-EFGH").unwrap(),
            "https://hub.example/devices/pair?code=ABCD-EFGH"
        );
        assert!(approval_url_for_hub("https://hub.example", "//evil.example").is_err());
        assert!(
            ConsolePairingError::Pairing(ClientPairingError::NetworkUnavailable)
                .retry_delay()
                .is_some()
        );
        assert!(
            ConsolePairingError::Pairing(ClientPairingError::InvalidDeviceCredential)
                .retry_delay()
                .is_none()
        );
    }
}
