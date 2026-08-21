//! Non-secret inventory and authority selection for Captain Console.

use captain_node::{
    ClientLocalConfigStore, ClientProfileEntry, ClientProfileRegistry, ClientProfileRegistryError,
};
use serde::Serialize;
use std::path::{Path, PathBuf};

const CONSOLE_DIRECTORY: &str = "console";
const LEGACY_CLIENT_DIRECTORY: &str = "client";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConsoleProfileSummary {
    pub id: String,
    pub label: String,
    pub active: bool,
    pub configured: bool,
}

pub struct ConsoleProfileCatalog {
    pub(crate) registry: ClientProfileRegistry,
}

impl ConsoleProfileCatalog {
    pub fn open(home: impl AsRef<Path>) -> Result<Self, ConsoleProfileError> {
        let home = home.as_ref();
        let registry = ClientProfileRegistry::open(home.join(CONSOLE_DIRECTORY))?;
        registry.import_legacy_profile(home.join(LEGACY_CLIENT_DIRECTORY), current_time_ms()?)?;
        Ok(Self { registry })
    }

    pub fn open_default() -> Result<Self, ConsoleProfileError> {
        let home = captain_home().ok_or(ConsoleProfileError::HomeUnavailable)?;
        Self::open(home)
    }

    pub fn list(&self) -> Result<Vec<ConsoleProfileSummary>, ConsoleProfileError> {
        self.registry
            .list()?
            .into_iter()
            .map(|profile| self.summary(profile))
            .collect()
    }

    pub fn active(&self) -> Result<Option<ConsoleProfileSummary>, ConsoleProfileError> {
        self.registry
            .active_profile()?
            .map(|profile| self.summary(profile))
            .transpose()
    }

    pub fn activate(&self, selector: &str) -> Result<ConsoleProfileSummary, ConsoleProfileError> {
        let profile = self.resolve(selector)?;
        let summary = self.summary(profile)?;
        if !summary.configured {
            return Err(ConsoleProfileError::ProfileUnconfigured);
        }
        self.registry.set_active(&summary.id)?;
        Ok(ConsoleProfileSummary {
            active: true,
            ..summary
        })
    }

    pub fn rename(
        &self,
        selector: &str,
        label: &str,
    ) -> Result<ConsoleProfileSummary, ConsoleProfileError> {
        let profile = self.resolve(selector)?;
        self.registry.set_label(&profile.id, label)?;
        let updated = self
            .registry
            .list()?
            .into_iter()
            .find(|candidate| candidate.id == profile.id)
            .ok_or(ConsoleProfileError::StateUnavailable)?;
        self.summary(updated)
    }

    pub(crate) fn exact_profile_root(
        &self,
        requested_profile_id: Option<&str>,
    ) -> Result<Option<(String, PathBuf)>, ConsoleProfileError> {
        let profile = match requested_profile_id {
            Some(profile_id) => self
                .registry
                .list()?
                .into_iter()
                .find(|profile| profile.id == profile_id)
                .ok_or(ConsoleProfileError::ProfileNotFound)?,
            None => match self.registry.active_profile()? {
                Some(profile) => profile,
                None => return Ok(None),
            },
        };
        let root = self.registry.profile_root(&profile.id)?;
        Ok(Some((profile.id, root)))
    }

    pub(crate) fn resolve(
        &self,
        selector: &str,
    ) -> Result<ClientProfileEntry, ConsoleProfileError> {
        let selector = selector.trim();
        if selector.is_empty() || selector.chars().any(char::is_control) {
            return Err(ConsoleProfileError::InvalidSelector);
        }
        let mut matches = self
            .registry
            .list()?
            .into_iter()
            .filter(|profile| {
                profile.id == selector
                    || (selector.len() >= 8
                        && profile.id.starts_with(&selector.to_ascii_lowercase()))
                    || profile_label(profile).eq_ignore_ascii_case(selector)
            })
            .collect::<Vec<_>>();
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(ConsoleProfileError::ProfileNotFound),
            _ => Err(ConsoleProfileError::AmbiguousSelector),
        }
    }

    pub(crate) fn summary(
        &self,
        profile: ClientProfileEntry,
    ) -> Result<ConsoleProfileSummary, ConsoleProfileError> {
        let root = self.registry.profile_root(&profile.id)?;
        let configured = ClientLocalConfigStore::open(root)?.load()?.is_some();
        Ok(ConsoleProfileSummary {
            label: profile_label(&profile),
            id: profile.id,
            active: profile.active,
            configured,
        })
    }
}

