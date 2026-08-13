//! Private, crash-safe local configuration for one outbound Node service.

use crate::{
    NodeExecutionPolicy, NodeExecutionPolicyError, NodeNetworkConfig, NodePairingProfile,
    NodeWorkspaceBinding,
};
use captain_types::durable_fs;
use captain_wire::{CapabilityDescriptor, DeviceGrant, LogicalWorkspace, NodeTransport};
use serde::{Deserialize, Serialize};
use std::{fmt, fs, path::PathBuf};
use thiserror::Error;

const NODE_LOCAL_CONFIG_SCHEMA_VERSION: u32 = 1;
const NODE_LOCAL_CONFIG_FILE: &str = "config.toml";
const MAX_NODE_LOCAL_CONFIG_BYTES: u64 = 256 * 1024;
const NODE_TOOL_FAMILIES: [&str; 2] = ["file", "shell-process"];
const NODE_TRANSPORTS: [NodeTransport; 3] = [
    NodeTransport::WebSocket,
    NodeTransport::HttpStream,
    NodeTransport::LongPoll,
];

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLocalWorkspace {
    pub workspace_id: String,
    pub label: String,
    pub root: PathBuf,
    #[serde(default)]
    pub read_only: bool,
}

impl fmt::Debug for NodeLocalWorkspace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeLocalWorkspace")
            .field("workspace_id", &self.workspace_id)
            .field("label", &self.label)
            .field("root", &"[REDACTED]")
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLocalConfig {
    pub schema_version: u32,
    pub display_name: String,
    pub platform: String,
    pub network: NodeNetworkConfig,
    pub workspaces: Vec<NodeLocalWorkspace>,
    #[serde(default)]
    pub allow_mutation: bool,
}

impl NodeLocalConfig {
    pub fn new(
        display_name: impl Into<String>,
        platform: impl Into<String>,
        network: NodeNetworkConfig,
        workspaces: Vec<NodeLocalWorkspace>,
        allow_mutation: bool,
    ) -> Result<Self, NodeLocalConfigError> {
        let config = Self {
            schema_version: NODE_LOCAL_CONFIG_SCHEMA_VERSION,
            display_name: display_name.into(),
            platform: platform.into(),
            network,
            workspaces,
            allow_mutation,
        };
        config.validate_shape()?;
        Ok(config)
    }

    pub fn requested_grants(&self) -> DeviceGrant {
        DeviceGrant {
            workspace_ids: self
                .workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone())
                .collect(),
            tool_families: NODE_TOOL_FAMILIES
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            allow_mutation: self.allow_mutation,
        }
    }

    pub fn capabilities(&self, captain_version: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            captain_version: captain_version.to_string(),
            platform: self.platform.clone(),
            transports: NODE_TRANSPORTS.to_vec(),
            tool_families: NODE_TOOL_FAMILIES
                .into_iter()
                .map(ToString::to_string)
                .collect(),
            workspaces: self
                .workspaces
                .iter()
                .map(|workspace| LogicalWorkspace {
                    workspace_id: workspace.workspace_id.clone(),
                    label: workspace.label.clone(),
                    read_only: workspace.read_only,
                })
                .collect(),
            supports_streaming_output: false,
        }
    }

    pub fn pairing_profile(&self, captain_version: &str) -> NodePairingProfile {
        NodePairingProfile::new(
            self.display_name.clone(),
            self.platform.clone(),
            self.capabilities(captain_version),
            self.requested_grants(),
        )
    }

    pub fn execution_policy(
        &self,
        approved_grants: DeviceGrant,
    ) -> Result<NodeExecutionPolicy, NodeLocalConfigError> {
        approved_grants
            .validate_against(&self.capabilities(env!("CARGO_PKG_VERSION")))
            .map_err(|_| NodeLocalConfigError::GrantInvalid)?;
        approved_grants
            .validate_subset_of(&self.requested_grants())
            .map_err(|_| NodeLocalConfigError::GrantInvalid)?;
        let bindings = self
            .workspaces
            .iter()
            .filter(|workspace| {
                approved_grants
                    .workspace_ids
                    .iter()
                    .any(|approved| approved == &workspace.workspace_id)
            })
            .map(|workspace| {
                NodeWorkspaceBinding::new(
                    &workspace.workspace_id,
                    &workspace.root,
                    workspace.read_only,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        NodeExecutionPolicy::new(approved_grants, bindings).map_err(Into::into)
    }

    fn validate_shape(&self) -> Result<(), NodeLocalConfigError> {
        if self.schema_version != NODE_LOCAL_CONFIG_SCHEMA_VERSION {
            return Err(NodeLocalConfigError::VersionUnsupported);
        }
        if !valid_text(&self.display_name, 128) || !valid_text(&self.platform, 128) {
            return Err(NodeLocalConfigError::InvalidShape);
        }
        if self.workspaces.is_empty() || self.workspaces.len() > 64 {
            return Err(NodeLocalConfigError::InvalidShape);
        }
        let mut ids = std::collections::BTreeSet::new();
        for workspace in &self.workspaces {
            if !valid_identifier(&workspace.workspace_id)
                || !valid_text(&workspace.label, 128)
                || workspace.root.as_os_str().is_empty()
                || !workspace.root.is_absolute()
                || !ids.insert(workspace.workspace_id.as_str())
            {
                return Err(NodeLocalConfigError::InvalidShape);
            }
        }
        let grants = self.requested_grants();
        grants
            .validate_against(&self.capabilities(env!("CARGO_PKG_VERSION")))
            .map_err(|_| NodeLocalConfigError::InvalidShape)
    }
}

impl fmt::Debug for NodeLocalConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeLocalConfig")
            .field("schema_version", &self.schema_version)
            .field("display_name", &self.display_name)
            .field("platform", &self.platform)
            .field("network", &self.network)
            .field("workspaces", &self.workspaces)
            .field("allow_mutation", &self.allow_mutation)
            .finish()
    }
}

