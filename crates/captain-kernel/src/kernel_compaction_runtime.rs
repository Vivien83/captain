use super::kernel_agent_runtime::{context_window_for_model, DEFAULT_CONTEXT_WINDOW_TOKENS};
use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use crate::event_bus::EventBus;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_runtime::compactor::{
    self, CompactionConfig, CompactionResult, CompactionStageObserver, CompactionStageUpdate,
};
use captain_runtime::session_repair::{self, RepairStats};
use captain_types::agent::{AgentId, AgentManifest, SessionId};
use captain_types::compaction::{
    CompactionPhase, CompactionProgress, CompactionState, COMPACTION_PROGRESS_SCHEMA_VERSION,
};
use captain_types::error::CaptainError;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use std::sync::Arc;
use tracing::{info, warn};

pub type CompactionProgressSink = Arc<dyn Fn(CompactionProgress) + Send + Sync>;

#[derive(Clone)]
struct CompactionProgressPublisher {
    memory: Arc<MemorySubstrate>,
    event_bus: EventBus,
    sink: Option<CompactionProgressSink>,
    operation_id: String,
    runtime_instance_id: String,
    agent_id: AgentId,
    session_id: SessionId,
    message_count: usize,
    estimated_tokens: usize,
    context_window_tokens: usize,
    started_at_ms: i64,
}

impl CompactionProgressPublisher {
    fn terminal_progress(
        &self,
        phase: CompactionPhase,
        state: CompactionState,
        detail: String,
    ) -> CompactionProgress {
        CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: self.operation_id.clone(),
            runtime_instance_id: self.runtime_instance_id.clone(),
            agent_id: self.agent_id,
            session_id: self.session_id,
            phase,
            state,
            detail,
            message_count: self.message_count,
            estimated_tokens: self.estimated_tokens,
            context_window_tokens: self.context_window_tokens,
            completed_units: None,
            total_units: None,
            unit: None,
            started_at_ms: self.started_at_ms,
            updated_at_ms: chrono::Utc::now()
                .timestamp_millis()
                .max(self.started_at_ms),
        }
    }

    async fn publish_stage(&self, update: CompactionStageUpdate) {
        self.publish(CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: self.operation_id.clone(),
            runtime_instance_id: self.runtime_instance_id.clone(),
            agent_id: self.agent_id,
            session_id: self.session_id,
            phase: update.phase,
            state: CompactionState::Running,
            detail: update.detail,
            message_count: self.message_count,
            estimated_tokens: self.estimated_tokens,
            context_window_tokens: self.context_window_tokens,
            completed_units: update.completed_units,
            total_units: update.total_units,
            unit: update.unit,
            started_at_ms: self.started_at_ms,
            updated_at_ms: chrono::Utc::now()
                .timestamp_millis()
                .max(self.started_at_ms),
        })
        .await;
    }

    async fn publish_terminal(
        &self,
        phase: CompactionPhase,
        state: CompactionState,
        detail: String,
    ) {
        self.publish(self.terminal_progress(phase, state, detail))
            .await;
    }

    async fn publish(&self, progress: CompactionProgress) {
        self.persist_and_notify(&progress);
        self.broadcast(progress).await;
    }

    fn persist_and_notify(&self, progress: &CompactionProgress) {
        if let Err(error) = self.memory.record_compaction_progress(progress) {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                operation_id = %self.operation_id,
                "compaction progress persistence failed: {error}"
            );
        }

        if let Some(sink) = self.sink.as_ref() {
            sink(progress.clone());
        }
    }

    async fn broadcast(&self, progress: CompactionProgress) {
        broadcast_compaction_progress(&self.event_bus, progress).await;
    }
}

