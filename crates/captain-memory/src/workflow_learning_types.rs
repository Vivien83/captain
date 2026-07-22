//! Shared data contracts for the Skill Learning V2 control plane.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowProposalState {
    Observed,
    Eligible,
    Drafting,
    Validating,
    Proposed,
    Dismissed,
    Snoozed,
    Superseded,
    ApprovedPendingInstall,
    ActiveCanary,
    Active,
    Rejected,
    InstallFailed,
    RolledBack,
}

impl WorkflowProposalState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Eligible => "eligible",
            Self::Drafting => "drafting",
            Self::Validating => "validating",
            Self::Proposed => "proposed",
            Self::Dismissed => "dismissed",
            Self::Snoozed => "snoozed",
            Self::Superseded => "superseded",
            Self::ApprovedPendingInstall => "approved_pending_install",
            Self::ActiveCanary => "active_canary",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::InstallFailed => "install_failed",
            Self::RolledBack => "rolled_back",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "observed" => Some(Self::Observed),
            "eligible" => Some(Self::Eligible),
            "drafting" => Some(Self::Drafting),
            "validating" => Some(Self::Validating),
            "proposed" => Some(Self::Proposed),
            "dismissed" => Some(Self::Dismissed),
            "snoozed" => Some(Self::Snoozed),
            "superseded" => Some(Self::Superseded),
            "approved_pending_install" => Some(Self::ApprovedPendingInstall),
            "active_canary" => Some(Self::ActiveCanary),
            "active" => Some(Self::Active),
            "rejected" => Some(Self::Rejected),
            "install_failed" => Some(Self::InstallFailed),
            "rolled_back" => Some(Self::RolledBack),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowArtifactKind {
    Skill,
    Capspec,
    Automation,
    Refinement,
}

impl WorkflowArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Capspec => "capspec",
            Self::Automation => "automation",
            Self::Refinement => "refinement",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "skill" => Some(Self::Skill),
            "capspec" => Some(Self::Capspec),
            "automation" => Some(Self::Automation),
            "refinement" => Some(Self::Refinement),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowProposal {
    pub id: String,
    pub idempotency_key: String,
    pub workflow_signature: String,
    pub source_agent_id: String,
    pub origin_channel: Option<String>,
    pub evidence_json: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProposalRecord {
    pub id: String,
    pub idempotency_key: String,
    pub workflow_signature: String,
    pub state: WorkflowProposalState,
    pub state_version: u64,
    pub revision_sha256: Option<String>,
    pub operator_token: Option<String>,
    pub artifact_sha256: Option<String>,
    pub staging_job_id: Option<String>,
    pub kind: Option<WorkflowArtifactKind>,
    pub name: Option<String>,
    pub source_agent_id: String,
    pub origin_channel: Option<String>,
    pub evidence_json: String,
    pub validation_json: Option<String>,
    pub isolated_test: Option<WorkflowIsolatedTestRecord>,
    pub snoozed_until_unix_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowIsolatedTestStatus {
    Queued,
    Passed,
    Failed,
}

impl WorkflowIsolatedTestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "passed" => Some(Self::Passed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowIsolatedTestRecord {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub job_id: String,
    pub status: WorkflowIsolatedTestStatus,
    pub requested_by: String,
    pub result_json: Option<String>,
    pub requested_at_unix_ms: i64,
    pub completed_at_unix_ms: Option<i64>,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowIsolatedTest {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub job_id: String,
    pub requested_by: String,
    pub requested_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowProposalEvent {
    pub sequence: u64,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub from_state: Option<WorkflowProposalState>,
    pub to_state: WorkflowProposalState,
    pub resulting_version: u64,
    pub revision_sha256: Option<String>,
    pub actor: String,
    pub reason: String,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowProposalTransition {
    pub proposal_id: String,
    pub expected_state: WorkflowProposalState,
    pub expected_version: u64,
    pub expected_revision_sha256: Option<String>,
    pub to_state: WorkflowProposalState,
    pub actor: String,
    pub reason: String,
    pub idempotency_key: String,
    pub snoozed_until_unix_ms: Option<i64>,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct PublishValidatedDraft {
    pub proposal_id: String,
    pub expected_version: u64,
    pub staging_job_id: String,
    pub revision_sha256: String,
    pub artifact_sha256: String,
    pub kind: WorkflowArtifactKind,
    pub name: String,
    pub validation_json: String,
    pub actor: String,
    pub reason: String,
    pub idempotency_key: String,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowLearningControlError {
    #[error("invalid workflow-learning control input: {0}")]
    InvalidInput(String),
    #[error("workflow-learning proposal not found: {0}")]
    NotFound(String),
    #[error("workflow-learning proposal conflict: {0}")]
    Conflict(String),
    #[error("illegal workflow-learning transition: {from} -> {to}")]
    IllegalTransition { from: String, to: String },
    #[error("corrupt workflow-learning control data: {0}")]
    CorruptData(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowJobKind {
    Analyze,
    Draft,
    Validate,
    Install,
    Canary,
    Rollback,
}

impl WorkflowJobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Analyze => "analyze",
            Self::Draft => "draft",
            Self::Validate => "validate",
            Self::Install => "install",
            Self::Canary => "canary",
            Self::Rollback => "rollback",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "analyze" => Some(Self::Analyze),
            "draft" => Some(Self::Draft),
            "validate" => Some(Self::Validate),
            "install" => Some(Self::Install),
            "canary" => Some(Self::Canary),
            "rollback" => Some(Self::Rollback),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowJobStatus {
    Pending,
    Running,
    RetryWait,
    Succeeded,
    Uncertain,
    Dead,
}

impl WorkflowJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::Uncertain => "uncertain",
            Self::Dead => "dead",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "retry_wait" => Some(Self::RetryWait),
            "succeeded" => Some(Self::Succeeded),
            "uncertain" => Some(Self::Uncertain),
            "dead" => Some(Self::Dead),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowJobEffectState {
    None,
    Started,
    Completed,
}

impl WorkflowJobEffectState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Started => "started",
            Self::Completed => "completed",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowJob {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: Option<String>,
    pub kind: WorkflowJobKind,
    pub payload_json: String,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowJobRecord {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: Option<String>,
    pub kind: WorkflowJobKind,
    pub status: WorkflowJobStatus,
    pub payload_json: String,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub lease_owner: Option<String>,
    pub lease_expires_at_unix_ms: Option<i64>,
    pub effect_state: WorkflowJobEffectState,
    pub result_json: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkflowJobRecoverySummary {
    pub retried_without_effect: usize,
    pub uncertain_effects: usize,
    pub dead: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowOutboxStatus {
    Pending,
    Delivering,
    RetryWait,
    Delivered,
    Dead,
}

impl WorkflowOutboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::RetryWait => "retry_wait",
            Self::Delivered => "delivered",
            Self::Dead => "dead",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "retry_wait" => Some(Self::RetryWait),
            "delivered" => Some(Self::Delivered),
            "dead" => Some(Self::Dead),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowOutboxItem {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: Option<String>,
    pub topic: String,
    pub payload_json: String,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowOutboxRecord {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: Option<String>,
    pub topic: String,
    pub payload_json: String,
    pub status: WorkflowOutboxStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub run_after_unix_ms: i64,
    pub lease_owner: Option<String>,
    pub lease_expires_at_unix_ms: Option<i64>,
    pub delivery_result_json: Option<String>,
    pub last_error: Option<String>,
    pub delivered_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkflowOutboxRecoverySummary {
    pub retried: usize,
    pub dead: usize,
}
