//! Local, fail-closed authorization boundary for Hub-offered tool runs.
//!
//! The Hub chooses work, but it never grants itself filesystem authority on a
//! Node. This module recoups every offer against the locally persisted grant,
//! a logical-workspace-to-path binding, and a reviewed runtime tool contract.
//! Raw local paths never implement `Serialize` and are redacted from `Debug`.

use captain_wire::hub_protocol::RunRejection;
use captain_wire::{DeviceGrant, RunEffect, RunLease};
use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};
use thiserror::Error;

/// One logical Hub workspace bound to an exact canonical directory on this
/// Node. The path remains local and is never serialized onto the Hub rail.
#[derive(Clone, PartialEq, Eq)]
pub struct NodeWorkspaceBinding {
    workspace_id: String,
    canonical_root: PathBuf,
    read_only: bool,
}

impl NodeWorkspaceBinding {
    pub fn new(
        workspace_id: impl Into<String>,
        root: impl AsRef<Path>,
        read_only: bool,
    ) -> Result<Self, NodeExecutionPolicyError> {
        let workspace_id = workspace_id.into();
        validate_identifier(&workspace_id)?;
        let metadata = fs::metadata(root.as_ref())
            .map_err(|_| NodeExecutionPolicyError::WorkspaceUnavailable)?;
        if !metadata.is_dir() {
            return Err(NodeExecutionPolicyError::WorkspaceUnavailable);
        }
        let canonical_root = fs::canonicalize(root.as_ref())
            .map_err(|_| NodeExecutionPolicyError::WorkspaceUnavailable)?;
        Ok(Self {
            workspace_id,
            canonical_root,
            read_only,
        })
    }

    fn current_root(&self) -> Result<&Path, NodeExecutionPolicyError> {
        let current = fs::canonicalize(&self.canonical_root)
            .map_err(|_| NodeExecutionPolicyError::WorkspaceUnavailable)?;
        if current != self.canonical_root
            || !fs::metadata(&current)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        {
            return Err(NodeExecutionPolicyError::WorkspaceUnavailable);
        }
        Ok(&self.canonical_root)
    }
}

impl fmt::Debug for NodeWorkspaceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeWorkspaceBinding")
            .field("workspace_id", &self.workspace_id)
            .field("canonical_root", &"[REDACTED]")
            .field("read_only", &self.read_only)
            .finish()
    }
}

/// Runtime-reviewed identity of the exact builtin tool offered by the Hub.
/// The runtime adapter owns classification; the Node independently verifies
/// that its result agrees with the signed lease and local grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeReviewedTool {
    pub tool_name: String,
    pub family: String,
    pub effect: RunEffect,
}

impl NodeReviewedTool {
    pub fn new(
        tool_name: impl Into<String>,
        family: impl Into<String>,
        effect: RunEffect,
    ) -> Result<Self, NodeExecutionPolicyError> {
        let tool_name = tool_name.into();
        let family = family.into();
        validate_identifier(&tool_name)?;
        validate_identifier(&family)?;
        Ok(Self {
            tool_name,
            family,
            effect,
        })
    }
}

/// Local scope applied before a run may be accepted or claimed.
pub struct NodeExecutionPolicy {
    grant: DeviceGrant,
    workspaces: BTreeMap<String, NodeWorkspaceBinding>,
}

impl NodeExecutionPolicy {
    pub fn new(
        grant: DeviceGrant,
        workspaces: impl IntoIterator<Item = NodeWorkspaceBinding>,
    ) -> Result<Self, NodeExecutionPolicyError> {
        grant
            .validate_shape()
            .map_err(|_| NodeExecutionPolicyError::InvalidGrant)?;
        let mut indexed = BTreeMap::new();
        for workspace in workspaces {
            if indexed
                .insert(workspace.workspace_id.clone(), workspace)
                .is_some()
            {
                return Err(NodeExecutionPolicyError::DuplicateWorkspace);
            }
        }
        Ok(Self {
            grant,
            workspaces: indexed,
        })
    }