async fn broadcast_compaction_progress(event_bus: &EventBus, progress: CompactionProgress) {
    let payload = EventPayload::ChatStream(ChatStreamEvent::CompactionProgress {
        progress: progress.clone(),
    });
    let agent_event = Event::new(
        progress.agent_id,
        EventTarget::Agent(progress.agent_id),
        payload.clone(),
    );
    event_bus.publish(agent_event).await;

    // Agent-targeted WebSockets and daemon-wide SSE subscribers use distinct
    // EventBus lanes. Mirror this idempotent state to the system lane without
    // broadcasting it to unrelated agents.
    let system_event = Event::new(progress.agent_id, EventTarget::System, payload);
    event_bus.publish(system_event).await;
}

struct CompactionTerminalGuard {
    publisher: CompactionProgressPublisher,
    armed: bool,
}

impl CompactionTerminalGuard {
    fn new(publisher: CompactionProgressPublisher) -> Self {
        Self {
            publisher,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CompactionTerminalGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let progress = self.publisher.terminal_progress(
            CompactionPhase::Interrupted,
            CompactionState::Interrupted,
            "Compaction was interrupted; the recoverable session was retained".to_string(),
        );
        self.publisher.persist_and_notify(&progress);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let publisher = self.publisher.clone();
            runtime.spawn(async move {
                publisher.broadcast(progress).await;
            });
        }
    }
}

