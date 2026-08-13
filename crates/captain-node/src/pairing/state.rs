//! Private, crash-safe persistence for one Node pairing identity.

use super::{sha256_hex, valid_device_id_shape, NodePairingError, NodePairingProgress};
use captain_types::durable_fs;
use captain_wire::{
    DeviceGrant, DevicePairingClaim, DeviceRole, PairingChallenge, ProtocolVersion,
    HUB_NODE_PROTOCOL_VERSION,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use zeroize::{Zeroize, Zeroizing};

const PAIRING_STATE_SCHEMA_VERSION: u16 = 1;
pub(super) const PAIRING_STATE_FILE: &str = "pairing.json";
pub(super) const PAIRING_LOCK_FILE: &str = "node.lock";
pub(crate) const NODE_RAIL_STATE_FILE: &str = "rail.sqlite3";
pub(crate) const NODE_RAIL_WAL_FILE: &str = "rail.sqlite3-wal";
pub(crate) const NODE_RAIL_SHM_FILE: &str = "rail.sqlite3-shm";
const NODE_RESET_MARKER_FILE: &str = "reset.pending";
const NODE_RESET_MARKER: &[u8] = b"captain-node-reset-v1\n";
const MAX_PAIRING_STATE_BYTES: u64 = 512 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct PersistedPairingState {
    schema_version: u16,
    pub(super) hub_sha256: String,
    pub(super) phase: PersistedPairingPhase,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(super) enum PersistedPairingPhase {
    Prepared {
        credential: Zeroizing<String>,
        claim: DevicePairingClaim,
    },
    AwaitingApproval {
        credential: Zeroizing<String>,
        claim: DevicePairingClaim,
        request_id: String,
        display_code: String,
        polling_secret: Zeroizing<String>,
        expires_at_ms: i64,
        approval_path: String,
        protocol_version: ProtocolVersion,
    },
    Paired {
        credential: Zeroizing<String>,
        device_id: String,
        protocol_version: ProtocolVersion,
        #[serde(default)]
        approved_grants: DeviceGrant,
        #[serde(default = "default_node_role")]
        role: DeviceRole,
    },
    Terminal {
        outcome: PersistedTerminalState,
        request_id: String,
        #[serde(default = "default_node_role")]
        role: DeviceRole,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PersistedTerminalState {
    Denied,
    Expired,
}

impl PersistedPairingState {
    pub(super) fn new(hub_sha256: String, phase: PersistedPairingPhase) -> Self {
        Self {
            schema_version: PAIRING_STATE_SCHEMA_VERSION,
            hub_sha256,
            phase,
        }
    }

    fn validate(&self) -> Result<(), NodePairingError> {
        if self.schema_version != PAIRING_STATE_SCHEMA_VERSION {
            return Err(NodePairingError::StateVersionUnsupported);
        }
        validate_raw_secret(&self.hub_sha256)?;
        match &self.phase {
            PersistedPairingPhase::Prepared { credential, claim } => {
                validate_claim_binding(credential, claim)
            }
            PersistedPairingPhase::AwaitingApproval {
                credential,
                claim,
                request_id,
                display_code,
                polling_secret,
                expires_at_ms,
                approval_path,
                protocol_version,
            } => {
                validate_claim_binding(credential, claim)?;
                let mut challenge = PairingChallenge {
                    request_id: request_id.clone(),
                    display_code: display_code.clone(),
                    polling_secret: polling_secret.to_string(),
                    expires_at_ms: *expires_at_ms,
                    approval_path: approval_path.clone(),
                    protocol_version: *protocol_version,
                };
                let result = challenge
                    .validate(0)
                    .map_err(|_| NodePairingError::StateCorrupt);
                challenge.polling_secret.zeroize();
                result
            }
            PersistedPairingPhase::Paired {
                credential,
                device_id,
                protocol_version,
                approved_grants,
                role,
            } => {
                validate_raw_secret(credential)?;
                if !valid_device_id_shape(device_id) {
                    return Err(NodePairingError::StateCorrupt);
                }
                HUB_NODE_PROTOCOL_VERSION
                    .negotiate(*protocol_version)
                    .map_err(|_| NodePairingError::StateCorrupt)?;
                approved_grants
                    .validate_shape()
                    .map_err(|_| NodePairingError::StateCorrupt)?;
                if *role == DeviceRole::Client && *approved_grants != DeviceGrant::default() {
                    return Err(NodePairingError::StateCorrupt);
                }
                Ok(())
            }
            PersistedPairingPhase::Terminal { request_id, .. } => validate_request_id(request_id),
        }
    }

    fn role(&self) -> DeviceRole {
        match &self.phase {
            PersistedPairingPhase::Prepared { claim, .. }
            | PersistedPairingPhase::AwaitingApproval { claim, .. } => claim.role,
            PersistedPairingPhase::Paired { role, .. }
            | PersistedPairingPhase::Terminal { role, .. } => *role,
        }
    }

    pub(super) fn progress(&self) -> NodePairingProgress {
        match &self.phase {
            PersistedPairingPhase::Prepared { .. } => NodePairingProgress::ReadyToClaim,
            PersistedPairingPhase::AwaitingApproval {
                request_id,
                display_code,
                approval_path,
                expires_at_ms,
                ..
            } => NodePairingProgress::AwaitingApproval {
                request_id: request_id.clone(),
                display_code: display_code.clone(),
                approval_path: approval_path.clone(),
                expires_at_ms: *expires_at_ms,
            },
            PersistedPairingPhase::Paired {
                device_id,
                protocol_version,
                ..
            } => NodePairingProgress::Paired {
                device_id: device_id.clone(),
                protocol_version: *protocol_version,
            },
            PersistedPairingPhase::Terminal {
                outcome: PersistedTerminalState::Denied,
                request_id,
                ..
            } => NodePairingProgress::Denied {
                request_id: request_id.clone(),
            },
            PersistedPairingPhase::Terminal {
                outcome: PersistedTerminalState::Expired,
                request_id,
                ..
            } => NodePairingProgress::Expired {
                request_id: request_id.clone(),
            },
        }
    }
}

impl fmt::Debug for PersistedPairingState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistedPairingState")
            .field("schema_version", &self.schema_version)
            .field("hub_sha256", &"[REDACTED]")
            .field("progress", &self.progress())
            .finish()
    }
}

pub(crate) struct NodeStateRoot {
    path: PathBuf,
    lock_file: File,
}

pub(crate) struct NodeStateBinding {
    pub(crate) root: Arc<NodeStateRoot>,
    pub(crate) hub_sha256: String,
    pub(crate) device_id: String,
    pub(crate) protocol_version: ProtocolVersion,
}

impl NodeStateRoot {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for NodeStateRoot {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

pub struct NodePairingStore {
    root: Arc<NodeStateRoot>,
    expected_role: DeviceRole,
}

impl NodePairingStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, NodePairingError> {
        Self::open_for_role(root, DeviceRole::Node)
    }

    fn open_for_role(
        root: impl Into<PathBuf>,
        expected_role: DeviceRole,
    ) -> Result<Self, NodePairingError> {
        let root = root.into();
        reject_unsafe_existing_path(&root, true)?;
        durable_fs::create_dir_all(&root).map_err(|_| NodePairingError::StateUnavailable)?;
        secure_directory(&root)?;

        let lock_path = root.join(PAIRING_LOCK_FILE);
        reject_unsafe_existing_path(&lock_path, false)?;
        let lock_file = open_lock_file(&lock_path)?;
        secure_file(&lock_path)?;
        lock_file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                NodePairingError::NodeAlreadyRunning
            } else {
                NodePairingError::StateUnavailable
            }
        })?;
        complete_pending_reset(&root)?;

        Ok(Self {
            root: Arc::new(NodeStateRoot {
                path: root,
                lock_file,
            }),
            expected_role,
        })
    }

    pub(crate) fn state_root_handle(&self) -> Arc<NodeStateRoot> {
        Arc::clone(&self.root)
    }

    pub(crate) fn rail_binding(&self) -> Result<NodeStateBinding, NodePairingError> {
        let state = self.load()?.ok_or(NodePairingError::PairingNotStarted)?;
        let PersistedPairingPhase::Paired {
            device_id,
            protocol_version,
            ..
        } = state.phase
        else {
            return Err(NodePairingError::PairingNotApproved);
        };
        Ok(NodeStateBinding {
            root: self.state_root_handle(),
            hub_sha256: state.hub_sha256,
            device_id,
            protocol_version,
        })
    }

    pub fn approved_grants(&self) -> Result<DeviceGrant, NodePairingError> {
        let state = self.load()?.ok_or(NodePairingError::PairingNotStarted)?;
        let PersistedPairingPhase::Paired {
            approved_grants, ..
        } = state.phase
        else {
            return Err(NodePairingError::PairingNotApproved);
        };
        Ok(approved_grants)
    }

    pub(super) fn load(&self) -> Result<Option<PersistedPairingState>, NodePairingError> {
        let state_path = self.root.path().join(PAIRING_STATE_FILE);
        let metadata = match fs::symlink_metadata(&state_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(NodePairingError::StateUnavailable),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(NodePairingError::UnsafeStatePath);
        }
        if metadata.len() > MAX_PAIRING_STATE_BYTES {
            return Err(NodePairingError::StateTooLarge);
        }
        secure_file(&state_path)?;
        let raw =
            Zeroizing::new(fs::read(&state_path).map_err(|_| NodePairingError::StateUnavailable)?);
        let state: PersistedPairingState =
            serde_json::from_slice(&raw).map_err(|_| NodePairingError::StateCorrupt)?;
        state.validate()?;
        if state.role() != self.expected_role {
            return Err(NodePairingError::RoleMismatch);
        }
        Ok(Some(state))
    }

    pub(super) fn save(&self, state: &PersistedPairingState) -> Result<(), NodePairingError> {
        state.validate()?;
        if state.role() != self.expected_role {
            return Err(NodePairingError::RoleMismatch);
        }
        let raw =
            Zeroizing::new(serde_json::to_vec(state).map_err(|_| NodePairingError::StateCorrupt)?);
        if raw.len() > MAX_PAIRING_STATE_BYTES as usize {
            return Err(NodePairingError::StateTooLarge);
        }
        let state_path = self.root.path().join(PAIRING_STATE_FILE);
        durable_fs::atomic_write(&state_path, &raw)
            .map_err(|_| NodePairingError::StateUnavailable)?;
        secure_file(&state_path)
    }

    pub fn status(&self) -> Result<Option<NodePairingProgress>, NodePairingError> {
        self.load()
            .map(|state| state.map(|persisted| persisted.progress()))
    }

    pub fn reset(self) -> Result<(), NodePairingError> {
        if Arc::strong_count(&self.root) != 1 {
            return Err(NodePairingError::StateInUse);
        }
        let marker_path = self.root.path().join(NODE_RESET_MARKER_FILE);
        durable_fs::atomic_write(&marker_path, NODE_RESET_MARKER)
            .map_err(|_| NodePairingError::StateUnavailable)?;
        secure_file(&marker_path)?;
        complete_pending_reset(self.root.path())
    }
}