pub struct NodeLocalConfigStore {
    root: PathBuf,
    config_path: PathBuf,
}

impl NodeLocalConfigStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, NodeLocalConfigError> {
        let root = root.into();
        reject_symlink(&root)?;
        durable_fs::create_dir_all(&root).map_err(|_| NodeLocalConfigError::StateUnavailable)?;
        set_private_dir(&root)?;
        let root = fs::canonicalize(root).map_err(|_| NodeLocalConfigError::StateUnavailable)?;
        let config_path = root.join(NODE_LOCAL_CONFIG_FILE);
        reject_symlink(&config_path)?;
        Ok(Self { root, config_path })
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn load(&self) -> Result<Option<NodeLocalConfig>, NodeLocalConfigError> {
        reject_symlink(&self.config_path)?;
        let metadata = match fs::metadata(&self.config_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(NodeLocalConfigError::StateUnavailable),
        };
        if !metadata.is_file() || metadata.len() > MAX_NODE_LOCAL_CONFIG_BYTES {
            return Err(NodeLocalConfigError::StateCorrupt);
        }
        let raw = fs::read_to_string(&self.config_path)
            .map_err(|_| NodeLocalConfigError::StateUnavailable)?;
        let config = toml::from_str::<NodeLocalConfig>(&raw)
            .map_err(|_| NodeLocalConfigError::StateCorrupt)?;
        config.validate_shape()?;
        Ok(Some(config))
    }

    pub fn save(&self, config: &NodeLocalConfig) -> Result<(), NodeLocalConfigError> {
        config.validate_shape()?;
        for workspace in &config.workspaces {
            NodeWorkspaceBinding::new(
                &workspace.workspace_id,
                &workspace.root,
                workspace.read_only,
            )?;
        }
        reject_symlink(&self.config_path)?;
        let raw = toml::to_string_pretty(config).map_err(|_| NodeLocalConfigError::StateCorrupt)?;
        if raw.len() as u64 > MAX_NODE_LOCAL_CONFIG_BYTES {
            return Err(NodeLocalConfigError::InvalidShape);
        }
        durable_fs::atomic_write(&self.config_path, raw.as_bytes())
            .map_err(|_| NodeLocalConfigError::StateUnavailable)?;
        set_private_file(&self.config_path)
    }
}

impl fmt::Debug for NodeLocalConfigStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeLocalConfigStore")
            .field("root", &"[REDACTED]")
            .field("config_path", &"[REDACTED]")
            .finish()
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

fn reject_symlink(path: &std::path::Path) -> Result<(), NodeLocalConfigError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(NodeLocalConfigError::UnsafePath),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(NodeLocalConfigError::StateUnavailable),
    }
}

#[cfg(unix)]
fn set_private_dir(path: &std::path::Path) -> Result<(), NodeLocalConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| NodeLocalConfigError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_dir(_path: &std::path::Path) -> Result<(), NodeLocalConfigError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) -> Result<(), NodeLocalConfigError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| NodeLocalConfigError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) -> Result<(), NodeLocalConfigError> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum NodeLocalConfigError {
    #[error("local Node configuration is unavailable")]
    StateUnavailable,
    #[error("local Node configuration is corrupt")]
    StateCorrupt,
    #[error("local Node configuration version is unsupported")]
    VersionUnsupported,
    #[error("local Node configuration contains an unsafe path")]
    UnsafePath,
    #[error("local Node configuration is invalid")]
    InvalidShape,
    #[error("Hub-approved Node grants are invalid locally")]
    GrantInvalid,
    #[error(transparent)]
    ExecutionPolicy(#[from] NodeExecutionPolicyError),
}

#[cfg(test)]
#[path = "local_config_tests.rs"]
mod tests;
