use super::kernel_agent_runtime::{context_window_for_model, DEFAULT_CONTEXT_WINDOW_TOKENS};
use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use captain_memory::session::Session;
use captain_runtime::compactor::{self, CompactionConfig, CompactionResult};
use captain_runtime::session_repair::{self, RepairStats};
use captain_types::agent::{AgentEntry, AgentId, SessionId};
use captain_types::error::CaptainError;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use tracing::warn;

impl CaptainKernel {
    async fn publish_compaction_visibility(&self, agent_id: AgentId, phase: &str, detail: &str) {
        let payload = serde_json::json!({
            "phase": phase,
            "detail": detail,
        });
        if let Err(err) =
            self.memory
                .append_session_event(&agent_id.to_string(), "compaction", &payload)
        {
            warn!(
                agent_id = %agent_id,
                phase,
                "compaction visibility persistence failed: {err}"
            );
        }

        let chat_payload = EventPayload::ChatStream(ChatStreamEvent::Phase {
            agent_id,
            phase: phase.to_string(),
            detail: Some(detail.to_string()),
        });
        let event = Event::new(agent_id, EventTarget::Agent(agent_id), chat_payload);
        self.event_bus.publish(event).await;
    }

    /// Compact an agent's session using LLM-based summarization.
    ///
    /// Replaces the existing text-truncation compaction with an intelligent
    /// LLM-generated summary of older messages, keeping only recent messages.
    pub async fn compact_agent_session(&self, agent_id: AgentId) -> KernelResult<String> {
        let entry = self.registry.get(agent_id).ok_or_else(|| {
            KernelError::Captain(CaptainError::AgentNotFound(agent_id.to_string()))
        })?;
        let session = self.compaction_session(agent_id, &entry)?;
        let config = self.compaction_config_for_entry(&entry);
        let estimated_tokens = self.estimated_compaction_tokens(agent_id, &entry, &session);

        if let Some(message) = no_compaction_needed_message(&session, &config, estimated_tokens) {
            return Ok(message);
        }

        let message_tokens = compactor::estimate_token_count(&session.messages, None, None);
        let overhead_tokens = estimated_tokens.saturating_sub(message_tokens);
        let result = self
            .run_session_compaction(
                agent_id,
                &entry,
                &session,
                &config,
                estimated_tokens,
                overhead_tokens,
            )
            .await?;
        let (updated_session, repair_stats) =
            self.save_compaction_result(agent_id, session, &config, &result)?;
        let msg = compaction_result_message(&result, updated_session.messages.len(), &repair_stats);

        self.publish_compaction_visibility(agent_id, "compacted", &msg)
            .await;

        Ok(msg)
    }

    fn compaction_session(&self, agent_id: AgentId, entry: &AgentEntry) -> KernelResult<Session> {
        self.memory
            .get_session(entry.session_id)
            .map_err(KernelError::Captain)
            .map(|session| session.unwrap_or_else(|| empty_session(entry.session_id, agent_id)))
    }

    fn compaction_config_for_entry(&self, entry: &AgentEntry) -> CompactionConfig {
        let effective_ctx_window = self.context_window_for_entry(entry);
        super::kernel_agent_runtime::compaction_config_for_manifest(
            &entry.manifest,
            Some(effective_ctx_window),
        )
    }

    fn context_window_for_entry(&self, entry: &AgentEntry) -> usize {
        self.model_catalog
            .read()
            .ok()
            .and_then(|cat| {
                context_window_for_model(
                    &cat,
                    &entry.manifest.model.provider,
                    &entry.manifest.model.model,
                )
            })
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }

    fn estimated_compaction_tokens(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &Session,
    ) -> usize {
        let tools = self.available_tools(agent_id);
        compactor::estimate_token_count(
            &session.messages,
            Some(&entry.manifest.model.system_prompt),
            Some(&tools),
        )
    }

    async fn run_session_compaction(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        session: &Session,
        config: &CompactionConfig,
        estimated_tokens: usize,
        overhead_tokens: usize,
    ) -> KernelResult<CompactionResult> {
        let driver = self.resolve_driver(&entry.manifest)?;
        let model = entry.manifest.model.model.clone();

        self.publish_compaction_visibility(
            agent_id,
            "compacting",
            &format!(
                "Compaction de session en cours : {} messages, ~{} tokens sur {} disponibles, conservation à partir d'une frontière de tour utilisateur.",
                session.messages.len(),
                estimated_tokens,
                config.context_window_tokens
            ),
        )
        .await;

        match compactor::compact_session(driver, &model, session, config, overhead_tokens).await {
            Ok(result) => Ok(result),
            Err(e) => {
                self.publish_compaction_visibility(
                    agent_id,
                    "compaction_failed",
                    "La compaction a échoué ; la session complète est conservée.",
                )
                .await;
                Err(KernelError::Captain(CaptainError::Internal(e)))
            }
        }
    }

    fn save_compaction_result(
        &self,
        agent_id: AgentId,
        mut session: Session,
        config: &CompactionConfig,
        result: &CompactionResult,
    ) -> KernelResult<(Session, RepairStats)> {
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

    fn context_report_window(&self, entry: &AgentEntry, session: &Session) -> u64 {
        if session.context_window_tokens > 0 {
            return session.context_window_tokens;
        }
        self.context_window_for_entry(entry) as u64
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
        let context_window = self.context_report_window(&entry, &session);

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
    let mut msg = if result.pruned_only {
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
    use crate::kernel::CaptainKernel;
    use captain_types::config::KernelConfig;
    use std::collections::HashMap;

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
}
