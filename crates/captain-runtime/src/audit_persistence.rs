use super::audit_chain::{build_entry, epoch_by_id, epoch_tip, next_sequence};
use super::{clip_integrity_error, AuditAction, AuditEntry, AuditEpoch, AuditError, EpochState};
use chrono::Utc;
use rusqlite::{Connection, Transaction};
use std::sync::{Arc, Mutex, MutexGuard};

pub(super) fn lock_db(
    conn: &Arc<Mutex<Connection>>,
) -> Result<MutexGuard<'_, Connection>, AuditError> {
    conn.lock().map_err(|_| AuditError::DatabaseLockPoisoned)
}

pub(super) fn load_entries(conn: &Connection) -> Result<Vec<AuditEntry>, AuditError> {
    let mut stmt = conn.prepare(
        "SELECT seq, epoch, hash_version, timestamp, agent_id, action, detail, outcome, prev_hash, hash
         FROM audit_entries ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let seq = non_negative_u64(row.get::<_, i64>(0)?, "audit entry seq")?;
        let epoch = non_negative_u64(row.get::<_, i64>(1)?, "audit entry epoch")?;
        let hash_version_i64: i64 = row.get(2)?;
        let hash_version = u8::try_from(hash_version_i64).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Integer,
                "audit hash version outside u8".into(),
            )
        })?;
        let action = AuditAction::from_stored(row.get(5)?);
        Ok(AuditEntry {
            seq,
            epoch,
            hash_version,
            timestamp: row.get(3)?,
            agent_id: row.get(4)?,
            action,
            detail: row.get(6)?,
            outcome: row.get(7)?,
            prev_hash: row.get(8)?,
            hash: row.get(9)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(AuditError::from)
}

pub(super) fn load_epochs(conn: &Connection) -> Result<Vec<AuditEpoch>, AuditError> {
    let mut stmt = conn.prepare(
        "SELECT epoch, start_seq, started_at, predecessor_tip_hash, status,
                terminal_hash, sealed_at, invalid_reason
         FROM audit_epochs ORDER BY epoch ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let id = non_negative_u64(row.get::<_, i64>(0)?, "audit epoch")?;
        let start_seq = non_negative_u64(row.get::<_, i64>(1)?, "audit epoch start_seq")?;
        let status: String = row.get(4)?;
        Ok(AuditEpoch {
            id,
            start_seq,
            started_at: row.get(2)?,
            predecessor_tip_hash: row.get(3)?,
            state: EpochState::parse(&status).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
            terminal_hash: row.get(5)?,
            sealed_at: row.get(6)?,
            invalid_reason: row.get(7)?,
        })
    })?;
    let epochs = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(AuditError::from)?;
    if epochs.is_empty() {
        return Err(AuditError::InvalidSchema(
            "audit_epochs contains no epoch".to_string(),
        ));
    }
    Ok(epochs)
}

fn non_negative_u64(value: i64, field: &'static str) -> Result<u64, rusqlite::Error> {
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("{field} must be non-negative").into(),
        )
    })
}

