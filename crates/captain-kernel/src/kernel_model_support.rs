use captain_types::agent::{
    effective_manifest_capabilities, AgentManifest, FallbackModel, ResourceQuota,
};
use captain_types::capability::Capability;
use captain_types::config::{BudgetConfig, FallbackProviderConfig};

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

/// Build the failure-only fallback chain explicitly declared by the user.
/// Captain never infers alternate models from credentials present on the host.
pub(super) fn build_configured_fallbacks(
    config_fallbacks: &[FallbackProviderConfig],
) -> Vec<FallbackModel> {
    config_fallbacks
        .iter()
        .map(|fallback| FallbackModel {
            provider: fallback.provider.clone(),
            model: fallback.model.clone(),
            api_key_env: if fallback.api_key_env.is_empty() {
                None
            } else {
                Some(fallback.api_key_env.clone())
            },
            base_url: fallback.base_url.clone(),
        })
        .collect()
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
