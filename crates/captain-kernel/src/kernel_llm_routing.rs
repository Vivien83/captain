use captain_runtime::agent_loop::strip_provider_prefix;
use captain_runtime::llm_driver::CompletionRequest;
use captain_runtime::routing::ModelRouter;
use captain_types::agent::{AgentManifest, OrchestrationMode};
use captain_types::config::KernelMode;
use captain_types::tool::ToolDefinition;
use tracing::info;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn route_non_streaming_llm_model(
        &self,
        manifest: &mut AgentManifest,
        first_session_turn: bool,
        message: &str,
        tools: &[ToolDefinition],
    ) {
        match llm_model_routing_gate(
            self.config.mode,
            manifest.orchestration_mode,
            first_session_turn,
        ) {
            LlmModelRoutingGate::Stable => {
                if let Some(ref pinned) = manifest.pinned_model {
                    info!(
                        agent = %manifest.name,
                        pinned_model = %pinned,
                        "Stable mode: using pinned model"
                    );
                    manifest.model.model = pinned.clone();
                }
            }
            LlmModelRoutingGate::Orchestration => {
                info!(
                    agent = %manifest.name,
                    mode = ?manifest.orchestration_mode,
                    "Orchestration mode skips auto-routing"
                );
            }
            LlmModelRoutingGate::FirstSessionTurn => {
                info!(
                    agent = %manifest.name,
                    model = %manifest.model.model,
                    "First session turn skips auto-routing"
                );
            }
            LlmModelRoutingGate::Route => {
                self.apply_configured_llm_model_route(manifest, message, tools);
            }
        }
    }

    fn apply_configured_llm_model_route(
        &self,
        manifest: &mut AgentManifest,
        message: &str,
        tools: &[ToolDefinition],
    ) {
        let Some(ref routing_config) = manifest.routing else {
            return;
        };

        let mut router = ModelRouter::new(routing_config.clone());
        router.resolve_aliases(&self.model_catalog.read().unwrap_or_else(|e| e.into_inner()));
        let probe = CompletionRequest {
            model: strip_provider_prefix(&manifest.model.model, &manifest.model.provider),
            messages: vec![captain_types::message::Message::user(message)],
            tools: tools.to_vec(),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            system: Some(manifest.model.system_prompt.clone()),
            thinking: None,
            tool_choice: None,
            cache_hints: captain_runtime::llm_driver::CacheHints::default(),
        };
        let (complexity, routed_model) = router.select_model(&probe);
        info!(
            agent = %manifest.name,
            complexity = %complexity,
            routed_model = %routed_model,
            "Model routing applied"
        );
        manifest.model.model = routed_model.clone();
        if let Ok(cat) = self.model_catalog.read() {
            if let Some(entry) = cat.find_model(&routed_model) {
                if entry.provider != manifest.model.provider {
                    info!(old = %manifest.model.provider, new = %entry.provider, "Model routing changed provider");
                    manifest.model.provider = entry.provider.clone();
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LlmModelRoutingGate {
    Stable,
    Orchestration,
    FirstSessionTurn,
    Route,
}

fn llm_model_routing_gate(
    mode: KernelMode,
    orchestration: OrchestrationMode,
    first_session_turn: bool,
) -> LlmModelRoutingGate {
    if mode == KernelMode::Stable {
        LlmModelRoutingGate::Stable
    } else if orchestration == OrchestrationMode::Pinned
        || orchestration == OrchestrationMode::Delegation
    {
        LlmModelRoutingGate::Orchestration
    } else if first_session_turn {
        LlmModelRoutingGate::FirstSessionTurn
    } else {
        LlmModelRoutingGate::Route
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_gate_keeps_stable_mode_authoritative() {
        assert_eq!(
            llm_model_routing_gate(KernelMode::Stable, OrchestrationMode::Routing, true),
            LlmModelRoutingGate::Stable
        );
    }

    #[test]
    fn routing_gate_skips_pinned_and_delegation_before_first_turn_rule() {
        assert_eq!(
            llm_model_routing_gate(KernelMode::Default, OrchestrationMode::Pinned, true),
            LlmModelRoutingGate::Orchestration
        );
        assert_eq!(
            llm_model_routing_gate(KernelMode::Default, OrchestrationMode::Delegation, false),
            LlmModelRoutingGate::Orchestration
        );
    }

    #[test]
    fn routing_gate_routes_after_first_turn_in_routing_mode() {
        assert_eq!(
            llm_model_routing_gate(KernelMode::Default, OrchestrationMode::Routing, true),
            LlmModelRoutingGate::FirstSessionTurn
        );
        assert_eq!(
            llm_model_routing_gate(KernelMode::Default, OrchestrationMode::Routing, false),
            LlmModelRoutingGate::Route
        );
    }
}
