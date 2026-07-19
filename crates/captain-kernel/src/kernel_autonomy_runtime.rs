use super::CaptainKernel;
use crate::background;
use crate::heartbeat::{
    check_agents, is_quiet_hours, HeartbeatConfig, HeartbeatStatus, RecoveryTracker,
};
use crate::triggers::TriggerEngine;
use captain_types::agent::{AgentId, AgentState, ScheduleMode};
use captain_types::event::{Event, EventPayload, EventTarget, LifecycleEvent, SystemEvent};
use std::sync::Arc;
use tracing::{debug, info, warn};

impl CaptainKernel {
    ///
    /// Periodically checks all running agents' last_active timestamps and
    /// publishes `HealthCheckFailed` events for unresponsive agents.
    pub(super) fn start_heartbeat_monitor(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        let config = HeartbeatConfig::default();
        let interval_secs = config.check_interval_secs;
        let recovery_tracker = RecoveryTracker::new();

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(config.check_interval_secs));

            loop {
                interval.tick().await;

                if kernel.supervisor.is_shutting_down() {
                    info!("Heartbeat monitor stopping (shutdown)");
                    break;
                }

                run_heartbeat_cycle(&kernel, &config, &recovery_tracker).await;
            }
        });

        info!("Heartbeat monitor started (interval: {}s)", interval_secs);
    }

    pub(super) fn register_proactive_triggers(
        &self,
        agent_id: AgentId,
        name: &str,
        conditions: &[String],
    ) {
        register_proactive_triggers(&self.triggers, agent_id, name, conditions);
    }

    /// Start the background loop / register triggers for a single agent.
    pub fn start_background_for_agent(
        self: &Arc<Self>,
        agent_id: AgentId,
        name: &str,
        schedule: &ScheduleMode,
    ) {
        // For proactive agents, auto-register triggers from conditions
        if let ScheduleMode::Proactive { conditions } = schedule {
            self.register_proactive_triggers(agent_id, name, conditions);
            info!(agent = %name, id = %agent_id, "Registered proactive triggers");
        }

        // Start continuous/periodic loops
        let kernel = Arc::clone(self);
        self.background
            .start_agent(agent_id, name, schedule, move |aid, msg| {
                let k = Arc::clone(&kernel);
                tokio::spawn(async move {
                    // Quota-throttled agents skip the tick entirely: retrying
                    // send_message every tick until the hourly window resets
                    // only produces a WARN storm and heartbeat churn.
                    if quota_throttled(&k, aid) {
                        debug!(agent_id = %aid, "Background tick skipped: quota exhausted");
                        return;
                    }
                    match k.send_message(aid, &msg).await {
                        Ok(_) => {}
                        Err(e) => {
                            // send_message already records the panic in supervisor,
                            // just log the background context here
                            warn!(agent_id = %aid, error = %e, "Background tick failed");
                        }
                    }
                })
            });
    }
}

/// True when the agent's hourly token quota is exhausted. Such an agent is
/// idle by design (throttled), not unresponsive.
fn quota_throttled(kernel: &CaptainKernel, agent_id: AgentId) -> bool {
    kernel.scheduler.check_quota(agent_id).is_err()
}

async fn run_heartbeat_cycle(
    kernel: &Arc<CaptainKernel>,
    config: &HeartbeatConfig,
    recovery_tracker: &RecoveryTracker,
) {
    kernel
        .active_streams
        .cleanup_stale(chrono::Duration::minutes(30));

    let statuses = check_agents(&kernel.registry, config);
    for status in &statuses {
        process_heartbeat_status(kernel, config, recovery_tracker, status).await;
    }
}

async fn process_heartbeat_status(
    kernel: &Arc<CaptainKernel>,
    config: &HeartbeatConfig,
    recovery_tracker: &RecoveryTracker,
    status: &HeartbeatStatus,
) {
    if heartbeat_agent_in_quiet_hours(kernel, status.agent_id) {
        return;
    }

    // An agent with a live run is working, not unresponsive — long LLM
    // turns routinely exceed the 60s heartbeat timeout.
    if kernel.running_tasks.contains_key(&status.agent_id) {
        debug!(
            agent = %status.name,
            "Heartbeat skip: agent has a run in progress"
        );
        return;
    }

    // A quota-throttled agent stops ticking on purpose. Marking it Crashed
    // and recovering it in a loop (observed live on researcher-hand: a
    // crash/recovery cycle every 2 minutes for hours) is pure churn — leave
    // it alone until the hourly usage window resets.
    if quota_throttled(kernel, status.agent_id) {
        debug!(
            agent = %status.name,
            "Heartbeat skip: agent is quota-throttled, not unresponsive"
        );
        return;
    }

    if status.state == AgentState::Crashed {
        handle_crashed_agent(kernel, config, recovery_tracker, status).await;
        return;
    }

    reset_recovered_agent_tracker(recovery_tracker, status);

    if status.unresponsive && status.state == AgentState::Running {
        mark_unresponsive_running_agent(kernel, status).await;
    }
}