impl CaptainKernel {
    pub(super) fn reconcile_compaction_progress_after_restart(&self) {
        let interrupted = match self.memory.reconcile_compaction_progress_after_restart(
            &self.instance_id,
            chrono::Utc::now().timestamp_millis(),
        ) {
            Ok(interrupted) => interrupted,
            Err(error) => {
                warn!("compaction restart reconciliation failed: {error}");
                return;
            }
        };
        if interrupted.is_empty() {
            return;
        }

        info!(
            count = interrupted.len(),
            "reconciled interrupted compaction operations after restart"
        );
        let event_bus = self.event_bus.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                for progress in interrupted {
                    broadcast_compaction_progress(&event_bus, progress).await;
                }
            });
        }
    }

    /// Compact an agent's session using LLM-based summarization.
    ///
    /// Replaces the existing text-truncation compaction with an intelligent
    /// LLM-generated summary of older messages, keeping only recent messages.
    pub async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String> {
        self.compact_agent_session_with_progress(agent_id, None)
            .await
    }

    pub async fn compact_agent_session_with_progress(
        &self,
        agent_id: AgentId,
        sink: Option<CompactionProgressSink>,
    ) -> KernelResult<String> {
        let lock = self
            .agent_msg_locks
            .entry(agent_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;
        let (session_id, manifest) = {
            let entry = self.registry.get(agent_id).ok_or_else(|| {
                KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
            })?;
            (entry.session_id, entry.manifest.clone())
        };
        self.compact_session_unlocked(agent_id, session_id, &manifest, sink)
            .await
    }

    pub(super) async fn compact_agent_session_in_turn(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        manifest: &AgentManifest,
        sink: Option<CompactionProgressSink>,
    ) -> KernelResult<String> {
        self.compact_session_unlocked(agent_id, session_id, manifest, sink)
            .await
    }

    async fn compact_session_unlocked(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        manifest: &AgentManifest,
        sink: Option<CompactionProgressSink>,
    ) -> KernelResult<String> {
        let session = self.compaction_session(agent_id, session_id)?;
        let config = self.compaction_config_for_manifest(manifest);
        let estimated_tokens = self.estimated_compaction_tokens(agent_id, manifest, &session);

        if let Some(message) = no_compaction_needed_message(&session, &config, estimated_tokens) {
            return Ok(message);
        }

        let publisher = CompactionProgressPublisher {
            memory: Arc::clone(&self.memory),
            event_bus: self.event_bus.clone(),
            sink,
            operation_id: uuid::Uuid::new_v4().to_string(),
            runtime_instance_id: self.instance_id.clone(),
            agent_id,
            session_id,
            message_count: session.messages.len(),
            estimated_tokens,
            context_window_tokens: config.context_window_tokens,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let mut terminal_guard = CompactionTerminalGuard::new(publisher.clone());
        publisher
            .publish_stage(CompactionStageUpdate {
                phase: CompactionPhase::Preparing,
                detail: format!(
                    "Preparing {} messages and approximately {} tokens",
                    session.messages.len(),
                    estimated_tokens
                ),
                completed_units: None,
                total_units: None,
                unit: None,
            })
            .await;

        let message_tokens = compactor::estimate_token_count(&session.messages, None, None);
        let overhead_tokens = estimated_tokens.saturating_sub(message_tokens);
        let result = match self
            .run_session_compaction(manifest, &session, &config, overhead_tokens, &publisher)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                terminal_guard.disarm();
                publisher
                    .publish_terminal(
                        CompactionPhase::Failed,
                        CompactionState::Failed,
                        "Compaction failed; the full session remains recoverable".to_string(),
                    )
                    .await;
                return Err(error);
            }
        };
        let (updated_session, repair_stats) = if result.is_unchanged() {
            (session, RepairStats::default())
        } else {
            publisher
                .publish_stage(CompactionStageUpdate {
                    phase: CompactionPhase::Persisting,
                    detail: "Persisting the compacted session".to_string(),
                    completed_units: None,
                    total_units: None,
                    unit: None,
                })
                .await;
            match self.save_compaction_result(agent_id, session, &config, &result) {
                Ok(saved) => saved,
                Err(error) => {
                    terminal_guard.disarm();
                    publisher
                    .publish_terminal(
                        CompactionPhase::Failed,
                        CompactionState::Failed,
                        "Compaction could not be persisted; the recoverable session was retained"
                            .to_string(),
                    )
                    .await;
                    return Err(error);
                }
            }
        };
        let msg = compaction_result_message(&result, updated_session.messages.len(), &repair_stats);

        terminal_guard.disarm();
        publisher
            .publish_terminal(
                CompactionPhase::Completed,
                CompactionState::Succeeded,
                msg.clone(),
            )
            .await;

        Ok(msg)
    }

    fn compaction_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> KernelResult<Session> {
        self.memory
            .get_session(session_id)
            .map_err(KernelError::Captain)
            .map(|session| session.unwrap_or_else(|| empty_session(session_id, agent_id)))
    }

    fn compaction_config_for_manifest(&self, manifest: &AgentManifest) -> CompactionConfig {
        let effective_ctx_window = self.context_window_for_manifest(manifest);
        super::kernel_agent_runtime::compaction_config_for_manifest(
            manifest,
            Some(effective_ctx_window),
        )
    }

    fn context_window_for_manifest(&self, manifest: &AgentManifest) -> usize {
        self.model_catalog
            .read()
            .ok()
            .and_then(|cat| {
                context_window_for_model(&cat, &manifest.model.provider, &manifest.model.model)
            })
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }

    fn estimated_compaction_tokens(
        &self,
        agent_id: AgentId,
        manifest: &AgentManifest,
        session: &Session,
    ) -> usize {
        let tools = self.available_tools(agent_id);
        compactor::estimate_token_count(
            &session.messages,
            Some(&manifest.model.system_prompt),
            Some(&tools),
        )
    }

    async fn run_session_compaction(
        &self,
        manifest: &AgentManifest,
        session: &Session,
        config: &CompactionConfig,
        overhead_tokens: usize,
        publisher: &CompactionProgressPublisher,
    ) -> KernelResult<CompactionResult> {
        let driver = self.resolve_driver(manifest)?;
        let model = manifest.model.model.clone();
        let (stage_tx, mut stage_rx) = tokio::sync::mpsc::unbounded_channel();
        let stage_publisher = publisher.clone();
        let stage_task = tokio::spawn(async move {
            while let Some(update) = stage_rx.recv().await {
                stage_publisher.publish_stage(update).await;
            }
        });
        let observer: CompactionStageObserver = Arc::new(move |update| {
            let _ = stage_tx.send(update);
        });

        let outcome = compactor::compact_session_with_progress(
            driver,
            &model,
            session,
            config,
            overhead_tokens,
            Some(observer.clone()),
        )
        .await;
        drop(observer);
        let _ = stage_task.await;

        outcome.map_err(|e| KernelError::Captain(CaptainError::Internal(e)))
    }

    fn save_compaction_result(
        &self,
        agent_id: AgentId,
        mut session: Session,
        config: &CompactionConfig,
        result: &CompactionResult,
    ) -> KernelResult<(Session, RepairStats)> {
        if result.is_unchanged() {
            return Ok((session, RepairStats::default()));
        }

        // Pruning-only rounds produce no summary: skip the canonical update so
        // the previous handoff summary is not clobbered by an empty string.
        if result.compacted_count > 0 {
            self.memory
                .store_llm_summary(agent_id, &result.summary, result.kept_messages.clone())
                .map_err(KernelError::Captain)?;
        }

        let (repaired_messages, repair_stats) =
            session_repair::validate_and_repair_with_stats(&result.kept_messages);
        session.messages = repaired_messages;
        session.context_window_tokens = config.context_window_tokens as u64;
        self.memory
            .save_session(&session)
            .map_err(KernelError::Captain)?;
        Ok((session, repair_stats))
    }

    fn context_report_window(&self, manifest: &AgentManifest, session: &Session) -> u64 {
        if session.context_window_tokens > 0 {
            return session.context_window_tokens;
        }
        self.context_window_for_manifest(manifest) as u64
    }

    /// Generate a context window usage report for an agent.
    pub fn context_report(
        &self,
        agent_id: AgentId,
    ) -> KernelResult<captain_runtime::compactor::ContextReport> {
        use captain_runtime::compactor::generate_context_report;

        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;

        let session = self
            .memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)?
            .unwrap_or_else(|| empty_session(entry.session_id, agent_id));

        let system_prompt = &entry.manifest.model.system_prompt;
        // Use the agent's actual filtered tools instead of all builtins
        let tools = self.available_tools(agent_id);
        // Use 200K default or the model's known context window
        let context_window = self.context_report_window(&entry.manifest, &session);

        Ok(generate_context_report(
            &session.messages,
            Some(system_prompt),
            Some(&tools),
            context_window as usize,
        ))
    }
}

