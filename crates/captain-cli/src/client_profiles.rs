//! Shared local profile selection for full and lightweight Client binaries.

use crate::cli_captain_home;
use captain_node::{ClientProfileRegistry, ClientProfileRegistryError};
use std::path::PathBuf;

const CONSOLE_DIRECTORY: &str = "console";
const LEGACY_CLIENT_DIRECTORY: &str = "client";
const PROFILE_REGISTRY_FILE: &str = "profiles.toml";

pub(crate) fn client_state_present() -> bool {
    let home = cli_captain_home();
    home.join(CONSOLE_DIRECTORY)
        .join(PROFILE_REGISTRY_FILE)
        .is_file()
        || home
            .join(LEGACY_CLIENT_DIRECTORY)
            .join("config.toml")
            .is_file()
}

pub(crate) fn open_registry() -> Result<ClientProfileRegistry, ClientProfilesError> {
    let home = cli_captain_home();
    let registry = ClientProfileRegistry::open(home.join(CONSOLE_DIRECTORY))?;
    registry.import_legacy_profile(home.join(LEGACY_CLIENT_DIRECTORY), current_time_ms()?)?;
    Ok(registry)
}

pub(crate) struct ActiveClientProfile {
    pub(crate) id: String,
    pub(crate) root: PathBuf,
}

pub(crate) fn active_profile_selection() -> Result<Option<ActiveClientProfile>, ClientProfilesError>
{
    if !client_state_present() {
        return Ok(None);
    }
    let registry = open_registry()?;
    registry
        .active_profile()?
        .map(|profile| {
            registry
                .profile_root(&profile.id)
                .map(|root| ActiveClientProfile {
                    id: profile.id,
                    root,
                })
        })
        .transpose()
        .map_err(Into::into)
}

fn current_time_ms() -> Result<i64, ClientProfilesError> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| ClientProfilesError::ClockUnavailable)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ClientProfilesError::ClockUnavailable)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ClientProfilesError {
    #[error("the Client profile registry is unavailable")]
    RegistryUnavailable,
    #[error("the system clock is unavailable")]
    ClockUnavailable,
}

impl From<ClientProfileRegistryError> for ClientProfilesError {
    fn from(_error: ClientProfileRegistryError) -> Self {
        Self::RegistryUnavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_errors_never_expose_local_paths() {
        let rendered = ClientProfilesError::RegistryUnavailable.to_string();
        assert!(!rendered.contains(".captain"));
        assert!(!rendered.contains("/"));
    }
}
