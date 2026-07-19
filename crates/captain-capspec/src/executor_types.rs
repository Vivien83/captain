use crate::{CapabilityScope, Effect, RegistryError, TemplateError};
use async_trait::async_trait;
use captain_types::config::ExecPolicy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("CapSpec state protection failed: {0}")]
    Crypto(String),
    #[error("CapSpec executor lock was poisoned")]
    Poisoned,
    #[error("CapSpec run '{0}' was not found")]
    RunNotFound(String),
    #[error("CapSpec run '{0}' is already executing")]
    RunAlreadyExecuting(String),
    #[error("CapSpec tool '{0}' is not operational in this scope")]
    CapabilityUnavailable(String),
    #[error("invalid CapSpec input: {0}")]
    InvalidInput(String),
    #[error("CapSpec project scope does not match execution workspace: {0}")]
    WorkspaceMismatch(String),
    #[error("CapSpec runtime payload exceeds {limit} bytes ({actual} bytes)")]
    PayloadTooLarge { actual: usize, limit: usize },
    #[error("CapSpec node '{node_id}' was not found in run '{run_id}'")]
    NodeNotFound { run_id: String, node_id: String },
    #[error("CapSpec node '{node_id}' in run '{run_id}' is no longer uncertain")]
    NodeNotUncertain { run_id: String, node_id: String },
    #[error(
        "stale CapSpec decision for node '{node_id}' in run '{run_id}': expected attempt {expected_attempt} / tool use '{expected_tool_use_id}', current state is {actual_status:?} at attempt {actual_attempt} / tool use {actual_tool_use_id:?}"
    )]
    StaleUncertainDecision {
        run_id: String,
        node_id: String,
        expected_tool_use_id: String,
        expected_attempt: u32,
        actual_tool_use_id: Option<String>,
        actual_attempt: u32,
        actual_status: CapabilityNodeStatus,
    },
    #[error("CapSpec run '{run_id}' cannot resume while it is {status:?}")]
    NotResumable {
        run_id: String,
        status: CapabilityRunStatus,
    },
    #[error("CapSpec run '{run_id}' is pinned to unavailable revision '{source_hash}'")]
    RevisionUnavailable { run_id: String, source_hash: String },
    #[error("CapSpec scope denied for step '{step_id}' ({tool}): {reason}")]
    ScopeDenied {
        step_id: String,
        tool: String,
        reason: String,
    },
    #[error("CapSpec run '{run_id}' failed: {message}")]
    RunFailed { run_id: String, message: String },
    #[error("CapSpec run '{run_id}' was interrupted and can be resumed: {message}")]
    RunInterrupted { run_id: String, message: String },
    #[error("CapSpec run '{run_id}' is waiting for a decision about node '{node_id}'")]
    WaitingDecision { run_id: String, node_id: String },
    #[error("invalid persisted CapSpec run state: {0}")]
    InvalidState(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRunStatus {
    Pending,
    Running,
    Interrupted,
    WaitingDecision,
    Succeeded,
    Failed,
    Cancelled,
}

impl CapabilityRunStatus {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Interrupted => "interrupted",
            Self::WaitingDecision => "waiting_decision",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, ExecutorError> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "interrupted" => Ok(Self::Interrupted),
            "waiting_decision" => Ok(Self::WaitingDecision),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(ExecutorError::InvalidState(format!(
                "unknown run status '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityNodeStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Uncertain,
    Skipped,
}

impl CapabilityNodeStatus {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Skipped => "skipped",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, ExecutorError> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "uncertain" => Ok(Self::Uncertain),
            "skipped" => Ok(Self::Skipped),
            other => Err(ExecutorError::InvalidState(format!(
                "unknown node status '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    pub run_id: String,
    pub source_hash: String,
    pub step_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: Value,
    pub attempt: u32,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInvocationResult {
    pub content: String,
    pub is_error: bool,
}

impl CapabilityInvocationResult {
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }

    pub fn from_tool_result(result: captain_types::tool::ToolResult) -> Self {
        Self {
            content: result.content,
            is_error: result.is_error,
        }
    }
}

#[async_trait]
pub trait CapabilityToolInvoker: Send + Sync {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult;

    fn reviewed_effect(&self, tool_name: &str) -> Effect {
        crate::reviewed_effect(tool_name)
    }

    fn supports_idempotency(&self, _tool_name: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityExecution {
    pub run_id: String,
    pub source_hash: String,
    pub output: Value,
    pub completed_nodes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CapabilityExecutionContext {
    pub caller_agent_id: Option<String>,
    pub workspace: Option<String>,
    pub origin: String,
    #[serde(default)]
    pub authority: Option<CapabilityExecutionAuthority>,
}

/// Exact caller authority captured when a run starts.
///
/// This payload is encrypted at rest and is never included in public run
/// projections. Resumes intersect it with the caller's current authority so a
/// manifest change can revoke access but can never expand an in-flight run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityExecutionAuthority {
    pub allowed_tools: Vec<String>,
    pub allowed_env_vars: Option<Vec<String>>,
    pub exec_policy: Option<ExecPolicy>,
    pub subagent_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResumeContext {
    pub run: CapabilityRunView,
    pub execution: CapabilityExecutionContext,
    pub required_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRunView {
    pub run_id: String,
    pub scope: CapabilityScope,
    pub capability_name: String,
    pub tool_name: String,
    pub source_hash: String,
    pub status: CapabilityRunStatus,
    pub caller_agent_id: Option<String>,
    pub workspace: Option<String>,
    pub origin: String,
    pub created_at: String,
    pub updated_at: String,
    pub nodes: Vec<CapabilityNodeView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityNodeView {
    pub step_id: String,
    pub ordinal: usize,
    pub tool_name: String,
    pub effect: Effect,
    pub status: CapabilityNodeStatus,
    pub attempts: u32,
    pub tool_use_id: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum UncertainResolution {
    ConfirmSucceeded { output: Value },
    Retry,
    MarkFailed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainNodeExpectation {
    pub tool_use_id: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncertainResolutionReceipt {
    pub run: CapabilityRunView,
    pub resume_required: bool,
}
