//! Channel-neutral operator projection for durable workflow learning.

use serde::{Deserialize, Serialize};

pub const PROPOSAL_CARD_SCHEMA_VERSION: u16 = 2;
pub const WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION: u16 = 1;
pub const WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCardState {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCardKind {
    Skill,
    Capspec,
    Automation,
    Refinement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCardRisk {
    ReadOnly,
    Mutation,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalCardAction {
    Activate,
    Test,
    Details,
    Edit,
    Later,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalInstallMode {
    Activate,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalIsolatedTestStatus {
    Queued,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalIsolatedTestCheck {
    pub code: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalIsolatedTest {
    pub status: ProposalIsolatedTestStatus,
    pub revision_sha256: String,
    pub job_id: String,
    pub checks: Vec<ProposalIsolatedTestCheck>,
    pub completed_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowIsolatedTestReport {
    pub schema_version: u16,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub artifact_sha256: String,
    pub kind: ProposalCardKind,
    pub name: String,
    pub passed: bool,
    pub checks: Vec<ProposalIsolatedTestCheck>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowLifecycleEvent {
    InstallationVerified,
    ActivationCompleted,
    ActivationFailed,
    RollbackCompleted,
    RollbackFailed,
}

/// Exact, channel-neutral projection of a durable activation transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLifecycleCard {
    pub schema_version: u16,
    pub event: WorkflowLifecycleEvent,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub decision_version: u64,
    pub state: ProposalCardState,
    pub kind: ProposalCardKind,
    pub name: String,
    pub lifecycle_job_id: String,
    pub continuation_job_id: Option<String>,
    pub target_locator: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub rollback_job_id: Option<String>,
    pub occurred_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowProjectionStatus {
    Building,
    Verified,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowInstallationViewPhase {
    Prepared,
    Promoted,
    Verified,
    Active,
    RollbackPending,
    RolledBack,
    Quarantined,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowInstallationView {
    pub phase: WorkflowInstallationViewPhase,
    pub phase_version: u64,
    pub target_locator: String,
    pub last_error: Option<String>,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "domain", rename_all = "snake_case")]
pub enum WorkflowTimelineEntry {
    Proposal {
        sequence: u64,
        from_state: Option<ProposalCardState>,
        to_state: ProposalCardState,
        resulting_version: u64,
        actor: String,
        reason: String,
        occurred_at_unix_ms: i64,
    },
    Installation {
        sequence: u64,
        from_phase: Option<WorkflowInstallationViewPhase>,
        to_phase: WorkflowInstallationViewPhase,
        resulting_version: u64,
        actor: String,
        reason: String,
        last_error: Option<String>,
        occurred_at_unix_ms: i64,
    },
}

impl WorkflowTimelineEntry {
    pub fn occurred_at_unix_ms(&self) -> i64 {
        match self {
            Self::Proposal {
                occurred_at_unix_ms,
                ..
            }
            | Self::Installation {
                occurred_at_unix_ms,
                ..
            } => *occurred_at_unix_ms,
        }
    }

    pub fn sequence(&self) -> u64 {
        match self {
            Self::Proposal { sequence, .. } | Self::Installation { sequence, .. } => *sequence,
        }
    }
}

/// Shared durable projection consumed by API, TUI, web and desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLearningView {
    pub schema_version: u16,
    pub proposal_id: String,
    pub decision_version: u64,
    pub state: ProposalCardState,
    pub revision_sha256: Option<String>,
    pub kind: Option<ProposalCardKind>,
    pub name: Option<String>,
    pub source_agent_id: String,
    pub origin_channel: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub projection_status: WorkflowProjectionStatus,
    pub projection_error: Option<String>,
    pub card: Option<ProposalCard>,
    pub installation: Option<WorkflowInstallationView>,
    pub timeline: Vec<WorkflowTimelineEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowLearningList {
    pub schema_version: u16,
    pub returned: usize,
    pub workflows: Vec<WorkflowLearningView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalOperatorOutcome {
    Details,
    EditRequested {
        request_id: String,
        expires_at_unix_ms: i64,
    },
    InstallQueued {
        mode: ProposalInstallMode,
    },
    Snoozed {
        until_unix_ms: i64,
    },
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalOperatorContext {
    pub surface: String,
    pub conversation_key: String,
    pub source_message_id: Option<String>,
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRefinementMessage {
    pub actor: String,
    pub surface: String,
    pub conversation_key: String,
    pub message_id: String,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRefinementCaptureResolution {
    pub request_id: String,
    pub parent_proposal_id: String,
    pub child_proposal_id: String,
    pub language: String,
    pub replayed: bool,
}

impl ProposalCardAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Activate => "activate",
            Self::Test => "test",
            Self::Details => "details",
            Self::Edit => "edit",
            Self::Later => "later",
            Self::Ignore => "ignore",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalCardEvidence {
    pub occurrences: u32,
    pub distinct_turns: u32,
    pub distinct_sessions: u32,
    pub explicit_reuse_request: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalCardStep {
    pub index: u32,
    pub tool_name: String,
    pub role: String,
    pub dependencies: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalCardValidationFact {
    pub code: String,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalCardModel {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalCard {
    pub schema_version: u16,
    pub proposal_id: String,
    pub lookup_token: String,
    pub decision_version: u64,
    pub revision_sha256: String,
    pub state: ProposalCardState,
    pub kind: ProposalCardKind,
    pub name: String,
    pub purpose: String,
    pub trigger: String,
    pub evidence: ProposalCardEvidence,
    pub steps: Vec<ProposalCardStep>,
    pub validation: Vec<ProposalCardValidationFact>,
    pub validation_limitations: Vec<String>,
    pub isolated_test: Option<ProposalIsolatedTest>,
    pub validated_by: ProposalCardModel,
    pub required_authority: Vec<String>,
    pub expected_benefit: String,
    pub risk: ProposalCardRisk,
    pub recommended_action: ProposalCardAction,
    pub available_actions: Vec<ProposalCardAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalOperatorResolution {
    pub card: ProposalCard,
    pub outcome: ProposalOperatorOutcome,
    pub replayed: bool,
    pub retire_keyboard: bool,
}