fn heartbeat_agent_in_quiet_hours(kernel: &CaptainKernel, agent_id: AgentId) -> bool {
    kernel
        .registry
        .get(agent_id)
        .and_then(|entry| {
            entry
                .manifest
                .autonomous
                .as_ref()
                .and_then(|auto_cfg| auto_cfg.quiet_hours.as_ref())
                .map(|quiet_hours| is_quiet_hours(quiet_hours))
        })
        .unwrap_or(false)
}

async fn handle_crashed_agent(
    kernel: &Arc<CaptainKernel>,
    config: &HeartbeatConfig,
    recovery_tracker: &RecoveryTracker,
    status: &HeartbeatStatus,
) {
    let failures = recovery_tracker.failure_count(status.agent_id);

    if failures >= config.max_recovery_attempts {
        terminate_exhausted_crashed_agent(kernel, status, failures).await;
        return;
    }

    if !recovery_tracker.can_attempt(status.agent_id, config.recovery_cooldown_secs) {
        debug!(agent = %status.name, "Recovery cooldown active, skipping");
        return;
    }

    attempt_crashed_agent_recovery(kernel, config, recovery_tracker, status).await;
}

async fn terminate_exhausted_crashed_agent(
    kernel: &Arc<CaptainKernel>,
    status: &HeartbeatStatus,
    failures: u32,
) {
    if let Some(entry) = kernel.registry.get(status.agent_id) {
        if entry.state != AgentState::Crashed {
            return;
        }
        let _ = kernel
            .registry
            .set_state(status.agent_id, AgentState::Terminated);
        warn!(
            agent = %status.name,
            attempts = failures,
            "Agent exhausted all recovery attempts — marked Terminated. Manual restart required."
        );
        kernel
            .event_bus
            .publish(heartbeat_health_check_event(
                status,
                status.inactive_secs as u64,
            ))
            .await;
        kernel.notify_agent_lifecycle_end(
            entry.parent,
            LifecycleEvent::Terminated {
                agent_id: status.agent_id,
                reason: format!("recovery_exhausted after {failures} attempts"),
            },
        );
    }
}

async fn attempt_crashed_agent_recovery(
    kernel: &Arc<CaptainKernel>,
    config: &HeartbeatConfig,
    recovery_tracker: &RecoveryTracker,
    status: &HeartbeatStatus,
) {
    let attempt = recovery_tracker.record_attempt(status.agent_id);
    info!(
        agent = %status.name,
        attempt = attempt,
        max = config.max_recovery_attempts,
        "Auto-recovering crashed agent (attempt {}/{})",
        attempt,
        config.max_recovery_attempts
    );
    let _ = kernel
        .registry
        .set_state(status.agent_id, AgentState::Running);
    kernel
        .event_bus
        .publish(heartbeat_health_check_event(status, 0))
        .await;
}

fn reset_recovered_agent_tracker(recovery_tracker: &RecoveryTracker, status: &HeartbeatStatus) {
    if status.state == AgentState::Running
        && !status.unresponsive
        && recovery_tracker.failure_count(status.agent_id) > 0
    {
        info!(
            agent = %status.name,
            "Agent recovered successfully — resetting recovery tracker"
        );
        recovery_tracker.reset(status.agent_id);
    }
}

async fn mark_unresponsive_running_agent(kernel: &Arc<CaptainKernel>, status: &HeartbeatStatus) {
    let parent = kernel.registry.get(status.agent_id).and_then(|e| e.parent);
    let _ = kernel
        .registry
        .set_state(status.agent_id, AgentState::Crashed);
    warn!(
        agent = %status.name,
        inactive_secs = status.inactive_secs,
        "Unresponsive Running agent marked as Crashed for recovery"
    );

    kernel
        .event_bus
        .publish(heartbeat_health_check_event(
            status,
            status.inactive_secs as u64,
        ))
        .await;
    kernel.notify_agent_lifecycle_end(
        parent,
        LifecycleEvent::Crashed {
            agent_id: status.agent_id,
            error: format!("unresponsive for {}s", status.inactive_secs),
        },
    );
}

