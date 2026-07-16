use crate::agent_loop_codex_request::{
    history_limit_for_provider, initial_visible_tools_for_provider, trim_oldest_context_messages,
};
use crate::agent_loop_control::{manifest_lean_direct_turn, max_iterations_for_manifest};
use crate::agent_loop_prompt::build_turn_system_prompt;
use crate::context_budget::ContextBudget;
use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::KernelHandle;
use crate::loop_guard::{LoopGuard, LoopGuardConfig};
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::message::{ContentBlock, Message};
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::warn;

pub(crate) const MAX_HISTORY_MESSAGES: usize = 20;
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

pub(crate) struct PreparedAgentTurn {
    pub(crate) hand_allowed_env: Vec<String>,
    pub(crate) agent_id_str: String,
    pub(crate) system_prompt: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) max_iterations: u32,
    pub(crate) loop_guard: LoopGuard,
    pub(crate) ctx_window: usize,
    pub(crate) context_budget: ContextBudget,
    pub(crate) visible_tools: Vec<ToolDefinition>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_agent_turn(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    kernel: Option<&Arc<dyn KernelHandle>>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    hooks: Option<&crate::hooks::HookRegistry>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    available_tools: &[ToolDefinition],
    context_window_tokens: Option<usize>,
    streaming: bool,
) -> PreparedAgentTurn {
    let lean_direct_turn = manifest_lean_direct_turn(manifest);
    let hand_allowed_env = manifest_hand_allowed_env(manifest);
    let memory_retractions = kernel.map(|kh| kh.memory_retractions()).unwrap_or_default();
    let agent_id_str = session.agent_id.0.to_string();

    let memories = crate::agent_loop_memory::recall_turn_memories(
        user_message,
        session.agent_id,
        memory,
        kernel,
        &memory_retractions,
        embedding_driver,
        lean_direct_turn,
        streaming,
    )
    .await;

    fire_before_prompt_build_hook(manifest, user_message, hooks, &agent_id_str);

    let system_prompt = build_turn_system_prompt(
        manifest,
        kernel,
        user_message,
        &memories,
        &memory_retractions,
        lean_direct_turn,
    )
    .await;

    let mut messages = crate::agent_loop_messages::prepare_turn_messages(
        session,
        user_message,
        user_content_blocks,
        lean_direct_turn,
        manifest
            .metadata
            .get("canonical_context_msg")
            .and_then(|v| v.as_str()),
        &memory_retractions,
    );
    trim_turn_history(manifest, &mut messages, streaming);

    let max_iterations = max_iterations_for_manifest(manifest);
    let ctx_window = context_window_tokens.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_budget = context_budget_for_provider(ctx_window, &manifest.model.provider);
    let visible_tools =
        initial_visible_tools_for_provider(available_tools, &manifest.model.provider);

    PreparedAgentTurn {
        hand_allowed_env,
        agent_id_str,
        system_prompt,
        messages,
        max_iterations,
        loop_guard: loop_guard_for_max_iterations(max_iterations),
        ctx_window,
        context_budget,
        visible_tools,
    }
}

fn manifest_hand_allowed_env(manifest: &AgentManifest) -> Vec<String> {
    manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn fire_before_prompt_build_hook(
    manifest: &AgentManifest,
    user_message: &str,
    hooks: Option<&crate::hooks::HookRegistry>,
    agent_id_str: &str,
) {
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str,
            event: captain_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "system_prompt": &manifest.model.system_prompt,
                "user_message": user_message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }
}

fn trim_turn_history(manifest: &AgentManifest, messages: &mut Vec<Message>, streaming: bool) {
    let history_limit = history_limit_for_provider(&manifest.model.provider, MAX_HISTORY_MESSAGES);
    if messages.len() <= history_limit {
        return;
    }

    let trim_count = trim_oldest_context_messages(messages, history_limit);
    let suffix = if streaming { " (streaming)" } else { "" };
    warn!(
        agent = %manifest.name,
        history_limit,
        trimming = trim_count,
        "Trimming old messages to prevent context overflow{}",
        suffix,
    );
    *messages = crate::session_repair::validate_and_repair(messages);
}

fn context_budget_for_provider(context_window_tokens: usize, provider: &str) -> ContextBudget {
    if matches!(provider, "codex" | "openai-codex") {
        ContextBudget::codex_economy(context_window_tokens)
    } else {
        ContextBudget::new(context_window_tokens)
    }
}

fn loop_guard_for_max_iterations(max_iterations: u32) -> LoopGuard {
    let mut cfg = LoopGuardConfig::default();
    if max_iterations > cfg.global_circuit_breaker {
        cfg.global_circuit_breaker = max_iterations * 3;
    }
    LoopGuard::new(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{AgentId, SessionId};

    fn test_session(messages: Vec<Message>) -> Session {
        Session {
            id: SessionId::new(),
            agent_id: AgentId::new(),
            messages,
            context_window_tokens: 0,
            label: None,
        }
    }

    fn test_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn max_history_messages_matches_legacy_guardrail() {
        assert_eq!(MAX_HISTORY_MESSAGES, 20);
    }

    #[tokio::test]
    async fn prepare_agent_turn_trims_history_and_keeps_new_user_message() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut history = Vec::new();
        for idx in 0..15 {
            history.push(Message::user(format!("old-user-{idx}")));
            history.push(Message::assistant(format!("old-assistant-{idx}")));
        }
        let mut session = test_session(history);
        let manifest = AgentManifest::default();

        let prepared = prepare_agent_turn(
            &manifest,
            "new task",
            &mut session,
            &memory,
            None,
            None,
            None,
            None,
            &[],
            Some(10_000),
            false,
        )
        .await;

        assert!(prepared.messages.len() <= MAX_HISTORY_MESSAGES);
        assert!(prepared
            .messages
            .last()
            .unwrap()
            .content
            .text_content()
            .ends_with("new task"));
        assert_eq!(prepared.max_iterations, 90);
    }

    #[tokio::test]
    async fn codex_turn_starts_with_core_tools_and_codex_budget() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let mut session = test_session(vec![Message::user("old")]);
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "codex".to_string();
        let tools = vec![test_tool("capability_search"), test_tool("custom_tool")];

        let prepared = prepare_agent_turn(
            &manifest,
            "hello",
            &mut session,
            &memory,
            None,
            None,
            None,
            None,
            &tools,
            Some(20_000),
            true,
        )
        .await;

        assert_eq!(prepared.visible_tools.len(), 1);
        assert_eq!(prepared.visible_tools[0].name, "capability_search");
        assert_eq!(prepared.context_budget.max_total_tool_chars, Some(6_000));
    }
}
