//! Durable, operator-mediated pairing for Hub Clients and Nodes.

use captain_memory::devices::{
    DeviceRecord, DeviceStore, DeviceStoreError, NewPairingRequest, PairingPollStatus,
    PairingRequestSummary,
};
use captain_types::config::PairingConfig;
use captain_wire::{
    DeviceAccessToken, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim, DeviceRole,
    PairingChallenge, PairingPollRequest, PairingPollResponse, PairingState, ProtocolVersion,
    HUB_NODE_PROTOCOL_VERSION,
};
use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard};
use thiserror::Error;

const MAX_PENDING_REQUESTS: usize = 5;
const MIN_PAIRING_EXPIRY_SECS: u64 = 60;
const MAX_PAIRING_EXPIRY_SECS: u64 = 15 * 60;
const ACCESS_TOKEN_TTL_MS: i64 = 15 * 60 * 1000;
const MAX_ACTIVE_ACCESS_TOKENS_PER_DEVICE: usize = 4;
const CLIENT_PRESENCE_TOUCH_INTERVAL_MS: i64 = 15 * 1000;
const DISPLAY_CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const RANDOM_GENERATION_ATTEMPTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceAccessIdentity {
    pub device_id: String,
    pub role: DeviceRole,
    pub grants_json: String,
    pub protocol_version: ProtocolVersion,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PairingServiceError {
    #[error("device pairing is disabled")]
    Disabled,
    #[error("device enrollment is closed")]
    EnrollmentClosed,
    #[error("too many pending pairing requests")]
    TooManyPending,
    #[error("maximum paired devices reached: {limit}")]
    MaximumDevices { limit: usize },
    #[error("pairing claim is invalid: {0}")]
    InvalidClaim(String),
    #[error("device grant is invalid: {0}")]
    InvalidGrant(String),
    #[error("display code is invalid")]
    InvalidDisplayCode,
    #[error("pairing request was not found")]
    PairingNotFound,
    #[error("pairing request has expired")]
    PairingExpired,
    #[error("pairing request is not pending: {0}")]
    PairingNotPending(String),
    #[error("this device credential already has a pairing claim")]
    CredentialAlreadyClaimed,
    #[error("pairing polling credential is invalid")]
    InvalidPollingCredential,
    #[error("device credential or access token is invalid")]
    InvalidDeviceCredential,
    #[error("device was not found")]
    DeviceNotFound,
    #[error("device is not active: {0}")]
    DeviceNotActive(String),
    #[error("device registry is temporarily unavailable")]
    StorageUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairingEnrollmentStatus {
    pub open: bool,
    pub expires_at_ms: Option<i64>,
}

pub struct HubPairingService {
    config: PairingConfig,
    store: DeviceStore,
    mutation_lock: Mutex<()>,
    enrollment_open_until_ms: AtomicI64,
}

impl HubPairingService {
    pub fn new(config: PairingConfig, store: DeviceStore) -> Self {
        Self {
            config,
            store,
            mutation_lock: Mutex::new(()),
            enrollment_open_until_ms: AtomicI64::new(0),
        }
    }

    pub fn hub_enabled(&self) -> bool {
        self.config.hub_enabled
    }

    pub fn open_enrollment_window(
        &self,
        duration_secs: u64,
    ) -> Result<PairingEnrollmentStatus, PairingServiceError> {
        self.open_enrollment_window_at(duration_secs, chrono::Utc::now().timestamp_millis())
    }

    fn open_enrollment_window_at(
        &self,
        duration_secs: u64,
        now_ms: i64,
    ) -> Result<PairingEnrollmentStatus, PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        let duration_secs = duration_secs.clamp(MIN_PAIRING_EXPIRY_SECS, MAX_PAIRING_EXPIRY_SECS);
        let expires_at_ms = now_ms.saturating_add((duration_secs as i64).saturating_mul(1000));
        self.enrollment_open_until_ms
            .store(expires_at_ms, Ordering::Release);
        Ok(PairingEnrollmentStatus {
            open: true,
            expires_at_ms: Some(expires_at_ms),
        })
    }

    pub fn close_enrollment_window(&self) -> Result<PairingEnrollmentStatus, PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        self.enrollment_open_until_ms.store(0, Ordering::Release);
        Ok(PairingEnrollmentStatus {
            open: false,
            expires_at_ms: None,
        })
    }

    pub fn enrollment_status(&self) -> Result<PairingEnrollmentStatus, PairingServiceError> {
        self.enrollment_status_at(chrono::Utc::now().timestamp_millis())
    }

    fn enrollment_status_at(
        &self,
        now_ms: i64,
    ) -> Result<PairingEnrollmentStatus, PairingServiceError> {
        self.ensure_enabled()?;
        let expires_at_ms = self.enrollment_open_until_ms.load(Ordering::Acquire);
        if expires_at_ms > now_ms {
            Ok(PairingEnrollmentStatus {
                open: true,
                expires_at_ms: Some(expires_at_ms),
            })
        } else {
            if expires_at_ms != 0 {
                let _ = self.enrollment_open_until_ms.compare_exchange(
                    expires_at_ms,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
            Ok(PairingEnrollmentStatus {
                open: false,
                expires_at_ms: None,
            })
        }
    }

    pub fn create_claim(
        &self,
        claim: &DevicePairingClaim,
    ) -> Result<PairingChallenge, PairingServiceError> {
        self.create_claim_at(claim, chrono::Utc::now().timestamp_millis())
    }

    fn create_claim_at(
        &self,
        claim: &DevicePairingClaim,
        now_ms: i64,
    ) -> Result<PairingChallenge, PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        if !self.enrollment_status_at(now_ms)?.open {
            return Err(PairingServiceError::EnrollmentClosed);
        }
        claim
            .validate()
            .map_err(|error| PairingServiceError::InvalidClaim(error.to_string()))?;
        let negotiated = HUB_NODE_PROTOCOL_VERSION
            .negotiate(claim.protocol_version)
            .map_err(|error| PairingServiceError::InvalidClaim(error.to_string()))?;
        let capabilities_json = serde_json::to_string(&claim.capabilities)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        let requested_grants_json = serde_json::to_string(&claim.requested_grants)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;

        if let Some(existing) = self
            .store
            .pairing_by_credential_digest(&claim.credential_sha256, now_ms)
            .map_err(map_store_error)?
        {
            if existing.status != "pending"
                || !pairing_claim_matches(
                    &existing,
                    claim,
                    negotiated,
                    &capabilities_json,
                    &requested_grants_json,
                )
            {
                return Err(PairingServiceError::CredentialAlreadyClaimed);
            }
            for _ in 0..RANDOM_GENERATION_ATTEMPTS {
                let display_code = random_display_code();
                let polling_secret = random_secret_hex();
                match self.store.rotate_pending_pairing_challenge(
                    &existing.request_id,
                    &sha256_hex(display_code.as_bytes()),
                    &sha256_hex(polling_secret.as_bytes()),
                    now_ms,
                ) {
                    Ok(()) => {
                        return Ok(PairingChallenge {
                            request_id: existing.request_id,
                            approval_path: format!("/devices/pair?code={display_code}"),
                            display_code,
                            polling_secret,
                            expires_at_ms: existing.expires_at_ms,
                            protocol_version: negotiated,
                        });
                    }
                    Err(DeviceStoreError::Database(error)) if is_constraint_error(&error) => {
                        continue;
                    }
                    Err(error) => return Err(map_store_error(error)),
                }
            }
            return Err(PairingServiceError::StorageUnavailable);
        }

        self.ensure_device_capacity()?;
        if self
            .store
            .pending_pairings(now_ms)
            .map_err(map_store_error)?
            .len()
            >= MAX_PENDING_REQUESTS
        {
            return Err(PairingServiceError::TooManyPending);
        }

        let expiry_secs = self
            .config
            .token_expiry_secs
            .clamp(MIN_PAIRING_EXPIRY_SECS, MAX_PAIRING_EXPIRY_SECS);
        let expires_at_ms = now_ms.saturating_add((expiry_secs as i64).saturating_mul(1000));

        for _ in 0..RANDOM_GENERATION_ATTEMPTS {
            let request_id = uuid::Uuid::new_v4().to_string();
            let display_code = random_display_code();
            let polling_secret = random_secret_hex();
            let request = NewPairingRequest {
                request_id: request_id.clone(),
                display_code_sha256: sha256_hex(display_code.as_bytes()),
                polling_secret_sha256: sha256_hex(polling_secret.as_bytes()),
                credential_sha256: claim.credential_sha256.clone(),
                display_name: claim.display_name.clone(),
                role: role_name(claim.role).to_string(),
                platform: claim.platform.clone(),
                captain_version: claim.capabilities.captain_version.clone(),
                protocol_major: negotiated.major,
                protocol_minor: negotiated.minor,
                capabilities_json: capabilities_json.clone(),
                requested_grants_json: requested_grants_json.clone(),
                created_at_ms: now_ms,
                expires_at_ms,
            };
            match self.store.create_pairing_request(&request) {
                Ok(()) => {
                    return Ok(PairingChallenge {
                        request_id,
                        approval_path: format!("/devices/pair?code={display_code}"),
                        display_code,
                        polling_secret,
                        expires_at_ms,
                        protocol_version: negotiated,
                    });
                }
                Err(DeviceStoreError::DuplicateCredential) => {
                    return Err(PairingServiceError::CredentialAlreadyClaimed);
                }
                Err(DeviceStoreError::Database(error)) if is_constraint_error(&error) => continue,
                Err(error) => return Err(map_store_error(error)),
            }
        }
        Err(PairingServiceError::StorageUnavailable)
    }

    pub fn pending_requests(&self) -> Result<Vec<PairingRequestSummary>, PairingServiceError> {
        self.pending_requests_at(chrono::Utc::now().timestamp_millis())
    }

    /// Resolve a one-time display code into its sanitized pending request so
    /// an operator can review the exact capabilities and requested grant
    /// before making a decision. The code itself is never persisted or
    /// returned by this method.
    pub fn review_display_code(
        &self,
        display_code: &str,
    ) -> Result<PairingRequestSummary, PairingServiceError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.ensure_enabled()?;
        let normalized = normalize_display_code(display_code)?;
        let request_id = self
            .store
            .request_id_for_display_code_digest(&sha256_hex(normalized.as_bytes()), now_ms)
            .map_err(map_store_error)?
            .ok_or(PairingServiceError::PairingNotFound)?;
        self.store
            .pairing_by_request_id(&request_id, now_ms)
            .map_err(map_store_error)?
            .filter(|request| request.status == "pending")
            .ok_or(PairingServiceError::PairingNotFound)
    }

    fn pending_requests_at(
        &self,
        now_ms: i64,
    ) -> Result<Vec<PairingRequestSummary>, PairingServiceError> {
        self.ensure_enabled()?;
        self.store.pending_pairings(now_ms).map_err(map_store_error)
    }

    pub fn approve_request(
        &self,
        request_id: &str,
        grant: &DeviceGrant,
    ) -> Result<DeviceRecord, PairingServiceError> {
        self.approve_request_at(request_id, grant, chrono::Utc::now().timestamp_millis())
    }

    fn approve_request_at(
        &self,
        request_id: &str,
        grant: &DeviceGrant,
        now_ms: i64,
    ) -> Result<DeviceRecord, PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        let summary = self
            .store
            .pairing_by_request_id(request_id, now_ms)
            .map_err(map_store_error)?
            .ok_or(PairingServiceError::PairingNotFound)?;
        match summary.status.as_str() {
            "approved" => {
                let device_id = summary
                    .approved_device_id
                    .ok_or(PairingServiceError::StorageUnavailable)?;
                return self
                    .store
                    .get_device(&device_id)
                    .map_err(map_store_error)?
                    .ok_or(PairingServiceError::DeviceNotFound);
            }
            "expired" => return Err(PairingServiceError::PairingExpired),
            "pending" => {}
            status => return Err(PairingServiceError::PairingNotPending(status.to_string())),
        }
        self.ensure_device_capacity()?;

        let capabilities = serde_json::from_str(&summary.capabilities_json)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        let requested: DeviceGrant = serde_json::from_str(&summary.requested_grants_json)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        grant
            .validate_against(&capabilities)
            .map_err(|error| PairingServiceError::InvalidGrant(error.to_string()))?;
        validate_grant_subset(grant, &requested)?;
        let grant_json =
            serde_json::to_string(grant).map_err(|_| PairingServiceError::StorageUnavailable)?;
        let device_id = format!("{}-{}", summary.role, uuid::Uuid::new_v4());
        self.store
            .approve_pairing(request_id, &device_id, &grant_json, now_ms)
            .map_err(map_store_error)
    }

    pub fn approve_display_code(
        &self,
        display_code: &str,
        grant: &DeviceGrant,
    ) -> Result<DeviceRecord, PairingServiceError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.ensure_enabled()?;
        let normalized = normalize_display_code(display_code)?;
        let request_id = self
            .store
            .request_id_for_display_code_digest(&sha256_hex(normalized.as_bytes()), now_ms)
            .map_err(map_store_error)?
            .ok_or(PairingServiceError::PairingNotFound)?;
        self.approve_request_at(&request_id, grant, now_ms)
    }

    pub fn deny_request(&self, request_id: &str) -> Result<(), PairingServiceError> {
        self.ensure_enabled()?;
        self.store
            .deny_pairing(request_id, chrono::Utc::now().timestamp_millis())
            .map_err(map_store_error)
    }

    pub fn poll(
        &self,
        request: &PairingPollRequest,
    ) -> Result<PairingPollResponse, PairingServiceError> {
        self.poll_at(request, chrono::Utc::now().timestamp_millis())
    }

    fn poll_at(
        &self,
        request: &PairingPollRequest,
        now_ms: i64,
    ) -> Result<PairingPollResponse, PairingServiceError> {
        self.ensure_enabled()?;
        request
            .validate()
            .map_err(|_| PairingServiceError::InvalidPollingCredential)?;
        let result = self
            .store
            .poll_pairing(
                &request.request_id,
                &sha256_hex(request.polling_secret.as_bytes()),
                now_ms,
            )
            .map_err(map_store_error)?;
        let mut status = match result.status {
            PairingPollStatus::Pending => PairingState::Pending,
            PairingPollStatus::Approved => PairingState::Approved,
            PairingPollStatus::Denied => PairingState::Denied,
            PairingPollStatus::Expired => PairingState::Expired,
        };
        let mut device_id = result.device_id;
        let approved_grants = if status == PairingState::Approved {
            let approved_device_id = device_id
                .as_deref()
                .ok_or(PairingServiceError::StorageUnavailable)?;
            match self
                .store
                .get_device(approved_device_id)
                .map_err(map_store_error)?
            {
                Some(device) if device.status == "active" => {
                    let grant: DeviceGrant = serde_json::from_str(&device.grants_json)
                        .map_err(|_| PairingServiceError::StorageUnavailable)?;
                    grant
                        .validate_shape()
                        .map_err(|_| PairingServiceError::StorageUnavailable)?;
                    Some(grant)
                }
                _ => {
                    status = PairingState::Denied;
                    device_id = None;
                    None
                }
            }
        } else {
            None
        };
        Ok(PairingPollResponse {
            status,
            device_id,
            approved_grants,
            expires_at_ms: result.expires_at_ms,
        })
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>, PairingServiceError> {
        self.ensure_enabled()?;
        self.store.list_devices().map_err(map_store_error)
    }

    pub fn revoke_device(&self, device_id: &str) -> Result<(), PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        self.store
            .revoke_device(device_id, chrono::Utc::now().timestamp_millis())
            .map_err(map_store_error)?;
        Ok(())
    }

    pub fn exchange_device_credential(
        &self,
        request: &DeviceCredentialExchange,
    ) -> Result<DeviceAccessToken, PairingServiceError> {
        self.exchange_device_credential_at(request, chrono::Utc::now().timestamp_millis())
    }

    fn exchange_device_credential_at(
        &self,
        request: &DeviceCredentialExchange,
        now_ms: i64,
    ) -> Result<DeviceAccessToken, PairingServiceError> {
        self.ensure_enabled()?;
        let _mutation_guard = self.lock_mutations()?;
        request
            .validate()
            .map_err(|_| PairingServiceError::InvalidDeviceCredential)?;
        let credential_digest = sha256_hex(request.credential.as_bytes());
        self.store
            .verify_device_credential_digest(&request.device_id, &credential_digest)
            .map_err(map_store_error)?;
        let device = self
            .store
            .get_device(&request.device_id)
            .map_err(map_store_error)?
            .ok_or(PairingServiceError::DeviceNotFound)?;
        let approved_grants: DeviceGrant = serde_json::from_str(&device.grants_json)
            .map_err(|_| PairingServiceError::StorageUnavailable)?;
        approved_grants
            .validate_shape()
            .map_err(|_| PairingServiceError::StorageUnavailable)?;

        let raw_token = random_secret_hex();
        let expires_at_ms = now_ms.saturating_add(ACCESS_TOKEN_TTL_MS);
        self.store
            .issue_access_token_digest(
                &request.device_id,
                &sha256_hex(raw_token.as_bytes()),
                now_ms,
                expires_at_ms,
                MAX_ACTIVE_ACCESS_TOKENS_PER_DEVICE,
            )
            .map_err(map_store_error)?;
        Ok(DeviceAccessToken {
            access_token: raw_token,
            token_type: "Bearer".to_string(),
            issued_at_ms: now_ms,
            expires_at_ms,
            protocol_version: ProtocolVersion {
                major: device.protocol_major,
                minor: device.protocol_minor,
            },
            approved_grants,
        })
    }

    pub fn authenticate_access_token(
        &self,
        raw_token: &str,
    ) -> Result<DeviceAccessIdentity, PairingServiceError> {
        self.authenticate_access_token_at(raw_token, chrono::Utc::now().timestamp_millis())
    }

    fn authenticate_access_token_at(
        &self,
        raw_token: &str,
        now_ms: i64,
    ) -> Result<DeviceAccessIdentity, PairingServiceError> {
        self.ensure_enabled()?;
        if !is_raw_secret(raw_token) {
            return Err(PairingServiceError::InvalidDeviceCredential);
        }
        let digest = sha256_hex(raw_token.as_bytes());
        let token = self
            .store
            .authenticate_access_token_digest(&digest, now_ms)
            .map_err(map_store_error)?;
        Ok(DeviceAccessIdentity {
            device_id: token.device_id,
            role: parse_role(&token.role)?,
            grants_json: token.grants_json,
            protocol_version: ProtocolVersion {
                major: token.protocol_major,
                minor: token.protocol_minor,
            },
        })
    }

    /// Authenticate a lightweight Client for the scoped Hub work API.
    /// Node credentials are intentionally rejected even though both roles use
    /// the same short-lived token format.
    pub fn authenticate_client_access_token(
        &self,
        raw_token: &str,
    ) -> Result<DeviceAccessIdentity, PairingServiceError> {
        self.authenticate_client_access_token_at(raw_token, chrono::Utc::now().timestamp_millis())
    }

    fn authenticate_client_access_token_at(
        &self,
        raw_token: &str,
        now_ms: i64,
    ) -> Result<DeviceAccessIdentity, PairingServiceError> {
        let identity = self.authenticate_access_token_at(raw_token, now_ms)?;
        if identity.role != DeviceRole::Client {
            return Err(PairingServiceError::InvalidDeviceCredential);
        }
        self.store
            .touch_active_client_presence(
                &identity.device_id,
                now_ms,
                CLIENT_PRESENCE_TOUCH_INTERVAL_MS,
            )
            .map_err(map_store_error)?;
        Ok(identity)
    }

    fn ensure_enabled(&self) -> Result<(), PairingServiceError> {
        if self.config.hub_enabled {
            Ok(())
        } else {
            Err(PairingServiceError::Disabled)
        }
    }

    fn ensure_device_capacity(&self) -> Result<(), PairingServiceError> {
        let active = self
            .store
            .list_devices()
            .map_err(map_store_error)?
            .into_iter()
            .filter(|device| device.status == "active")
            .count();
        if active >= self.config.max_devices {
            Err(PairingServiceError::MaximumDevices {
                limit: self.config.max_devices,
            })
        } else {
            Ok(())
        }
    }

    fn lock_mutations(&self) -> Result<MutexGuard<'_, ()>, PairingServiceError> {
        self.mutation_lock.lock().map_err(|error| {
            tracing::error!(error = %error, "Hub pairing mutation lock is unavailable");
            PairingServiceError::StorageUnavailable
        })
    }

    #[cfg(test)]
    fn active_access_token_count_at(&self, now_ms: i64) -> usize {
        self.store.active_access_token_count(now_ms).unwrap()
    }
}

