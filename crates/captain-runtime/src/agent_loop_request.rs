use crate::agent_loop_codex_request::{
    apply_provider_request_context_economy, request_tools_for_provider, CODEX_REQUEST_TARGET_TOKENS,
};
use crate::agent_loop_control::cache_hints_for_session;
use crate::agent_loop_policy::manifest_subagent_depth;
use crate::context_budget::{apply_context_guard_preserving_recent, ContextBudget};
use crate::context_overflow::{recover_from_overflow, RecoveryStage};
use crate::llm_driver::CompletionRequest;
use captain_memory::session::Session;
use captain_types::agent::AgentManifest;
use captain_types::message::Message;
use captain_types::reasoning::ReasoningEffort;
use captain_types::tool::ToolDefinition;
use tracing::info;

const CODEX_ULTRA_ROOT_INSTRUCTIONS: &str = "\
## Captain Ultra orchestration

This is a root-agent Ultra turn. Proactively delegate only when doing so \
materially improves execution or verification:
- delegate independent, long-running, or genuinely parallelizable subtasks;
- preserve dependencies and execute sequential work in order;
- give every delegation a bounded objective, budget, and success criteria;
- monitor detached work and verify its result before synthesis;
- handle simple or tightly coupled work directly.

Never delegate merely because a task uses several tools.";

pub(crate) struct PreparedRequestContext {
    pub(crate) request_tools: Vec<ToolDefinition>,
    pub(crate) recovery: RecoveryStage,
}

