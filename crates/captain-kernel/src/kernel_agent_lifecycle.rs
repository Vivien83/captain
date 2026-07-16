use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget, LifecycleEvent};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{info, warn};

impl CaptainKernel {
    /// Kill an agent and remove its runtime, persisted state, schedules, and audit footprint.
    pub fn kill_agent(&self, agent_id: AgentId) -> KernelResult<()> {
        let entry = self
            .registry
            .remove(agent_id)
            .map_err(KernelError::Captain)?;
        self.background.stop_agent(agent_id);
        self.scheduler.unregister(agent_id);
        self.capabilities.revoke_all(agent_id);
        self.event_bus.unsubscribe_agent(agent_id);
        let file_trigger_ids: Vec<_> = self
            .triggers
            .list_agent_file_triggers(agent_id)
            .into_iter()
            .map(|trigger| trigger.id)
            .collect();
        self.triggers.remove_agent_triggers(agent_id);
        for trigger_id in file_trigger_ids {
            self.drop_file_watcher(trigger_id);
        }

        // Hand reactivation deliberately preserves cron ownership until the
        // replacement hand agent is spawned with its stable id.
        if !self.reactivating_hand.load(Ordering::Relaxed) {
            let cron_removed = self.cron_scheduler.remove_agent_jobs(agent_id);
            if cron_removed > 0 {
                if let Err(e) = self.cron_scheduler.persist() {
                    warn!("Failed to persist cron jobs after agent deletion: {e}");
                }
            }
        }

        let _ = self.memory.remove_agent(agent_id);

        let _ = self.graph_memory.store_turn(
            &entry.manifest.name,
            "system",
            &format!("Agent {} arrêté", entry.manifest.name),
        );

        self.audit_log.record(
            agent_id.to_string(),
            captain_runtime::audit::AuditAction::AgentKill,
            format!("name={}", entry.name),
            "ok",
        );

        info!(agent = %entry.name, id = %agent_id, "Agent killed");

        // Publish the termination on the event bus (TUI/SSE/agent-API webhook
        // visibility) and wake the parent, if any, so it doesn't have to
        // remember to poll agent_status/agent_watch for a sub-agent it spawned.
        self.notify_agent_lifecycle_end(
            entry.parent,
            LifecycleEvent::Terminated {
                agent_id,
                reason: "killed".to_string(),
            },
        );

        Ok(())
    }

    /// Publish a `LifecycleEvent::Terminated`/`Crashed` event on the bus and,
    /// if the agent had a parent, inject a system message waking it up so it
    /// doesn't have to remember to poll agent_status/agent_watch/tool_run_status
    /// for work it dispatched to a sub-agent. Fire-and-forget: best-effort
    /// visibility/wake-up, never blocks the (synchronous) caller and never
    /// fails the underlying lifecycle transition it accompanies.
    pub(crate) fn notify_agent_lifecycle_end(
        &self,
        parent: Option<AgentId>,
        lifecycle_event: LifecycleEvent,
    ) {
        let (agent_id, description) = match &lifecycle_event {
            LifecycleEvent::Terminated { agent_id, reason } => {
                (*agent_id, format!("terminé ({reason})"))
            }
            LifecycleEvent::Crashed { agent_id, error } => {
                (*agent_id, format!("a planté ({error})"))
            }
            other => {
                warn!(
                    ?other,
                    "notify_agent_lifecycle_end called with an unsupported variant"
                );
                return;
            }
        };
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::Lifecycle(lifecycle_event),
        );
        // `try_current` because kill_agent is a plain sync fn reachable from
        // non-async unit tests — best-effort notification, safe to skip
        // there rather than panic ("no reactor running").
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let bus = self.event_bus.clone();
        let handle: Option<Arc<dyn KernelHandle>> = self
            .self_handle
            .get()
            .and_then(|w| w.upgrade())
            .map(|arc| arc as Arc<dyn KernelHandle>);