fn pairing_claim_matches(
    existing: &PairingRequestSummary,
    claim: &DevicePairingClaim,
    negotiated: ProtocolVersion,
    capabilities_json: &str,
    requested_grants_json: &str,
) -> bool {
    existing.display_name == claim.display_name
        && existing.role == role_name(claim.role)
        && existing.platform == claim.platform
        && existing.captain_version == claim.capabilities.captain_version
        && existing.protocol_major == negotiated.major
        && existing.protocol_minor == negotiated.minor
        && existing.capabilities_json == capabilities_json
        && existing.requested_grants_json == requested_grants_json
}

fn validate_grant_subset(
    approved: &DeviceGrant,
    requested: &DeviceGrant,
) -> Result<(), PairingServiceError> {
    approved
        .validate_subset_of(requested)
        .map_err(|error| PairingServiceError::InvalidGrant(error.to_string()))
}

fn normalize_display_code(value: &str) -> Result<String, PairingServiceError> {
    let compact = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && *byte != b'-')
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    if compact.len() != 8
        || compact
            .iter()
            .any(|byte| !DISPLAY_CODE_ALPHABET.contains(byte))
    {
        return Err(PairingServiceError::InvalidDisplayCode);
    }
    let first =
        std::str::from_utf8(&compact[..4]).map_err(|_| PairingServiceError::InvalidDisplayCode)?;
    let second =
        std::str::from_utf8(&compact[4..]).map_err(|_| PairingServiceError::InvalidDisplayCode)?;
    Ok(format!("{first}-{second}"))
}

