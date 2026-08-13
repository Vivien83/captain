//! Durable Hub device registry and pairing claims.

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};
use subtle::ConstantTimeEq;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device_id: String,
    pub display_name: String,
    pub role: String,
    pub platform: String,
    pub captain_version: String,
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub capabilities_json: String,
    pub grants_json: String,
    pub status: String,
    pub paired_at_ms: i64,
    pub last_seen_ms: i64,
    pub updated_at_ms: i64,
    pub last_transport: Option<String>,
    pub last_error_code: Option<String>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NewPairingRequest {
    pub request_id: String,
    pub display_code_sha256: String,
    pub polling_secret_sha256: String,
    pub credential_sha256: String,
    pub display_name: String,
    pub role: String,
    pub platform: String,
    pub captain_version: String,
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub capabilities_json: String,
    pub requested_grants_json: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
}

impl std::fmt::Debug for NewPairingRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewPairingRequest")
            .field("request_id", &self.request_id)
            .field("display_name", &self.display_name)
            .field("role", &self.role)
            .field("platform", &self.platform)
            .field("captain_version", &self.captain_version)
            .field("protocol_major", &self.protocol_major)
            .field("protocol_minor", &self.protocol_minor)
            .field("created_at_ms", &self.created_at_ms)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequestSummary {
    pub request_id: String,
    pub display_name: String,
    pub role: String,
    pub platform: String,
    pub captain_version: String,
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub capabilities_json: String,
    pub requested_grants_json: String,
    pub status: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub decided_at_ms: Option<i64>,
    pub approved_device_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingPollStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPollResult {
    pub status: PairingPollStatus,
    pub device_id: Option<String>,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAccessTokenRecord {
    pub device_id: String,
    pub role: String,
    pub grants_json: String,
    pub protocol_major: u16,
    pub protocol_minor: u16,
}

#[derive(Debug, Error)]
pub enum DeviceStoreError {
    #[error("device not found")]
    DeviceNotFound,
    #[error("pairing request not found")]
    PairingNotFound,
    #[error("pairing request expired")]
    PairingExpired,
    #[error("pairing request is {0}")]
    PairingNotPending(String),
    #[error("device credential is already registered")]
    DuplicateCredential,
    #[error("pairing polling credential is invalid")]
    InvalidPollingCredential,
    #[error("device credential is invalid or revoked")]
    InvalidDeviceCredential,
    #[error("device is not active: {0}")]
    DeviceNotActive(String),
    #[error("device store lock failed: {0}")]
    Lock(String),
    #[error("device store database error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct DeviceStore {
    conn: Arc<Mutex<Connection>>,
}

impl DeviceStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn list_devices(&self) -> Result<Vec<DeviceRecord>, DeviceStoreError> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT device_id, display_name, role, platform, captain_version,
                    protocol_major, protocol_minor, capabilities_json,
                    grants_json, status, paired_at_ms,
                    last_seen_ms, updated_at_ms, last_transport,
                    last_error_code, revoked_at_ms
             FROM captain_devices
             ORDER BY display_name COLLATE NOCASE, device_id",
        )?;
        let rows = statement.query_map([], device_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_device(&self, device_id: &str) -> Result<Option<DeviceRecord>, DeviceStoreError> {
        let conn = self.lock()?;
        get_device_on(&conn, device_id).map_err(Into::into)
    }

    pub fn create_pairing_request(
        &self,
        request: &NewPairingRequest,
    ) -> Result<(), DeviceStoreError> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction()?;
        let registered = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM captain_devices WHERE credential_sha256 = ?1
             )",
            [request.credential_sha256.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        let requested = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM device_pairing_requests WHERE credential_sha256 = ?1
             )",
            [request.credential_sha256.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if registered || requested {
            return Err(DeviceStoreError::DuplicateCredential);
        }
        transaction.execute(
            "INSERT INTO device_pairing_requests (
                 request_id, display_code_sha256, polling_secret_sha256,
                 credential_sha256, display_name, role, platform,
                 captain_version, protocol_major, protocol_minor,
                 capabilities_json, requested_grants_json, status,
                 created_at_ms, expires_at_ms
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 'pending', ?13, ?14
             )",
            params![
                request.request_id,
                request.display_code_sha256,
                request.polling_secret_sha256,
                request.credential_sha256,
                request.display_name,
                request.role,
                request.platform,
                request.captain_version,
                request.protocol_major,
                request.protocol_minor,
                request.capabilities_json,
                request.requested_grants_json,
                request.created_at_ms,
                request.expires_at_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn pending_pairings(
        &self,
        now_ms: i64,
    ) -> Result<Vec<PairingRequestSummary>, DeviceStoreError> {
        self.expire_pending_pairings(now_ms)?;
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT request_id, display_name, role, platform, captain_version,
                    protocol_major, protocol_minor, capabilities_json,
                    requested_grants_json, status, created_at_ms,
                    expires_at_ms, decided_at_ms, approved_device_id
             FROM device_pairing_requests
             WHERE status = 'pending'
             ORDER BY created_at_ms, request_id",
        )?;
        let rows = statement.query_map([], pairing_summary_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn pairing_by_request_id(
        &self,
        request_id: &str,
        now_ms: i64,
    ) -> Result<Option<PairingRequestSummary>, DeviceStoreError> {
        self.expire_pending_pairings(now_ms)?;
        let conn = self.lock()?;
        pairing_summary_by_request_id(&conn, request_id).map_err(Into::into)
    }

    pub fn pairing_by_credential_digest(
        &self,
        credential_sha256: &str,
        now_ms: i64,
    ) -> Result<Option<PairingRequestSummary>, DeviceStoreError> {
        self.expire_pending_pairings(now_ms)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT request_id, display_name, role, platform, captain_version,
                    protocol_major, protocol_minor, capabilities_json,
                    requested_grants_json, status, created_at_ms,
                    expires_at_ms, decided_at_ms, approved_device_id
             FROM device_pairing_requests WHERE credential_sha256 = ?1",
            [credential_sha256],
            pairing_summary_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn rotate_pending_pairing_challenge(
        &self,
        request_id: &str,
        display_code_sha256: &str,
        polling_secret_sha256: &str,
        now_ms: i64,
    ) -> Result<(), DeviceStoreError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE device_pairing_requests
             SET display_code_sha256 = ?2, polling_secret_sha256 = ?3
             WHERE request_id = ?1 AND status = 'pending' AND expires_at_ms > ?4",
            params![
                request_id,
                display_code_sha256,
                polling_secret_sha256,
                now_ms,
            ],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let pairing = pairing_private_by_request_id(&conn, request_id)?
            .ok_or(DeviceStoreError::PairingNotFound)?;
        if pairing.status == "pending" && pairing.expires_at_ms <= now_ms {
            return Err(DeviceStoreError::PairingExpired);
        }
        Err(DeviceStoreError::PairingNotPending(pairing.status))
    }

    pub fn request_id_for_display_code_digest(
        &self,
        display_code_sha256: &str,
        now_ms: i64,
    ) -> Result<Option<String>, DeviceStoreError> {
        self.expire_pending_pairings(now_ms)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT request_id FROM device_pairing_requests
             WHERE display_code_sha256 = ?1 AND status = 'pending'",
            [display_code_sha256],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn approve_pairing(
        &self,
        request_id: &str,
        device_id: &str,
        approved_grants_json: &str,
        now_ms: i64,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction()?;
        let pairing = pairing_private_by_request_id(&transaction, request_id)?
            .ok_or(DeviceStoreError::PairingNotFound)?;

        if pairing.status == "approved" {
            let approved_id = pairing
                .approved_device_id
                .ok_or_else(|| DeviceStoreError::PairingNotPending("invalid".to_string()))?;
            let device = get_device_on(&transaction, &approved_id)?
                .ok_or(DeviceStoreError::DeviceNotFound)?;
            transaction.commit()?;
            return Ok(device);
        }
        if pairing.status != "pending" {
            return Err(DeviceStoreError::PairingNotPending(pairing.status));
        }
        if pairing.expires_at_ms <= now_ms {
            mark_pairing_expired(&transaction, request_id, now_ms)?;
            transaction.commit()?;
            return Err(DeviceStoreError::PairingExpired);
        }
        let credential_exists = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM captain_devices WHERE credential_sha256 = ?1
             )",
            [pairing.credential_sha256.as_str()],
            |row| row.get::<_, bool>(0),
        )?;
        if credential_exists {
            return Err(DeviceStoreError::DuplicateCredential);
        }

        transaction.execute(
            "INSERT INTO captain_devices (
                 device_id, display_name, role, platform, captain_version,
                 protocol_major, protocol_minor, credential_sha256,
                 capabilities_json, grants_json, status, paired_at_ms,
                 last_seen_ms, updated_at_ms
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active',
                 ?11, ?11, ?11
             )",
            params![
                device_id,
                pairing.display_name,
                pairing.role,
                pairing.platform,
                pairing.captain_version,
                pairing.protocol_major,
                pairing.protocol_minor,
                pairing.credential_sha256,
                pairing.capabilities_json,
                approved_grants_json,
                now_ms,
            ],
        )?;
        transaction.execute(
            "UPDATE device_pairing_requests
             SET status = 'approved', decided_at_ms = ?2, approved_device_id = ?3
             WHERE request_id = ?1 AND status = 'pending'",
            params![request_id, now_ms, device_id],
        )?;
        let device =
            get_device_on(&transaction, device_id)?.ok_or(DeviceStoreError::DeviceNotFound)?;
        transaction.commit()?;
        Ok(device)
    }

    pub fn deny_pairing(&self, request_id: &str, now_ms: i64) -> Result<(), DeviceStoreError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE device_pairing_requests
             SET status = CASE WHEN expires_at_ms <= ?2 THEN 'expired' ELSE 'denied' END,
                 decided_at_ms = ?2
             WHERE request_id = ?1 AND status = 'pending'",
            params![request_id, now_ms],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let status = conn
            .query_row(
                "SELECT status FROM device_pairing_requests WHERE request_id = ?1",
                [request_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        match status {
            Some(status) => Err(DeviceStoreError::PairingNotPending(status)),
            None => Err(DeviceStoreError::PairingNotFound),
        }
    }

    pub fn poll_pairing(
        &self,
        request_id: &str,
        polling_secret_sha256: &str,
        now_ms: i64,
    ) -> Result<PairingPollResult, DeviceStoreError> {
        self.expire_pending_pairings(now_ms)?;
        let conn = self.lock()?;
        let pairing = pairing_private_by_request_id(&conn, request_id)?
            .ok_or(DeviceStoreError::PairingNotFound)?;
        if !constant_time_digest_eq(&pairing.polling_secret_sha256, polling_secret_sha256) {
            return Err(DeviceStoreError::InvalidPollingCredential);
        }
        let status = match pairing.status.as_str() {
            "pending" => PairingPollStatus::Pending,
            "approved" => PairingPollStatus::Approved,
            "denied" => PairingPollStatus::Denied,
            "expired" => PairingPollStatus::Expired,
            other => return Err(DeviceStoreError::PairingNotPending(other.to_string())),
        };
        Ok(PairingPollResult {
            status,
            device_id: pairing.approved_device_id,
            expires_at_ms: pairing.expires_at_ms,
        })
    }

    pub fn verify_device_credential_digest(
        &self,
        device_id: &str,
        credential_sha256: &str,
    ) -> Result<(), DeviceStoreError> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT credential_sha256, status FROM captain_devices WHERE device_id = ?1",
                [device_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((Some(stored), status)) = row else {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        };
        if status != "active" || !constant_time_digest_eq(&stored, credential_sha256) {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        Ok(())
    }

    /// Persist a short-lived bearer verifier while retaining only the newest
    /// bounded set for this device. The raw bearer token never reaches this
    /// store.
    pub fn issue_access_token_digest(
        &self,
        device_id: &str,
        token_sha256: &str,
        issued_at_ms: i64,
        expires_at_ms: i64,
        max_active_tokens: usize,
    ) -> Result<(), DeviceStoreError> {
        if !is_sha256_digest(token_sha256)
            || issued_at_ms < 0
            || expires_at_ms <= issued_at_ms
            || max_active_tokens == 0
        {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        let keep_before_insert = i64::try_from(max_active_tokens.saturating_sub(1))
            .map_err(|_| DeviceStoreError::InvalidDeviceCredential)?;
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM captain_devices
                 WHERE device_id = ?1 AND status = 'active'
             )",
            [device_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !active {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        tx.execute(
            "DELETE FROM device_access_tokens WHERE expires_at_ms <= ?1",
            [issued_at_ms],
        )?;
        tx.execute(
            "DELETE FROM device_access_tokens
             WHERE token_sha256 IN (
                 SELECT token_sha256 FROM device_access_tokens
                 WHERE device_id = ?1
                 ORDER BY issued_at_ms DESC, expires_at_ms DESC, token_sha256 DESC
                 LIMIT -1 OFFSET ?2
             )",
            params![device_id, keep_before_insert],
        )?;
        tx.execute(
            "INSERT INTO device_access_tokens (
                 token_sha256, device_id, issued_at_ms, expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4)",
            params![token_sha256, device_id, issued_at_ms, expires_at_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn authenticate_access_token_digest(
        &self,
        token_sha256: &str,
        now_ms: i64,
    ) -> Result<DeviceAccessTokenRecord, DeviceStoreError> {
        if !is_sha256_digest(token_sha256) || now_ms < 0 {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        let conn = self.lock()?;
        let record = conn
            .query_row(
                "SELECT token.device_id, device.role, device.grants_json,
                        device.protocol_major, device.protocol_minor
                 FROM device_access_tokens AS token
                 JOIN captain_devices AS device ON device.device_id = token.device_id
                 WHERE token.token_sha256 = ?1
                   AND token.expires_at_ms > ?2
                   AND device.status = 'active'",
                params![token_sha256, now_ms],
                |row| {
                    Ok(DeviceAccessTokenRecord {
                        device_id: row.get(0)?,
                        role: row.get(1)?,
                        grants_json: row.get(2)?,
                        protocol_major: row.get(3)?,
                        protocol_minor: row.get(4)?,
                    })
                },
            )
            .optional()?;
        record.ok_or(DeviceStoreError::InvalidDeviceCredential)
    }

    pub fn active_access_token_count(&self, now_ms: i64) -> Result<usize, DeviceStoreError> {
        if now_ms < 0 {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        let conn = self.lock()?;
        conn.query_row(
            "SELECT COUNT(*)
             FROM device_access_tokens AS token
             JOIN captain_devices AS device ON device.device_id = token.device_id
             WHERE token.expires_at_ms > ?1 AND device.status = 'active'",
            [now_ms],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub fn set_device_grants(
        &self,
        device_id: &str,
        grants_json: &str,
        now_ms: i64,
    ) -> Result<(), DeviceStoreError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE captain_devices
             SET grants_json = ?2, updated_at_ms = ?3
             WHERE device_id = ?1 AND status = 'active'",
            params![device_id, grants_json, now_ms],
        )?;
        active_device_update_result(&conn, device_id, changed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn touch_device(
        &self,
        device_id: &str,
        captain_version: &str,
        protocol_major: u16,
        protocol_minor: u16,
        capabilities_json: &str,
        transport: &str,
        last_error_code: Option<&str>,
        now_ms: i64,
    ) -> Result<(), DeviceStoreError> {
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE captain_devices
             SET captain_version = ?2, protocol_major = ?3, protocol_minor = ?4,
                 capabilities_json = ?5, last_transport = ?6,
                 last_error_code = ?7, last_seen_ms = MAX(last_seen_ms, ?8),
                 updated_at_ms = MAX(updated_at_ms, ?8)
             WHERE device_id = ?1 AND status = 'active'",
            params![
                device_id,
                captain_version,
                protocol_major,
                protocol_minor,
                capabilities_json,
                transport,
                last_error_code,
                now_ms,
            ],
        )?;
        active_device_update_result(&conn, device_id, changed)
    }

    /// Refresh a paired Client's presence without letting API traffic rewrite
    /// its advertised capabilities. Writes are throttled to keep ordinary UI
    /// polling from turning into database churn.
    pub fn touch_active_client_presence(
        &self,
        device_id: &str,
        now_ms: i64,
        minimum_interval_ms: i64,
    ) -> Result<bool, DeviceStoreError> {
        if now_ms < 0 {
            return Err(DeviceStoreError::InvalidDeviceCredential);
        }
        let cutoff_ms = now_ms.saturating_sub(minimum_interval_ms.max(0));
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE captain_devices
             SET last_error_code = NULL,
                 last_seen_ms = MAX(last_seen_ms, ?2),
                 updated_at_ms = MAX(updated_at_ms, ?2)
             WHERE device_id = ?1 AND status = 'active' AND role = 'client'
               AND last_seen_ms <= ?3",
            params![device_id, now_ms, cutoff_ms],
        )?;
        if changed == 1 {
            return Ok(true);
        }

        let active_client = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM captain_devices
                 WHERE device_id = ?1 AND status = 'active' AND role = 'client'
             )",
            [device_id],
            |row| row.get::<_, bool>(0),
        )?;
        if active_client {
            Ok(false)
        } else {
            Err(DeviceStoreError::InvalidDeviceCredential)
        }
    }

    pub fn revoke_device(&self, device_id: &str, now_ms: i64) -> Result<(), DeviceStoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if get_device_on(&tx, device_id)?.is_none() {
            return Err(DeviceStoreError::DeviceNotFound);
        }
        tx.execute(
            "UPDATE captain_devices
             SET status = 'revoked',
                 revoked_at_ms = COALESCE(revoked_at_ms, MAX(paired_at_ms, ?2)),
                 updated_at_ms = MAX(updated_at_ms, ?2),
                 last_error_code = 'device_revoked'
             WHERE device_id = ?1",
            params![device_id, now_ms],
        )?;
        tx.execute(
            "UPDATE hub_node_connections
             SET status = 'offline',
                 disconnected_at_ms = COALESCE(
                     disconnected_at_ms, MAX(connected_at_ms, ?2)
                 ),
                 last_error_code = 'device_revoked',
                 updated_at_ms = MAX(updated_at_ms, ?2)
             WHERE device_id = ?1",
            params![device_id, now_ms],
        )?;
        tx.execute(
            "UPDATE hub_node_outbox
             SET superseded_at_ms = COALESCE(
                 superseded_at_ms, MAX(created_at_ms, ?2)
             )
             WHERE device_id = ?1 AND acked_at_ms IS NULL",
            params![device_id, now_ms],
        )?;
        tx.execute(
            "UPDATE hub_node_runs
             SET status = 'cancelled', effect_state = 'completed',
                 lease_owner = NULL, lease_expires_at_ms = NULL,
                 error_code = 'device_revoked',
                 terminal_at_ms = MAX(created_at_ms, ?2),
                 updated_at_ms = MAX(updated_at_ms, ?2)
             WHERE device_id = ?1 AND (
                 status = 'queued' OR (
                     status IN ('leased', 'accepted', 'cancel_requested')
                     AND (effect = 'read_only' OR effect_state = 'not_started')
                 )
             )",
            params![device_id, now_ms],
        )?;
        tx.execute(
            "UPDATE hub_node_runs
             SET status = 'uncertain', effect_state = 'started',
                 lease_owner = NULL, lease_expires_at_ms = NULL,
                 error_code = 'device_revoked_effect_uncertain',
                 terminal_at_ms = MAX(created_at_ms, ?2),
                 updated_at_ms = MAX(updated_at_ms, ?2)
             WHERE device_id = ?1
               AND status IN ('leased', 'accepted', 'cancel_requested')",
            params![device_id, now_ms],
        )?;
        tx.execute(
            "DELETE FROM device_access_tokens WHERE device_id = ?1",
            [device_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn expire_pending_pairings(&self, now_ms: i64) -> Result<usize, DeviceStoreError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE device_pairing_requests
             SET status = 'expired', decided_at_ms = ?1
             WHERE status = 'pending' AND expires_at_ms <= ?1",
            [now_ms],
        )
        .map_err(Into::into)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DeviceStoreError> {
        self.conn
            .lock()
            .map_err(|error| DeviceStoreError::Lock(error.to_string()))
    }
}

struct PairingPrivate {
    polling_secret_sha256: String,
    credential_sha256: String,
    display_name: String,
    role: String,
    platform: String,
    captain_version: String,
    protocol_major: u16,
    protocol_minor: u16,
    capabilities_json: String,
    status: String,
    expires_at_ms: i64,
    approved_device_id: Option<String>,
}

fn get_device_on(conn: &Connection, device_id: &str) -> rusqlite::Result<Option<DeviceRecord>> {
    conn.query_row(
        "SELECT device_id, display_name, role, platform, captain_version,
                protocol_major, protocol_minor, capabilities_json,
                grants_json, status, paired_at_ms,
                last_seen_ms, updated_at_ms, last_transport,
                last_error_code, revoked_at_ms
         FROM captain_devices WHERE device_id = ?1",
        [device_id],
        device_from_row,
    )
    .optional()
}

fn device_from_row(row: &Row<'_>) -> rusqlite::Result<DeviceRecord> {
    Ok(DeviceRecord {
        device_id: row.get(0)?,
        display_name: row.get(1)?,
        role: row.get(2)?,
        platform: row.get(3)?,
        captain_version: row.get(4)?,
        protocol_major: row.get(5)?,
        protocol_minor: row.get(6)?,
        capabilities_json: row.get(7)?,
        grants_json: row.get(8)?,
        status: row.get(9)?,
        paired_at_ms: row.get(10)?,
        last_seen_ms: row.get(11)?,
        updated_at_ms: row.get(12)?,
        last_transport: row.get(13)?,
        last_error_code: row.get(14)?,
        revoked_at_ms: row.get(15)?,
    })
}

fn pairing_summary_by_request_id(
    conn: &Connection,
    request_id: &str,
) -> rusqlite::Result<Option<PairingRequestSummary>> {
    conn.query_row(
        "SELECT request_id, display_name, role, platform, captain_version,
                protocol_major, protocol_minor, capabilities_json,
                requested_grants_json, status, created_at_ms,
                expires_at_ms, decided_at_ms, approved_device_id
         FROM device_pairing_requests WHERE request_id = ?1",
        [request_id],
        pairing_summary_from_row,
    )
    .optional()
}

fn pairing_summary_from_row(row: &Row<'_>) -> rusqlite::Result<PairingRequestSummary> {
    Ok(PairingRequestSummary {
        request_id: row.get(0)?,
        display_name: row.get(1)?,
        role: row.get(2)?,
        platform: row.get(3)?,
        captain_version: row.get(4)?,
        protocol_major: row.get(5)?,
        protocol_minor: row.get(6)?,
        capabilities_json: row.get(7)?,
        requested_grants_json: row.get(8)?,
        status: row.get(9)?,
        created_at_ms: row.get(10)?,
        expires_at_ms: row.get(11)?,
        decided_at_ms: row.get(12)?,
        approved_device_id: row.get(13)?,
    })
}

fn pairing_private_by_request_id(
    conn: &Connection,
    request_id: &str,
) -> rusqlite::Result<Option<PairingPrivate>> {
    conn.query_row(
        "SELECT polling_secret_sha256, credential_sha256, display_name, role,
                platform, captain_version, protocol_major, protocol_minor,
                capabilities_json, status, expires_at_ms, approved_device_id
         FROM device_pairing_requests WHERE request_id = ?1",
        [request_id],
        |row| {
            Ok(PairingPrivate {
                polling_secret_sha256: row.get(0)?,
                credential_sha256: row.get(1)?,
                display_name: row.get(2)?,
                role: row.get(3)?,
                platform: row.get(4)?,
                captain_version: row.get(5)?,
                protocol_major: row.get(6)?,
                protocol_minor: row.get(7)?,
                capabilities_json: row.get(8)?,
                status: row.get(9)?,
                expires_at_ms: row.get(10)?,
                approved_device_id: row.get(11)?,
            })
        },
    )
    .optional()
}

fn mark_pairing_expired(
    transaction: &Transaction<'_>,
    request_id: &str,
    now_ms: i64,
) -> rusqlite::Result<()> {
    transaction.execute(
        "UPDATE device_pairing_requests
         SET status = 'expired', decided_at_ms = ?2
         WHERE request_id = ?1 AND status = 'pending'",
        params![request_id, now_ms],
    )?;
    Ok(())
}

fn active_device_update_result(
    conn: &Connection,
    device_id: &str,
    changed: usize,
) -> Result<(), DeviceStoreError> {
    if changed == 1 {
        return Ok(());
    }
    let status = conn
        .query_row(
            "SELECT status FROM captain_devices WHERE device_id = ?1",
            [device_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    match status {
        Some(status) => Err(DeviceStoreError::DeviceNotActive(status)),
        None => Err(DeviceStoreError::DeviceNotFound),
    }
}

fn constant_time_digest_eq(stored: &str, provided: &str) -> bool {
    stored.len() == provided.len() && bool::from(stored.as_bytes().ct_eq(provided.as_bytes()))
}

fn is_sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
