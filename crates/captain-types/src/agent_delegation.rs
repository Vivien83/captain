//! Durable sub-agent delegation contracts shared by storage, kernel and UI.

use serde::{Deserialize, Serialize};

pub const AGENT_DELEGATION_MAX_TASK_BYTES: usize = 64 * 1024;
pub const AGENT_DELEGATION_MAX_RESULT_BYTES: usize = 256 * 1024;
pub const AGENT_DELEGATION_MAX_DEPENDENCIES: usize = 16;
pub const AGENT_DELEGATION_MAX_ACTIVE_PER_CALLER: usize = 32;
pub const AGENT_DELEGATION_MAX_TOKENS: u64 = 500_000;
pub const AGENT_DELEGATION_MAX_LINEAGE_TOKENS: u64 = 500_000;
pub const AGENT_DELEGATION_MAX_DEPTH: u32 = 10;
pub const AGENT_DELEGATION_MAX_ATTEMPTS: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDelegationStatus {
    Blocked,
    Queued,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
    DependencyFailed,
}

impl AgentDelegationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Queued => "queued",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Uncertain => "uncertain",
            Self::DependencyFailed => "dependency_failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "blocked" => Some(Self::Blocked),
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "cancel_requested" => Some(Self::CancelRequested),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "uncertain" => Some(Self::Uncertain),
            "dependency_failed" => Some(Self::DependencyFailed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Cancelled
                | Self::Uncertain
                | Self::DependencyFailed
        )
    }

    pub fn is_success(self) -> bool {
        self == Self::Succeeded
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDelegationEffectState {
    NotStarted,
    Started,
    Completed,
}

impl AgentDelegationEffectState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::Started => "started",
            Self::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "not_started" => Some(Self::NotStarted),
            "started" => Some(Self::Started),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAgentDelegationJob {
    pub id: String,
    pub idempotency_key: String,
    pub root_job_id: String,
    pub parent_job_id: Option<String>,
    pub depth: u32,
    pub caller_agent_id: String,
    pub target_agent_id: String,
    pub title: String,
    pub task: String,
    pub max_tokens: u64,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationJobRecord {
    pub id: String,
    pub idempotency_key: String,
    pub root_job_id: String,
    pub parent_job_id: Option<String>,
    pub depth: u32,
    pub lineage_reserved_tokens: u64,
    pub caller_agent_id: String,
    pub target_agent_id: String,
    pub title: String,
    pub task: String,
    pub max_tokens: u64,
    pub depends_on: Vec<String>,
    pub status: AgentDelegationStatus,
    pub state_version: u64,
    pub attempt_count: u32,
    pub lease_owner: Option<String>,
    pub lease_expires_at_unix_ms: Option<i64>,
    pub effect_state: AgentDelegationEffectState,
    pub result: Option<String>,
    pub result_truncated: bool,
    pub used_tokens: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub cancel_requested_at_unix_ms: Option<i64>,
    pub started_at_unix_ms: Option<i64>,
    pub completed_at_unix_ms: Option<i64>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDelegationRecoverySummary {
    pub requeued_without_effect: usize,
    pub cancelled_without_effect: usize,
    pub uncertain_after_effect: usize,
    pub dependency_failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_contract_is_stable_and_terminal_is_exact() {
        for status in [
            AgentDelegationStatus::Blocked,
            AgentDelegationStatus::Queued,
            AgentDelegationStatus::Running,
            AgentDelegationStatus::CancelRequested,
            AgentDelegationStatus::Succeeded,
            AgentDelegationStatus::Failed,
            AgentDelegationStatus::Cancelled,
            AgentDelegationStatus::Uncertain,
            AgentDelegationStatus::DependencyFailed,
        ] {
            assert_eq!(AgentDelegationStatus::parse(status.as_str()), Some(status));
        }
        assert!(!AgentDelegationStatus::CancelRequested.is_terminal());
        assert!(AgentDelegationStatus::Uncertain.is_terminal());
        assert!(AgentDelegationStatus::Succeeded.is_success());
        assert!(!AgentDelegationStatus::Failed.is_success());
    }

    #[test]
    fn effect_state_contract_is_stable() {
        for state in [
            AgentDelegationEffectState::NotStarted,
            AgentDelegationEffectState::Started,
            AgentDelegationEffectState::Completed,
        ] {
            assert_eq!(
                AgentDelegationEffectState::parse(state.as_str()),
                Some(state)
            );
        }
    }
}
