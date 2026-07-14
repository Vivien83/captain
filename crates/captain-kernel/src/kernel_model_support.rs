use captain_runtime::model_catalog::ModelCatalog;
use captain_types::agent::{
    effective_manifest_capabilities, AgentManifest, FallbackModel, ModelRoutingConfig,
    ResourceQuota,
};
use captain_types::capability::Capability;
use captain_types::config::{BudgetConfig, FallbackProviderConfig};
use captain_types::model_catalog::AuthStatus;
use std::sync::RwLock;

/// Convert a manifest's capability declarations into Capability enums.
///
/// If a profile is set and the manifest has no explicit tools, the profile's
/// implied capabilities are used as a base, preserving any non-tool overrides
/// from the manifest.
pub(super) fn manifest_to_capabilities(manifest: &AgentManifest) -> Vec<Capability> {
    let mut caps = Vec::new();

    let effective_caps = effective_manifest_capabilities(manifest);

    for host in &effective_caps.network {
        caps.push(Capability::NetConnect(host.clone()));
    }
    for tool in &effective_caps.tools {
        caps.push(Capability::ToolInvoke(tool.clone()));
    }
    for scope in &effective_caps.memory_read {
        caps.push(Capability::MemoryRead(scope.clone()));
    }
    for scope in &effective_caps.memory_write {
        caps.push(Capability::MemoryWrite(scope.clone()));
    }
    if effective_caps.agent_spawn {
        caps.push(Capability::AgentSpawn);
    }
    for pattern in &effective_caps.agent_message {
        caps.push(Capability::AgentMessage(pattern.clone()));
    }
    for cmd in &effective_caps.shell {
        caps.push(Capability::ShellExec(cmd.clone()));
    }
    if effective_caps.ofp_discover {
        caps.push(Capability::OfpDiscover);
    }
    for peer in &effective_caps.ofp_connect {
        caps.push(Capability::OfpConnect(peer.clone()));
    }

    caps
}

/// Build fallback models from other providers for service continuity.
///
/// Configured fallbacks are trusted as-is. Auto-discovered fallbacks only use
/// configured providers and skip the primary provider.
pub(super) fn build_default_fallbacks(
    primary_provider: &str,
    catalog: &RwLock<ModelCatalog>,
    config_fallbacks: &[FallbackProviderConfig],
) -> Vec<FallbackModel> {
    if !config_fallbacks.is_empty() {
        return config_fallbacks
            .iter()
            .map(|fb| FallbackModel {
                provider: fb.provider.clone(),
                model: fb.model.clone(),
                api_key_env: if fb.api_key_env.is_empty() {
                    None
                } else {
                    Some(fb.api_key_env.clone())
                },
                base_url: fb.base_url.clone(),
            })
            .collect();
    }

    let cat = catalog.read().unwrap_or_else(|e| e.into_inner());
    let providers = cat.list_providers();
    let candidates: &[(&str, &str)] = &[
        ("gemini", "gemini-2.5-flash"),
        ("groq", "llama-3.3-70b-versatile"),
        ("anthropic", "claude-haiku-4-5"),
        ("openai", "gpt-4.1-mini"),
        ("mistral", "mistral-small-latest"),
    ];

    candidates
        .iter()
        .filter(|(provider, _)| *provider != primary_provider)
        .filter(|(provider, _)| {
            providers
                .iter()
                .any(|p| p.id == *provider && matches!(p.auth_status, AuthStatus::Configured))
        })
        .take(2)
        .map(|(provider, model)| FallbackModel {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: None,
            base_url: None,
        })
        .collect()
}

/// Build a model routing config based on the default model's provider/family.
pub(super) fn build_default_routing(provider: &str, model: &str) -> Option<ModelRoutingConfig> {
    let model_lower = model.to_lowercase();

    if provider == "openrouter" && model_lower.contains("claude") {
        return Some(ModelRoutingConfig {
            simple_model: "anthropic/claude-haiku-4.5".to_string(),
            medium_model: "anthropic/claude-haiku-4.5".to_string(),
            complex_model: "anthropic/claude-sonnet-4.6".to_string(),
            simple_threshold: 50,
            complex_threshold: 200,
        });
    }

    if provider == "openrouter" && !model_lower.contains("mimo") {
        return None;
    }

    if model_lower.contains("mimo") {
        return Some(ModelRoutingConfig {
            simple_model: "xiaomi/mimo-v2-flash".to_string(),
            medium_model: "xiaomi/mimo-v2-omni".to_string(),
            complex_model: "xiaomi/mimo-v2-pro".to_string(),
            simple_threshold: 50,
            complex_threshold: 200,
        });
    }

    if provider == "anthropic" || model_lower.contains("claude") {
        return Some(ModelRoutingConfig {
            simple_model: "claude-haiku-4-5".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: model.to_string(),
            simple_threshold: 100,
            complex_threshold: 500,
        });
    }

    if provider == "codex" || provider == "openai-codex" {
        let complex_model = if model.contains('/') {
            model.to_string()
        } else {
            format!("codex/{model}")
        };
        let cached = captain_runtime::model_catalog::codex_cached_model_ids();
        let has_cached = |id: &str| cached.iter().any(|m| m == id);
        let medium_model = if has_cached("codex/gpt-5.4") {
            "codex/gpt-5.4".to_string()
        } else {
            complex_model.clone()
        };
        let simple_model = if has_cached("codex/gpt-5.4-mini") {
            "codex/gpt-5.4-mini".to_string()
        } else {
            medium_model.clone()
        };
        return Some(ModelRoutingConfig {
            simple_model,
            medium_model,
            complex_model,
            simple_threshold: 100,
            complex_threshold: 500,
        });
    }

    if provider == "openai" || model_lower.contains("gpt") {
        return Some(ModelRoutingConfig {
            simple_model: "gpt-4.1-nano".to_string(),
            medium_model: "gpt-4.1-mini".to_string(),
            complex_model: model.to_string(),
            simple_threshold: 100,
            complex_threshold: 500,
        });
    }

    if provider == "gemini" || model_lower.contains("gemini") {
        return Some(ModelRoutingConfig {
            simple_model: "gemini-2.5-flash-lite".to_string(),
            medium_model: "gemini-2.5-flash".to_string(),
            complex_model: model.to_string(),
            simple_threshold: 50,
            complex_threshold: 200,
        });
    }

    None
}

