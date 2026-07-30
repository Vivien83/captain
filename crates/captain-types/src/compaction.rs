//! Shared contract for truthful session-compaction progress.

use crate::agent::{AgentId, SessionId};
use serde::{Deserialize, Serialize};

pub const COMPACTION_PROGRESS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionPhase {
    Preparing,
    Pruning,
    Summarizing,
    Chunking,
    Merging,
    Persisting,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionState {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionProgressUnit {
    Chunks,
}

/// Canonical progress projected to every Captain surface.
///
/// A percentage is intentionally absent from the wire contract. Consumers may
/// derive one only when exact completed/total units are present. Opaque LLM
/// calls therefore remain indeterminate instead of displaying invented work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionProgress {
    pub schema_version: u16,
    pub operation_id: String,
    pub runtime_instance_id: String,
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub phase: CompactionPhase,
    pub state: CompactionState,
    pub detail: String,
    pub message_count: usize,
    pub estimated_tokens: usize,
    pub context_window_tokens: usize,
    pub completed_units: Option<u32>,
    pub total_units: Option<u32>,
    pub unit: Option<CompactionProgressUnit>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
}

impl CompactionProgress {
    pub fn determinate_percent(&self) -> Option<u8> {
        let completed = self.completed_units?;
        let total = self.total_units?;
        if total == 0 {
            return None;
        }
        Some(((completed.min(total) as u64 * 100) / total as u64) as u8)
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self.state, CompactionState::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(completed_units: Option<u32>, total_units: Option<u32>) -> CompactionProgress {
        CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: "op-1".to_string(),
            runtime_instance_id: "runtime-1".to_string(),
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            phase: CompactionPhase::Chunking,
            state: CompactionState::Running,
            detail: "chunking".to_string(),
            message_count: 24,
            estimated_tokens: 10_000,
            context_window_tokens: 200_000,
            completed_units,
            total_units,
            unit: Some(CompactionProgressUnit::Chunks),
            started_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    #[test]
    fn percentage_exists_only_for_exact_non_zero_units() {
        assert_eq!(progress(Some(1), Some(4)).determinate_percent(), Some(25));
        assert_eq!(progress(Some(9), Some(4)).determinate_percent(), Some(100));
        assert_eq!(progress(None, None).determinate_percent(), None);
        assert_eq!(progress(Some(0), Some(0)).determinate_percent(), None);
    }

    #[test]
    fn wire_shape_is_stable_and_explicit() {
        let value = serde_json::to_value(progress(Some(2), Some(5))).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["phase"], "chunking");
        assert_eq!(value["state"], "running");
        assert_eq!(value["unit"], "chunks");
    }
}
