use crate::{CapabilityStatus, CompiledCapability, RegistryError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::Path;

const DURABLE_PRAGMAS: &str = "
    PRAGMA journal_mode=WAL;
    PRAGMA synchronous=FULL;
    PRAGMA fullfsync=ON;
    PRAGMA checkpoint_fullfsync=ON;
    PRAGMA busy_timeout=5000;
";

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS capspec_revisions (
        scope_key TEXT NOT NULL,
        name TEXT NOT NULL,
        source_hash TEXT NOT NULL,
        source_text TEXT NOT NULL,
        compiled_json TEXT NOT NULL,
        permission_fingerprint TEXT NOT NULL,
        created_at TEXT NOT NULL,
        approved_by TEXT,
        approved_at TEXT,
        rejected_by TEXT,
        rejected_at TEXT,
        PRIMARY KEY (scope_key, name, source_hash)
    );
    CREATE TABLE IF NOT EXISTS capspec_slots (
        scope_key TEXT NOT NULL,
        name TEXT NOT NULL,
        source_path TEXT NOT NULL,
        status TEXT NOT NULL,
        active_hash TEXT,
        pending_hash TEXT,
        last_error TEXT,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (scope_key, name)
    );
    CREATE INDEX IF NOT EXISTS capspec_revisions_created
        ON capspec_revisions(scope_key, name, created_at DESC);
";

pub(crate) struct CapabilityStore {
    connection: Connection,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredSlot {
    pub scope_key: String,
    pub name: String,
    pub source_path: String,
    pub status: CapabilityStatus,
    pub active_hash: Option<String>,
    pub pending_hash: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredRevision {
    pub scope_key: String,
    pub name: String,
    pub source_hash: String,
    pub source_text: String,
    pub compiled: CompiledCapability,
    pub permission_fingerprint: String,
    pub created_at: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub rejected_by: Option<String>,
    pub rejected_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RevisionDecision {
    Approved,
    Rejected,
    Undecided,
}

impl CapabilityStore {
    pub fn open(path: &Path) -> Result<Self, RegistryError> {
        if let Some(parent) = path.parent() {
            captain_types::durable_fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        make_database_private(path)?;
        connection.execute_batch(DURABLE_PRAGMAS)?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self { connection })
    }

    pub fn load_slots(&self) -> Result<Vec<StoredSlot>, RegistryError> {
        let mut statement = self.connection.prepare(
            "SELECT scope_key, name, source_path, status, active_hash, pending_hash,
                    last_error, updated_at
             FROM capspec_slots
             ORDER BY scope_key, name",
        )?;
        let rows = statement.query_map([], |row| {
            let status: String = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                status,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })?;

        rows.map(|row| {
            let (
                scope_key,
                name,
                source_path,
                status,
                active_hash,
                pending_hash,
                last_error,
                updated_at,
            ): (
                String,
                String,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            ) = row?;
            Ok(StoredSlot {
                scope_key,
                name,
                source_path,
                status: CapabilityStatus::from_storage(&status)?,
                active_hash,
                pending_hash,
                last_error,
                updated_at,
            })
        })
        .collect()
    }

    pub fn load_revision(
        &self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
    ) -> Result<Option<StoredRevision>, RegistryError> {
        self.connection
            .query_row(
                "SELECT scope_key, name, source_hash, source_text, compiled_json,
                        permission_fingerprint, created_at, approved_by, approved_at,
                        rejected_by, rejected_at
                 FROM capspec_revisions
                 WHERE scope_key = ?1 AND name = ?2 AND source_hash = ?3",
                params![scope_key, name, source_hash],
                row_to_revision,
            )
            .optional()
            .map_err(RegistryError::from)
    }

    pub fn list_revisions(
        &self,
        scope_key: &str,
        name: &str,
    ) -> Result<Vec<StoredRevision>, RegistryError> {
        let mut statement = self.connection.prepare(
            "SELECT scope_key, name, source_hash, source_text, compiled_json,
                    permission_fingerprint, created_at, approved_by, approved_at,
                    rejected_by, rejected_at
             FROM capspec_revisions
             WHERE scope_key = ?1 AND name = ?2
             ORDER BY created_at DESC, source_hash DESC",
        )?;
        let revisions = statement
            .query_map(params![scope_key, name], row_to_revision)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(RegistryError::from)?;
        Ok(revisions)
    }

    pub fn decision(
        &self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
    ) -> Result<RevisionDecision, RegistryError> {
        let decision = self
            .connection
            .query_row(
                "SELECT approved_at, rejected_at FROM capspec_revisions
                 WHERE scope_key = ?1 AND name = ?2 AND source_hash = ?3",
                params![scope_key, name, source_hash],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        Ok(match decision {
            Some((Some(_), _)) => RevisionDecision::Approved,
            Some((None, Some(_))) => RevisionDecision::Rejected,
            _ => RevisionDecision::Undecided,
        })
    }

    pub fn save_revision_and_slot(
        &mut self,
        scope_key: &str,
        source_text: &str,
        compiled: &CompiledCapability,
        slot: &StoredSlot,
    ) -> Result<(), RegistryError> {
        let transaction = self.connection.transaction()?;
        upsert_revision(&transaction, scope_key, source_text, compiled)?;
        upsert_slot(&transaction, slot)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn save_slot(&mut self, slot: &StoredSlot) -> Result<(), RegistryError> {
        let transaction = self.connection.transaction()?;
        upsert_slot(&transaction, slot)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn approve_revision_and_slot(
        &mut self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
        actor: &str,
        slot: &StoredSlot,
    ) -> Result<(), RegistryError> {
        self.decide_revision_and_slot(scope_key, name, source_hash, actor, slot, true)
    }

    pub fn reject_revision_and_slot(
        &mut self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
        actor: &str,
        slot: &StoredSlot,
    ) -> Result<(), RegistryError> {
        self.decide_revision_and_slot(scope_key, name, source_hash, actor, slot, false)
    }

    pub fn mark_approved(
        &mut self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
        actor: &str,
    ) -> Result<(), RegistryError> {
        let changed = self.connection.execute(
            "UPDATE capspec_revisions
             SET approved_by = ?4, approved_at = ?5, rejected_by = NULL, rejected_at = NULL
             WHERE scope_key = ?1 AND name = ?2 AND source_hash = ?3",
            params![scope_key, name, source_hash, actor, now()],
        )?;
        ensure_revision_changed(changed, scope_key, name, source_hash)
    }

    fn decide_revision_and_slot(
        &mut self,
        scope_key: &str,
        name: &str,
        source_hash: &str,
        actor: &str,
        slot: &StoredSlot,
        approve: bool,
    ) -> Result<(), RegistryError> {
        let transaction = self.connection.transaction()?;
        let timestamp = now();
        let changed = if approve {
            transaction.execute(
                "UPDATE capspec_revisions
                 SET approved_by = ?4, approved_at = ?5,
                     rejected_by = NULL, rejected_at = NULL
                 WHERE scope_key = ?1 AND name = ?2 AND source_hash = ?3",
                params![scope_key, name, source_hash, actor, timestamp],
            )?
        } else {
            transaction.execute(
                "UPDATE capspec_revisions
                 SET rejected_by = ?4, rejected_at = ?5,
                     approved_by = NULL, approved_at = NULL
                 WHERE scope_key = ?1 AND name = ?2 AND source_hash = ?3",
                params![scope_key, name, source_hash, actor, timestamp],
            )?
        };
        ensure_revision_changed(changed, scope_key, name, source_hash)?;
        upsert_slot(&transaction, slot)?;
        transaction.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub fn durability_settings(&self) -> Result<(String, i64, i64, i64), RegistryError> {
        Ok((
            self.connection
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))?,
            self.connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))?,
            self.connection
                .query_row("PRAGMA fullfsync", [], |row| row.get(0))?,
            self.connection
                .query_row("PRAGMA checkpoint_fullfsync", [], |row| row.get(0))?,
        ))
    }
}

fn upsert_revision(
    transaction: &Transaction<'_>,
    scope_key: &str,
    source_text: &str,
    compiled: &CompiledCapability,
) -> Result<(), RegistryError> {
    transaction.execute(
        "INSERT INTO capspec_revisions (
            scope_key, name, source_hash, source_text, compiled_json,
            permission_fingerprint, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(scope_key, name, source_hash) DO UPDATE SET
            source_text = excluded.source_text,
            compiled_json = excluded.compiled_json,
            permission_fingerprint = excluded.permission_fingerprint",
        params![
            scope_key,
            compiled.name,
            compiled.source_hash,
            source_text,
            serde_json::to_string(compiled)?,
            compiled.permission_fingerprint,
            now(),
        ],
    )?;
    Ok(())
}

fn upsert_slot(transaction: &Transaction<'_>, slot: &StoredSlot) -> Result<(), RegistryError> {
    transaction.execute(
        "INSERT INTO capspec_slots (
            scope_key, name, source_path, status, active_hash, pending_hash,
            last_error, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(scope_key, name) DO UPDATE SET
            source_path = excluded.source_path,
            status = excluded.status,
            active_hash = excluded.active_hash,
            pending_hash = excluded.pending_hash,
            last_error = excluded.last_error,
            updated_at = excluded.updated_at",
        params![
            slot.scope_key,
            slot.name,
            slot.source_path,
            slot.status.as_storage(),
            slot.active_hash,
            slot.pending_hash,
            slot.last_error,
            slot.updated_at,
        ],
    )?;
    Ok(())
}

fn row_to_revision(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRevision> {
    let compiled_json: String = row.get(4)?;
    let compiled = serde_json::from_str(&compiled_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(StoredRevision {
        scope_key: row.get(0)?,
        name: row.get(1)?,
        source_hash: row.get(2)?,
        source_text: row.get(3)?,
        compiled,
        permission_fingerprint: row.get(5)?,
        created_at: row.get(6)?,
        approved_by: row.get(7)?,
        approved_at: row.get(8)?,
        rejected_by: row.get(9)?,
        rejected_at: row.get(10)?,
    })
}

fn ensure_revision_changed(
    changed: usize,
    scope_key: &str,
    name: &str,
    source_hash: &str,
) -> Result<(), RegistryError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(RegistryError::RevisionNotFound {
            scope: scope_key.to_string(),
            name: name.to_string(),
            source_hash: source_hash.to_string(),
        })
    }
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(unix)]
fn make_database_private(path: &Path) -> Result<(), RegistryError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_database_private(_path: &Path) -> Result<(), RegistryError> {
    Ok(())
}