fn no_compaction_needed_message(
    session: &Session,
    config: &CompactionConfig,
    estimated_tokens: usize,
) -> Option<String> {
    let by_messages = compactor::needs_compaction(session, config);
    let by_tokens = compactor::needs_compaction_by_tokens(estimated_tokens, config);
    if by_messages || by_tokens {
        return None;
    }
    Some(format!(
        "No compaction needed ({} messages, threshold {}, estimated {} / {} tokens)",
        session.messages.len(),
        config.threshold,
        estimated_tokens,
        compaction_token_threshold(config)
    ))
}

fn compaction_token_threshold(config: &CompactionConfig) -> usize {
    (config.context_window_tokens as f64 * config.token_threshold_ratio) as usize
}

fn compaction_result_message(
    result: &CompactionResult,
    kept_messages: usize,
    repair_stats: &RepairStats,
) -> String {
    let mut msg = if result.is_unchanged() {
        format!(
            "No completed older turn could be compacted safely; the recent coherent context remains intact ({} messages kept).",
            kept_messages
        )
    } else if result.compacted_count == 0 {
        format!(
            "Pruned {} old tool outputs; no LLM compaction needed ({} messages kept).",
            result.pruned_tool_results, kept_messages
        )
    } else {
        let mut base = format!(
            "Compacted {} messages into summary ({} chars), kept {} recent messages.",
            result.compacted_count,
            result.summary.len(),
            kept_messages
        );
        if result.pruned_tool_results > 0 {
            base.push_str(&format!(
                " Pruned {} old tool outputs first.",
                result.pruned_tool_results
            ));
        }
        base
    };
    append_repair_audit(&mut msg, repair_stats);
    msg
}

