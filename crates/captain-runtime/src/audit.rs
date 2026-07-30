//! Versioned SHA-256 hash-chain audit trail for security-critical actions.
//!
//! Entries are append-only. Each epoch is independently verifiable and starts
//! from an explicit predecessor digest. If the active epoch is corrupt at boot,
//! Captain seals it as invalid and opens a new epoch with a `ChainRecovery`
//! entry. Existing audit rows are never rewritten.

use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

#[path = "audit_chain.rs"]
mod audit_chain;
#[path = "audit_persistence.rs"]
mod audit_persistence;

#[cfg(test)]
use audit_chain::compute_entry_hash;
use audit_chain::{
    build_entry, epoch_by_id, epoch_tip, invalid_epoch_ids, next_sequence, unique_active_epoch,
    verify_epoch,
};
use audit_persistence::{
    insert_entry, load_entries, load_epochs, lock_db, seal_epoch_and_open_recovery,
};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const LEGACY_HASH_VERSION: u8 = 1;
const CURRENT_HASH_VERSION: u8 = 2;
const MAX_INTEGRITY_ERROR_CHARS: usize = 512;

/// Categories of auditable actions within the agent runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    ToolInvoke,
    CapabilityCheck,
    AgentSpawn,
    AgentKill,
    AgentMessage,
    MemoryAccess,
    FileAccess,
    NetworkAccess,
    ShellExec,
    AuthAttempt,
    WireConnect,
    ConfigChange,
    LearningDecision,
    ApprovalDecision,
    ChainRecovery,
    /// Preserves future or third-party action names without changing meaning.
    Unknown(String),
}

impl AuditAction {
    fn from_stored(value: String) -> Self {
        match value.as_str() {
            "ToolInvoke" => Self::ToolInvoke,
            "CapabilityCheck" => Self::CapabilityCheck,
            "AgentSpawn" => Self::AgentSpawn,
            "AgentKill" => Self::AgentKill,
            "AgentMessage" => Self::AgentMessage,
            "MemoryAccess" => Self::MemoryAccess,
            "FileAccess" => Self::FileAccess,
            "NetworkAccess" => Self::NetworkAccess,
            "ShellExec" => Self::ShellExec,
            "AuthAttempt" => Self::AuthAttempt,
            "WireConnect" => Self::WireConnect,
            "ConfigChange" => Self::ConfigChange,
            "LearningDecision" => Self::LearningDecision,
            "ApprovalDecision" => Self::ApprovalDecision,
            "ChainRecovery" => Self::ChainRecovery,
            _ => Self::Unknown(value),
        }
    }
}

impl std::fmt::Display for AuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::ToolInvoke => "ToolInvoke",
            Self::CapabilityCheck => "CapabilityCheck",
            Self::AgentSpawn => "AgentSpawn",
            Self::AgentKill => "AgentKill",
            Self::AgentMessage => "AgentMessage",
            Self::MemoryAccess => "MemoryAccess",
            Self::FileAccess => "FileAccess",
            Self::NetworkAccess => "NetworkAccess",
            Self::ShellExec => "ShellExec",
            Self::AuthAttempt => "AuthAttempt",
            Self::WireConnect => "WireConnect",
            Self::ConfigChange => "ConfigChange",
            Self::LearningDecision => "LearningDecision",
            Self::ApprovalDecision => "ApprovalDecision",
            Self::ChainRecovery => "ChainRecovery",
            Self::Unknown(value) => value,
        };
        f.write_str(value)
    }
}

fn legacy_hash_version() -> u8 {
    LEGACY_HASH_VERSION
}

/// A single append-only entry in the audit hash chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number across all epochs.
    pub seq: u64,
    /// Epoch containing this entry.
    #[serde(default)]
    pub epoch: u64,
    /// Hash encoding version. Version 1 is retained only for compatibility.
    #[serde(default = "legacy_hash_version")]
    pub hash_version: u8,
    /// ISO-8601 timestamp of when this entry was recorded.
    pub timestamp: String,
    /// The agent that triggered (or is the subject of) this action.
    pub agent_id: String,
    /// The category of action being audited.
    pub action: AuditAction,
    /// Free-form detail about the action.
    pub detail: String,
    /// The outcome of the action.
    pub outcome: String,
    /// SHA-256 hash anchoring this entry.
    pub prev_hash: String,
    /// SHA-256 hash of this entry.
    pub hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EpochState {
    Active,
    Invalid,
}

