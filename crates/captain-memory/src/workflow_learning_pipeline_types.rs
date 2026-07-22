use crate::workflow_learning_control::{
    PublishValidatedDraft, WorkflowProposalRecord, WorkflowProposalTransition,
};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxRecord};
use crate::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind, WorkflowJobRecord};

#[derive(Debug, Clone)]
pub struct WorkflowAnalysisCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub eligibility_transition: WorkflowProposalTransition,
    pub drafting_transition: WorkflowProposalTransition,
    pub draft_job: NewWorkflowJob,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowDraftCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub proposal_transition: WorkflowProposalTransition,
    pub validation_job: NewWorkflowJob,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowValidationCompletion {
    pub job_id: String,
    pub worker: String,
    pub result_json: Option<String>,
    pub publish: PublishValidatedDraft,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkflowPipelineRejection {
    pub job_id: String,
    pub worker: String,
    pub job_kind: WorkflowJobKind,
    pub result_json: Option<String>,
    pub proposal_transition: WorkflowProposalTransition,
    pub notification: Option<NewWorkflowOutboxItem>,
    pub completed_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowPipelineResult {
    pub proposal: WorkflowProposalRecord,
    pub job: WorkflowJobRecord,
    pub next_job: Option<WorkflowJobRecord>,
    pub notification: Option<WorkflowOutboxRecord>,
}
