//! Reaper for ephemeral agents.
//!
//! Agents spawned for a one-off, throwaway purpose (demo/test sub-agents,
//! ad-hoc scouts/critics) have no automatic lifecycle end: `agent_kill`
//! exists, but nothing forces or reminds the orchestrating agent to call it.
//! Left unattended, such agents accumulate indefinitely in the registry —
//! observed live across multiple sessions with `demo-scout`/`demo-critic`
//! still `Running` well after their task was done.
//!
//! This sweep only ever touches agents explicitly tagged [`EPHEMERAL_TAG`]
//! (see `agent_spawn`'s tool description), so it never affects `captain`,
//! long-lived hands, or any agent the caller didn't mark as disposable.

use std::sync::Arc;
use std::time::Duration;

use tracing::info;

use crate::kernel::CaptainKernel;

/// Tag an `agent_spawn` manifest can set (`tags = ["ephemeral"]`) to opt an
/// agent into automatic cleanup once it goes idle.
pub const EPHEMERAL_TAG: &str = "ephemeral";

/// How long an ephemeral agent may sit idle before being reaped.
pub const EPHEMERAL_IDLE_TIMEOUT_SECS: i64 = 30 * 60;

/// How often the sweep runs.
pub const EPHEMERAL_SWEEP_INTERVAL_SECS: u64 = 5 * 60;

/// Pure decision: should this agent be reaped, given its tags and idle time?
/// Kept free of any kernel/registry dependency so it is trivially testable.
pub fn is_reapable_ephemeral_agent(tags: &[String], idle_secs: i64) -> bool {
    idle_secs >= EPHEMERAL_IDLE_TIMEOUT_SECS && tags.iter().any(|tag| tag == EPHEMERAL_TAG)
}

/// Start the periodic background sweep. Call once at kernel boot.
pub fn spawn_ephemeral_agent_reaper(kernel: Arc<CaptainKernel>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(EPHEMERAL_SWEEP_INTERVAL_SECS));
        loop {
            interval.tick().await;
            reap_idle_ephemeral_agents(&kernel);
        }
    });
}

/// Scan the registry once and kill any idle ephemeral agent found.
pub fn reap_idle_ephemeral_agents(kernel: &Arc<CaptainKernel>) {
    let now = chrono::Utc::now();
    for entry in kernel.registry.list() {
        let idle_secs = (now - entry.last_active).num_seconds();
        if !is_reapable_ephemeral_agent(&entry.tags, idle_secs) {
            continue;
        }
        match kernel.kill_agent(entry.id) {
            Ok(()) => info!(
                agent = %entry.name,
                id = %entry.id,
                idle_secs,
                "reaped idle ephemeral agent"
            ),
            Err(e) => tracing::warn!(
                agent = %entry.name,
                id = %entry.id,
                error = %e,
                "failed to reap idle ephemeral agent"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaps_ephemeral_agent_past_idle_timeout() {
        let tags = vec![EPHEMERAL_TAG.to_string()];
        assert!(is_reapable_ephemeral_agent(
            &tags,
            EPHEMERAL_IDLE_TIMEOUT_SECS
        ));
        assert!(is_reapable_ephemeral_agent(
            &tags,
            EPHEMERAL_IDLE_TIMEOUT_SECS + 1
        ));
    }

    #[test]
    fn keeps_ephemeral_agent_still_active() {
        let tags = vec![EPHEMERAL_TAG.to_string()];
        assert!(!is_reapable_ephemeral_agent(
            &tags,
            EPHEMERAL_IDLE_TIMEOUT_SECS - 1
        ));
    }

    #[test]
    fn never_reaps_agent_without_the_ephemeral_tag() {
        let tags = vec!["custom".to_string()];
        assert!(!is_reapable_ephemeral_agent(
            &tags,
            EPHEMERAL_IDLE_TIMEOUT_SECS + 1000
        ));

        let no_tags: Vec<String> = Vec::new();
        assert!(!is_reapable_ephemeral_agent(
            &no_tags,
            EPHEMERAL_IDLE_TIMEOUT_SECS + 1000
        ));
    }
}