pub(super) fn model_routing_needs_repair(
    provider: &str,
    routing: &ModelRoutingConfig,
    catalog: &ModelCatalog,
) -> bool {
    let provider = provider.trim().to_ascii_lowercase();
    if provider != "codex" && provider != "openai-codex" {
        return false;
    }

    [
        &routing.simple_model,
        &routing.medium_model,
        &routing.complex_model,
    ]
    .iter()
    .any(|model| model.contains("o4-mini") || catalog.find_model(model).is_none())
}

pub(super) fn apply_budget_defaults(budget: &BudgetConfig, resources: &mut ResourceQuota) {
    if budget.max_hourly_usd > 0.0 && resources.max_cost_per_hour_usd == 0.0 {
        resources.max_cost_per_hour_usd = budget.max_hourly_usd;
    }
    if budget.max_daily_usd > 0.0 && resources.max_cost_per_day_usd == 0.0 {
        resources.max_cost_per_day_usd = budget.max_daily_usd;
    }
    if budget.max_monthly_usd > 0.0 && resources.max_cost_per_month_usd == 0.0 {
        resources.max_cost_per_month_usd = budget.max_monthly_usd;
    }
    if budget.default_max_llm_tokens_per_hour > 0 {
        resources.max_llm_tokens_per_hour = budget.default_max_llm_tokens_per_hour;
    }
}

pub(super) fn default_embedding_model_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai" => "text-embedding-3-small",
        "mistral" => "mistral-embed",
        "cohere" => "embed-english-v3.0",
        "local" => "all-MiniLM-L6-v2",
        "ollama" | "vllm" | "lmstudio" => "nomic-embed-text",
        _ => "text-embedding-3-small",
    }
}

pub(super) fn infer_provider_from_model(model: &str) -> Option<String> {
    let lower = model.to_lowercase();
    let (prefix, has_delim) = if let Some(idx) = lower.find('/') {
        (&lower[..idx], true)
    } else if let Some(idx) = lower.find(':') {
        (&lower[..idx], true)
    } else {
        (lower.as_str(), false)
    };
    if has_delim {
        if lower.chars().filter(|&c| c == '/').count() >= 2 {
            return Some(prefix.to_string());
        }
        match prefix {
            "minimax" | "gemini" | "anthropic" | "openai" | "groq" | "deepseek" | "mistral"
            | "cohere" | "xai" | "ollama" | "together" | "fireworks" | "perplexity"
            | "cerebras" | "sambanova" | "replicate" | "huggingface" | "ai21" | "codex"
            | "claude-code" | "copilot" | "github-copilot" | "qwen" | "zhipu" | "zai"
            | "moonshot" | "openrouter" | "volcengine" | "doubao" | "dashscope" => {
                return Some(prefix.to_string());
            }
            "kimi" => {
                return Some("moonshot".to_string());
            }
            _ => {}
        }
    }

    if lower.starts_with("minimax") {
        Some("minimax".to_string())
    } else if lower.starts_with("gemini") {
        Some("gemini".to_string())
    } else if lower.starts_with("claude") {
        Some("anthropic".to_string())
    } else if lower.starts_with("gpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        Some("openai".to_string())
    } else if lower.starts_with("llama")
        || lower.starts_with("mixtral")
        || lower.starts_with("qwen")
    {
        None
    } else if lower.starts_with("grok") {
        Some("xai".to_string())
    } else if lower.starts_with("deepseek") {
        Some("deepseek".to_string())
    } else if lower.starts_with("mistral")
        || lower.starts_with("codestral")
        || lower.starts_with("pixtral")
    {
        Some("mistral".to_string())
    } else if lower.starts_with("command") || lower.starts_with("embed-") {
        Some("cohere".to_string())
    } else if lower.starts_with("jamba") {
        Some("ai21".to_string())
    } else if lower.starts_with("sonar") {
        Some("perplexity".to_string())
    } else if lower.starts_with("glm") {
        Some("zhipu".to_string())
    } else if lower.starts_with("ernie") {
        Some("qianfan".to_string())
    } else if lower.starts_with("abab") {
        Some("minimax".to_string())
    } else if lower.starts_with("moonshot") || lower.starts_with("kimi") {
        Some("moonshot".to_string())
    } else {
        None
    }
}

#[cfg(test)]
#[path = "kernel_model_support_tests.rs"]
mod tests;
