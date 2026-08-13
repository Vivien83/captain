//! Bounded, private operator snapshot for a running local Node process.

use crate::{NodeBootstrapCapabilityState, NodeRailSnapshot};
use captain_types::durable_fs;
use captain_wire::NodeTransport;
use serde::{Deserialize, Serialize};
use std::{fmt, fs, path::PathBuf};
use thiserror::Error;

const RUNTIME_STATUS_SCHEMA_VERSION: u32 = 1;
const RUNTIME_STATUS_FILE: &str = "runtime-status.json";
const MAX_RUNTIME_STATUS_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRuntimeState {
    Connected,
    Degraded,
    Stopped,
}

impl NodeRuntimeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Degraded => "degraded",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRuntimeStatus {
    schema_version: u32,
    state: NodeRuntimeState,
    updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<NodeTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    capability_state: Option<NodeBootstrapCapabilityState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_mutation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rail: Option<NodeRailSnapshot>,
    #[serde(default)]
    fallback_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error_code: Option<String>,
}

impl NodeRuntimeStatus {
    pub fn connected(
        updated_at_ms: i64,
        transport: NodeTransport,
        capability_state: NodeBootstrapCapabilityState,
        allow_mutation: bool,
        rail: NodeRailSnapshot,
        fallback_count: usize,
        last_error_code: Option<&str>,
    ) -> Result<Self, NodeRuntimeStatusError> {
        let status = Self {
            schema_version: RUNTIME_STATUS_SCHEMA_VERSION,
            state: if last_error_code.is_some() {
                NodeRuntimeState::Degraded
            } else {
                NodeRuntimeState::Connected
            },
            updated_at_ms,
            transport: Some(transport),
            capability_state: Some(capability_state),
            allow_mutation: Some(allow_mutation),
            rail: Some(rail),
            fallback_count,
            last_error_code: last_error_code.map(ToString::to_string),
        };
        status.validate()?;
        Ok(status)
    }

    pub fn stopped(updated_at_ms: i64) -> Self {
        Self {
            schema_version: RUNTIME_STATUS_SCHEMA_VERSION,
            state: NodeRuntimeState::Stopped,
            updated_at_ms,
            transport: None,
            capability_state: None,
            allow_mutation: None,
            rail: None,
            fallback_count: 0,
            last_error_code: None,
        }
    }

    pub const fn state(&self) -> &'static str {
        self.state.as_str()
    }

    pub fn device_id(&self) -> Option<&str> {
        self.rail.as_ref().map(|rail| rail.device_id.as_str())
    }

    pub const fn updated_at_ms(&self) -> i64 {
        self.updated_at_ms
    }

    pub const fn transport(&self) -> Option<NodeTransport> {
        self.transport
    }

    pub const fn capability_state(&self) -> Option<NodeBootstrapCapabilityState> {
        self.capability_state
    }

    pub const fn allow_mutation(&self) -> Option<bool> {
        self.allow_mutation
    }

    pub const fn fallback_count(&self) -> usize {
        self.fallback_count
    }

    pub fn last_error_code(&self) -> Option<&str> {
        self.last_error_code.as_deref()
    }

    pub fn rail_snapshot(&self) -> Option<&NodeRailSnapshot> {
        self.rail.as_ref()
    }

    fn validate(&self) -> Result<(), NodeRuntimeStatusError> {
        if self.schema_version != RUNTIME_STATUS_SCHEMA_VERSION {
            return Err(NodeRuntimeStatusError::VersionUnsupported);
        }
        if self.updated_at_ms < 0 || self.fallback_count > 1_000_000 {
            return Err(NodeRuntimeStatusError::StateCorrupt);
        }
        if self.last_error_code.as_deref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        }) {
            return Err(NodeRuntimeStatusError::StateCorrupt);
        }
        if self.rail.as_ref().is_some_and(|rail| {
            !valid_identifier(&rail.device_id)
                || !valid_identifier(&rail.connection_id)
                || rail.acknowledged_node_sequence > rail.last_node_sequence
                || rail.confirmed_hub_ack_sequence > rail.last_hub_sequence
                || rail.pending_outbound > 4_096
                || rail.pending_inbound > 4_096
        }) {
            return Err(NodeRuntimeStatusError::StateCorrupt);
        }
        match self.state {
            NodeRuntimeState::Connected if self.last_error_code.is_some() => {
                Err(NodeRuntimeStatusError::StateCorrupt)
            }
            NodeRuntimeState::Degraded if self.last_error_code.is_none() => {
                Err(NodeRuntimeStatusError::StateCorrupt)
            }
            NodeRuntimeState::Connected | NodeRuntimeState::Degraded
                if self.transport.is_none()
                    || self.capability_state.is_none()
                    || self.allow_mutation.is_none()
                    || self.rail.is_none() =>
            {
                Err(NodeRuntimeStatusError::StateCorrupt)
            }
            NodeRuntimeState::Stopped
                if self.transport.is_some()
                    || self.capability_state.is_some()
                    || self.allow_mutation.is_some()
                    || self.rail.is_some()
                    || self.fallback_count != 0
                    || self.last_error_code.is_some() =>
            {
                Err(NodeRuntimeStatusError::StateCorrupt)
            }
            _ => Ok(()),
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

impl fmt::Debug for NodeRuntimeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRuntimeStatus")
            .field("state", &self.state)
            .field("updated_at_ms", &self.updated_at_ms)
            .field("transport", &self.transport)
            .field("capability_state", &self.capability_state)
            .field("allow_mutation", &self.allow_mutation)
            .field(
                "rail",
                &self
                    .rail
                    .as_ref()
                    .map(|rail| (rail.pending_outbound, rail.pending_inbound)),
            )
            .field("fallback_count", &self.fallback_count)
            .field("last_error_code", &self.last_error_code)
            .finish()
    }
}