fn append_repair_audit(msg: &mut String, repair_stats: &RepairStats) {
    let repairs = repair_stats.orphaned_results_removed
        + repair_stats.synthetic_results_inserted
        + repair_stats.duplicates_removed
        + repair_stats.messages_merged;
    if repairs > 0 {
        msg.push_str(&format!(" Post-audit: repaired ({} orphaned removed, {} synthetic inserted, {} merged, {} deduped).",
            repair_stats.orphaned_results_removed,
            repair_stats.synthetic_results_inserted,
            repair_stats.messages_merged,
            repair_stats.duplicates_removed,
        ));
    } else {
        msg.push_str(" Post-audit: clean.");
    }
}

fn empty_session(session_id: SessionId, agent_id: AgentId) -> Session {
    Session {
        id: session_id,
        agent_id,
        messages: Vec::new(),
        context_window_tokens: 0,
        label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_bus::EventBus;
    use crate::kernel::CaptainKernel;
    use captain_memory::event_log::RangeQuery;
    use captain_types::config::KernelConfig;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn test_publisher(
        memory: Arc<MemorySubstrate>,
        event_bus: EventBus,
        agent_id: AgentId,
        session_id: SessionId,
        sink: Option<CompactionProgressSink>,
    ) -> CompactionProgressPublisher {
        CompactionProgressPublisher {
            memory,
            event_bus,
            sink,
            operation_id: "compact-test".to_string(),
            runtime_instance_id: "runtime-test".to_string(),
            agent_id,
            session_id,
            message_count: 42,
            estimated_tokens: 12_000,
            context_window_tokens: 200_000,
            started_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn progress_is_persisted_on_the_real_session_and_reaches_both_event_lanes() {
        let memory = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let event_bus = EventBus::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let mut agent_events = event_bus.subscribe_agent(agent_id);
        let mut global_events = event_bus.subscribe_all();
        let publisher = test_publisher(Arc::clone(&memory), event_bus, agent_id, session_id, None);

        publisher
            .publish_stage(CompactionStageUpdate {
                phase: CompactionPhase::Chunking,
                detail: "Processed chunk 1 of 4".to_string(),
                completed_units: Some(1),
                total_units: Some(4),
                unit: Some(captain_types::compaction::CompactionProgressUnit::Chunks),
            })
            .await;

        for event in [
            tokio::time::timeout(std::time::Duration::from_secs(1), agent_events.recv())
                .await
                .expect("agent event timeout")
                .expect("agent event"),
            tokio::time::timeout(std::time::Duration::from_secs(1), global_events.recv())
                .await
                .expect("global event timeout")
                .expect("global event"),
        ] {
            let EventPayload::ChatStream(ChatStreamEvent::CompactionProgress { progress }) =
                event.payload
            else {
                panic!("unexpected event payload");
            };
            assert_eq!(progress.session_id, session_id);
            assert_eq!(progress.determinate_percent(), Some(25));
        }

        let events = memory
            .read_session_events(&RangeQuery {
                session_id: session_id.to_string(),
                limit: Some(10),
                ..RangeQuery::default()
            })
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "compaction_progress");
        assert_eq!(events[0].payload["session_id"], session_id.to_string());
        assert!(memory
            .read_session_events(&RangeQuery {
                session_id: agent_id.to_string(),
                limit: Some(10),
                ..RangeQuery::default()
            })
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn dropping_an_active_compaction_records_an_interrupted_terminal_state() {
        let memory = Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap());
        let event_bus = EventBus::new();
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let mut global_events = event_bus.subscribe_all();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_sink = Arc::clone(&captured);
        let sink: CompactionProgressSink = Arc::new(move |progress| {
            captured_sink.lock().unwrap().push(progress);
        });
        let publisher = test_publisher(
            Arc::clone(&memory),
            event_bus,
            agent_id,
            session_id,
            Some(sink),
        );

        drop(CompactionTerminalGuard::new(publisher));

        let sink_progress = captured.lock().unwrap().clone();
        assert_eq!(sink_progress.len(), 1);
        assert_eq!(sink_progress[0].state, CompactionState::Interrupted);
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), global_events.recv())
            .await
            .expect("global interruption timeout")
            .expect("global interruption event");
        assert!(matches!(
            event.payload,
            EventPayload::ChatStream(ChatStreamEvent::CompactionProgress {
                progress: CompactionProgress {
                    state: CompactionState::Interrupted,
                    ..
                }
            })
        ));
        let events = memory
            .read_session_events(&RangeQuery {
                session_id: session_id.to_string(),
                limit: Some(10),
                ..RangeQuery::default()
            })
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["state"], "interrupted");
    }

    #[tokio::test]
    async fn kernel_restart_reconciles_and_broadcasts_stale_compaction() {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().join("captain-home"),
            data_dir: tmp.path().join("captain-data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let progress = CompactionProgress {
            schema_version: COMPACTION_PROGRESS_SCHEMA_VERSION,
            operation_id: "stale-compaction".to_string(),
            runtime_instance_id: "previous-runtime".to_string(),
            agent_id,
            session_id,
            phase: CompactionPhase::Summarizing,
            state: CompactionState::Running,
            detail: "Opaque model call in progress".to_string(),
            message_count: 30,
            estimated_tokens: 18_000,
            context_window_tokens: 200_000,
            completed_units: None,
            total_units: None,
            unit: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };
        kernel
            .memory
            .record_compaction_progress(&progress)
            .expect("running progress persists");
        let mut agent_events = kernel.event_bus.subscribe_agent(agent_id);

        kernel.reconcile_compaction_progress_after_restart();

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), agent_events.recv())
            .await
            .expect("reconciliation broadcast timeout")
            .expect("reconciliation event");
        assert!(matches!(
            event.payload,
            EventPayload::ChatStream(ChatStreamEvent::CompactionProgress {
                progress: CompactionProgress {
                    state: CompactionState::Interrupted,
                    operation_id,
                    ..
                }
            }) if operation_id == "stale-compaction"
        ));

        let events = kernel
            .memory
            .read_session_events(&RangeQuery {
                session_id: session_id.to_string(),
                limit: Some(10),
                ..RangeQuery::default()
            })
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["state"], "interrupted");

        kernel.reconcile_compaction_progress_after_restart();
        assert_eq!(
            kernel
                .memory
                .count_session_events_by_type(
                    &session_id.to_string(),
                    captain_memory::compaction_progress::COMPACTION_PROGRESS_EVENT_TYPE,
                )
                .unwrap(),
            2
        );
        kernel.shutdown();
    }

    #[test]
    fn context_report_uses_agent_filtered_tools_and_effective_window() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-context-report-test");
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

        let report = kernel.context_report(agent_id).expect("context report");

        assert!(report.context_window > 0);
        assert!(
            report.breakdown.tool_definition_tokens > 0,
            "context report should count the agent's filtered visible tools"
        );

        kernel.shutdown();
    }

    #[test]
    fn no_compaction_message_reports_message_and_token_thresholds() {
        let session = empty_session(SessionId::new(), AgentId::new());
        let config = CompactionConfig {
            threshold: 10,
            context_window_tokens: 1_000,
            token_threshold_ratio: 0.5,
            ..CompactionConfig::default()
        };

        let msg = no_compaction_needed_message(&session, &config, 100)
            .expect("empty session should not need compaction");

        assert!(msg.contains("0 messages"));
        assert!(msg.contains("threshold 10"));
        assert!(msg.contains("estimated 100 / 500 tokens"));
    }

    #[test]
    fn compaction_result_message_reports_clean_and_repaired_audits() {
        let result = CompactionResult {
            summary: "summary".to_string(),
            kept_messages: Vec::new(),
            compacted_count: 12,
            chunks_used: 1,
            used_fallback: false,
            pruned_tool_results: 0,
            pruned_only: false,
        };

        let clean = compaction_result_message(&result, 4, &RepairStats::default());
        assert_eq!(
            clean,
            "Compacted 12 messages into summary (7 chars), kept 4 recent messages. Post-audit: clean."
        );

        let repaired = compaction_result_message(
            &result,
            4,
            &RepairStats {
                orphaned_results_removed: 1,
                synthetic_results_inserted: 2,
                messages_merged: 3,
                duplicates_removed: 4,
                ..RepairStats::default()
            },
        );
        assert!(repaired.contains(
            "Post-audit: repaired (1 orphaned removed, 2 synthetic inserted, 3 merged, 4 deduped)."
        ));
    }

    #[test]
    fn compaction_result_message_reports_pruning() {
        let pruned_only = CompactionResult {
            summary: String::new(),
            kept_messages: Vec::new(),
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
            pruned_tool_results: 3,
            pruned_only: true,
        };
        let msg = compaction_result_message(&pruned_only, 20, &RepairStats::default());
        assert_eq!(
            msg,
            "Pruned 3 old tool outputs; no LLM compaction needed (20 messages kept). Post-audit: clean."
        );

        let pruned_then_compacted = CompactionResult {
            summary: "s".to_string(),
            kept_messages: Vec::new(),
            compacted_count: 8,
            chunks_used: 1,
            used_fallback: false,
            pruned_tool_results: 2,
            pruned_only: false,
        };
        let msg = compaction_result_message(&pruned_then_compacted, 6, &RepairStats::default());
        assert!(msg.contains("Pruned 2 old tool outputs first."));
    }

    #[test]
    fn unchanged_compaction_is_reported_without_an_empty_summary_claim() {
        let unchanged = CompactionResult {
            summary: String::new(),
            kept_messages: Vec::new(),
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
            pruned_tool_results: 0,
            pruned_only: false,
        };

        let msg = compaction_result_message(&unchanged, 34, &RepairStats::default());

        assert!(msg.contains("No completed older turn could be compacted safely"));
        assert!(msg.contains("34 messages kept"));
        assert!(!msg.contains("Compacted 0"));
        assert!(!msg.contains("into summary"));
    }

    #[test]
    fn unchanged_compaction_does_not_repair_or_rewrite_the_session() {
        use captain_types::message::{ContentBlock, Message, MessageContent, Role};

        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().join("captain-home"),
            data_dir: tmp.path().join("captain-data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let agent_id = AgentId::new();
        let session = Session {
            id: SessionId::new(),
            agent_id,
            messages: vec![
                Message::user("keep this exact active turn"),
                Message {
                    role: Role::User,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: "orphan-kept-for-proof".to_string(),
                        tool_name: "shell_exec".to_string(),
                        content: "exact raw result".to_string(),
                        is_error: false,
                    }]),
                },
            ],
            context_window_tokens: 272_000,
            label: None,
        };
        kernel
            .memory
            .save_session(&session)
            .expect("seed exact session");
        let result = CompactionResult {
            summary: String::new(),
            kept_messages: session.messages.clone(),
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
            pruned_tool_results: 0,
            pruned_only: false,
        };

        let (returned, repairs) = kernel
            .save_compaction_result(
                agent_id,
                session.clone(),
                &CompactionConfig::default(),
                &result,
            )
            .expect("unchanged result");
        let reloaded = kernel
            .memory
            .get_session(session.id)
            .expect("session read")
            .expect("stored session");

        assert_eq!(repairs, RepairStats::default());
        let expected_messages = serde_json::to_value(&session.messages).unwrap();
        assert_eq!(
            serde_json::to_value(&returned.messages).unwrap(),
            expected_messages
        );
        assert_eq!(
            serde_json::to_value(&reloaded.messages).unwrap(),
            expected_messages
        );
        assert_eq!(reloaded.context_window_tokens, 272_000);
        kernel.shutdown();
    }
}