        runtime.spawn(async move {
            bus.publish(event).await;
            let Some(parent_id) = parent else { return };
            let Some(handle) = handle else { return };
            let message = format!(
                "Le sous-agent {agent_id} s'est {description}. \
                 Vérifie son résultat avec agent_status/agent_watch avant de continuer."
            );
            if let Err(e) = handle
                .inject_system_message(&parent_id.to_string(), &message)
                .await
            {
                warn!(parent = %parent_id, child = %agent_id, error = %e, "Failed to wake parent agent after child termination");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::CaptainKernel;
    use async_trait::async_trait;
    use captain_runtime::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
    use captain_types::agent::{
        AgentManifest, ManifestCapabilities, ModelConfig, Priority, ResourceQuota, ScheduleMode,
    };
    use captain_types::config::KernelConfig;
    use captain_types::event::{EventPayload, EventTarget, LifecycleEvent};
    use captain_types::message::{ContentBlock, MessageContent, StopReason, TokenUsage};
    use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    /// Captures every prompt it's asked to complete, so tests can assert on
    /// what a "wake-up" system message actually put in front of the model —
    /// without ever making a real network call.
    struct CapturingDriver {
        captured_user_texts: Mutex<Vec<String>>,
        captured: Notify,
    }

    #[async_trait]
    impl LlmDriver for CapturingDriver {
        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            for msg in &request.messages {
                match &msg.content {
                    MessageContent::Text(text) => {
                        self.captured_user_texts.lock().unwrap().push(text.clone());
                    }
                    MessageContent::Blocks(blocks) => {
                        for block in blocks {
                            if let ContentBlock::Text { text, .. } = block {
                                self.captured_user_texts.lock().unwrap().push(text.clone());
                            }
                        }
                    }
                }
            }
            self.captured.notify_one();
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "ok".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    ..Default::default()
                },
            })
        }
    }

    fn child_manifest(name: &str) -> AgentManifest {
        AgentManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: "test child agent".to_string(),
            author: "test".to_string(),
            module: "builtin:chat".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: vec![],
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: vec![],
            mcp_servers: vec![],
            metadata: HashMap::new(),
            tags: vec![],
            autonomous: None,
            workspace: None,
            generate_identity_files: false,
            exec_policy: None,
            tool_allowlist: vec![],
            tool_blocklist: vec![],
            orchestration_mode: captain_types::agent::OrchestrationMode::default(),
        }
    }

    #[test]
    fn kill_agent_removes_runtime_persistence_and_cron_jobs() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-agent-lifecycle-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };

        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");

        kernel
            .cron_scheduler
            .add_job(
                CronJob {
                    id: CronJobId::new(),
                    agent_id,
                    name: "agent cleanup test".to_string(),
                    enabled: true,
                    schedule: CronSchedule::Every { every_secs: 60 },
                    action: CronAction::AgentTurn {
                        message: "ping".to_string(),
                        model_override: None,
                        timeout_secs: None,
                    },
                    delivery: CronDelivery::None,
                    created_at: Utc::now(),
                    last_run: None,
                    next_run: None,
                },
                false,
            )
            .expect("cron job added");

        assert!(kernel.registry.get(agent_id).is_some());
        assert!(kernel
            .memory
            .load_agent(agent_id)
            .expect("agent load before kill")
            .is_some());
        assert_eq!(kernel.cron_scheduler.list_jobs(agent_id).len(), 1);

        kernel.kill_agent(agent_id).expect("agent killed");

        assert!(kernel.registry.get(agent_id).is_none());
        assert!(kernel
            .memory
            .load_agent(agent_id)
            .expect("agent load after kill")
            .is_none());
        assert!(kernel.cron_scheduler.list_jobs(agent_id).is_empty());

        kernel.shutdown();
    }

    #[tokio::test]
    async fn kill_agent_publishes_terminated_event_and_wakes_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp
            .path()
            .join("captain-kernel-agent-lifecycle-wakeup-test");
        std::fs::create_dir_all(&home_dir).unwrap();

        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            default_model: captain_types::config::DefaultModelConfig {
                provider: "static-test".to_string(),
                model: "static-test-model".to_string(),
                api_key_env: String::new(),
                base_url: None,
            },
            assistant: captain_types::config::AssistantConfig {
                onboarding_completed: true,
                ..captain_types::config::AssistantConfig::default()
            },
            ..KernelConfig::default()
        };

        let mut kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));
        let driver = Arc::new(CapturingDriver {
            captured_user_texts: Mutex::new(Vec::new()),
            captured: Notify::new(),
        });
        Arc::get_mut(&mut kernel)
            .expect("kernel has no shared references yet")
            .default_driver = driver.clone();
        kernel.set_self_handle();

        let parent_id = kernel
            .registry
            .list()
            .into_iter()
            .next()
            .expect("kernel should boot with at least one agent")
            .id;

        let child_id = kernel
            .spawn_agent_with_parent(child_manifest("wakeup-child"), Some(parent_id), None)
            .expect("child agent spawns");

        let mut bus_rx = kernel.event_bus.subscribe_all();

        kernel.kill_agent(child_id).expect("child agent killed");

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let event = bus_rx.recv().await.expect("event bus should not close");
                if let EventPayload::Lifecycle(LifecycleEvent::Terminated { agent_id, .. }) =
                    &event.payload
                {
                    if *agent_id == child_id {
                        return event;
                    }
                }
            }
        })
        .await
        .expect("Terminated lifecycle event must be published for the killed child");
        assert!(matches!(event.target, EventTarget::Broadcast));

        // The wake-up runs in a spawned task and executes a complete parent
        // turn. Wait on the test driver instead of relying on scheduler-speed
        // polling, which is flaky when the full release suite is under load.
        let wake_observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let captured = driver.captured.notified();
                if driver
                    .captured_user_texts
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|t| t.contains(&child_id.to_string()) && t.contains("s'est terminé"))
                {
                    return;
                }
                captured.await;
            }
        })
        .await
        .is_ok();
        let captured_texts = driver.captured_user_texts.lock().unwrap().clone();
        assert!(
            wake_observed
                && captured_texts
                    .iter()
                    .any(|t| t.contains(&child_id.to_string()) && t.contains("s'est terminé")),
            "parent agent should have been woken up with a message naming the terminated child, got: {captured_texts:?}"
        );

        kernel.shutdown();
    }
}
