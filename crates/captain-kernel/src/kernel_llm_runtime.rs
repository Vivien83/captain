use captain_memory::session::Session;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_types::agent::{AgentId, AgentManifest};
use captain_types::config::UsageFooterMode;
use captain_types::tool::ToolDefinition;
use tracing::{info, warn};

use super::kernel_memory_bridge::append_daily_memory_log;
use super::CaptainKernel;

#[derive(Debug, Clone, Copy)]
pub(super) enum LlmPreLoopCompactionStage {
    StreamingAuto,
    NonStreamingInitial,
    NonStreamingFinal,
}

impl CaptainKernel {
    pub(super) fn plan_llm_session_compaction_before_loop(
        &self,
        agent_id: AgentId,
        session: &Session,
        manifest: &AgentManifest,
        tools: &[ToolDefinition],
        context_window: usize,
    ) -> LlmPreLoopCompactionDecision {
        llm_pre_loop_compaction_decision(
            session,
            manifest,
            tools,
            context_window,
            self.scheduler.token_headroom(agent_id),
        )
    }

    pub(super) async fn compact_llm_session_before_loop(
        &self,
        agent_id: AgentId,
        session: &mut Session,
        manifest: &AgentManifest,
        tools: &[ToolDefinition],
        context_window: usize,
        stage: LlmPreLoopCompactionStage,
    ) {
        let decision = self.plan_llm_session_compaction_before_loop(
            agent_id,
            session,
            manifest,
            tools,
            context_window,
        );
        self.execute_llm_session_compaction_plan(
            agent_id,
            session,
            context_window,
            stage,
            decision,
        )
        .await;
    }

    pub(super) async fn execute_llm_session_compaction_plan(
        &self,
        agent_id: AgentId,
        session: &mut Session,
        context_window: usize,
        stage: LlmPreLoopCompactionStage,
        decision: LlmPreLoopCompactionDecision,
    ) {
        log_llm_compaction_decision(agent_id, session, context_window, stage, &decision);
        // Durable task checkpoint: fires each time context usage crosses
        // into a new 20%-of-budget bucket, and always right before a
        // compaction runs (count-triggered compactions happen far below
        // the first bucket).
        self.maybe_write_task_checkpoint(
            agent_id,
            session,
            decision.estimated_tokens,
            context_window,
            decision.should_compact(),
        );
        if !decision.should_compact() {
            return;
        }

        log_llm_compaction_start(agent_id, session, context_window, stage, &decision);
        match self.compact_agent_session(agent_id).await {
            Ok(msg) => {
                info!(agent_id = %agent_id, "{msg}");
                if let Ok(Some(reloaded)) = self.memory.get_session(session.id) {
                    *session = reloaded;
                    if matches!(stage, LlmPreLoopCompactionStage::NonStreamingFinal) {
                        session.context_window_tokens = context_window as u64;
                    }
                }
            }
            Err(e) => {
                log_llm_compaction_failure(agent_id, stage, &e.to_string());
            }
        }
    }

