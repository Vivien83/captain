use crate::state_cipher::StateCipher;
use crate::{
    CapabilityExecutionContext, CapabilityNodeStatus, CapabilityNodeView, CapabilityRunStatus,
    CapabilityRunView, CompiledCapability, Effect, ExecutorError, Idempotency, ResolvedCapability,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{Map, Value};
use std::path::Path;

#[path = "run_store_crypto.rs"]
mod crypto;
#[path = "run_store_read.rs"]
mod read;
#[path = "run_store_recovery.rs"]
mod recovery;
#[path = "run_store_state.rs"]
mod state;

const DURABLE_PRAGMAS: &str = "
    PRAGMA journal_mode=WAL;
    PRAGMA synchronous=FULL;
    PRAGMA fullfsync=ON;
    PRAGMA checkpoint_fullfsync=ON;
    PRAGMA busy_timeout=5000;
    PRAGMA foreign_keys=ON;
";

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS capspec_runs (
        run_id TEXT PRIMARY KEY,
        scope_json TEXT NOT NULL,
        capability_name TEXT NOT NULL,
        tool_name TEXT NOT NULL,
        source_hash TEXT NOT NULL,
        input_blob BLOB NOT NULL,
        status TEXT NOT NULL,
        output_blob BLOB,
        error_blob BLOB,
        caller_agent_id TEXT,
        workspace TEXT,
        origin TEXT NOT NULL,
        authority_blob BLOB,
        operator_resume_state TEXT NOT NULL DEFAULT 'none',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS capspec_run_nodes (
        run_id TEXT NOT NULL REFERENCES capspec_runs(run_id),
        step_id TEXT NOT NULL,
        ordinal INTEGER NOT NULL,
        tool_name TEXT NOT NULL,
        effect TEXT NOT NULL,
        idempotency TEXT NOT NULL,
        status TEXT NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        replay_safe INTEGER NOT NULL DEFAULT 0,
        operator_retry_permit INTEGER NOT NULL DEFAULT 0,
        input_blob BLOB,
        output_blob BLOB,
        error_blob BLOB,
        idempotency_key_blob BLOB,
        tool_use_id TEXT,
        started_at TEXT,
        finished_at TEXT,
        PRIMARY KEY (run_id, step_id)
    );
    CREATE INDEX IF NOT EXISTS capspec_runs_updated
        ON capspec_runs(updated_at DESC);
    CREATE INDEX IF NOT EXISTS capspec_run_nodes_status
        ON capspec_run_nodes(run_id, status, ordinal);
";

pub(crate) struct RunStore {
    connection: Connection,
    cipher: StateCipher,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredRun {
    pub view: CapabilityRunView,
    pub execution: CapabilityExecutionContext,
    pub input: Map<String, Value>,
    pub output: Option<Value>,
    pub nodes: Vec<StoredNode>,
}

#[derive(Debug, Clone)]
pub(crate) struct StoredNode {
    pub step_id: String,
    pub ordinal: usize,
    pub tool_name: String,
    pub effect: Effect,
    pub status: CapabilityNodeStatus,
    pub attempts: u32,
    pub operator_retry_permit: bool,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub tool_use_id: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl RunStore {
    pub(crate) fn open(database_path: &Path, key_path: &Path) -> Result<Self, ExecutorError> {
        if let Some(parent) = database_path.parent() {
            captain_types::durable_fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(database_path)?;
        make_database_private(database_path)?;
        connection.execute_batch(DURABLE_PRAGMAS)?;
        connection.execute_batch(SCHEMA)?;
        ensure_schema_columns(&connection)?;
        connection.execute(
            "CREATE INDEX IF NOT EXISTS capspec_runs_operator_resume
             ON capspec_runs(operator_resume_state, status, updated_at)",
            [],
        )?;
        let cipher = StateCipher::open(key_path)?;
        let mut store = Self { connection, cipher };
        store.recover_interrupted_runs()?;
        Ok(store)
    }

    pub(crate) fn create_run(
        &mut self,
        run_id: &str,
        resolved: &ResolvedCapability,
        input: &Map<String, Value>,
        context: &CapabilityExecutionContext,
    ) -> Result<(), ExecutorError> {
        let now = crate::store::now();
        let scope_json = serde_json::to_string(&resolved.scope)?;
        let input_blob = self.seal_json(
            &format!("run:{run_id}:input"),
            &Value::Object(input.clone()),
        )?;
        let authority_blob = context
            .authority
            .as_ref()
            .map(|authority| {
                serde_json::to_value(authority)
                    .map_err(ExecutorError::from)
                    .and_then(|value| self.seal_json(&format!("run:{run_id}:authority"), &value))
            })
            .transpose()?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO capspec_runs (
                run_id, scope_json, capability_name, tool_name, source_hash,
                input_blob, status, caller_agent_id, workspace, origin,
                authority_blob, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![
                run_id,
                scope_json,
                resolved.compiled.name,
                resolved.compiled.tool_name,
                resolved.compiled.source_hash,
                input_blob,
                CapabilityRunStatus::Pending.as_storage(),
                context.caller_agent_id,
                context.workspace,
                normalized_origin(&context.origin),
                authority_blob,
                now,
            ],
        )?;
        insert_nodes(&transaction, run_id, &resolved.compiled)?;
        transaction.commit()?;
        Ok(())
    }
}

fn ensure_schema_columns(connection: &Connection) -> Result<(), ExecutorError> {
    let mut statement = connection.prepare("PRAGMA table_info(capspec_runs)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|column| column == "authority_blob") {
        connection.execute(
            "ALTER TABLE capspec_runs ADD COLUMN authority_blob BLOB",
            [],
        )?;
    }
    if !columns
        .iter()
        .any(|column| column == "operator_resume_state")
    {
        connection.execute(
            "ALTER TABLE capspec_runs
             ADD COLUMN operator_resume_state TEXT NOT NULL DEFAULT 'none'",
            [],
        )?;
    }
    let mut node_statement = connection.prepare("PRAGMA table_info(capspec_run_nodes)")?;
    let node_columns = node_statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !node_columns
        .iter()
        .any(|column| column == "operator_retry_permit")
    {
        connection.execute(
            "ALTER TABLE capspec_run_nodes
             ADD COLUMN operator_retry_permit INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

fn insert_nodes(
    transaction: &Transaction<'_>,
    run_id: &str,
    capability: &CompiledCapability,
) -> Result<(), ExecutorError> {
    for (ordinal, step) in capability.steps.iter().enumerate() {
        transaction.execute(
            "INSERT INTO capspec_run_nodes (
                run_id, step_id, ordinal, tool_name, effect, idempotency, status
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run_id,
                step.id,
                ordinal,
                step.tool,
                effect_storage(step.effect),
                idempotency_storage(step.idempotency),
                CapabilityNodeStatus::Pending.as_storage(),
            ],
        )?;
    }
    Ok(())
}

fn normalized_origin(origin: &str) -> &str {
    let origin = origin.trim();
    if origin.is_empty() {
        "unknown"
    } else {
        origin
    }
}

fn effect_storage(effect: Effect) -> &'static str {
    match effect {
        Effect::Read => "read",
        Effect::Write => "write",
        Effect::External => "external",
        Effect::Destructive => "destructive",
    }
}

fn idempotency_storage(value: Idempotency) -> &'static str {
    match value {
        Idempotency::Safe => "safe",
        Idempotency::Keyed => "keyed",
        Idempotency::Manual => "manual",
    }
}

#[cfg(unix)]
fn make_database_private(path: &Path) -> Result<(), ExecutorError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_database_private(_path: &Path) -> Result<(), ExecutorError> {
    Ok(())
}
