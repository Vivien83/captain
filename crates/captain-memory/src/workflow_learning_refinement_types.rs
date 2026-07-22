#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRefinementState {
    AwaitingInput,
    Queued,
    Completed,
    Failed,
    Cancelled,
    Expired,
}

impl WorkflowRefinementState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AwaitingInput => "awaiting_input",
            Self::Queued => "queued",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "awaiting_input" => Some(Self::AwaitingInput),
            "queued" => Some(Self::Queued),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkflowRefinementRequest {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub expected_proposal_version: u64,
    pub actor: String,
    pub surface: String,
    pub conversation_key: String,
    pub source_message_id: Option<String>,
    pub language: String,
    pub expires_at_unix_ms: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRefinementRecord {
    pub id: String,
    pub idempotency_key: String,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub expected_proposal_version: u64,
    pub actor: String,
    pub surface: String,
    pub conversation_key: String,
    pub source_message_id: Option<String>,
    pub language: String,
    pub state: WorkflowRefinementState,
    pub state_version: u64,
    pub instruction: Option<String>,
    pub captured_message_id: Option<String>,
    pub child_proposal_id: Option<String>,
    pub draft_job_id: Option<String>,
    pub last_error: Option<String>,
    pub expires_at_unix_ms: i64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRefinementEvent {
    pub sequence: u64,
    pub idempotency_key: String,
    pub request_id: String,
    pub from_state: Option<WorkflowRefinementState>,
    pub to_state: WorkflowRefinementState,
    pub resulting_version: u64,
    pub actor: String,
    pub reason: String,
    pub created_at_unix_ms: i64,
}
