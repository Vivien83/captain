//! Private local configuration for a lightweight Captain Client.

use crate::{ClientPairingProfile, NodeNetworkConfig};
use captain_types::durable_fs;
use captain_wire::{CapabilityDescriptor, NodeTransport};
use serde::{Deserialize, Serialize};
use std::{fmt, fs, path::PathBuf};
use thiserror::Error;

const CLIENT_CONFIG_SCHEMA_VERSION: u32 = 1;
const CLIENT_CONFIG_FILE: &str = "config.toml";
const MAX_CLIENT_CONFIG_BYTES: u64 = 128 * 1024;
const CLIENT_TRANSPORTS: [NodeTransport; 1] = [NodeTransport::HttpStream];

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientLocalConfig {
    pub schema_version: u32,
    pub display_name: String,
    pub platform: String,
    pub network: NodeNetworkConfig,
}

impl ClientLocalConfig {
    pub fn new(
        display_name: impl Into<String>,
        platform: impl Into<String>,
        network: NodeNetworkConfig,
    ) -> Result<Self, ClientLocalConfigError> {
        let config = Self {
            schema_version: CLIENT_CONFIG_SCHEMA_VERSION,
            display_name: display_name.into(),
            platform: platform.into(),
            network,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn pairing_profile(&self, captain_version: &str) -> ClientPairingProfile {
        ClientPairingProfile::new(
            self.display_name.clone(),
            self.platform.clone(),
            CapabilityDescriptor {
                captain_version: captain_version.to_string(),
                platform: self.platform.clone(),
                transports: CLIENT_TRANSPORTS.to_vec(),
                tool_families: Vec::new(),
                workspaces: Vec::new(),
                supports_streaming_output: true,
            },
        )
    }

    fn validate(&self) -> Result<(), ClientLocalConfigError> {
        if self.schema_version != CLIENT_CONFIG_SCHEMA_VERSION {
            return Err(ClientLocalConfigError::VersionUnsupported);
        }
        if !valid_text(&self.display_name, 128) || !valid_identifier(&self.platform) {
            return Err(ClientLocalConfigError::InvalidShape);
        }
        self.pairing_profile(env!("CARGO_PKG_VERSION"))
            .validate()
            .map_err(|_| ClientLocalConfigError::InvalidShape)
    }
}

impl fmt::Debug for ClientLocalConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientLocalConfig")
            .field("schema_version", &self.schema_version)
            .field("display_name", &self.display_name)
            .field("platform", &self.platform)
            .field("network", &self.network)
            .finish()
    }
}

pub struct ClientLocalConfigStore {
    root: PathBuf,
    config_path: PathBuf,
}

impl ClientLocalConfigStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ClientLocalConfigError> {
        let root = root.into();
        reject_symlink(&root)?;
        durable_fs::create_dir_all(&root).map_err(|_| ClientLocalConfigError::StateUnavailable)?;
        set_private_dir(&root)?;
        let root = fs::canonicalize(root).map_err(|_| ClientLocalConfigError::StateUnavailable)?;
        let config_path = root.join(CLIENT_CONFIG_FILE);
        reject_symlink(&config_path)?;
        Ok(Self { root, config_path })
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn load(&self) -> Result<Option<ClientLocalConfig>, ClientLocalConfigError> {
        reject_symlink(&self.config_path)?;
        let metadata = match fs::metadata(&self.config_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ClientLocalConfigError::StateUnavailable),
        };
        if !metadata.is_file() || metadata.len() > MAX_CLIENT_CONFIG_BYTES {
            return Err(ClientLocalConfigError::StateCorrupt);
        }
        let raw = fs::read_to_string(&self.config_path)
            .map_err(|_| ClientLocalConfigError::StateUnavailable)?;
        let config = toml::from_str::<ClientLocalConfig>(&raw)
            .map_err(|_| ClientLocalConfigError::StateCorrupt)?;
        config.validate()?;
        Ok(Some(config))
    }

    pub fn save(&self, config: &ClientLocalConfig) -> Result<(), ClientLocalConfigError> {
        config.validate()?;
        reject_symlink(&self.config_path)?;
        let raw =
            toml::to_string_pretty(config).map_err(|_| ClientLocalConfigError::StateCorrupt)?;
        if raw.len() as u64 > MAX_CLIENT_CONFIG_BYTES {
            return Err(ClientLocalConfigError::InvalidShape);
        }
        durable_fs::atomic_write(&self.config_path, raw.as_bytes())
            .map_err(|_| ClientLocalConfigError::StateUnavailable)?;
        set_private_file(&self.config_path)
    }

    pub fn remove_config(&self) -> Result<(), ClientLocalConfigError> {
        reject_symlink(&self.config_path)?;
        durable_fs::remove_file(&self.config_path)
            .map(|_| ())
            .map_err(|_| ClientLocalConfigError::StateUnavailable)
    }
}

impl fmt::Debug for ClientLocalConfigStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientLocalConfigStore")
            .field("root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn reject_symlink(path: &std::path::Path) -> Result<(), ClientLocalConfigError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ClientLocalConfigError::UnsafePath)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ClientLocalConfigError::StateUnavailable),
    }
}

#[cfg(unix)]
fn set_private_dir(path: &std::path::Path) -> Result<(), ClientLocalConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| ClientLocalConfigError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_dir(_path: &std::path::Path) -> Result<(), ClientLocalConfigError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) -> Result<(), ClientLocalConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| ClientLocalConfigError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) -> Result<(), ClientLocalConfigError> {
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClientLocalConfigError {
    #[error("local Client configuration is unavailable")]
    StateUnavailable,
    #[error("local Client configuration is corrupt")]
    StateCorrupt,
    #[error("local Client configuration version is unsupported")]
    VersionUnsupported,
    #[error("local Client configuration contains an unsafe path")]
    UnsafePath,
    #[error("local Client configuration is invalid")]
    InvalidShape,
}

#[cfg(test)]
#[path = "client_config_tests.rs"]
mod tests;
