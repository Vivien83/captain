//! Process-local manager for immutable per-Captain gateways.

use crate::{
    gateway::{load_transport, start_gateway_at},
    observation::observe_profiles,
    ConsoleAuthorityObservation, ConsoleProfileCatalog, ConsoleProfileError, ConsoleProfileSummary,
    GatewayError, GatewayHandle,
};
use captain_node::ClientAccessTransport;
use std::{collections::HashMap, fmt, path::PathBuf, sync::Arc};
use zeroize::Zeroizing;

pub struct ConsoleManager {
    home: PathBuf,
    catalog: ConsoleProfileCatalog,
    gateways: HashMap<String, GatewayHandle>,
}

impl ConsoleManager {
    pub fn open(home: impl Into<PathBuf>) -> Result<Self, ConsoleManagerError> {
        let home = home.into();
        let catalog = ConsoleProfileCatalog::open(&home)?;
        Ok(Self {
            home,
            catalog,
            gateways: HashMap::new(),
        })
    }

    pub fn open_default() -> Result<Self, ConsoleManagerError> {
        let home = std::env::var_os("CAPTAIN_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".captain")))
            .ok_or(ConsoleManagerError::HomeUnavailable)?;
        Self::open(home)
    }

    pub fn list(&self) -> Result<Vec<ConsoleProfileSummary>, ConsoleManagerError> {
        self.catalog.list().map_err(Into::into)
    }

    pub fn local_inventory(&self) -> Result<Vec<ConsoleAuthorityObservation>, ConsoleManagerError> {
        Ok(self
            .list()?
            .into_iter()
            .map(ConsoleAuthorityObservation::local)
            .collect())
    }

    pub async fn live_inventory(
        &self,
    ) -> Result<Vec<ConsoleAuthorityObservation>, ConsoleManagerError> {
        Ok(observe_profiles(&self.home, self.list()?).await)
    }

    pub fn activate(&self, selector: &str) -> Result<ConsoleProfileSummary, ConsoleManagerError> {
        self.catalog.activate(selector).map_err(Into::into)
    }

    pub fn rename(
        &self,
        selector: &str,
        label: &str,
    ) -> Result<ConsoleProfileSummary, ConsoleManagerError> {
        self.catalog.rename(selector, label).map_err(Into::into)
    }

    pub(crate) fn connect(&self, selector: &str) -> Result<ConsoleConnection, ConsoleManagerError> {
        let candidate = self.catalog.resolve(selector)?;
        let profile = self.catalog.summary(candidate)?;
        if !profile.configured {
            return Err(ConsoleProfileError::ProfileUnconfigured.into());
        }
        let (transport, _, selected_profile_id) = load_transport(&self.home, Some(&profile.id));
        if selected_profile_id.as_deref() != Some(profile.id.as_str()) {
            return Err(ConsoleManagerError::GatewayProfileMismatch);
        }
        let transport = transport.ok_or(ConsoleManagerError::AuthorityUnavailable)?;
        Ok(ConsoleConnection { profile, transport })
    }

    pub fn launch_active(&mut self) -> Result<ConsoleLaunch, ConsoleManagerError> {
        let active = self
            .catalog
            .active()?
            .ok_or(ConsoleManagerError::NoActiveProfile)?;
        if !active.configured {
            return Err(ConsoleProfileError::ProfileUnconfigured.into());
        }
        self.launch_profile(active)
    }

    pub fn launch(&mut self, selector: &str) -> Result<ConsoleLaunch, ConsoleManagerError> {
        let candidate = self.catalog.resolve(selector)?;
        let profile = self.catalog.summary(candidate)?;
        if !profile.configured {
            return Err(ConsoleProfileError::ProfileUnconfigured.into());
        }
        let mut launch = self.launch_profile(profile)?;
        launch.profile = self.catalog.activate(&launch.profile.id)?;
        Ok(launch)
    }

    fn launch_profile(
        &mut self,
        profile: ConsoleProfileSummary,
    ) -> Result<ConsoleLaunch, ConsoleManagerError> {
        if let Some(gateway) = self.gateways.get(&profile.id) {
            return Ok(ConsoleLaunch::new(
                profile,
                gateway.port,
                gateway.paired_profile_loaded,
                gateway.issue_bootstrap_url()?,
            ));
        }
        let mut gateway = start_gateway_at(&self.home, Some(&profile.id))?;
        if gateway.active_profile_id.as_deref() != Some(profile.id.as_str()) {
            return Err(ConsoleManagerError::GatewayProfileMismatch);
        }
        let bootstrap_url = gateway.take_bootstrap_url()?;
        let port = gateway.port;
        let paired_profile_loaded = gateway.paired_profile_loaded;
        self.gateways.insert(profile.id.clone(), gateway);
        Ok(ConsoleLaunch::new(
            profile,
            port,
            paired_profile_loaded,
            bootstrap_url,
        ))
    }
}

impl Drop for ConsoleManager {
    fn drop(&mut self) {
        for (_, gateway) in self.gateways.drain() {
            gateway.shutdown();
        }
    }
}

pub(crate) struct ConsoleConnection {
    pub(crate) profile: ConsoleProfileSummary,
    pub(crate) transport: Arc<ClientAccessTransport>,
}

impl fmt::Debug for ConsoleConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleConnection")
            .field("profile", &self.profile)
            .field("transport", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for ConsoleManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleManager")
            .field("home", &"[REDACTED]")
            .field("gateway_count", &self.gateways.len())
            .finish_non_exhaustive()
    }
}