pub struct ClientPairingStore(NodePairingStore);

impl ClientPairingStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, NodePairingError> {
        NodePairingStore::open_for_role(root, DeviceRole::Client).map(Self)
    }

    pub fn status(&self) -> Result<Option<NodePairingProgress>, NodePairingError> {
        self.0.status()
    }

    pub fn reset(self) -> Result<(), NodePairingError> {
        self.0.reset()
    }

    pub(super) fn into_inner(self) -> NodePairingStore {
        self.0
    }

    #[cfg(test)]
    pub(super) fn save_for_test(
        &self,
        state: &PersistedPairingState,
    ) -> Result<(), NodePairingError> {
        self.0.save(state)
    }
}

impl fmt::Debug for ClientPairingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientPairingStore")
            .field("state_root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for NodePairingStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodePairingStore")
            .field("state_root", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

fn validate_claim_binding(
    credential: &Zeroizing<String>,
    claim: &DevicePairingClaim,
) -> Result<(), NodePairingError> {
    validate_raw_secret(credential)?;
    claim
        .validate()
        .map_err(|_| NodePairingError::StateCorrupt)?;
    if sha256_hex(credential.as_bytes()) == claim.credential_sha256 {
        Ok(())
    } else {
        Err(NodePairingError::StateCorrupt)
    }
}

fn default_node_role() -> DeviceRole {
    DeviceRole::Node
}

fn validate_raw_secret(value: &str) -> Result<(), NodePairingError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(NodePairingError::StateCorrupt)
    }
}

