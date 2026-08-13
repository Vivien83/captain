//! Crash-safe Node pairing orchestration.

mod http;
mod state;

use crate::network::NodeHttpClient;
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    PairingPollRequest, PairingState, ProtocolVersion, HUB_NODE_PROTOCOL_VERSION,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

pub use state::{ClientPairingStore, NodePairingStore};
pub(crate) use state::{
    NodeStateBinding, NodeStateRoot, NODE_RAIL_SHM_FILE, NODE_RAIL_STATE_FILE, NODE_RAIL_WAL_FILE,
};
use state::{PersistedPairingPhase, PersistedPairingState, PersistedTerminalState};

#[cfg(test)]
use http::TEST_MAX_HUB_RESPONSE_BYTES as MAX_HUB_RESPONSE_BYTES;
#[cfg(test)]
use state::{PAIRING_LOCK_FILE, PAIRING_STATE_FILE};

#[derive(Debug, Clone, PartialEq, Eq)]
struct DevicePairingProfile {
    display_name: String,
    role: DeviceRole,
    platform: String,
    capabilities: CapabilityDescriptor,
    requested_grants: DeviceGrant,
}

impl DevicePairingProfile {
    fn claim(&self, credential_sha256: String) -> DevicePairingClaim {
        DevicePairingClaim {
            display_name: self.display_name.clone(),
            role: self.role,
            platform: self.platform.clone(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            credential_sha256,
            capabilities: self.capabilities.clone(),
            requested_grants: self.requested_grants.clone(),
        }
    }

    fn validate(&self) -> Result<(), NodePairingError> {
        self.claim("0".repeat(64))
            .validate()
            .map_err(|_| NodePairingError::InvalidProfile)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePairingProfile(DevicePairingProfile);

impl NodePairingProfile {
    pub fn new(
        display_name: impl Into<String>,
        platform: impl Into<String>,
        capabilities: CapabilityDescriptor,
        requested_grants: DeviceGrant,
    ) -> Self {
        Self(DevicePairingProfile {
            display_name: display_name.into(),
            role: DeviceRole::Node,
            platform: platform.into(),
            capabilities,
            requested_grants,
        })
    }

    #[cfg(test)]
    fn claim(&self, credential_sha256: String) -> DevicePairingClaim {
        self.0.claim(credential_sha256)
    }

    #[cfg(test)]
    fn set_display_name(&mut self, display_name: impl Into<String>) {
        self.0.display_name = display_name.into();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPairingProfile(DevicePairingProfile);

impl ClientPairingProfile {
    pub fn new(
        display_name: impl Into<String>,
        platform: impl Into<String>,
        capabilities: CapabilityDescriptor,
    ) -> Self {
        Self(DevicePairingProfile {
            display_name: display_name.into(),
            role: DeviceRole::Client,
            platform: platform.into(),
            capabilities,
            requested_grants: DeviceGrant::default(),
        })
    }

    pub(crate) fn validate(&self) -> Result<(), ClientPairingError> {
        self.0.validate()
    }

    #[cfg(test)]
    pub(crate) fn claim_for_test(&self, credential_sha256: String) -> DevicePairingClaim {
        self.0.claim(credential_sha256)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodePairingProgress {
    ReadyToClaim,
    AwaitingApproval {
        request_id: String,
        display_code: String,
        approval_path: String,
        expires_at_ms: i64,
    },
    Paired {
        device_id: String,
        protocol_version: ProtocolVersion,
    },
    Denied {
        request_id: String,
    },
    Expired {
        request_id: String,
    },
}

pub struct NodeAccessToken {
    access_token: Zeroizing<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub protocol_version: ProtocolVersion,
    approved_grants: DeviceGrant,
}

/// Read-only, multi-surface access to one already paired lightweight Client.
/// The durable store lock is released during construction; only the
/// zeroizing device credential remains in this process.
pub struct ClientAccessSession {
    http: NodeHttpClient,
    credential: Zeroizing<String>,
    device_id: String,
    protocol_version: ProtocolVersion,
}

impl ClientAccessSession {
    pub fn open(
        http: NodeHttpClient,
        store: ClientPairingStore,
    ) -> Result<Self, ClientPairingError> {
        let hub_sha256 = http.hub_sha256();
        let store = store.into_inner();
        let state = store.load()?.ok_or(ClientPairingError::PairingNotStarted)?;
        if state.hub_sha256 != hub_sha256 {
            return Err(ClientPairingError::HubIdentityMismatch);
        }
        let PersistedPairingPhase::Paired {
            credential,
            device_id,
            protocol_version,
            approved_grants,
            role,
        } = state.phase
        else {
            return Err(ClientPairingError::PairingNotApproved);
        };
        if role != DeviceRole::Client || approved_grants != DeviceGrant::default() {
            return Err(ClientPairingError::RoleMismatch);
        }
        drop(store);
        Ok(Self {
            http,
            credential,
            device_id,
            protocol_version,
        })
    }

    pub async fn issue_access_token(&self) -> Result<ClientAccessToken, ClientPairingError> {
        let mut request = DeviceCredentialExchange {
            device_id: self.device_id.clone(),
            credential: self.credential.to_string(),
        };
        let response = self.http.exchange_credential(&request).await;
        request.credential.zeroize();
        let mut response = response?;
        response
            .validate(current_time_ms()?)
            .map_err(|_| ClientPairingError::InvalidHubResponse)?;
        if response.approved_grants != DeviceGrant::default()
            || HUB_NODE_PROTOCOL_VERSION
                .negotiate(response.protocol_version)
                .map_err(|_| ClientPairingError::InvalidHubResponse)?
                != self.protocol_version
        {
            return Err(ClientPairingError::InvalidHubResponse);
        }
        let access_token = Zeroizing::new(std::mem::take(&mut response.access_token));
        Ok(NodeAccessToken {
            access_token,
            issued_at_ms: response.issued_at_ms,
            expires_at_ms: response.expires_at_ms,
            protocol_version: response.protocol_version,
            approved_grants: DeviceGrant::default(),
        })
    }
}

impl fmt::Debug for ClientAccessSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientAccessSession")
            .field("http", &self.http)
            .field("credential", &"[REDACTED]")
            .field("device_id", &self.device_id)
            .field("protocol_version", &self.protocol_version)
            .finish()
    }
}

impl NodeAccessToken {
    pub fn as_str(&self) -> &str {
        self.access_token.as_str()
    }

    pub fn approved_grants(&self) -> &DeviceGrant {
        &self.approved_grants
    }

    #[cfg(test)]
    pub(crate) fn for_test(access_token: String, issued_at_ms: i64, expires_at_ms: i64) -> Self {
        Self {
            access_token: Zeroizing::new(access_token),
            issued_at_ms,
            expires_at_ms,
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            approved_grants: DeviceGrant::default(),
        }
    }
}

impl fmt::Debug for NodeAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeAccessToken")
            .field("access_token", &"[REDACTED]")
            .field("issued_at_ms", &self.issued_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("protocol_version", &self.protocol_version)
            .finish()
    }
}

pub struct NodePairingClient {
    http: NodeHttpClient,
    store: NodePairingStore,
    hub_sha256: String,
}

impl NodePairingClient {
    pub fn new(http: NodeHttpClient, store: NodePairingStore) -> Self {
        Self::new_with_store(http, store)
    }

    fn new_with_store(http: NodeHttpClient, store: NodePairingStore) -> Self {
        let hub_sha256 = http.hub_sha256();
        Self {
            http,
            store,
            hub_sha256,
        }
    }

    pub fn status(&self) -> Result<Option<NodePairingProgress>, NodePairingError> {
        let state = self.store.load()?;
        if let Some(state) = &state {
            self.ensure_hub(state)?;
        }
        Ok(state.map(|persisted| persisted.progress()))
    }

    pub async fn start_or_resume(
        &self,
        profile: &NodePairingProfile,
    ) -> Result<NodePairingProgress, NodePairingError> {
        self.start_or_resume_profile(&profile.0).await
    }

    async fn start_or_resume_profile(
        &self,
        profile: &DevicePairingProfile,
    ) -> Result<NodePairingProgress, NodePairingError> {
        profile.validate()?;
        let state = match self.store.load()? {
            Some(state) => state,
            None => {
                let credential = random_secret();
                let claim = profile.claim(sha256_hex(credential.as_bytes()));
                let state = PersistedPairingState::new(
                    self.hub_sha256.clone(),
                    PersistedPairingPhase::Prepared { credential, claim },
                );
                self.store.save(&state)?;
                state
            }
        };
        self.ensure_hub(&state)?;

        match state.phase {
            PersistedPairingPhase::Prepared { credential, claim } => {
                ensure_profile_matches(profile, &claim)?;
                self.submit_claim(credential, claim).await
            }
            PersistedPairingPhase::AwaitingApproval { ref claim, .. } => {
                ensure_profile_matches(profile, claim)?;
                Ok(state.progress())
            }
            _ => Ok(state.progress()),
        }
    }

    pub async fn poll(&self) -> Result<NodePairingProgress, NodePairingError> {
        let state = self
            .store
            .load()?
            .ok_or(NodePairingError::PairingNotStarted)?;
        self.ensure_hub(&state)?;
        let PersistedPairingPhase::AwaitingApproval {
            credential,
            request_id,
            display_code,
            polling_secret,
            expires_at_ms,
            approval_path,
            protocol_version,
            claim,
        } = state.phase
        else {
            return Ok(state.progress());
        };

        if expires_at_ms <= current_time_ms()? {
            return self.persist_terminal(PersistedTerminalState::Expired, request_id, claim.role);
        }
        let mut request = PairingPollRequest {
            request_id: request_id.clone(),
            polling_secret: polling_secret.to_string(),
        };
        let response = self.http.poll_pairing(&request).await;
        request.polling_secret.zeroize();
        let response = response?;
        response
            .validate()
            .map_err(|_| NodePairingError::InvalidHubResponse)?;
        if response.expires_at_ms != expires_at_ms {
            return Err(NodePairingError::InvalidHubResponse);
        }
        match response.status {
            PairingState::Pending if response.device_id.is_none() => {
                Ok(NodePairingProgress::AwaitingApproval {
                    request_id,
                    display_code,
                    approval_path,
                    expires_at_ms,
                })
            }
            PairingState::Approved => {
                let device_id = response
                    .device_id
                    .ok_or(NodePairingError::InvalidHubResponse)?;
                let approved_grants = response
                    .approved_grants
                    .ok_or(NodePairingError::InvalidHubResponse)?;
                validate_device_id(&device_id)?;
                approved_grants
                    .validate_against(&claim.capabilities)
                    .map_err(|_| NodePairingError::InvalidHubResponse)?;
                approved_grants
                    .validate_subset_of(&claim.requested_grants)
                    .map_err(|_| NodePairingError::InvalidHubResponse)?;
                let paired = PersistedPairingState::new(
                    self.hub_sha256.clone(),
                    PersistedPairingPhase::Paired {
                        credential,
                        device_id: device_id.clone(),
                        protocol_version,
                        approved_grants,
                        role: claim.role,
                    },
                );
                self.store.save(&paired)?;
                Ok(NodePairingProgress::Paired {
                    device_id,
                    protocol_version,
                })
            }
            PairingState::Denied if response.device_id.is_none() => {
                self.persist_terminal(PersistedTerminalState::Denied, request_id, claim.role)
            }
            PairingState::Expired if response.device_id.is_none() => {
                self.persist_terminal(PersistedTerminalState::Expired, request_id, claim.role)
            }
            _ => Err(NodePairingError::InvalidHubResponse),
        }
    }

    pub async fn issue_access_token(&self) -> Result<NodeAccessToken, NodePairingError> {
        let state = self
            .store
            .load()?
            .ok_or(NodePairingError::PairingNotStarted)?;
        self.ensure_hub(&state)?;
        let PersistedPairingPhase::Paired {
            credential,
            device_id,
            role,
            ..
        } = state.phase
        else {
            return Err(NodePairingError::PairingNotApproved);
        };
        let mut request = DeviceCredentialExchange {
            device_id,
            credential: credential.to_string(),
        };
        let response = self.http.exchange_credential(&request).await;
        request.credential.zeroize();
        let mut response = response?;
        response
            .validate(current_time_ms()?)
            .map_err(|_| NodePairingError::InvalidHubResponse)?;
        let approved_grants = response.approved_grants.clone();
        self.store.save(&PersistedPairingState::new(
            self.hub_sha256.clone(),
            PersistedPairingPhase::Paired {
                credential,
                device_id: request.device_id,
                protocol_version: response.protocol_version,
                approved_grants: approved_grants.clone(),
                role,
            },
        ))?;
        let access_token = Zeroizing::new(std::mem::take(&mut response.access_token));
        Ok(NodeAccessToken {
            access_token,
            issued_at_ms: response.issued_at_ms,
            expires_at_ms: response.expires_at_ms,
            protocol_version: response.protocol_version,
            approved_grants,
        })
    }

    async fn submit_claim(
        &self,
        credential: Zeroizing<String>,
        claim: DevicePairingClaim,
    ) -> Result<NodePairingProgress, NodePairingError> {
        let mut challenge = self.http.submit_pairing_claim(&claim).await?;
        challenge
            .validate(current_time_ms()?)
            .map_err(|_| NodePairingError::InvalidHubResponse)?;
        let polling_secret = Zeroizing::new(std::mem::take(&mut challenge.polling_secret));
        let progress = NodePairingProgress::AwaitingApproval {
            request_id: challenge.request_id.clone(),
            display_code: challenge.display_code.clone(),
            approval_path: challenge.approval_path.clone(),
            expires_at_ms: challenge.expires_at_ms,
        };
        self.store.save(&PersistedPairingState::new(
            self.hub_sha256.clone(),
            PersistedPairingPhase::AwaitingApproval {
                credential,
                claim,
                request_id: challenge.request_id,
                display_code: challenge.display_code,
                polling_secret,
                expires_at_ms: challenge.expires_at_ms,
                approval_path: challenge.approval_path,
                protocol_version: challenge.protocol_version,
            },
        ))?;
        Ok(progress)
    }

    fn persist_terminal(
        &self,
        outcome: PersistedTerminalState,
        request_id: String,
        role: DeviceRole,
    ) -> Result<NodePairingProgress, NodePairingError> {
        let state = PersistedPairingState::new(
            self.hub_sha256.clone(),
            PersistedPairingPhase::Terminal {
                outcome,
                request_id: request_id.clone(),
                role,
            },
        );
        self.store.save(&state)?;
        Ok(state.progress())
    }

    fn ensure_hub(&self, state: &PersistedPairingState) -> Result<(), NodePairingError> {
        if state.hub_sha256 == self.hub_sha256 {
            Ok(())
        } else {
            Err(NodePairingError::HubIdentityMismatch)
        }
    }
}

pub struct ClientPairingClient(NodePairingClient);

impl ClientPairingClient {
    pub fn new(http: NodeHttpClient, store: ClientPairingStore) -> Self {
        Self(NodePairingClient::new_with_store(http, store.into_inner()))
    }

    pub fn status(&self) -> Result<Option<ClientPairingProgress>, ClientPairingError> {
        self.0.status()
    }

    pub async fn start_or_resume(
        &self,
        profile: &ClientPairingProfile,
    ) -> Result<ClientPairingProgress, ClientPairingError> {
        self.0.start_or_resume_profile(&profile.0).await
    }

    pub async fn poll(&self) -> Result<ClientPairingProgress, ClientPairingError> {
        self.0.poll().await
    }

    pub async fn issue_access_token(&self) -> Result<ClientAccessToken, ClientPairingError> {
        self.0.issue_access_token().await
    }
}

impl fmt::Debug for ClientPairingClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientPairingClient")
            .field("inner", &self.0)
            .finish()
    }
}

pub type ClientAccessToken = NodeAccessToken;
pub type ClientPairingProgress = NodePairingProgress;
pub type ClientPairingError = NodePairingError;

impl fmt::Debug for NodePairingClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodePairingClient")
            .field("http", &self.http)
            .field("store", &self.store)
            .field("hub_sha256", &"[REDACTED]")
            .finish()
    }
}

