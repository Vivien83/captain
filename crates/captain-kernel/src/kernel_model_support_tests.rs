use super::*;
use captain_types::agent::ToolProfile;

#[test]
fn manifest_to_capabilities_maps_explicit_tools_and_agent_spawn() {
    let mut manifest = AgentManifest::default();
    manifest.capabilities.tools = vec!["file_read".to_string(), "web_fetch".to_string()];
    manifest.capabilities.agent_spawn = true;

    let caps = manifest_to_capabilities(&manifest);

    assert!(caps.contains(&Capability::ToolInvoke("file_read".to_string())));
    assert!(caps.contains(&Capability::ToolInvoke("web_fetch".to_string())));
    assert!(caps.contains(&Capability::NetConnect("*".to_string())));
    assert!(caps.contains(&Capability::AgentSpawn));
    assert_eq!(caps.len(), 4);
}

#[test]
fn manifest_to_capabilities_expands_profile_without_explicit_tools() {
    let manifest = AgentManifest {
        profile: Some(ToolProfile::Coding),
        ..Default::default()
    };

    let caps = manifest_to_capabilities(&manifest);

    assert!(caps
        .iter()
        .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "file_read")));
    assert!(caps
        .iter()
        .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "shell_exec")));
    assert!(caps.iter().any(|c| matches!(c, Capability::ShellExec(_))));
    assert!(caps.iter().any(|c| matches!(c, Capability::NetConnect(_))));
}

#[test]
fn manifest_to_capabilities_keeps_explicit_tools_over_profile() {
    let mut manifest = AgentManifest {
        profile: Some(ToolProfile::Coding),
        ..Default::default()
    };
    manifest.capabilities.tools = vec!["file_read".to_string()];

    let caps = manifest_to_capabilities(&manifest);

    assert!(caps
        .iter()
        .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "file_read")));
    assert!(!caps
        .iter()
        .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "shell_exec")));
}

#[test]
fn manifest_to_capabilities_uses_tool_allowlist_with_implications() {
    let mut manifest = AgentManifest::default();
    manifest.tool_allowlist = vec![
        "web_fetch".to_string(),
        "memory_recall".to_string(),
        "memory_save".to_string(),
    ];

    let caps = manifest_to_capabilities(&manifest);

    assert!(caps.contains(&Capability::ToolInvoke("web_fetch".to_string())));
    assert!(caps.contains(&Capability::ToolInvoke("memory_recall".to_string())));
    assert!(caps.contains(&Capability::ToolInvoke("memory_save".to_string())));
    assert!(caps.contains(&Capability::NetConnect("*".to_string())));
    assert!(caps.contains(&Capability::MemoryRead("self.*".to_string())));
    assert!(caps.contains(&Capability::MemoryWrite("self.*".to_string())));
}

#[test]
fn manifest_to_capabilities_prefers_tool_allowlist_over_capability_tools() {
    let mut manifest = AgentManifest::default();
    manifest.capabilities.tools = vec!["shell_exec".to_string()];
    manifest.tool_allowlist = vec!["web_fetch".to_string()];

    let caps = manifest_to_capabilities(&manifest);

    assert!(caps.contains(&Capability::ToolInvoke("web_fetch".to_string())));
    assert!(!caps
        .iter()
        .any(|c| matches!(c, Capability::ToolInvoke(name) if name == "shell_exec")));
}

#[test]
fn configured_fallbacks_are_preserved_in_order() {
    let fallbacks = vec![
        FallbackProviderConfig {
            provider: "ollama".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key_env: String::new(),
            base_url: Some("http://localhost:11434".to_string()),
        },
        FallbackProviderConfig {
            provider: "groq".to_string(),
            model: "llama-3.3-70b-versatile".to_string(),
            api_key_env: "GROQ_API_KEY".to_string(),
            base_url: None,
        },
    ];

    let built = build_configured_fallbacks(&fallbacks);

    assert_eq!(built.len(), 2);
    assert_eq!(built[0].provider, "ollama");
    assert_eq!(built[0].api_key_env, None);
    assert_eq!(built[0].base_url.as_deref(), Some("http://localhost:11434"));
    assert_eq!(built[1].provider, "groq");
    assert_eq!(built[1].api_key_env.as_deref(), Some("GROQ_API_KEY"));
}

#[test]
fn empty_fallback_config_never_discovers_alternate_models() {
    assert!(build_configured_fallbacks(&[]).is_empty());
}

#[test]
fn budget_defaults_only_fill_unlimited_costs_and_override_tokens() {
    let budget = BudgetConfig {
        max_hourly_usd: 10.0,
        max_daily_usd: 25.0,
        max_monthly_usd: 100.0,
        default_max_llm_tokens_per_hour: 50_000,
        ..Default::default()
    };
    let mut resources = ResourceQuota {
        max_cost_per_hour_usd: 2.0,
        ..Default::default()
    };

    apply_budget_defaults(&budget, &mut resources);

    assert_eq!(resources.max_cost_per_hour_usd, 2.0);
    assert_eq!(resources.max_cost_per_day_usd, 25.0);
    assert_eq!(resources.max_cost_per_month_usd, 100.0);
    assert_eq!(resources.max_llm_tokens_per_hour, 50_000);
}

#[test]
fn provider_inference_preserves_known_prefixes_and_ambiguous_models() {
    assert_eq!(
        infer_provider_from_model("openrouter/anthropic/claude-sonnet-4.6"),
        Some("openrouter".to_string())
    );
    assert_eq!(
        infer_provider_from_model("kimi-k2"),
        Some("moonshot".to_string())
    );
    assert_eq!(infer_provider_from_model("qwen3-235b"), None);
}

#[test]
fn embedding_model_defaults_match_provider_family() {
    assert_eq!(
        default_embedding_model_for_provider("local"),
        "all-MiniLM-L6-v2"
    );
    assert_eq!(
        default_embedding_model_for_provider("ollama"),
        "nomic-embed-text"
    );
    assert_eq!(
        default_embedding_model_for_provider("unknown"),
        "text-embedding-3-small"
    );
}
