use crate::event_bus::EventBus;
use captain_memory::skill_proposals::Proposal;
use captain_runtime::outcome_detector::ClassifiedSignal;
use captain_runtime::reflection_job::ReflectionBatch;
use captain_types::agent::{AgentEntry, AgentId, ScheduleMode};
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use super::kernel_memory_bridge::KernelCommitNotifier;
use super::CaptainKernel;

impl CaptainKernel {
    /// Start background loops for all non-reactive agents.
    ///
    /// Must be called after the kernel is wrapped in `Arc` (e.g., from the daemon).
    /// Iterates the agent registry and starts background tasks for agents with
    /// `Continuous`, `Periodic`, or `Proactive` schedules.
    pub fn start_background_agents(self: &Arc<Self>) {
        self.spawn_bootstrap_support_tasks();
        let learning_aggressiveness = self.config.learning.effective_autonomy_aggressiveness();
        self.start_learning_pipeline(learning_aggressiveness);
        self.start_checkpoint_summarizer_if_configured();
        self.start_skill_synthesizer_pipeline(learning_aggressiveness);
        self.restore_persisted_hands();
        self.start_registry_background_agent_loops();
        self.spawn_runtime_service_loops();
    }

    fn spawn_bootstrap_support_tasks(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            kernel.generate_graph_snapshot().await;
        });
        if let Some(ref driver) = self.embedding_driver {
            crate::tool_rag::spawn_tool_embedding_task(
                Arc::clone(&self.tool_embedding_cache),
                Arc::clone(driver),
            );
        }
        captain_runtime::active_project::install(&self.config.home_dir);
        crate::builtin_crons::ensure_all(self);
        crate::codex_model_updates::spawn_codex_model_catalog_monitor(Arc::clone(self));
        crate::milestone_alerts::spawn_deadline_alert_task(Arc::clone(self));
        crate::ephemeral_agents::spawn_ephemeral_agent_reaper(Arc::clone(self));
        self.bootstrap_user_facts_if_empty();
        captain_runtime::memory_writer::spawn_resync_worker(
            self.memory.usage_conn(),
            Arc::clone(&self.mcp_connections),
        );
    }

    fn start_learning_pipeline(self: &Arc<Self>, learning_aggressiveness: f32) {
        if self.config.learning.enabled {
            if let Some(signal_rx) = captain_runtime::learning_bus::install(
                captain_runtime::learning_bus::DEFAULT_CAPACITY,
            ) {
                let (_det_handle, classified_rx) =
                    captain_runtime::outcome_detector::OutcomeDetector::spawn(signal_rx, 256);
                let (reflection_model, reflection_rx) =
                    self.spawn_learning_reflection_consumer(classified_rx);
                let (declarative_rx, procedural_skill_rx) =
                    self.spawn_cognitive_learning_router(reflection_rx, learning_aggressiveness);
                spawn_skill_proposal_event_publisher(
                    self.event_bus.clone(),
                    self.config.language.clone(),
                    procedural_skill_rx,
                );
                let filtered_rx = self.spawn_learning_memory_policy(declarative_rx);
                self.spawn_learning_memory_committer(filtered_rx, reflection_model.clone());
                info!(
                    mode = ?self.config.learning.mode,
                    model = %reflection_model,
                    autonomy_aggressiveness = learning_aggressiveness,
                    "v3.12b-g learning pipeline live end-to-end"
                );
            }
        } else {
            info!("v3.12 learning pipeline disabled via [learning] enabled=false");
        }
    }

    fn spawn_learning_reflection_consumer(
        &self,
        classified_rx: mpsc::Receiver<ClassifiedSignal>,
    ) -> (String, mpsc::Receiver<ReflectionBatch>) {
        let mut reflection_cfg: captain_runtime::reflection_job::ReflectionConfig =
            (&self.config.learning).into();
        reflection_cfg.primary_model = self.resolve_learning_reflection_model();
        reflection_cfg.fallback_models = self.resolve_learning_reflection_fallbacks();
        let reflection_model = reflection_cfg.primary_model.clone();
        let completer = self.build_reflection_completer();
        let (_reflect_handle, reflection_rx) = captain_runtime::reflection_job::spawn_consumer(
            classified_rx,
            completer,
            reflection_cfg,
            64,
        );
        (reflection_model, reflection_rx)
    }

    fn spawn_cognitive_learning_router(
        &self,
        reflection_rx: mpsc::Receiver<ReflectionBatch>,
        learning_aggressiveness: f32,
    ) -> (mpsc::Receiver<ReflectionBatch>, mpsc::Receiver<Proposal>) {
        let cognitive_skill_policy = if self.config.skills.enabled {
            Some(self.skill_proposal_policy(learning_aggressiveness))
        } else {
            None
        };
        let (_cog_handle, declarative_rx, procedural_skill_rx) =
            captain_runtime::cognitive_router::spawn_router(
                reflection_rx,
                cognitive_skill_policy,
                self.memory.usage_conn(),
                self.config.language.clone(),
                64,
            );
        (declarative_rx, procedural_skill_rx)
    }

    fn spawn_learning_memory_policy(
        &self,
        declarative_rx: mpsc::Receiver<ReflectionBatch>,
    ) -> mpsc::Receiver<ReflectionBatch> {
        let policy_cfg: captain_runtime::memory_policy::PolicyConfig =
            (&self.config.learning).into();
        let policy = Arc::new(captain_runtime::memory_policy::MemoryPolicy::new(
            policy_cfg,
        ));
        let dedup: Arc<dyn captain_runtime::memory_policy::DedupChecker> = Arc::new(
            captain_runtime::memory_policy::SqliteMemoryDedupChecker::new(self.memory.usage_conn()),
        );
        let (_pol_handle, filtered_rx) =
            captain_runtime::memory_policy::spawn_filter(declarative_rx, policy, dedup, 64);
        filtered_rx
    }

    fn spawn_learning_memory_committer(
        &self,
        filtered_rx: mpsc::Receiver<ReflectionBatch>,
        reflection_model: String,
    ) {
        let notifier: Option<Arc<dyn captain_runtime::memory_committer::CommitNotifier>> =
            Some(Arc::new(KernelCommitNotifier::new(self.event_bus.clone())));
        let (_com_handle, _committed_rx) = captain_runtime::memory_committer::spawn_consumer(
            filtered_rx,
            self.memory.usage_conn(),
            Arc::clone(&self.mcp_connections),
            self.config.learning.mode,
            64,
            reflection_model,
            notifier,
        );
    }

    fn start_checkpoint_summarizer_if_configured(&self) {
        if self.config.checkpoints.enabled {
            let sessions_root = self.config.home_dir.join("sessions");
            let completer = self.build_checkpoint_completer();
            let model = self.resolve_checkpoint_model();
            let summarizer_config = captain_runtime::session_summarizer::SessionSummarizerConfig {
                inactivity_secs: self.config.checkpoints.inactivity_secs,
                scan_interval_secs: self.config.checkpoints.scan_interval_secs,
                per_summary_delay_secs: self.config.checkpoints.per_summary_delay_secs,
                transcript_cap_chars: self.config.checkpoints.transcript_cap_chars,
                emit_learning_review: self.config.checkpoints.emit_learning_review,
            };
            captain_runtime::session_summarizer::spawn_session_summarizer_task(
                completer,
                sessions_root,
                model,
                summarizer_config,
            );
        }
    }

    fn start_skill_synthesizer_pipeline(self: &Arc<Self>, learning_aggressiveness: f32) {
        if self.config.skills.enabled {
            let mut det_cfg: captain_runtime::pattern_detector::DetectorConfig =
                (&self.config.skills).into();
            det_cfg.apply_autonomy_aggressiveness(learning_aggressiveness);
            let effective_pattern_threshold = det_cfg.threshold;
            if let Some(candidate_rx) =
                captain_runtime::pattern_detector::install(self.memory.usage_conn(), det_cfg, 64)
            {
                let mut proposer_cfg: captain_runtime::skill_proposer::ProposerConfig =
                    (&self.config.skills).into();
                proposer_cfg.apply_autonomy_aggressiveness(learning_aggressiveness);
                proposer_cfg.primary_model = self.resolve_skills_proposer_model();
                proposer_cfg.fallback_models = self.resolve_skills_proposer_fallbacks();
                proposer_cfg.language = self.config.language.clone();
                let completer = self.build_proposer_completer();
                let (_prop_handle, proposal_rx) = captain_runtime::skill_proposer::spawn_consumer(
                    candidate_rx,
                    completer,
                    proposer_cfg,
                    self.memory.usage_conn(),
                    32,
                );
                let policy = self.skill_proposal_policy(learning_aggressiveness);
                let (_pol_handle, enqueued_rx) = captain_runtime::proposal_policy::spawn_middleware(
                    proposal_rx,
                    policy,
                    self.memory.usage_conn(),
                    "captain".to_string(),
                    32,
                );
                spawn_skill_proposal_event_publisher(
                    self.event_bus.clone(),
                    self.config.language.clone(),
                    enqueued_rx,
                );
                info!(
                    mode = ?self.config.skills.mode,
                    threshold = effective_pattern_threshold,
                    autonomy_aggressiveness = learning_aggressiveness,
                    model = %self.resolve_skills_proposer_model(),
                    "v3.13a-e skill synthesizer pipeline live"
                );
            }
        } else {
            info!("v3.13 skill synthesizer disabled via [skills] enabled=false");
        }
    }

    fn skill_proposal_policy(
        &self,
        learning_aggressiveness: f32,
    ) -> Arc<captain_runtime::proposal_policy::ProposalPolicy> {
        let mut policy_cfg: captain_runtime::proposal_policy::PolicyConfig =
            (&self.config.skills).into();
        policy_cfg.apply_autonomy_aggressiveness(learning_aggressiveness);
        Arc::new(
            captain_runtime::proposal_policy::ProposalPolicy::with_skill_diff(
                policy_cfg,
                self.skill_diff_config(),
            ),
        )
    }

    fn start_registry_background_agent_loops(self: &Arc<Self>) {
        let bg_agents = background_agent_specs(&self.registry.list());
        if !bg_agents.is_empty() {
            let count = bg_agents.len();
            let kernel = Arc::clone(self);
            tokio::spawn(async move {
                for (i, (id, name, schedule)) in bg_agents.into_iter().enumerate() {
                    kernel.start_background_for_agent(id, &name, &schedule);
                    if let Some(delay) = background_agent_start_delay(i) {
                        tokio::time::sleep(delay).await;
                    }
                }
                info!("Started {count} background agent loop(s) (staggered)");
            });
        }
    }

    fn spawn_runtime_service_loops(self: &Arc<Self>) {
        self.start_heartbeat_monitor();

        if self.config.network_enabled && !self.config.network.shared_secret.is_empty() {
            let kernel = Arc::clone(self);
            tokio::spawn(async move {
                kernel.start_ofp_node().await;
            });
        }

        self.spawn_local_provider_probe();
        self.spawn_metering_cleanup_loop();
        self.spawn_memory_consolidation_loop();

        self.spawn_graph_dream_cycle();
        self.spawn_neural_heartbeat();
        self.spawn_telegram_consciousness_digest();

        self.spawn_mcp_connection_if_configured();
        self.spawn_extension_health_monitor();
        self.spawn_workflow_autoload();

        self.spawn_cron_scheduler_loop();
        self.log_network_status_from_config();
        self.spawn_a2a_discovery_if_configured();
        self.spawn_whatsapp_gateway_if_configured();
    }
}

