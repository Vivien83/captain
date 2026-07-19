//! Agent scheduler — manages agent execution and resource tracking.

use captain_memory::usage::{HourlyTokenUsage, UsageStore};
use captain_types::agent::{AgentId, ResourceQuota};
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::message::TokenUsage;
use captain_types::quota::QuotaExceededInfo;
use dashmap::DashMap;
use std::time::Instant;
use tokio::task::JoinHandle;
use tracing::debug;

/// Tracks resource usage for an agent with a rolling hourly window.
#[derive(Debug)]
pub struct UsageTracker {
    /// Total tokens consumed within the current window.
    pub total_tokens: u64,
    /// Total tool calls made within the current window.
    pub tool_calls: u64,
    /// Start of the current usage window.
    pub window_start: Instant,
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self {
            total_tokens: 0,
            tool_calls: 0,
            window_start: Instant::now(),
        }
    }
}

impl UsageTracker {
    /// Reset counters if the current window has expired (1 hour).
    fn reset_if_expired(&mut self) {
        if self.window_start.elapsed() >= std::time::Duration::from_secs(3600) {
            self.total_tokens = 0;
            self.tool_calls = 0;
            self.window_start = Instant::now();
        }
    }
}

/// The agent scheduler manages execution ordering and resource quotas.
pub struct AgentScheduler {
    /// Resource quotas per agent.
    quotas: DashMap<AgentId, ResourceQuota>,
    /// Usage tracking per agent.
    usage: DashMap<AgentId, UsageTracker>,
    /// Active task handles per agent.
    tasks: DashMap<AgentId, JoinHandle<()>>,
    /// Durable source of truth used by production kernels.
    usage_store: Option<UsageStore>,
}

impl AgentScheduler {
    /// Create a new scheduler.
    pub fn new() -> Self {
        Self {
            quotas: DashMap::new(),
            usage: DashMap::new(),
            tasks: DashMap::new(),
            usage_store: None,
        }
    }

    /// Create a scheduler backed by Captain's durable usage ledger.
    pub fn with_usage_store(usage_store: UsageStore) -> Self {
        Self {
            usage_store: Some(usage_store),
            ..Self::new()
        }
    }

    /// Register an agent with its resource quota.
    pub fn register(&self, agent_id: AgentId, quota: ResourceQuota) {
        self.quotas.insert(agent_id, quota);
        self.usage.insert(agent_id, UsageTracker::default());
    }

