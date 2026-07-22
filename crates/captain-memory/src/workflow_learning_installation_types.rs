use serde::{Deserialize, Serialize};

use crate::workflow_learning_types::WorkflowArtifactKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowInstallationPhase {
    Prepared,
    Promoted,
    Verified,
    Active,
    RollbackPending,
    RolledBack,
    Quarantined,
    Failed,
}

impl WorkflowInstallationPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Promoted => "promoted",
            Self::Verified => "verified",
            Self::Active => "active",
            Self::RollbackPending => "rollback_pending",
            Self::RolledBack => "rolled_back",
            Self::Quarantined => "quarantined",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "prepared" => Some(Self::Prepared),
            "promoted" => Some(Self::Promoted),
            "verified" => Some(Self::Verified),
            "active" => Some(Self::Active),
            "rollback_pending" => Some(Self::RollbackPending),
            "rolled_back" => Some(Self::RolledBack),
            "quarantined" => Some(Self::Quarantined),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowInstallation {
    pub proposal_id: String,
    pub revision_sha256: String,
    pub kind: WorkflowArtifactKind,
    pub target_locator: String,
    pub backup_locator: Option<String>,
    pub backup_sha256: Option<String>,
    pub installed_sha256: String,
    pub actor: String,
    pub reason: String,
    pub idempotency_key: String,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInstallationTransition {
    pub proposal_id: String,
    pub revision_sha256: String,
    pub expected_phase: WorkflowInstallationPhase,
    pub expected_version: u64,
    pub to_phase: WorkflowInstallationPhase,
    pub last_error: Option<String>,
    pub actor: String,
    pub reason: String,
    pub idempotency_key: String,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInstallationRecord {
    pub proposal_id: String,
    pub revision_sha256: String,
    pub kind: WorkflowArtifactKind,
    pub phase: WorkflowInstallationPhase,
    pub phase_version: u64,
    pub target_locator: String,
    pub backup_locator: Option<String>,
    pub backup_sha256: Option<String>,
    pub installed_sha256: String,
    pub last_error: Option<String>,
    pub prepared_at_unix_ms: i64,
    pub promoted_at_unix_ms: Option<i64>,
    pub verified_at_unix_ms: Option<i64>,
    pub rolled_back_at_unix_ms: Option<i64>,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowInstallationEvent {
    pub sequence: u64,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub from_phase: Option<WorkflowInstallationPhase>,
    pub to_phase: WorkflowInstallationPhase,
    pub resulting_version: u64,
    pub last_error: Option<String>,
    pub actor: String,
    pub reason: String,
    pub created_at_unix_ms: i64,
}
