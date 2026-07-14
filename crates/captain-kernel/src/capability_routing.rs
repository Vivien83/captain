//! Capability-aware auto-routing — delegates messages to specialized agents
//! when the target agent's model lacks required capabilities (e.g., vision).
//!
//! Pre-flight check: before sending to LLM, verify the model can handle the input.
//! If not, find or spawn a capable agent and delegate transparently.

use captain_types::agent::AgentId;
use captain_types::message::ContentBlock;
use tracing::{info, warn};

/// Capability required to process the given content blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCapability {
    Vision,
}

/// Check what capabilities are needed for the given content blocks.
pub fn detect_required_capabilities(
    content_blocks: Option<&[ContentBlock]>,
) -> Vec<RequiredCapability> {
    let mut caps = Vec::new();
    if let Some(blocks) = content_blocks {
        let has_images = blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::Image { .. }));
        if has_images {
            caps.push(RequiredCapability::Vision);
        }
    }
    caps
}

/// Check if a model supports the given capability, using the model catalog.
pub fn model_supports(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    model_id: &str,
    cap: RequiredCapability,
) -> bool {
    match cap {
        RequiredCapability::Vision => catalog
            .find_model(model_id)
            .map(|m| m.supports_vision)
            .unwrap_or(false),
    }
}

/// Find an existing agent whose model supports the required capability.
pub fn find_capable_agent(
    registry: &crate::registry::AgentRegistry,
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    cap: RequiredCapability,
    exclude_agent_id: AgentId,
) -> Option<AgentId> {
    registry
        .list()
        .into_iter()
        .filter(|e| e.id != exclude_agent_id)
        .filter(|e| matches!(e.state, captain_types::agent::AgentState::Running))
        .find(|e| model_supports(catalog, &e.manifest.model.model, cap))
        .map(|e| e.id)
}

/// Pick the best available vision model from the catalog based on configured API keys.
pub fn best_vision_model(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
) -> Option<(String, String)> {
    // Priority: gemini-2.5-flash (cheap + fast) > claude-sonnet > gpt-4o
    let candidates = [
        ("mistral", "pixtral-large-latest"),
        ("gemini", "gemini-2.5-flash"),
        ("anthropic", "claude-sonnet-4-20250514"),
        ("openai", "gpt-4o"),
        ("groq", "llama-4-scout-17b-16e-instruct"),
    ];

    for (provider, model) in &candidates {
        if let Some(entry) = catalog.find_model(model) {
            if entry.supports_vision {
                // Check if provider has auth configured
                let provider_info = catalog.list_providers().iter().find(|p| p.id == *provider);
                if let Some(p) = provider_info {
                    if matches!(
                        p.auth_status,
                        captain_types::model_catalog::AuthStatus::Configured
                    ) {
                        return Some((provider.to_string(), model.to_string()));
                    }
                }
            }
        }
    }

    // Fallback: any vision model with configured auth
    for entry in catalog.list_models() {
        if entry.supports_vision {
            let provider_info = catalog
                .list_providers()
                .iter()
                .find(|p| p.id == entry.provider);
            if let Some(p) = provider_info {
                if matches!(
                    p.auth_status,
                    captain_types::model_catalog::AuthStatus::Configured
                ) {
                    return Some((entry.provider.clone(), entry.id.clone()));
                }
            }
        }
    }

    None
}

/// Build a TOML manifest for a vision-capable agent.
pub fn build_vision_agent_manifest(provider: &str, model: &str) -> String {
    format!(
        r#"name = "vision"
version = "1.0.0"
description = "Vision-capable agent for image analysis (auto-spawned)"
module = "builtin:chat"
tags = ["vision", "auto-spawned"]

[model]
provider = "{provider}"
model = "{model}"
system_prompt = "You are a vision analysis assistant. Images are sent to you inline as part of the conversation — you can see them directly. Describe what you see accurately and concisely. Always respond in the same language as the user's message."

[capabilities]
tools = ["memory_store", "memory_recall"]
agent_spawn = false
"#
    )
}

/// Result of a capability routing decision.
#[derive(Debug)]
pub enum RoutingDecision {
    /// No routing needed — the target agent can handle the input.
    Proceed,
    /// Delegate to an existing capable agent.
    DelegateTo(AgentId),
    /// Need to spawn a new capable agent first (returns manifest TOML).
    SpawnAndDelegate {
        manifest_toml: String,
        capability: RequiredCapability,
    },
    /// No capable model available at all.
    NoCandidateAvailable(RequiredCapability),
}

/// Make a routing decision for the given message and content blocks.
pub fn decide_routing(
    registry: &crate::registry::AgentRegistry,
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    agent_id: AgentId,
    agent_model: &str,
    content_blocks: Option<&[ContentBlock]>,
) -> RoutingDecision {
    let caps = detect_required_capabilities(content_blocks);

    for cap in caps {
        if !model_supports(catalog, agent_model, cap) {
            info!(
                agent_id = %agent_id,
                model = agent_model,
                capability = ?cap,
                "Model lacks required capability, searching for capable agent"
            );

            // Try to find an existing capable agent
            if let Some(target_id) = find_capable_agent(registry, catalog, cap, agent_id) {
                info!(
                    target_agent = %target_id,
                    capability = ?cap,
                    "Found existing capable agent, delegating"
                );
                return RoutingDecision::DelegateTo(target_id);
            }

            // No existing agent — try to spawn one
            if let Some((provider, model)) = best_vision_model(catalog) {
                info!(
                    provider = provider,
                    model = model,
                    capability = ?cap,
                    "No capable agent found, will auto-spawn"
                );
                return RoutingDecision::SpawnAndDelegate {
                    manifest_toml: build_vision_agent_manifest(&provider, &model),
                    capability: cap,
                };
            }

            warn!(capability = ?cap, "No capable model available in catalog");
            return RoutingDecision::NoCandidateAvailable(cap);
        }
    }

    RoutingDecision::Proceed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_no_images_no_caps() {
        assert!(detect_required_capabilities(None).is_empty());
        assert!(detect_required_capabilities(Some(&[])).is_empty());
    }

    #[test]
    fn test_detect_text_only_no_caps() {
        let blocks = vec![ContentBlock::Text {
            text: "hello".to_string(),
            provider_metadata: None,
        }];
        assert!(detect_required_capabilities(Some(&blocks)).is_empty());
    }

    #[test]
    fn test_detect_image_requires_vision() {
        let blocks = vec![ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        }];
        let caps = detect_required_capabilities(Some(&blocks));
        assert_eq!(caps, vec![RequiredCapability::Vision]);
    }

    #[test]
    fn test_detect_mixed_content_requires_vision() {
        let blocks = vec![
            ContentBlock::Text {
                text: "What is this?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "base64data".to_string(),
            },
        ];
        let caps = detect_required_capabilities(Some(&blocks));
        assert_eq!(caps, vec![RequiredCapability::Vision]);
    }

    #[test]
    fn test_build_vision_manifest_valid_toml() {
        let manifest = build_vision_agent_manifest("gemini", "gemini-2.5-flash");
        let parsed: toml::Value = toml::from_str(&manifest).expect("Invalid TOML");
        assert_eq!(parsed["name"].as_str(), Some("vision"));
        assert_eq!(parsed["model"]["provider"].as_str(), Some("gemini"));
        assert_eq!(parsed["model"]["model"].as_str(), Some("gemini-2.5-flash"));
        assert!(manifest.contains("vision"));
    }
}