    pub(super) fn finish_non_streaming_llm_success(
        &self,
        agent_id: AgentId,
        session: &Session,
        messages_before: usize,
        manifest: &AgentManifest,
        mut result: AgentLoopResult,
    ) -> AgentLoopResult {
        if session.messages.len() > messages_before {
            let new_messages = session.messages[messages_before..].to_vec();
            if let Err(e) = self.memory.append_canonical(agent_id, &new_messages, None) {
                warn!("Failed to update canonical session: {e}");
            }
        }

        if let Some(ref workspace) = manifest.workspace {
            if let Err(e) = self
                .memory
                .write_jsonl_mirror(session, &workspace.join("sessions"))
            {
                warn!("Failed to write JSONL session mirror: {e}");
            }
            append_daily_memory_log(workspace, &result.response);
        }

        let cost = self.record_usage_metering(
            agent_id,
            &manifest.model.provider,
            &manifest.model.model,
            &result.total_usage,
            result.iterations,
        );
        apply_usage_footer_cost(&self.config.usage_footer, &mut result, cost);
        result
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LlmPreLoopCompactionDecision {
    by_messages: bool,
    by_tokens: bool,
    by_quota: bool,
    estimated_tokens: usize,
    quota_headroom: Option<u64>,
}

impl LlmPreLoopCompactionDecision {
    fn should_compact(&self) -> bool {
        self.by_messages || self.by_tokens || self.by_quota
    }
}

fn llm_pre_loop_compaction_decision(
    session: &Session,
    manifest: &AgentManifest,
    tools: &[ToolDefinition],
    context_window: usize,
    quota_headroom: Option<u64>,
) -> LlmPreLoopCompactionDecision {
    let config =
        super::kernel_agent_runtime::compaction_config_for_manifest(manifest, Some(context_window));
    let by_messages = captain_runtime::compactor::needs_compaction(session, &config);
    let estimated_tokens = captain_runtime::compactor::estimate_token_count(
        &session.messages,
        Some(&manifest.model.system_prompt),
        Some(tools),
    );
    let by_tokens =
        captain_runtime::compactor::needs_compaction_by_tokens(estimated_tokens, &config);
    let by_quota = quota_headroom
        .map(|headroom| {
            let threshold = (headroom as f64 * 0.8) as u64;
            estimated_tokens as u64 > threshold && session.messages.len() > 4
        })
        .unwrap_or(false);

    LlmPreLoopCompactionDecision {
        by_messages,
        by_tokens,
        by_quota,
        estimated_tokens,
        quota_headroom,
    }
}

fn log_llm_compaction_decision(
    agent_id: AgentId,
    session: &Session,
    _context_window: usize,
    stage: LlmPreLoopCompactionStage,
    decision: &LlmPreLoopCompactionDecision,
) {
    if matches!(stage, LlmPreLoopCompactionStage::StreamingAuto)
        && decision.by_tokens
        && !decision.by_messages
    {
        info!(
            agent_id = %agent_id,
            estimated_tokens = decision.estimated_tokens,
            messages = session.messages.len(),
            "Token-based compaction triggered (messages below threshold but tokens above)"
        );
    }

    if matches!(stage, LlmPreLoopCompactionStage::StreamingAuto) && decision.by_quota {
        if let Some(headroom) = decision.quota_headroom {
            info!(
                agent_id = %agent_id,
                estimated_tokens = decision.estimated_tokens,
                quota_headroom = headroom,
                "Quota-headroom compaction triggered (session would consume >80% of remaining quota)"
            );
        }
    }
}

fn log_llm_compaction_start(
    agent_id: AgentId,
    session: &Session,
    context_window: usize,
    stage: LlmPreLoopCompactionStage,
    decision: &LlmPreLoopCompactionDecision,
) {
    match stage {
        LlmPreLoopCompactionStage::StreamingAuto => {
            info!(
                agent_id = %agent_id,
                messages = session.messages.len(),
                "Auto-compacting session"
            );
        }
        LlmPreLoopCompactionStage::NonStreamingInitial => {
            info!(
                agent_id = %agent_id,
                messages = session.messages.len(),
                estimated_tokens = decision.estimated_tokens,
                "Pre-emptive compaction before LLM call"
            );
        }
        LlmPreLoopCompactionStage::NonStreamingFinal => {
            info!(
                agent_id = %agent_id,
                messages = session.messages.len(),
                estimated_tokens = decision.estimated_tokens,
                context_window = context_window,
                "Final pre-emptive compaction before LLM call"
            );
        }
    }
}

fn log_llm_compaction_failure(agent_id: AgentId, stage: LlmPreLoopCompactionStage, error: &str) {
    match stage {
        LlmPreLoopCompactionStage::StreamingAuto => {
            warn!(agent_id = %agent_id, "Auto-compaction failed: {error}");
        }
        LlmPreLoopCompactionStage::NonStreamingInitial => {
            warn!(agent_id = %agent_id, "Pre-emptive compaction failed: {error}");
        }
        LlmPreLoopCompactionStage::NonStreamingFinal => {
            warn!(agent_id = %agent_id, "Final pre-emptive compaction failed: {error}");
        }
    }
}

fn apply_usage_footer_cost(mode: &UsageFooterMode, result: &mut AgentLoopResult, cost: f64) {
    match mode {
        UsageFooterMode::Off | UsageFooterMode::Tokens => {
            result.cost_usd = None;
        }
        UsageFooterMode::Cost | UsageFooterMode::Full => {
            result.cost_usd = if cost > 0.0 { Some(cost) } else { None };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::SessionId;
    use captain_types::message::Message;
    use captain_types::message::{ReplyDirectives, TokenUsage};
    use captain_types::tool::ToolDefinition;

    fn session_with_messages(count: usize) -> Session {
        Session {
            id: SessionId(uuid::Uuid::new_v4()),
            agent_id: AgentId(uuid::Uuid::new_v4()),
            messages: (0..count)
                .map(|idx| Message::user(format!("message {idx}")))
                .collect(),
            context_window_tokens: 200_000,
            label: None,
        }
    }

    fn manifest_with_prompt(provider: &str, prompt: &str) -> AgentManifest {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = provider.to_string();
        manifest.model.system_prompt = prompt.to_string();
        manifest
    }

    fn tiny_tool() -> ToolDefinition {
        ToolDefinition {
            name: "tiny".to_string(),
            description: "small test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn result_with_cost(cost: Option<f64>) -> AgentLoopResult {
        AgentLoopResult {
            response: "ok".to_string(),
            total_usage: TokenUsage::default(),
            iterations: 1,
            cost_usd: cost,
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls: Vec::new(),
        }
    }

    #[test]
    fn pre_loop_compaction_uses_quota_headroom_after_four_messages() {
        let manifest = manifest_with_prompt("anthropic", "");
        let tools = vec![tiny_tool()];
        let below_message_floor = llm_pre_loop_compaction_decision(
            &session_with_messages(4),
            &manifest,
            &tools,
            200_000,
            Some(1),
        );
        assert!(!below_message_floor.by_quota);
        assert!(!below_message_floor.should_compact());

        let above_message_floor = llm_pre_loop_compaction_decision(
            &session_with_messages(5),
            &manifest,
            &tools,
            200_000,
            Some(1),
        );
        assert!(above_message_floor.by_quota);
        assert!(above_message_floor.should_compact());
    }

    #[test]
    fn pre_loop_compaction_uses_effective_context_window_for_tokens() {
        let manifest = manifest_with_prompt("anthropic", &"system ".repeat(80));
        let tools = vec![tiny_tool()];
        let decision =
            llm_pre_loop_compaction_decision(&session_with_messages(1), &manifest, &tools, 1, None);
        assert!(decision.by_tokens);
        assert!(decision.should_compact());
    }

    #[test]
    fn usage_footer_hides_cost_when_disabled_or_tokens_only() {
        let mut off = result_with_cost(Some(1.25));
        apply_usage_footer_cost(&UsageFooterMode::Off, &mut off, 9.0);
        assert_eq!(off.cost_usd, None);

        let mut tokens = result_with_cost(Some(1.25));
        apply_usage_footer_cost(&UsageFooterMode::Tokens, &mut tokens, 9.0);
        assert_eq!(tokens.cost_usd, None);
    }

    #[test]
    fn usage_footer_sets_positive_cost_for_cost_modes() {
        let mut cost = result_with_cost(None);
        apply_usage_footer_cost(&UsageFooterMode::Cost, &mut cost, 0.42);
        assert_eq!(cost.cost_usd, Some(0.42));

        let mut full = result_with_cost(Some(1.25));
        apply_usage_footer_cost(&UsageFooterMode::Full, &mut full, 0.0);
        assert_eq!(full.cost_usd, None);
    }
}