fn validate_request_id(value: &str) -> Result<(), NodePairingError> {
    let canonical = uuid::Uuid::parse_str(value)
        .ok()
        .map(|parsed| parsed.hyphenated().to_string());
    if canonical.as_deref() == Some(value) {
        Ok(())
    } else {
        Err(NodePairingError::StateCorrupt)
    }
}

fn reject_unsafe_existing_path(
    path: &Path,
    expect_directory: bool,
) -> Result<(), NodePairingError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(NodePairingError::UnsafeStatePath),
        Ok(metadata) if expect_directory && metadata.is_dir() => Ok(()),
        Ok(metadata) if !expect_directory && metadata.is_file() => Ok(()),
        Ok(_) => Err(NodePairingError::UnsafeStatePath),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(NodePairingError::StateUnavailable),
    }
}

fn open_lock_file(path: &Path) -> Result<File, NodePairingError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|_| NodePairingError::StateUnavailable)
}

fn complete_pending_reset(root: &Path) -> Result<(), NodePairingError> {
    let marker_path = root.join(NODE_RESET_MARKER_FILE);
    let metadata = match fs::symlink_metadata(&marker_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(NodePairingError::StateUnavailable),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(NodePairingError::UnsafeStatePath);
    }
    secure_file(&marker_path)?;
    let marker = fs::read(&marker_path).map_err(|_| NodePairingError::StateUnavailable)?;
    if marker != NODE_RESET_MARKER {
        return Err(NodePairingError::StateCorrupt);
    }
    for file_name in [
        PAIRING_STATE_FILE,
        NODE_RAIL_WAL_FILE,
        NODE_RAIL_SHM_FILE,
        NODE_RAIL_STATE_FILE,
    ] {
        durable_fs::remove_file(&root.join(file_name))
            .map_err(|_| NodePairingError::StateUnavailable)?;
    }
    durable_fs::remove_file(&marker_path)
        .map(|_| ())
        .map_err(|_| NodePairingError::StateUnavailable)
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<(), NodePairingError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| NodePairingError::StateUnavailable)
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<(), NodePairingError> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<(), NodePairingError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| NodePairingError::StateUnavailable)
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<(), NodePairingError> {
    Ok(())
}