fn heartbeat_health_check_event(status: &HeartbeatStatus, unresponsive_secs: u64) -> Event {
    Event::new(
        status.agent_id,
        EventTarget::System,
        EventPayload::System(SystemEvent::HealthCheckFailed {
            agent_id: status.agent_id,
            unresponsive_secs,
        }),
    )
}

fn register_proactive_triggers(
    triggers: &TriggerEngine,
    agent_id: AgentId,
    name: &str,
    conditions: &[String],
) {
    for condition in conditions {
        if let Some(pattern) = background::parse_condition(condition) {
            let prompt = format!(
                "[PROACTIVE ALERT] Condition '{condition}' matched: {{{{event}}}}. \
                 Review and take appropriate action. Agent: {name}"
            );
            triggers.register(agent_id, pattern, prompt, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;
    use captain_types::message::TokenUsage;
    use std::collections::HashMap;

    #[tokio::test]
    async fn quota_throttled_agent_is_not_marked_crashed_by_heartbeat() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-quota-heartbeat-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");
        let _ = kernel.registry.set_state(agent_id, AgentState::Running);

        // Exhaust the hourly quota.
        kernel.scheduler.set_hourly_quota(agent_id, 100);
        let usage = TokenUsage {
            input_tokens: 200,
            output_tokens: 100,
            ..Default::default()
        };
        kernel.scheduler.record_usage(agent_id, &usage);
        kernel.record_usage_metering(agent_id, "codex", "quota-heartbeat-test", &usage, 1);
        assert!(quota_throttled(&kernel, agent_id));

        let status = HeartbeatStatus {
            agent_id,
            name: "browser".to_string(),
            inactive_secs: 90,
            unresponsive: true,
            state: AgentState::Running,
        };
        let heartbeat_config = HeartbeatConfig::default();
        let tracker = RecoveryTracker::new();

        // Unresponsive + throttled: the agent must stay Running.
        process_heartbeat_status(&kernel, &heartbeat_config, &tracker, &status).await;
        assert_eq!(
            kernel.registry.get(agent_id).unwrap().state,
            AgentState::Running
        );

        // A process-local reset must not bypass the durable quota.
        kernel.scheduler.reset_usage(agent_id);
        assert!(quota_throttled(&kernel, agent_id));

        // Same status with quota headroom: normal crash-marking applies.
        kernel.scheduler.set_hourly_quota(agent_id, 1_000);
        assert!(!quota_throttled(&kernel, agent_id));
        process_heartbeat_status(&kernel, &heartbeat_config, &tracker, &status).await;
        assert_eq!(
            kernel.registry.get(agent_id).unwrap().state,
            AgentState::Crashed
        );

        kernel.shutdown();
    }

    #[test]
    fn proactive_trigger_registration_keeps_prompt_and_ignores_invalid_conditions() {
        let triggers = TriggerEngine::new();
        let agent_id = AgentId::new();
        let conditions = vec![
            "event:agent_spawned".to_string(),
            "badprefix:ignored".to_string(),
            "memory:agent.*.status".to_string(),
        ];

        register_proactive_triggers(&triggers, agent_id, "watcher", &conditions);

        let registered = triggers.list_agent_triggers(agent_id);
        assert_eq!(registered.len(), 2);
        assert!(registered
            .iter()
            .all(|trigger| trigger.prompt_template.contains("Agent: watcher")));
        assert!(registered
            .iter()
            .any(|trigger| trigger.prompt_template.contains("event:agent_spawned")));
        assert!(registered
            .iter()
            .any(|trigger| trigger.prompt_template.contains("memory:agent.*.status")));
    }

    #[test]
    fn heartbeat_health_check_event_keeps_agent_and_unresponsive_secs() {
        let agent_id = AgentId::new();
        let status = HeartbeatStatus {
            agent_id,
            name: "watcher".to_string(),
            inactive_secs: 42,
            unresponsive: true,
            state: AgentState::Running,
        };

        let event = heartbeat_health_check_event(&status, status.inactive_secs as u64);

        assert_eq!(event.source, agent_id);
        assert!(matches!(event.target, EventTarget::System));
        assert!(matches!(
            event.payload,
            EventPayload::System(SystemEvent::HealthCheckFailed {
                agent_id: payload_agent,
                unresponsive_secs: 42,
            }) if payload_agent == agent_id
        ));
    }
}