    /// Record token usage for an agent.
    pub fn record_usage(&self, agent_id: AgentId, usage: &TokenUsage) {
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.reset_if_expired();
            tracker.total_tokens += usage.total();
        }
    }

    /// Check if an agent has exceeded its quota.
    pub fn check_quota(&self, agent_id: AgentId) -> CaptainResult<()> {
        let quota = match self.quotas.get(&agent_id) {
            Some(q) => q.clone(),
            None => return Ok(()), // No quota = no limit
        };
        if quota.max_llm_tokens_per_hour == 0 {
            return Ok(());
        }

        let window = self.hourly_usage(agent_id, quota.max_llm_tokens_per_hour)?;
        if window.total_tokens >= quota.max_llm_tokens_per_hour {
            return Err(CaptainError::quota_exceeded(
                QuotaExceededInfo::agent_hourly_tokens(
                    agent_id.to_string(),
                    window.total_tokens,
                    quota.max_llm_tokens_per_hour,
                    window.resets_at,
                ),
            ));
        }

        Ok(())
    }

    /// Get current token usage for an agent. Returns (input, output) approximation.
    pub fn get_agent_usage(&self, agent_id: AgentId) -> Option<captain_types::message::TokenUsage> {
        let limit = self
            .quotas
            .get(&agent_id)
            .map(|quota| quota.max_llm_tokens_per_hour)
            .unwrap_or(0);
        self.hourly_usage_for_status(agent_id, limit).map(|usage| {
            captain_types::message::TokenUsage {
                input_tokens: usage.total_tokens / 2,
                output_tokens: usage.total_tokens - (usage.total_tokens / 2),
                ..Default::default()
            }
        })
    }

    /// Set (or update) the hourly token quota for a specific agent.
    pub fn set_hourly_quota(&self, agent_id: AgentId, max_tokens: u64) {
        if let Some(mut quota) = self.quotas.get_mut(&agent_id) {
            quota.max_llm_tokens_per_hour = max_tokens;
        } else {
            self.quotas.insert(
                agent_id,
                ResourceQuota {
                    max_llm_tokens_per_hour: max_tokens,
                    ..ResourceQuota::default()
                },
            );
            self.usage.insert(agent_id, UsageTracker::default());
        }
    }

    /// Reset the process-local approximation for an agent.
    ///
    /// The durable hourly ledger is deliberately preserved: session resets,
    /// model switches, and daemon restarts must not bypass a safety quota.
    pub fn reset_usage(&self, agent_id: AgentId) {
        if let Some(mut tracker) = self.usage.get_mut(&agent_id) {
            tracker.total_tokens = 0;
            tracker.tool_calls = 0;
            tracker.window_start = Instant::now();
        }
    }

    /// Abort an agent's active task.
    pub fn abort_task(&self, agent_id: AgentId) {
        if let Some((_, handle)) = self.tasks.remove(&agent_id) {
            handle.abort();
            debug!(agent = %agent_id, "Aborted agent task");
        }
    }

    /// Remove an agent from the scheduler.
    pub fn unregister(&self, agent_id: AgentId) {
        self.abort_task(agent_id);
        self.quotas.remove(&agent_id);
        self.usage.remove(&agent_id);
    }

    /// Get usage stats for an agent.
    pub fn get_usage(&self, agent_id: AgentId) -> Option<(u64, u64)> {
        let limit = self
            .quotas
            .get(&agent_id)
            .map(|quota| quota.max_llm_tokens_per_hour)
            .unwrap_or(0);
        self.hourly_usage_for_status(agent_id, limit)
            .map(|usage| (usage.total_tokens, usage.tool_calls))
    }

    /// Return the durable rolling window used by status surfaces.
    pub fn get_hourly_usage(&self, agent_id: AgentId) -> Option<HourlyTokenUsage> {
        let limit = self
            .quotas
            .get(&agent_id)
            .map(|quota| quota.max_llm_tokens_per_hour)
            .unwrap_or(0);
        self.hourly_usage_for_status(agent_id, limit)
    }

    /// Returns remaining token headroom before quota is hit.
    /// Returns `None` if no token quota is configured (unlimited).
    pub fn token_headroom(&self, agent_id: AgentId) -> Option<u64> {
        let quota = self.quotas.get(&agent_id)?;
        if quota.max_llm_tokens_per_hour == 0 {
            return None;
        }
        let used = self
            .hourly_usage_for_status(agent_id, quota.max_llm_tokens_per_hour)?
            .total_tokens;
        Some(quota.max_llm_tokens_per_hour.saturating_sub(used))
    }

    fn hourly_usage(&self, agent_id: AgentId, limit: u64) -> CaptainResult<HourlyTokenUsage> {
        if let Some(store) = &self.usage_store {
            return store.query_hourly_tokens(agent_id, limit);
        }
        Ok(self.local_hourly_usage(agent_id))
    }

    fn hourly_usage_for_status(&self, agent_id: AgentId, limit: u64) -> Option<HourlyTokenUsage> {
        match self.hourly_usage(agent_id, limit) {
            Ok(usage) => Some(usage),
            Err(error) => {
                tracing::warn!(agent = %agent_id, error = %error, "Durable quota usage unavailable");
                self.usage
                    .contains_key(&agent_id)
                    .then(|| self.local_hourly_usage(agent_id))
            }
        }
    }

    fn local_hourly_usage(&self, agent_id: AgentId) -> HourlyTokenUsage {
        self.usage
            .get_mut(&agent_id)
            .map(|mut tracker| {
                tracker.reset_if_expired();
                HourlyTokenUsage {
                    total_tokens: tracker.total_tokens,
                    tool_calls: tracker.tool_calls,
                    resets_at: None,
                }
            })
            .unwrap_or(HourlyTokenUsage {
                total_tokens: 0,
                tool_calls: 0,
                resets_at: None,
            })
    }
}

impl Default for AgentScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::migration::run_migrations;
    use captain_memory::usage::UsageRecord;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_record_usage() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        scheduler.register(id, ResourceQuota::default());
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        );
        let (tokens, _) = scheduler.get_usage(id).unwrap();
        assert_eq!(tokens, 150);
    }

    #[test]
    fn test_quota_check() {
        let scheduler = AgentScheduler::new();
        let id = AgentId::new();
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: 100,
            ..Default::default()
        };
        scheduler.register(id, quota);
        scheduler.record_usage(
            id,
            &TokenUsage {
                input_tokens: 60,
                output_tokens: 50,
                ..Default::default()
            },
        );
        assert!(scheduler.check_quota(id).is_err());
    }

    #[test]
    fn durable_quota_survives_reset_and_scheduler_restart() {
        let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        run_migrations(&connection.lock().unwrap()).unwrap();
        let store = UsageStore::new(connection);
        let id = AgentId::new();
        store
            .record(&UsageRecord {
                agent_id: id,
                model: "codex:test".to_string(),
                input_tokens: 70,
                output_tokens: 30,
                cached_input_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.0,
                tool_calls: 0,
            })
            .unwrap();
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: 100,
            ..Default::default()
        };

        let scheduler = AgentScheduler::with_usage_store(store.clone());
        scheduler.register(id, quota.clone());
        scheduler.reset_usage(id);
        let error = scheduler.check_quota(id).unwrap_err();
        assert_eq!(
            error.quota_info().map(|info| info.code.as_str()),
            Some("captain_agent_hourly_token_quota")
        );

        let restarted = AgentScheduler::with_usage_store(store);
        restarted.register(id, quota);
        assert!(restarted.check_quota(id).is_err());
    }
}
