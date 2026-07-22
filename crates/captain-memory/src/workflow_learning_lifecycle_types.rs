use crate::workflow_learning_control::{WorkflowProposalRecord, WorkflowProposalTransition};
use crate::workflow_learning_installation::{
    WorkflowInstallationRecord, WorkflowInstallationTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxRecord};
use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobRecord};

#[derive(Debug, Clone)]
pub struct WorkflowInstallCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub proposal_transition: WorkflowProposalTransition,
    pub canary_job: NewWorkflowJob,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowCanaryCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub proposal_transition: WorkflowProposalTransition,
    pub installation_transition: WorkflowInstallationTransition,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowEffectFailure {
    pub job_id: String,
    pub worker: String,
    pub job_kind: WorkflowJobKind,
    pub error_code: String,
    pub error_message: String,
    pub proposal_transition: WorkflowProposalTransition,
    pub installation_transition: Option<WorkflowInstallationTransition>,
    pub rollback_job: Option<NewWorkflowJob>,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub failed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowRollbackCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub proposal_transition: WorkflowProposalTransition,
    pub installation_transition: WorkflowInstallationTransition,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLifecycleResult {
    pub proposal: WorkflowProposalRecord,
    pub job: WorkflowJobRecord,
    pub installation: Option<WorkflowInstallationRecord>,
    pub next_job: Option<WorkflowJobRecord>,
    pub notification: Option<WorkflowOutboxRecord>,
}
