use crate::model_catalog::ModelCatalog;
use captain_types::model_catalog::{
    AuthStatus, ModelTier, CODEX_BASE_URL, LMSTUDIO_BASE_URL, OLLAMA_BASE_URL,
};
use std::collections::HashMap;

#[test]
fn test_catalog_has_models() {
    let catalog = ModelCatalog::new();
    assert!(catalog.list_models().len() >= 30);
}

#[test]
fn test_catalog_has_providers() {
    let catalog = ModelCatalog::new();
    assert_eq!(catalog.list_providers().len(), 41);
}

#[test]
fn test_find_model_by_id() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("claude-sonnet-4-20250514").unwrap();
    assert_eq!(entry.display_name, "Claude Sonnet 4");
    assert_eq!(entry.provider, "anthropic");
    assert_eq!(entry.tier, ModelTier::Smart);
}

#[test]
fn test_find_model_by_alias() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("sonnet").unwrap();
    assert_eq!(entry.id, "claude-sonnet-4-6");
}

#[test]
fn test_find_model_case_insensitive() {
    let catalog = ModelCatalog::new();
    assert!(catalog.find_model("Claude-Sonnet-4-20250514").is_some());
    assert!(catalog.find_model("SONNET").is_some());
}

#[test]
fn test_find_model_not_found() {
    let catalog = ModelCatalog::new();
    assert!(catalog.find_model("nonexistent-model").is_none());
}

#[test]
fn test_resolve_alias() {
    let catalog = ModelCatalog::new();
    assert_eq!(catalog.resolve_alias("sonnet"), Some("claude-sonnet-4-6"));
    assert_eq!(
        catalog.resolve_alias("haiku"),
        Some("claude-haiku-4-5-20251001")
    );
    assert!(catalog.resolve_alias("nonexistent").is_none());
}

#[test]
fn test_models_by_provider() {
    let catalog = ModelCatalog::new();
    let anthropic = catalog.models_by_provider("anthropic");
    assert_eq!(anthropic.len(), 7);
    assert!(anthropic.iter().all(|m| m.provider == "anthropic"));
}

#[test]
fn test_models_by_tier() {
    let catalog = ModelCatalog::new();
    let frontier = catalog.models_by_tier(ModelTier::Frontier);
    assert!(frontier.len() >= 3); // At least opus, gpt-4.1, gemini-2.5-pro
    assert!(frontier.iter().all(|m| m.tier == ModelTier::Frontier));
}

#[test]
fn test_pricing_lookup() {
    let catalog = ModelCatalog::new();
    let (input, output) = catalog.pricing("claude-sonnet-4-20250514").unwrap();
    assert!((input - 3.0).abs() < 0.001);
    assert!((output - 15.0).abs() < 0.001);
}

#[test]
fn test_pricing_via_alias() {
    let catalog = ModelCatalog::new();
    let (input, output) = catalog.pricing("sonnet").unwrap();
    assert!((input - 3.0).abs() < 0.001);
    assert!((output - 15.0).abs() < 0.001);
}

#[test]
fn test_pricing_not_found() {
    let catalog = ModelCatalog::new();
    assert!(catalog.pricing("nonexistent").is_none());
}

#[test]
fn test_detect_auth_local_providers() {
    let mut catalog = ModelCatalog::new();
    catalog.detect_auth();
    // Local providers should be NotRequired
    let ollama = catalog.get_provider("ollama").unwrap();
    assert_eq!(ollama.auth_status, AuthStatus::NotRequired);
    let vllm = catalog.get_provider("vllm").unwrap();
    assert_eq!(vllm.auth_status, AuthStatus::NotRequired);
}

#[test]
fn test_available_models_includes_local() {
    let mut catalog = ModelCatalog::new();
    catalog.detect_auth();
    let available = catalog.available_models();
    // Local providers (ollama, vllm, lmstudio) should always be available
    assert!(available.iter().any(|m| m.provider == "ollama"));
}

#[test]
fn test_provider_model_counts() {
    let catalog = ModelCatalog::new();
    let anthropic = catalog.get_provider("anthropic").unwrap();
    assert_eq!(anthropic.model_count, 7);
    let groq = catalog.get_provider("groq").unwrap();
    assert_eq!(groq.model_count, 10);
}