impl EpochState {
    fn parse(value: &str) -> Result<Self, AuditError> {
        match value {
            "active" => Ok(Self::Active),
            "invalid" => Ok(Self::Invalid),
            _ => Err(AuditError::InvalidSchema(format!(
                "unknown audit epoch state {value:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
struct AuditEpoch {
    id: u64,
    start_seq: u64,
    started_at: String,
    predecessor_tip_hash: String,
    state: EpochState,
    terminal_hash: Option<String>,
    sealed_at: Option<String>,
    invalid_reason: Option<String>,
}

#[derive(Debug)]
struct AuditState {
    entries: Vec<AuditEntry>,
    epochs: Vec<AuditEpoch>,
    active_epoch: u64,
    next_seq: u64,
    tip: String,
    runtime_write_error: Option<String>,
}

/// Public, redacted integrity state used by health, CLI and TUI surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditIntegrityStatus {
    pub valid: bool,
    pub status: String,
    pub active_epoch: u64,
    pub active_epoch_valid: bool,
    pub invalid_epochs: Vec<u64>,
    pub entry_count: usize,
    pub tip_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Errors that prevent an audit event from becoming a validated entry.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("audit database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("audit database lock is poisoned")]
    DatabaseLockPoisoned,
    #[error("audit state lock is poisoned")]
    StateLockPoisoned,
    #[error("invalid audit schema: {0}")]
    InvalidSchema(String),
    #[error("audit sequence exhausted")]
    SequenceExhausted,
}

/// Append-only, tamper-evident audit log backed by a versioned hash chain.
pub struct AuditLog {
    state: Mutex<AuditState>,
    db: Option<Arc<Mutex<Connection>>>,
}

impl AuditLog {
    /// Creates an in-memory audit log with one empty active epoch.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AuditState {
                entries: Vec::new(),
                epochs: vec![initial_epoch()],
                active_epoch: 0,
                next_seq: 0,
                tip: GENESIS_HASH.to_string(),
                runtime_write_error: None,
            }),
            db: None,
        }
    }

    /// Loads a persistent log and opens a recovery epoch if the active epoch
    /// was altered. Loading or recovery I/O failures abort boot.
    pub fn with_db(conn: Arc<Mutex<Connection>>) -> Result<Self, AuditError> {
        let (mut entries, mut epochs) = {
            let db = lock_db(&conn)?;
            (load_entries(&db)?, load_epochs(&db)?)
        };

        let active_epoch = unique_active_epoch(&epochs)?;
        if let Err(reason) = verify_epoch(&entries, epoch_by_id(&epochs, active_epoch)?) {
            seal_epoch_and_open_recovery(&conn, &mut entries, &mut epochs, active_epoch, &reason)?;
        }

        let active_epoch = unique_active_epoch(&epochs)?;
        let active = epoch_by_id(&epochs, active_epoch)?;
        verify_epoch(&entries, active).map_err(AuditError::InvalidSchema)?;
        let tip = epoch_tip(&entries, active);
        let next_seq = next_sequence(&entries)?;
        let invalid_count = epochs
            .iter()
            .filter(|epoch| epoch.state == EpochState::Invalid)
            .count();
        let count = entries.len();

        if invalid_count == 0 {
            tracing::info!(
                entries = count,
                epoch = active_epoch,
                "Audit hash chain loaded and verified"
            );
        } else {
            tracing::error!(
                entries = count,
                active_epoch,
                invalid_epochs = invalid_count,
                "Audit history contains sealed invalid epochs; active recovery epoch is writable"
            );
        }

        Ok(Self {
            state: Mutex::new(AuditState {
                entries,
                epochs,
                active_epoch,
                next_seq,
                tip,
                runtime_write_error: None,
            }),
            db: Some(conn),
        })
    }

    /// Persists and validates a new event.
    ///
    /// Persistence happens before the in-memory chain advances. On any write
    /// failure, the candidate entry is discarded, the health state degrades,
    /// and the error is returned.
    pub fn record(
        &self,
        agent_id: impl Into<String>,
        action: AuditAction,
        detail: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<String, AuditError> {
        let result = self.try_record(agent_id.into(), action, detail.into(), outcome.into());
        if let Err(error) = &result {
            self.mark_runtime_write_error(error);
            tracing::error!(
                error = %error,
                "Audit append failed; candidate event was not added to the validated chain"
            );
        }
        result
    }

    /// Explicit fail-loud policy for operations that cannot be rolled back
    /// after their audit append. The error remains visible in health surfaces.
    pub fn record_or_alert(
        &self,
        agent_id: impl Into<String>,
        action: AuditAction,
        detail: impl Into<String>,
        outcome: impl Into<String>,
    ) {
        if self.record(agent_id, action, detail, outcome).is_err() {
            // `record` already emitted the alert and degraded health.
        }
    }

    fn try_record(
        &self,
        agent_id: String,
        action: AuditAction,
        detail: String,
        outcome: String,
    ) -> Result<String, AuditError> {
        let mut state = self.lock_state()?;
        let entry = build_entry(
            state.next_seq,
            state.active_epoch,
            Utc::now().to_rfc3339(),
            agent_id,
            action,
            detail,
            outcome,
            state.tip.clone(),
        )?;

        if let Some(db) = &self.db {
            let conn = lock_db(db)?;
            insert_entry(&conn, &entry)?;
        }

        state.next_seq = state
            .next_seq
            .checked_add(1)
            .ok_or(AuditError::SequenceExhausted)?;
        state.tip.clone_from(&entry.hash);
        state.entries.push(entry.clone());
        Ok(entry.hash)
    }

    fn mark_runtime_write_error(&self, error: &AuditError) {
        let message = clip_integrity_error(&error.to_string());
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.runtime_write_error = Some(message);
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, AuditState>, AuditError> {
        self.state.lock().map_err(|_| AuditError::StateLockPoisoned)
    }

    /// Verifies the active epoch and reports any sealed invalid history or
    /// runtime write failure.
    pub fn verify_integrity(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let active = epoch_by_id(&state.epochs, state.active_epoch).map_err(|e| e.to_string())?;
        verify_epoch(&state.entries, active)?;

        let invalid = invalid_epoch_ids(&state.epochs);
        if !invalid.is_empty() {
            let reason = state
                .epochs
                .iter()
                .find(|epoch| epoch.state == EpochState::Invalid)
                .and_then(|epoch| epoch.invalid_reason.as_deref())
                .unwrap_or("historical audit epoch failed integrity verification");
            return Err(format!(
                "sealed invalid audit epoch(s) {}: {reason}",
                invalid
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(error) = &state.runtime_write_error {
            return Err(format!("runtime audit append failure: {error}"));
        }
        Ok(())
    }

    /// Returns the redacted integrity state without exposing audit contents.
    pub fn integrity_status(&self) -> AuditIntegrityStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let active_epoch_valid = epoch_by_id(&state.epochs, state.active_epoch)
            .ok()
            .is_some_and(|epoch| verify_epoch(&state.entries, epoch).is_ok());
        let invalid_epochs = invalid_epoch_ids(&state.epochs);
        let last_error = state.runtime_write_error.clone().or_else(|| {
            state
                .epochs
                .iter()
                .find(|epoch| epoch.state == EpochState::Invalid)
                .and_then(|epoch| epoch.invalid_reason.clone())
        });
        let valid = active_epoch_valid && invalid_epochs.is_empty() && last_error.is_none();

        AuditIntegrityStatus {
            valid,
            status: if valid { "healthy" } else { "degraded" }.to_string(),
            active_epoch: state.active_epoch,
            active_epoch_valid,
            invalid_epochs,
            entry_count: state.entries.len(),
            tip_hash: state.tip.clone(),
            last_error,
        }
    }

    pub fn tip_hash(&self) -> String {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .tip
            .clone()
    }

    pub fn len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entries
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn recent(&self, n: usize) -> Vec<AuditEntry> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let start = state.entries.len().saturating_sub(n);
        state.entries[start..].to_vec()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

fn initial_epoch() -> AuditEpoch {
    AuditEpoch {
        id: 0,
        start_seq: 0,
        started_at: Utc::now().to_rfc3339(),
        predecessor_tip_hash: GENESIS_HASH.to_string(),
        state: EpochState::Active,
        terminal_hash: None,
        sealed_at: None,
        invalid_reason: None,
    }
}

fn clip_integrity_error(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    cleaned.chars().take(MAX_INTEGRITY_ERROR_CHARS).collect()
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