pub(crate) fn profile_label(profile: &ClientProfileEntry) -> String {
    profile
        .label
        .clone()
        .unwrap_or_else(|| format!("Captain {}", profile.id.get(..8).unwrap_or("unknown")))
}

fn captain_home() -> Option<PathBuf> {
    std::env::var_os("CAPTAIN_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".captain")))
}

fn current_time_ms() -> Result<i64, ConsoleProfileError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ConsoleProfileError::ClockUnavailable)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ConsoleProfileError::ClockUnavailable)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConsoleProfileError {
    #[error("the Captain Console profile inventory is unavailable")]
    StateUnavailable,
    #[error("the Captain Console home directory is unavailable")]
    HomeUnavailable,
    #[error("the system clock is unavailable")]
    ClockUnavailable,
    #[error("the Captain profile selector is invalid")]
    InvalidSelector,
    #[error("no Captain profile matches that selector")]
    ProfileNotFound,
    #[error("the Captain profile selector is ambiguous")]
    AmbiguousSelector,
    #[error("the Captain profile is not configured")]
    ProfileUnconfigured,
}

impl From<ClientProfileRegistryError> for ConsoleProfileError {
    fn from(_error: ClientProfileRegistryError) -> Self {
        Self::StateUnavailable
    }
}

impl From<captain_node::ClientLocalConfigError> for ConsoleProfileError {
    fn from(_error: captain_node::ClientLocalConfigError) -> Self {
        Self::StateUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_node::{ClientLocalConfig, NodeNetworkConfig};

    fn configure(catalog: &ConsoleProfileCatalog, profile: &ClientProfileEntry, hub: &str) {
        let root = catalog.registry.profile_root(&profile.id).unwrap();
        let config =
            ClientLocalConfig::new("Test Client", "test-platform", NodeNetworkConfig::new(hub))
                .unwrap();
        ClientLocalConfigStore::open(root)
            .unwrap()
            .save(&config)
            .unwrap();
    }

    #[test]
    fn catalog_outputs_never_expose_hub_origins_or_device_names() {
        let home = tempfile::tempdir().unwrap();
        let catalog = ConsoleProfileCatalog::open(home.path()).unwrap();
        let profile = catalog.registry.create_profile(10).unwrap();
        catalog
            .registry
            .set_label(&profile.id, "Production")
            .unwrap();
        configure(&catalog, &profile, "https://private.example");

        let rendered = serde_json::to_string(&catalog.list().unwrap()).unwrap();
        assert!(rendered.contains("Production"));
        assert!(!rendered.contains("private.example"));
        assert!(!rendered.contains("Test Client"));
    }

    #[test]
    fn selection_is_explicit_and_unconfigured_profiles_are_never_activated() {
        let home = tempfile::tempdir().unwrap();
        let catalog = ConsoleProfileCatalog::open(home.path()).unwrap();
        let first = catalog.registry.create_profile(10).unwrap();
        let second = catalog.registry.create_profile(20).unwrap();
        catalog.registry.set_label(&first.id, "Office").unwrap();
        catalog.registry.set_label(&second.id, "Personal").unwrap();
        configure(&catalog, &first, "https://office.example");

        assert_eq!(
            catalog.activate("personal"),
            Err(ConsoleProfileError::ProfileUnconfigured)
        );
        assert_eq!(catalog.active().unwrap().unwrap().id, first.id);
        assert_eq!(catalog.activate("office").unwrap().id, first.id);
    }
}