#[test]
fn test_list_aliases() {
    let catalog = ModelCatalog::new();
    let aliases = catalog.list_aliases();
    assert!(aliases.len() >= 20);
    assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
    // New aliases
    assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
    assert_eq!(aliases.get("jamba").unwrap(), "jamba-1.5-large");
}

#[test]
fn test_find_grok_by_alias() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("grok").unwrap();
    assert_eq!(entry.id, "grok-4-0709");
    assert_eq!(entry.provider, "xai");
}

#[test]
fn test_new_providers_in_catalog() {
    let catalog = ModelCatalog::new();
    assert!(catalog.get_provider("perplexity").is_some());
    assert!(catalog.get_provider("cohere").is_some());
    assert!(catalog.get_provider("ai21").is_some());
    assert!(catalog.get_provider("cerebras").is_some());
    assert!(catalog.get_provider("sambanova").is_some());
    assert!(catalog.get_provider("huggingface").is_some());
    assert!(catalog.get_provider("xai").is_some());
    assert!(catalog.get_provider("replicate").is_some());
}

#[test]
fn test_xai_models() {
    let catalog = ModelCatalog::new();
    let xai = catalog.models_by_provider("xai");
    assert_eq!(xai.len(), 9);
    assert!(xai.iter().any(|m| m.id == "grok-4-0709"));
    assert!(xai.iter().any(|m| m.id == "grok-4-fast-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-fast-non-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-4-1-fast-non-reasoning"));
    assert!(xai.iter().any(|m| m.id == "grok-3"));
    assert!(xai.iter().any(|m| m.id == "grok-3-mini"));
    assert!(xai.iter().any(|m| m.id == "grok-2"));
    assert!(xai.iter().any(|m| m.id == "grok-2-mini"));
}

#[test]
fn test_perplexity_models() {
    let catalog = ModelCatalog::new();
    let pp = catalog.models_by_provider("perplexity");
    assert_eq!(pp.len(), 4);
}

#[test]
fn test_cohere_models() {
    let catalog = ModelCatalog::new();
    let co = catalog.models_by_provider("cohere");
    assert_eq!(co.len(), 4);
}

#[test]
fn test_default_creates_valid_catalog() {
    let catalog = ModelCatalog::default();
    assert!(!catalog.list_models().is_empty());
    assert!(!catalog.list_providers().is_empty());
}

#[test]
fn test_merge_adds_new_models() {
    let mut catalog = ModelCatalog::new();
    let before = catalog.models_by_provider("ollama").len();
    catalog.merge_discovered_models(
        "ollama",
        &["codestral:latest".to_string(), "qwen2:7b".to_string()],
    );
    let after = catalog.models_by_provider("ollama").len();
    assert_eq!(after, before + 2);
    // Verify the new models are Local tier with zero cost
    let qwen = catalog.find_model("qwen2:7b").unwrap();
    assert_eq!(qwen.tier, ModelTier::Local);
    assert!((qwen.input_cost_per_m).abs() < f64::EPSILON);
}

#[test]
fn test_merge_skips_existing() {
    let mut catalog = ModelCatalog::new();
    // "llama3.2" is already a builtin Ollama model
    let before = catalog.list_models().len();
    catalog.merge_discovered_models("ollama", &["llama3.2".to_string()]);
    let after = catalog.list_models().len();
    assert_eq!(after, before); // no new model added
}

#[test]
fn test_merge_updates_model_count() {
    let mut catalog = ModelCatalog::new();
    let before_count = catalog.get_provider("ollama").unwrap().model_count;
    catalog.merge_discovered_models("ollama", &["new-model:latest".to_string()]);
    let after_count = catalog.get_provider("ollama").unwrap().model_count;
    assert_eq!(after_count, before_count + 1);
}

#[test]
fn test_chinese_providers_in_catalog() {
    let catalog = ModelCatalog::new();
    assert!(catalog.get_provider("qwen").is_some());
    assert!(catalog.get_provider("minimax").is_some());
    assert!(catalog.get_provider("zhipu").is_some());
    assert!(catalog.get_provider("zhipu_coding").is_some());
    assert!(catalog.get_provider("moonshot").is_some());
    assert!(catalog.get_provider("qianfan").is_some());
    assert!(catalog.get_provider("bedrock").is_some());
}