fn random_display_code() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut code = String::with_capacity(9);
    for (index, byte) in bytes.into_iter().enumerate() {
        if index == 4 {
            code.push('-');
        }
        code.push(DISPLAY_CODE_ALPHABET[(byte as usize) % DISPLAY_CODE_ALPHABET.len()] as char);
    }
    code
}

fn random_secret_hex() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn is_raw_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn role_name(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Client => "client",
        DeviceRole::Node => "node",
    }
}

fn parse_role(value: &str) -> Result<DeviceRole, PairingServiceError> {
    match value {
        "client" => Ok(DeviceRole::Client),
        "node" => Ok(DeviceRole::Node),
        _ => Err(PairingServiceError::StorageUnavailable),
    }
}

fn is_constraint_error(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

fn map_store_error(error: DeviceStoreError) -> PairingServiceError {
    match error {
        DeviceStoreError::DeviceNotFound => PairingServiceError::DeviceNotFound,
        DeviceStoreError::PairingNotFound => PairingServiceError::PairingNotFound,
        DeviceStoreError::PairingExpired => PairingServiceError::PairingExpired,
        DeviceStoreError::PairingNotPending(status) => {
            PairingServiceError::PairingNotPending(status)
        }
        DeviceStoreError::DuplicateCredential => PairingServiceError::CredentialAlreadyClaimed,
        DeviceStoreError::InvalidPollingCredential => PairingServiceError::InvalidPollingCredential,
        DeviceStoreError::InvalidDeviceCredential => PairingServiceError::InvalidDeviceCredential,
        DeviceStoreError::DeviceNotActive(status) => PairingServiceError::DeviceNotActive(status),
        DeviceStoreError::Lock(_) | DeviceStoreError::Database(_) => {
            tracing::error!(error = %error, "Hub device registry operation failed");
            PairingServiceError::StorageUnavailable
        }
    }
}

#[cfg(test)]
#[path = "hub_pairing_service_tests.rs"]
mod tests;
