//! Durable execution targets pinned to Hub sessions and projects.

use captain_wire::ExecutionTarget;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

const MAX_SCOPE_ID_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTargetScope {
    Session,
    Project,
}

impl ExecutionTargetScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Project => "project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionTargetBinding {
    pub scope: ExecutionTargetScope,
    pub scope_id: String,
    pub target: ExecutionTarget,
    pub updated_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum ExecutionTargetStoreError {
    #[error("invalid execution target scope identifier")]
    InvalidScopeId,
    #[error("invalid execution target")]
    InvalidTarget,
    #[error("invalid execution target timestamp")]
    InvalidTimestamp,
    #[error("execution target store lock failed")]
    Lock,
    #[error("execution target store database error")]
    Database(#[from] rusqlite::Error),
}

#[derive(Clone)]
pub struct ExecutionTargetStore {
    conn: Arc<Mutex<Connection>>,
}

impl ExecutionTargetStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn get(
        &self,
        scope: ExecutionTargetScope,
        scope_id: &str,
    ) -> Result<Option<ExecutionTargetBinding>, ExecutionTargetStoreError> {
        validate_scope_id(scope_id)?;
        let conn = self.lock()?;
        conn.query_row(
            "SELECT target_kind, device_id, workspace_id, updated_at_ms
             FROM execution_target_bindings
             WHERE scope_kind = ?1 AND scope_id = ?2",
            params![scope.as_str(), scope_id],
            |row| {
                let target_kind: String = row.get(0)?;
                let device_id: Option<String> = row.get(1)?;
                let workspace_id: Option<String> = row.get(2)?;
                let target = target_from_columns(&target_kind, device_id, workspace_id)?;
                Ok(ExecutionTargetBinding {
                    scope,
                    scope_id: scope_id.to_string(),
                    target,
                    updated_at_ms: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set(
        &self,
        scope: ExecutionTargetScope,
        scope_id: &str,
        target: &ExecutionTarget,
        updated_at_ms: i64,
    ) -> Result<ExecutionTargetBinding, ExecutionTargetStoreError> {
        validate_scope_id(scope_id)?;
        target
            .validate()
            .map_err(|_| ExecutionTargetStoreError::InvalidTarget)?;
        if updated_at_ms < 0 {
            return Err(ExecutionTargetStoreError::InvalidTimestamp);
        }
        let (target_kind, device_id, workspace_id) = target_columns(target);
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO execution_target_bindings
                (scope_kind, scope_id, target_kind, device_id, workspace_id, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(scope_kind, scope_id) DO UPDATE SET
                target_kind = excluded.target_kind,
                device_id = excluded.device_id,
                workspace_id = excluded.workspace_id,
                updated_at_ms = excluded.updated_at_ms",
            params![
                scope.as_str(),
                scope_id,
                target_kind,
                device_id,
                workspace_id,
                updated_at_ms,
            ],
        )?;
        Ok(ExecutionTargetBinding {
            scope,
            scope_id: scope_id.to_string(),
            target: target.clone(),
            updated_at_ms,
        })
    }

    pub fn delete(
        &self,
        scope: ExecutionTargetScope,
        scope_id: &str,
    ) -> Result<bool, ExecutionTargetStoreError> {
        validate_scope_id(scope_id)?;
        let conn = self.lock()?;
        Ok(conn.execute(
            "DELETE FROM execution_target_bindings
             WHERE scope_kind = ?1 AND scope_id = ?2",
            params![scope.as_str(), scope_id],
        )? > 0)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ExecutionTargetStoreError> {
        self.conn
            .lock()
            .map_err(|_| ExecutionTargetStoreError::Lock)
    }
}

fn validate_scope_id(scope_id: &str) -> Result<(), ExecutionTargetStoreError> {
    if scope_id.is_empty()
        || scope_id.len() > MAX_SCOPE_ID_LEN
        || !scope_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ExecutionTargetStoreError::InvalidScopeId);
    }
    Ok(())
}

fn target_columns(target: &ExecutionTarget) -> (&'static str, Option<&str>, Option<&str>) {
    match target {
        ExecutionTarget::Auto => ("auto", None, None),
        ExecutionTarget::Hub => ("hub", None, None),
        ExecutionTarget::Node {
            device_id,
            workspace_id,
        } => ("node", Some(device_id), Some(workspace_id)),
    }
}

fn target_from_columns(
    target_kind: &str,
    device_id: Option<String>,
    workspace_id: Option<String>,
) -> Result<ExecutionTarget, rusqlite::Error> {
    let target = match (target_kind, device_id, workspace_id) {
        ("auto", None, None) => ExecutionTarget::Auto,
        ("hub", None, None) => ExecutionTarget::Hub,
        ("node", Some(device_id), Some(workspace_id)) => ExecutionTarget::Node {
            device_id,
            workspace_id,
        },
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    target
        .validate()
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(target)
}