#[test]
fn test_chinese_model_aliases() {
    let catalog = ModelCatalog::new();
    assert!(catalog.find_model("kimi").is_some());
    assert!(catalog.find_model("glm").is_some());
    assert!(catalog.find_model("codegeex").is_some());
    assert!(catalog.find_model("ernie").is_some());
    assert!(catalog.find_model("minimax").is_some());
    // MiniMax M2.5 — by exact ID, alias, and case-insensitive
    let m25 = catalog.find_model("MiniMax-M2.5").unwrap();
    assert_eq!(m25.provider, "minimax");
    assert_eq!(m25.tier, ModelTier::Frontier);
    assert!(catalog.find_model("minimax-m2.5").is_some());
    // Default "minimax" alias now points to M2.5
    let default = catalog.find_model("minimax").unwrap();
    assert_eq!(default.id, "MiniMax-M2.5");
    // MiniMax M2.5 Highspeed — by exact ID and aliases
    let hs = catalog.find_model("MiniMax-M2.5-highspeed").unwrap();
    assert_eq!(hs.provider, "minimax");
    assert_eq!(hs.tier, ModelTier::Smart);
    assert!(hs.supports_vision);
    assert!(hs.supports_tools);
    assert!(catalog.find_model("minimax-m2.5-highspeed").is_some());
    assert!(catalog.find_model("minimax-highspeed").is_some());
    // abab7-chat
    let abab7 = catalog.find_model("abab7-chat").unwrap();
    assert_eq!(abab7.provider, "minimax");
    assert!(abab7.supports_vision);
}

#[test]
fn test_bedrock_models() {
    let catalog = ModelCatalog::new();
    let bedrock = catalog.models_by_provider("bedrock");
    assert_eq!(bedrock.len(), 8);
}

#[test]
fn test_set_provider_url() {
    let mut catalog = ModelCatalog::new();
    let old_url = catalog.get_provider("ollama").unwrap().base_url.clone();
    assert_eq!(old_url, OLLAMA_BASE_URL);

    let updated = catalog.set_provider_url("ollama", "http://192.168.1.100:11434/v1");
    assert!(updated);
    assert_eq!(
        catalog.get_provider("ollama").unwrap().base_url,
        "http://192.168.1.100:11434/v1"
    );
}

#[test]
fn test_set_provider_url_unknown() {
    let mut catalog = ModelCatalog::new();
    let initial_count = catalog.list_providers().len();
    let updated = catalog.set_provider_url("my-custom-llm", "http://localhost:9999");
    // Unknown providers are now auto-registered as custom entries
    assert!(updated);
    assert_eq!(catalog.list_providers().len(), initial_count + 1);
    assert_eq!(
        catalog.get_provider("my-custom-llm").unwrap().base_url,
        "http://localhost:9999"
    );
}

#[test]
fn test_apply_url_overrides() {
    let mut catalog = ModelCatalog::new();
    let mut overrides = HashMap::new();
    overrides.insert("ollama".to_string(), "http://10.0.0.5:11434/v1".to_string());
    overrides.insert("vllm".to_string(), "http://10.0.0.6:8000/v1".to_string());
    overrides.insert("nonexistent".to_string(), "http://nowhere".to_string());

    catalog.apply_url_overrides(&overrides);

    assert_eq!(
        catalog.get_provider("ollama").unwrap().base_url,
        "http://10.0.0.5:11434/v1"
    );
    assert_eq!(
        catalog.get_provider("vllm").unwrap().base_url,
        "http://10.0.0.6:8000/v1"
    );
    // lmstudio should be unchanged
    assert_eq!(
        catalog.get_provider("lmstudio").unwrap().base_url,
        LMSTUDIO_BASE_URL
    );
}

#[test]
fn test_codex_provider() {
    let catalog = ModelCatalog::new();
    let codex = catalog.get_provider("codex").unwrap();
    assert_eq!(codex.display_name, "OpenAI Codex");
    assert_eq!(codex.api_key_env, "");
    assert_eq!(codex.base_url, CODEX_BASE_URL);
    assert!(codex.key_required);
}