/// Strip a provider prefix from a model ID before sending to the API.
///
/// Many models are stored as `provider/org/model` (e.g. `openrouter/google/gemini-2.5-flash`)
/// but the upstream API expects just `org/model` (e.g. `google/gemini-2.5-flash`).
pub fn strip_provider_prefix(model: &str, provider: &str) -> String {
    let slash_prefix = format!("{provider}/");
    let colon_prefix = format!("{provider}:");
    if model.starts_with(&slash_prefix) {
        model[slash_prefix.len()..].to_string()
    } else if model.starts_with(&colon_prefix) {
        model[colon_prefix.len()..].to_string()
    } else {
        model.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_request_context(
    agent_name: &str,
    messages: &mut Vec<Message>,
    system_prompt: &str,
    visible_tools: &[ToolDefinition],
    provider_name: &str,
    context_budget: &ContextBudget,
    ctx_window: usize,
    iteration: u32,
    repair_after_recovery: bool,
    streaming: bool,
) -> PreparedRequestContext {
    let request_tools = request_tools_for_provider(visible_tools, provider_name);

    let recovery = recover_from_overflow(messages, system_prompt, &request_tools, ctx_window);
    if repair_after_recovery && recovery != RecoveryStage::None {
        *messages = crate::session_repair::validate_and_repair(messages);
    }

    let preserve_recent_tool_results = usize::from(iteration > 0);
    apply_context_guard_preserving_recent(
        messages,
        context_budget,
        &request_tools,
        preserve_recent_tool_results,
    );

    let trimmed_for_economy = apply_provider_request_context_economy(
        messages,
        system_prompt,
        &request_tools,
        provider_name,
    );
    if trimmed_for_economy > 0 {
        if streaming {
            info!(
                agent = %agent_name,
                provider = provider_name,
                trimmed = trimmed_for_economy,
                target_tokens = CODEX_REQUEST_TARGET_TOKENS,
                "Applied provider request context economy (streaming)"
            );
        } else {
            info!(
                agent = %agent_name,
                provider = provider_name,
                trimmed = trimmed_for_economy,
                target_tokens = CODEX_REQUEST_TARGET_TOKENS,
                "Applied provider request context economy"
            );
        }
        *messages = crate::session_repair::validate_and_repair(messages);
        apply_context_guard_preserving_recent(
            messages,
            context_budget,
            &request_tools,
            preserve_recent_tool_results,
        );
    }

    PreparedRequestContext {
        request_tools,
        recovery,
    }
}

pub(crate) fn build_completion_request(
    manifest: &AgentManifest,
    session: &Session,
    api_model: String,
    messages: &[Message],
    request_tools: Vec<ToolDefinition>,
    system_prompt: &str,
) -> CompletionRequest {
    let reasoning_effort = validated_reasoning_effort(manifest);
    let system_prompt = system_prompt_for_request(
        manifest,
        system_prompt,
        reasoning_effort.as_ref(),
        &request_tools,
    );
    CompletionRequest {
        model: api_model,
        messages: messages.to_vec(),
        tools: request_tools,
        max_tokens: manifest.model.max_tokens,
        temperature: manifest.model.temperature,
        system: Some(system_prompt),
        thinking: None,
        reasoning_effort,
        tool_choice: None,
        cache_hints: cache_hints_for_session(manifest, session),
    }
}

fn system_prompt_for_request(
    manifest: &AgentManifest,
    system_prompt: &str,
    reasoning_effort: Option<&ReasoningEffort>,
    request_tools: &[ToolDefinition],
) -> String {
    let ultra_root = matches!(
        manifest.model.provider.to_ascii_lowercase().as_str(),
        "codex" | "openai-codex"
    ) && reasoning_effort
        .is_some_and(|effort| effort.as_str() == ReasoningEffort::ULTRA)
        && manifest_subagent_depth(manifest) == 0
        && request_tools.iter().any(|tool| {
            matches!(
                tool.name.as_str(),
                "agent_delegate" | "agent_spawn" | "tool_search" | "capability_search"
            )
        });

    if ultra_root {
        format!("{system_prompt}\n\n{CODEX_ULTRA_ROOT_INSTRUCTIONS}")
    } else {
        system_prompt.to_string()
    }
}

fn validated_reasoning_effort(manifest: &AgentManifest) -> Option<ReasoningEffort> {
    let configured = manifest.model.reasoning_effort.clone()?;
    if !matches!(
        manifest.model.provider.to_ascii_lowercase().as_str(),
        "codex" | "openai-codex"
    ) {
        return Some(configured);
    }
    let Some(capabilities) =
        crate::model_catalog_codex::codex_reasoning_capabilities(&manifest.model.model)
    else {
        tracing::warn!(
            model = %manifest.model.model,
            effort = %configured,
            "Ignoring reasoning override because Codex does not advertise reasoning for this model"
        );
        return None;
    };
    if capabilities.supports(&configured) {
        Some(configured)
    } else {
        tracing::warn!(
            model = %manifest.model.model,
            effort = %configured,
            "Ignoring reasoning override no longer supported by the Codex model catalog"
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest(provider: &str) -> AgentManifest {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = provider.to_string();
        manifest.model.max_tokens = 123;
        manifest.model.temperature = 0.2;
        manifest
    }

    fn test_session() -> Session {
        Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        }
    }

    fn test_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: "tool".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn strip_provider_prefix_removes_slash_prefix() {
        assert_eq!(
            strip_provider_prefix("openrouter/google/gemini-2.5-flash", "openrouter"),
            "google/gemini-2.5-flash"
        );
    }

    #[test]
    fn strip_provider_prefix_removes_colon_prefix() {
        assert_eq!(
            strip_provider_prefix("codex:gpt-5-codex", "codex"),
            "gpt-5-codex"
        );
    }

    #[test]
    fn strip_provider_prefix_keeps_plain_model() {
        assert_eq!(
            strip_provider_prefix("anthropic/claude-3-5-sonnet", "openrouter"),
            "anthropic/claude-3-5-sonnet"
        );
    }

    #[test]
    fn prepare_request_context_returns_provider_tools_without_recovery() {
        let mut messages = vec![Message::user("hello")];
        let tools = vec![test_tool("shell_exec")];
        let context_budget = ContextBudget::new(200_000);

        let prepared = prepare_request_context(
            "captain",
            &mut messages,
            "system",
            &tools,
            "anthropic",
            &context_budget,
            200_000,
            0,
            true,
            false,
        );

        assert_eq!(prepared.recovery, RecoveryStage::None);
        assert_eq!(prepared.request_tools.len(), 1);
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn prepare_request_context_can_run_overflow_recovery() {
        let mut messages = (0..30)
            .map(|i| Message::user(format!("message {i} {}", "x".repeat(500))))
            .collect::<Vec<_>>();
        let context_budget = ContextBudget::new(4_000);

        let prepared = prepare_request_context(
            "captain",
            &mut messages,
            "system",
            &[],
            "anthropic",
            &context_budget,
            1_000,
            0,
            true,
            false,
        );

        assert_ne!(prepared.recovery, RecoveryStage::None);
        assert!(messages.len() <= 10);
    }

    #[test]
    fn build_completion_request_preserves_runtime_contract() {
        let manifest = test_manifest("anthropic");
        let session = test_session();
        let messages = vec![Message::user("hello")];
        let tools = vec![test_tool("file_read")];

        let request = build_completion_request(
            &manifest,
            &session,
            "claude-3-5".to_string(),
            &messages,
            tools,
            "system prompt",
        );

        assert_eq!(request.model, "claude-3-5");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.max_tokens, 123);
        assert_eq!(request.temperature, 0.2);
        assert_eq!(request.system.as_deref(), Some("system prompt"));
        assert!(request.cache_hints.prompt_cache_key.is_some());
    }

    #[test]
    fn ultra_adds_proactive_orchestration_only_to_a_codex_root() {
        let ultra = ReasoningEffort::ULTRA.parse::<ReasoningEffort>().unwrap();
        let mut root = test_manifest("codex");
        let coordination_tools = vec![test_tool("agent_delegate")];
        let root_prompt =
            system_prompt_for_request(&root, "base", Some(&ultra), &coordination_tools);
        assert!(root_prompt.contains("Captain Ultra orchestration"));
        assert!(root_prompt.contains("genuinely parallelizable"));

        root.metadata
            .insert("subagent_depth".to_string(), serde_json::json!(1));
        assert_eq!(
            system_prompt_for_request(&root, "base", Some(&ultra), &coordination_tools),
            "base"
        );
    }

    #[test]
    fn auto_explicit_none_and_other_providers_do_not_add_ultra_orchestration() {
        let codex = test_manifest("codex");
        let coordination_tools = vec![test_tool("tool_search")];
        assert_eq!(
            system_prompt_for_request(&codex, "base", None, &coordination_tools),
            "base"
        );
        let none = "none".parse::<ReasoningEffort>().unwrap();
        assert_eq!(
            system_prompt_for_request(&codex, "base", Some(&none), &coordination_tools),
            "base"
        );

        let ultra = ReasoningEffort::ULTRA.parse::<ReasoningEffort>().unwrap();
        let anthropic = test_manifest("anthropic");
        assert_eq!(
            system_prompt_for_request(&anthropic, "base", Some(&ultra), &coordination_tools),
            "base"
        );
        assert_eq!(
            system_prompt_for_request(&codex, "base", Some(&ultra), &[]),
            "base"
        );
    }
}