pub struct ConsoleLaunch {
    pub profile: ConsoleProfileSummary,
    pub port: u16,
    pub paired_profile_loaded: bool,
    bootstrap_url: Option<Zeroizing<String>>,
}

impl ConsoleLaunch {
    fn new(
        profile: ConsoleProfileSummary,
        port: u16,
        paired_profile_loaded: bool,
        bootstrap_url: String,
    ) -> Self {
        Self {
            profile,
            port,
            paired_profile_loaded,
            bootstrap_url: Some(Zeroizing::new(bootstrap_url)),
        }
    }

    pub fn take_bootstrap_url(&mut self) -> Result<String, ConsoleManagerError> {
        self.bootstrap_url
            .take()
            .map(|url| url.to_string())
            .ok_or(ConsoleManagerError::BootstrapUnavailable)
    }
}

impl fmt::Debug for ConsoleLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsoleLaunch")
            .field("profile", &self.profile)
            .field("port", &self.port)
            .field("paired_profile_loaded", &self.paired_profile_loaded)
            .field("bootstrap_url", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConsoleManagerError {
    #[error(transparent)]
    Profile(#[from] ConsoleProfileError),
    #[error(transparent)]
    Gateway(#[from] GatewayError),
    #[error("no active Captain profile is selected")]
    NoActiveProfile,
    #[error("the Console gateway selected a different Captain profile")]
    GatewayProfileMismatch,
    #[error("the Console bootstrap URL is unavailable")]
    BootstrapUnavailable,
    #[error("the Console output could not be serialized")]
    SerializationUnavailable,
    #[error("the selected Captain authority is not paired or its credential is unavailable")]
    AuthorityUnavailable,
    #[error("the Captain Console home directory is unavailable")]
    HomeUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_node::{
        ClientLocalConfig, ClientLocalConfigStore, ClientProfileRegistry, NodeNetworkConfig,
    };

    fn configured_profile(home: &std::path::Path, at: i64, label: &str, hub: &str) -> String {
        let registry = ClientProfileRegistry::open(home.join("console")).unwrap();
        let profile = registry.create_profile(at).unwrap();
        registry.set_label(&profile.id, label).unwrap();
        let config =
            ClientLocalConfig::new("Test Client", "test-platform", NodeNetworkConfig::new(hub))
                .unwrap();
        ClientLocalConfigStore::open(registry.profile_root(&profile.id).unwrap())
            .unwrap()
            .save(&config)
            .unwrap();
        profile.id
    }

    #[test]
    fn manager_reuses_one_gateway_per_profile_and_keeps_authorities_isolated() {
        let home = tempfile::tempdir().unwrap();
        let office = configured_profile(home.path(), 10, "Office", "https://office.example");
        let personal = configured_profile(home.path(), 20, "Personal", "https://personal.example");
        let mut manager = ConsoleManager::open(home.path()).unwrap();

        let mut first = manager.launch("Office").unwrap();
        let first_port = first.port;
        let first_url = first.take_bootstrap_url().unwrap();
        assert!(first_url.starts_with(&format!("http://127.0.0.1:{first_port}/")));
        assert!(!first_url.contains("office.example"));

        let second_ticket = manager.launch(&office).unwrap();
        assert_eq!(second_ticket.port, first_port);
        assert_eq!(manager.gateways.len(), 1);

        let prepared_personal = manager
            .catalog
            .summary(manager.catalog.resolve(&personal).unwrap())
            .unwrap();
        let prepared_personal = manager.launch_profile(prepared_personal).unwrap();
        assert!(!prepared_personal.profile.active);
        assert_eq!(manager.catalog.active().unwrap().unwrap().id, office);

        let personal_launch = manager.launch(&personal).unwrap();
        assert_ne!(personal_launch.port, first_port);
        assert_eq!(manager.gateways.len(), 2);
        assert_eq!(manager.catalog.active().unwrap().unwrap().id, personal);
    }

    #[test]
    fn unavailable_tui_authority_never_changes_the_active_profile() {
        let home = tempfile::tempdir().unwrap();
        let office = configured_profile(home.path(), 10, "Office", "https://office.example");
        let personal = configured_profile(home.path(), 20, "Personal", "https://personal.example");
        let manager = ConsoleManager::open(home.path()).unwrap();

        assert_eq!(
            manager.connect(&personal).unwrap_err().to_string(),
            ConsoleManagerError::AuthorityUnavailable.to_string()
        );
        assert_eq!(manager.catalog.active().unwrap().unwrap().id, office);
    }
}