#[test]
fn test_codex_models() {
    let catalog = ModelCatalog::new();
    let models = catalog.models_by_provider("codex");
    assert!(!models.is_empty());
    assert!(models.iter().all(|m| m.supports_tools));
    assert!(models.iter().all(|m| m.supports_streaming));
    assert!(!models.iter().any(|m| m.id == "codex/o4-mini"));
}

#[test]
fn test_codex_aliases() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("codex").unwrap();
    assert_eq!(entry.provider, "codex");
    assert!(!entry.id.contains("o4-mini"));
}

#[test]
fn test_claude_code_provider() {
    let catalog = ModelCatalog::new();
    let cc = catalog.get_provider("claude-code").unwrap();
    assert_eq!(cc.display_name, "Claude Code");
    assert!(!cc.key_required);
}

#[test]
fn test_claude_code_models() {
    let catalog = ModelCatalog::new();
    let models = catalog.models_by_provider("claude-code");
    assert_eq!(models.len(), 3);
    assert!(models.iter().any(|m| m.id == "claude-code/opus"));
    assert!(models.iter().any(|m| m.id == "claude-code/sonnet"));
    assert!(models.iter().any(|m| m.id == "claude-code/haiku"));
}

#[test]
fn test_claude_code_aliases() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("claude-code").unwrap();
    assert_eq!(entry.id, "claude-code/sonnet");
}

#[test]
fn test_qwen_code_provider() {
    let catalog = ModelCatalog::new();
    let qc = catalog.get_provider("qwen-code").unwrap();
    assert_eq!(qc.display_name, "Qwen Code");
    assert!(!qc.key_required);
}

#[test]
fn test_qwen_code_models() {
    let catalog = ModelCatalog::new();
    let models = catalog.models_by_provider("qwen-code");
    assert_eq!(models.len(), 3);
    assert!(models.iter().any(|m| m.id == "qwen-code/qwen3-coder"));
    assert!(models.iter().any(|m| m.id == "qwen-code/qwen-coder-plus"));
    assert!(models.iter().any(|m| m.id == "qwen-code/qwq-32b"));
}

#[test]
fn test_qwen_code_aliases() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("qwen-code").unwrap();
    assert_eq!(entry.id, "qwen-code/qwen3-coder");
}

#[test]
fn test_azure_provider_in_catalog() {
    let catalog = ModelCatalog::new();
    let azure = catalog.get_provider("azure").unwrap();
    assert_eq!(azure.display_name, "Azure OpenAI");
    assert_eq!(azure.api_key_env, "AZURE_OPENAI_API_KEY");
    assert!(azure.key_required);
    assert!(azure.base_url.is_empty()); // user must supply their own
}

#[test]
fn test_azure_models() {
    let catalog = ModelCatalog::new();
    let models = catalog.models_by_provider("azure");
    assert_eq!(models.len(), 4);
    assert!(models.iter().any(|m| m.id == "azure/gpt-4o"));
    assert!(models.iter().any(|m| m.id == "azure/gpt-4o-mini"));
    assert!(models.iter().any(|m| m.id == "azure/gpt-4.1"));
    assert!(models.iter().any(|m| m.id == "azure/gpt-4.1-mini"));
}

#[test]
fn test_azure_model_lookup() {
    let catalog = ModelCatalog::new();
    let entry = catalog.find_model("azure/gpt-4o").unwrap();
    assert_eq!(entry.provider, "azure");
    assert_eq!(entry.display_name, "GPT-4o (Azure)");
    assert_eq!(entry.tier, ModelTier::Smart);
    assert!(entry.supports_tools);
    assert!(entry.supports_vision);
}

#[test]
fn reload_codex_cache_preserves_other_runtime_catalog_changes() {
    let mut catalog = ModelCatalog::new();
    assert!(catalog.update_pricing("claude-sonnet-4-6", 12.34, 56.78));

    let count = catalog.reload_codex_models_cache();

    assert!(count > 0);
    assert_eq!(catalog.pricing("claude-sonnet-4-6"), Some((12.34, 56.78)));
    assert_eq!(catalog.get_provider("codex").unwrap().model_count, count);
    assert!(catalog.find_model("codex").is_some());
}