pub(super) fn seal_epoch_and_open_recovery(
    conn: &Arc<Mutex<Connection>>,
    entries: &mut Vec<AuditEntry>,
    epochs: &mut Vec<AuditEpoch>,
    active_epoch_id: u64,
    reason: &str,
) -> Result<(), AuditError> {
    let current_epoch = epoch_by_id(epochs, active_epoch_id)?.clone();
    let terminal_hash = epoch_tip(entries, &current_epoch);
    let next_epoch_id = epochs
        .iter()
        .map(|epoch| epoch.id)
        .chain(entries.iter().map(|entry| entry.epoch))
        .max()
        .unwrap_or(active_epoch_id)
        .checked_add(1)
        .ok_or(AuditError::SequenceExhausted)?;
    let next_seq = next_sequence(entries)?;
    let timestamp = Utc::now().to_rfc3339();
    let safe_reason = clip_integrity_error(reason);
    let detail = serde_json::json!({
        "previous_epoch": active_epoch_id,
        "previous_terminal_hash": terminal_hash,
        "reason": safe_reason,
    })
    .to_string();
    let recovery_entry = build_entry(
        next_seq,
        next_epoch_id,
        timestamp.clone(),
        "system".to_string(),
        AuditAction::ChainRecovery,
        detail,
        "recovery_epoch_opened".to_string(),
        terminal_hash.clone(),
    )?;

    {
        let mut db = lock_db(conn)?;
        let transaction = db.transaction()?;
        persist_recovery(
            &transaction,
            &current_epoch,
            next_epoch_id,
            &timestamp,
            &terminal_hash,
            &safe_reason,
            &recovery_entry,
        )?;
        transaction.commit()?;
    }

    let sealed_at = timestamp.clone();
    let old_epoch = epochs
        .iter_mut()
        .find(|epoch| epoch.id == active_epoch_id)
        .ok_or_else(|| AuditError::InvalidSchema("active audit epoch disappeared".to_string()))?;
    old_epoch.state = EpochState::Invalid;
    old_epoch.terminal_hash = Some(terminal_hash.clone());
    old_epoch.sealed_at = Some(sealed_at);
    old_epoch.invalid_reason = Some(safe_reason.clone());
    epochs.push(AuditEpoch {
        id: next_epoch_id,
        start_seq: next_seq,
        started_at: timestamp,
        predecessor_tip_hash: terminal_hash,
        state: EpochState::Active,
        terminal_hash: None,
        sealed_at: None,
        invalid_reason: None,
    });
    entries.push(recovery_entry);

    tracing::error!(
        invalid_epoch = active_epoch_id,
        recovery_epoch = next_epoch_id,
        reason = %safe_reason,
        "Audit epoch sealed after integrity failure; original entries were not rewritten"
    );
    Ok(())
}

fn persist_recovery(
    transaction: &Transaction<'_>,
    current_epoch: &AuditEpoch,
    next_epoch_id: u64,
    timestamp: &str,
    terminal_hash: &str,
    reason: &str,
    recovery_entry: &AuditEntry,
) -> Result<(), AuditError> {
    let changed = transaction.execute(
        "UPDATE audit_epochs
         SET status = 'invalid', terminal_hash = ?1, sealed_at = ?2, invalid_reason = ?3
         WHERE epoch = ?4 AND status = 'active'",
        rusqlite::params![
            terminal_hash,
            timestamp,
            reason,
            to_sql_i64(current_epoch.id, "audit epoch")?
        ],
    )?;
    if changed != 1 {
        return Err(AuditError::InvalidSchema(format!(
            "failed to seal active audit epoch {}",
            current_epoch.id
        )));
    }
    transaction.execute(
        "INSERT INTO audit_epochs (
             epoch, start_seq, started_at, predecessor_tip_hash, status,
             terminal_hash, sealed_at, invalid_reason
         ) VALUES (?1, ?2, ?3, ?4, 'active', NULL, NULL, NULL)",
        rusqlite::params![
            to_sql_i64(next_epoch_id, "audit epoch")?,
            to_sql_i64(recovery_entry.seq, "audit sequence")?,
            timestamp,
            terminal_hash
        ],
    )?;
    insert_entry(transaction, recovery_entry)?;
    Ok(())
}

pub(super) fn insert_entry(conn: &Connection, entry: &AuditEntry) -> Result<(), AuditError> {
    conn.execute(
        "INSERT INTO audit_entries (
             seq, epoch, hash_version, timestamp, agent_id, action,
             detail, outcome, prev_hash, hash
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            to_sql_i64(entry.seq, "audit sequence")?,
            to_sql_i64(entry.epoch, "audit epoch")?,
            i64::from(entry.hash_version),
            &entry.timestamp,
            &entry.agent_id,
            entry.action.to_string(),
            &entry.detail,
            &entry.outcome,
            &entry.prev_hash,
            &entry.hash,
        ],
    )?;
    Ok(())
}

fn to_sql_i64(value: u64, field: &'static str) -> Result<i64, AuditError> {
    i64::try_from(value)
        .map_err(|_| AuditError::InvalidSchema(format!("{field} exceeds SQLite INTEGER")))
}
