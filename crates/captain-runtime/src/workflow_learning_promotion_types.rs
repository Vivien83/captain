use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::workflow_learning_proposer::WorkflowDraftKind;

pub const WORKFLOW_PROMOTION_MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Clone)]
pub struct PromoteWorkflowDraftRequest<'a> {
    pub proposal_id: &'a str,
    pub staging_job_id: &'a str,
    pub revision_sha256: &'a str,
    pub artifact_sha256: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPromotionTargetKind {
    Skill,
    Capspec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPromotionPhase {
    Prepared,
    Promoted,
    RegistryVerified,
    Active,
    RollbackPending,
    RolledBack,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPromotionManifest {
    pub manifest_version: u16,
    pub proposal_id: String,
    pub staging_job_id: String,
    pub revision_sha256: String,
    pub artifact_sha256: String,
    pub draft_kind: WorkflowDraftKind,
    pub target_kind: WorkflowPromotionTargetKind,
    pub target_name: String,
    /// Path relative to Captain home. Absolute paths are never persisted.
    pub target_relative_path: PathBuf,
    pub previous_sha256: Option<String>,
    pub previous_backup_relative_path: Option<PathBuf>,
    pub phase: WorkflowPromotionPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWorkflowPromotion {
    pub manifest: WorkflowPromotionManifest,
    pub target_path: PathBuf,
    pub journal_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedWorkflowPromotion {
    pub(crate) proposal_id: String,
    pub(crate) revision_sha256: String,
    pub(crate) artifact_sha256: String,
    pub(crate) target_kind: WorkflowPromotionTargetKind,
    pub(crate) target_name: String,
}

impl VerifiedWorkflowPromotion {
    pub(crate) fn exact(manifest: &WorkflowPromotionManifest) -> VerifiedWorkflowPromotion {
        Self {
            proposal_id: manifest.proposal_id.clone(),
            revision_sha256: manifest.revision_sha256.clone(),
            artifact_sha256: manifest.artifact_sha256.clone(),
            target_kind: manifest.target_kind,
            target_name: manifest.target_name.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowPromotionError {
    #[error("invalid promotion request: {0}")]
    InvalidRequest(String),
    #[error("staged workflow is invalid: {0}")]
    InvalidStaging(String),
    #[error("automation activation must use the durable scheduler backend")]
    ExternalActivationRequired,
    #[error("unsafe promotion filesystem: {0}")]
    UnsafeFilesystem(String),
    #[error("promotion conflict: {0}")]
    Conflict(String),
    #[error("promotion is not prepared: {0}")]
    NotPrepared(String),
    #[error("invalid promotion phase: expected {expected}, found {actual:?}")]
    InvalidPhase {
        expected: &'static str,
        actual: WorkflowPromotionPhase,
    },
    #[error("registry verification failed: {0}")]
    RegistryVerification(String),
    #[error("promotion I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("promotion serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}