fn ensure_profile_matches(
    profile: &DevicePairingProfile,
    stored: &DevicePairingClaim,
) -> Result<(), NodePairingError> {
    let expected = profile.claim(stored.credential_sha256.clone());
    if expected == *stored {
        Ok(())
    } else {
        Err(NodePairingError::ProfileChangedDuringPairing)
    }
}

fn validate_device_id(value: &str) -> Result<(), NodePairingError> {
    if valid_device_id_shape(value) {
        Ok(())
    } else {
        Err(NodePairingError::InvalidHubResponse)
    }
}

fn valid_device_id_shape(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn random_secret() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand::thread_rng().fill_bytes(bytes.as_mut());
    Zeroizing::new(hex::encode(bytes.as_ref()))
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn current_time_ms() -> Result<i64, NodePairingError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NodePairingError::ClockInvalid)?;
    i64::try_from(duration.as_millis()).map_err(|_| NodePairingError::ClockInvalid)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NodePairingError {
    #[error("device pairing profile is invalid")]
    InvalidProfile,
    #[error("device pairing state is unavailable")]
    StateUnavailable,
    #[error("device pairing state path is unsafe")]
    UnsafeStatePath,
    #[error("another device process already owns this state")]
    NodeAlreadyRunning,
    #[error("device pairing state is too large")]
    StateTooLarge,
    #[error("device pairing state is corrupt")]
    StateCorrupt,
    #[error("device pairing state version is unsupported")]
    StateVersionUnsupported,
    #[error("device pairing state belongs to a different Hub")]
    HubIdentityMismatch,
    #[error("device state is still in use by the local runtime")]
    StateInUse,
    #[error("device profile changed during pairing")]
    ProfileChangedDuringPairing,
    #[error("device pairing has not started")]
    PairingNotStarted,
    #[error("device pairing is not approved")]
    PairingNotApproved,
    #[error("device pairing role does not match this local profile")]
    RoleMismatch,
    #[error("Hub device enrollment is closed")]
    EnrollmentClosed,
    #[error("Hub device pairing is disabled")]
    PairingDisabled,
    #[error("this device credential conflicts with another pairing")]
    CredentialConflict,
    #[error("Hub pairing request was rate limited; retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("Hub pairing request expired")]
    PairingExpired,
    #[error("Hub rejected the polling credential")]
    InvalidPollingCredential,
    #[error("Hub rejected the device credential")]
    InvalidDeviceCredential,
    #[error("Hub pairing storage is unavailable")]
    HubUnavailable,
    #[error("Hub rejected pairing request with HTTP {status} ({code})")]
    HubRejected { status: u16, code: String },
    #[error("Hub pairing response is too large")]
    HubResponseTooLarge,
    #[error("Hub pairing response is invalid")]
    InvalidHubResponse,
    #[error("Hub network is unavailable")]
    NetworkUnavailable,
    #[error("Hub request timed out")]
    RequestTimedOut,
    #[error("system clock is invalid")]
    ClockInvalid,
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
