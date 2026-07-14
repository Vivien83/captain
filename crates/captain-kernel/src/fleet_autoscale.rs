//! Fleet auto-scaling — decides when to spawn or kill workers based on load.
//!
//! Pure decision logic, isolated from kernel for easy testing. The kernel
//! evaluates each manager periodically and applies the resulting decision.

use captain_types::agent::{AgentId, AutoScaleConfig};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Snapshot of fleet load used to make a scaling decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadMetrics {
    pub manager_id: AgentId,
    pub active_workers: u32,
    pub idle_workers: u32,
    pub queue_depth: u32,
    pub tokens_used_last_window: u64,
    pub last_scale_event: Option<DateTime<Utc>>,
}

/// Scaling decision produced by the autoscaler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScaleDecision {
    NoChange,
    Spawn,
    Kill,
}

/// Evaluate scaling policy given current metrics and configured thresholds.
///
/// The cooldown window prevents thrashing: once a scale event happens,
/// the autoscaler waits `cooldown_secs` before acting again.
pub fn decide(
    metrics: &LoadMetrics,
    config: &AutoScaleConfig,
    now: DateTime<Utc>,
) -> ScaleDecision {
    if let Some(last) = metrics.last_scale_event {
        let elapsed = (now - last).num_seconds().max(0) as u64;
        if elapsed < config.cooldown_secs {
            return ScaleDecision::NoChange;
        }
    }

    let total = metrics.active_workers + metrics.idle_workers;

    if total < config.min_workers {
        return ScaleDecision::Spawn;
    }

    if total > config.max_workers {
        return ScaleDecision::Kill;
    }

    if metrics.queue_depth >= config.spawn_threshold && total < config.max_workers {
        return ScaleDecision::Spawn;
    }

    if metrics.queue_depth <= config.kill_threshold
        && metrics.idle_workers > 0
        && total > config.min_workers
    {
        return ScaleDecision::Kill;
    }

    ScaleDecision::NoChange
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(min: u32, max: u32, spawn_th: u32, kill_th: u32, cooldown: u64) -> AutoScaleConfig {
        AutoScaleConfig {
            enabled: true,
            min_workers: min,
            max_workers: max,
            spawn_threshold: spawn_th,
            kill_threshold: kill_th,
            cooldown_secs: cooldown,
            worker_template: None,
        }
    }

    fn metrics(active: u32, idle: u32, queue: u32, last: Option<DateTime<Utc>>) -> LoadMetrics {
        LoadMetrics {
            manager_id: AgentId::new(),
            active_workers: active,
            idle_workers: idle,
            queue_depth: queue,
            tokens_used_last_window: 0,
            last_scale_event: last,
        }
    }

    #[test]
    fn spawns_when_queue_exceeds_threshold() {
        let now = Utc::now();
        let d = decide(&metrics(1, 0, 3, None), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::Spawn);
    }

    #[test]
    fn does_not_spawn_above_max() {
        let now = Utc::now();
        let d = decide(&metrics(3, 0, 10, None), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::NoChange);
    }

    #[test]
    fn kills_idle_worker_when_queue_empty() {
        let now = Utc::now();
        let d = decide(&metrics(0, 2, 0, None), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::Kill);
    }

    #[test]
    fn respects_min_workers_on_kill() {
        let now = Utc::now();
        let d = decide(&metrics(0, 1, 0, None), &cfg(1, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::NoChange);
    }

    #[test]
    fn spawns_to_reach_min_workers() {
        let now = Utc::now();
        let d = decide(&metrics(0, 0, 0, None), &cfg(2, 3, 5, 0, 60), now);
        assert_eq!(d, ScaleDecision::Spawn);
    }

    #[test]
    fn cooldown_blocks_scaling() {
        let now = Utc::now();
        let recent = now - chrono::Duration::seconds(10);
        let d = decide(&metrics(1, 0, 10, Some(recent)), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::NoChange);
    }

    #[test]
    fn cooldown_expires_allows_scaling() {
        let now = Utc::now();
        let old = now - chrono::Duration::seconds(120);
        let d = decide(&metrics(1, 0, 10, Some(old)), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::Spawn);
    }

    #[test]
    fn no_kill_when_no_idle() {
        let now = Utc::now();
        let d = decide(&metrics(2, 0, 0, None), &cfg(0, 3, 2, 0, 60), now);
        assert_eq!(d, ScaleDecision::NoChange);
    }

    #[test]
    fn worker_tools_research_has_web_search() {
        let tools = crate::kernel::worker_tools_for_domain("research", "veille");
        assert!(tools.contains(&"web_search"));
        assert!(tools.contains(&"knowledge_add"));
    }

    #[test]
    fn worker_tools_ops_has_shell_exec() {
        let tools = crate::kernel::worker_tools_for_domain("ops", "devops");
        assert!(tools.contains(&"shell_exec"));
    }

    #[test]
    fn worker_tools_generic_falls_back_to_web() {
        let tools = crate::kernel::worker_tools_for_domain("random", "misc");
        assert!(tools.contains(&"web_search"));
        assert!(!tools.contains(&"shell_exec"));
    }
}