fn spawn_skill_proposal_event_publisher(
    event_bus: EventBus,
    language: String,
    mut rx: mpsc::Receiver<Proposal>,
) {
    tokio::spawn(async move {
        while let Some(proposal) = rx.recv().await {
            let payload = EventPayload::ChatStream(ChatStreamEvent::SkillProposalQueued {
                proposal_id: proposal.id,
                name: proposal.name,
                description: proposal.description,
                trigger_hint: proposal.trigger_hint,
                tool_sequence: proposal.tool_sequence,
                confidence: proposal.confidence,
                family: Some(proposal.family),
                language: Some(language.clone()),
                source_agent_id: proposal.source_agent_id,
                channel: proposal.origin_channel,
            });
            let event = Event::new(AgentId::default(), EventTarget::Broadcast, payload);
            event_bus.publish(event).await;
        }
    });
}

fn background_agent_specs(entries: &[AgentEntry]) -> Vec<(AgentId, String, ScheduleMode)> {
    entries
        .iter()
        .filter(|entry| !matches!(entry.manifest.schedule, ScheduleMode::Reactive))
        .map(|entry| {
            (
                entry.id,
                entry.name.clone(),
                entry.manifest.schedule.clone(),
            )
        })
        .collect()
}

fn background_agent_start_delay(index: usize) -> Option<std::time::Duration> {
    (index > 0).then(|| std::time::Duration::from_millis(500))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{
        AgentIdentity, AgentManifest, AgentMode, AgentState, ManifestCapabilities, SessionId,
    };

    fn entry(name: &str, schedule: ScheduleMode) -> AgentEntry {
        let manifest = AgentManifest {
            name: name.to_string(),
            schedule,
            capabilities: ManifestCapabilities::default(),
            ..AgentManifest::default()
        };

        AgentEntry {
            id: AgentId::new(),
            name: manifest.name.clone(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::Full,
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        }
    }

    #[test]
    fn background_specs_skip_reactive_agents_and_preserve_order() {
        let periodic = ScheduleMode::Periodic {
            cron: "*/5 * * * *".to_string(),
        };
        let continuous = ScheduleMode::Continuous {
            check_interval_secs: 30,
        };
        let entries = vec![
            entry("reactive", ScheduleMode::Reactive),
            entry("periodic", periodic),
            entry("continuous", continuous),
        ];

        let specs = background_agent_specs(&entries);

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].1, "periodic");
        assert_eq!(specs[1].1, "continuous");
    }

    #[test]
    fn background_agent_start_delay_skips_first_agent_only() {
        assert_eq!(background_agent_start_delay(0), None);
        assert_eq!(
            background_agent_start_delay(1),
            Some(std::time::Duration::from_millis(500))
        );
        assert_eq!(
            background_agent_start_delay(3),
            Some(std::time::Duration::from_millis(500))
        );
    }
}