pub struct NodeRuntimeStatusStore {
    path: PathBuf,
}

impl NodeRuntimeStatusStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, NodeRuntimeStatusError> {
        let root = root.into();
        reject_symlink(&root)?;
        let metadata = fs::metadata(&root).map_err(|_| NodeRuntimeStatusError::StateUnavailable)?;
        if !metadata.is_dir() {
            return Err(NodeRuntimeStatusError::UnsafePath);
        }
        let root = fs::canonicalize(root).map_err(|_| NodeRuntimeStatusError::StateUnavailable)?;
        let path = root.join(RUNTIME_STATUS_FILE);
        reject_symlink(&path)?;
        Ok(Self { path })
    }

    pub fn load(&self) -> Result<Option<NodeRuntimeStatus>, NodeRuntimeStatusError> {
        reject_symlink(&self.path)?;
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(NodeRuntimeStatusError::StateUnavailable),
        };
        if !metadata.is_file() || metadata.len() > MAX_RUNTIME_STATUS_BYTES {
            return Err(NodeRuntimeStatusError::StateCorrupt);
        }
        let raw = fs::read(&self.path).map_err(|_| NodeRuntimeStatusError::StateUnavailable)?;
        let status = serde_json::from_slice::<NodeRuntimeStatus>(&raw)
            .map_err(|_| NodeRuntimeStatusError::StateCorrupt)?;
        status.validate()?;
        Ok(Some(status))
    }

    pub fn save(&self, status: &NodeRuntimeStatus) -> Result<(), NodeRuntimeStatusError> {
        status.validate()?;
        reject_symlink(&self.path)?;
        let raw = serde_json::to_vec(status).map_err(|_| NodeRuntimeStatusError::StateCorrupt)?;
        if raw.len() as u64 > MAX_RUNTIME_STATUS_BYTES {
            return Err(NodeRuntimeStatusError::StateCorrupt);
        }
        durable_fs::atomic_write(&self.path, &raw)
            .map_err(|_| NodeRuntimeStatusError::StateUnavailable)?;
        set_private_file(&self.path)
    }
}

impl fmt::Debug for NodeRuntimeStatusStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRuntimeStatusStore")
            .field("path", &"[REDACTED]")
            .finish()
    }
}

fn reject_symlink(path: &std::path::Path) -> Result<(), NodeRuntimeStatusError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(NodeRuntimeStatusError::UnsafePath)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(NodeRuntimeStatusError::StateUnavailable),
    }
}

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) -> Result<(), NodeRuntimeStatusError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| NodeRuntimeStatusError::StateUnavailable)
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) -> Result<(), NodeRuntimeStatusError> {
    Ok(())
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeRuntimeStatusError {
    #[error("local Node runtime status is unavailable")]
    StateUnavailable,
    #[error("local Node runtime status is corrupt")]
    StateCorrupt,
    #[error("local Node runtime status version is unsupported")]
    VersionUnsupported,
    #[error("local Node runtime status path is unsafe")]
    UnsafePath,
}

#[cfg(test)]
#[path = "runtime_status_tests.rs"]
mod tests;