    /// Recoups one exact lease against local authority. A denial is returned
    /// as sanitized protocol evidence so callers can atomically persist it
    /// with `NodeRailStore::apply_run_offer` before any effect starts.
    pub fn authorize(
        &self,
        lease: &RunLease,
        reviewed: &NodeReviewedTool,
    ) -> NodeExecutionAuthorization {
        let denial = |code: &str, message: &str, retryable: bool| {
            NodeExecutionAuthorization::Rejected(RunRejection {
                run_id: lease.run_id.clone(),
                attempt: lease.attempt,
                code: code.to_string(),
                message: message.to_string(),
                retryable,
                path_policy_applied: true,
            })
        };

        if lease.validate().is_err() {
            return denial(
                "invalid_offer",
                "The offered run does not satisfy the Node protocol contract",
                false,
            );
        }
        if reviewed.tool_name != lease.tool_name {
            return denial(
                "tool_contract_mismatch",
                "The local reviewed tool identity does not match the offered run",
                false,
            );
        }
        if reviewed.effect != lease.effect {
            return denial(
                "effect_contract_mismatch",
                "The local reviewed effect does not match the offered run",
                false,
            );
        }
        if !self
            .grant
            .workspace_ids
            .iter()
            .any(|workspace| workspace == &lease.workspace_id)
        {
            return denial(
                "workspace_not_granted",
                "The logical workspace is not granted on this Node",
                false,
            );
        }
        let Some(workspace) = self.workspaces.get(&lease.workspace_id) else {
            return denial(
                "workspace_unavailable",
                "The granted logical workspace is not available on this Node",
                true,
            );
        };
        let workspace_root = match workspace.current_root() {
            Ok(root) => root.to_path_buf(),
            Err(_) => {
                return denial(
                    "workspace_unavailable",
                    "The granted logical workspace is not available on this Node",
                    true,
                )
            }
        };
        if !self
            .grant
            .tool_families
            .iter()
            .any(|family| family == &reviewed.family)
        {
            return denial(
                "tool_family_not_granted",
                "The reviewed tool family is not granted on this Node",
                false,
            );
        }
        if reviewed.effect != RunEffect::ReadOnly
            && (!self.grant.allow_mutation || workspace.read_only)
        {
            return denial(
                "mutation_not_granted",
                "The local grant does not authorize mutation in this workspace",
                false,
            );
        }

        NodeExecutionAuthorization::Authorized(AuthorizedNodeRun {
            lease: lease.clone(),
            family: reviewed.family.clone(),
            workspace_root,
        })
    }
}

impl fmt::Debug for NodeExecutionPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeExecutionPolicy")
            .field("granted_workspaces", &self.grant.workspace_ids)
            .field("granted_tool_families", &self.grant.tool_families)
            .field("allow_mutation", &self.grant.allow_mutation)
            .field("workspace_paths", &"[REDACTED]")
            .finish()
    }
}

pub enum NodeExecutionAuthorization {
    Authorized(AuthorizedNodeRun),
    Rejected(RunRejection),
}

impl fmt::Debug for NodeExecutionAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Authorized(run) => formatter.debug_tuple("Authorized").field(run).finish(),
            Self::Rejected(rejection) => {
                formatter.debug_tuple("Rejected").field(rejection).finish()
            }
        }
    }
}

/// Exact execution scope handed to the runtime adapter after local policy
/// succeeds. It intentionally has no serialization implementation.
#[derive(Clone, PartialEq)]
pub struct AuthorizedNodeRun {
    lease: RunLease,
    family: String,
    workspace_root: PathBuf,
}

impl AuthorizedNodeRun {
    pub fn lease(&self) -> &RunLease {
        &self.lease
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

impl fmt::Debug for AuthorizedNodeRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizedNodeRun")
            .field("run_id", &self.lease.run_id)
            .field("attempt", &self.lease.attempt)
            .field("workspace_id", &self.lease.workspace_id)
            .field("tool_name", &self.lease.tool_name)
            .field("input", &"[REDACTED]")
            .field("effect", &self.lease.effect)
            .field("family", &self.family)
            .field("workspace_root", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NodeExecutionPolicyError {
    #[error("invalid local Node grant")]
    InvalidGrant,
    #[error("invalid local Node identifier")]
    InvalidIdentifier,
    #[error("duplicate logical workspace binding")]
    DuplicateWorkspace,
    #[error("logical workspace is unavailable")]
    WorkspaceUnavailable,
}

fn validate_identifier(value: &str) -> Result<(), NodeExecutionPolicyError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        Err(NodeExecutionPolicyError::InvalidIdentifier)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "execution_policy_tests.rs"]
mod tests;
