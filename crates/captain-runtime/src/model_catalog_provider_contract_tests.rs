use crate::model_catalog::ModelCatalog;
use captain_types::model_catalog::{
    AuthStatus, ModelCatalogEntry, ModelTier, ProviderInfo, ANTHROPIC_BASE_URL, CODEX_BASE_URL,
    GEMINI_BASE_URL, LMSTUDIO_BASE_URL, OLLAMA_BASE_URL, OPENAI_BASE_URL, QWEN_BASE_URL,
    VLLM_BASE_URL,
};

fn provider<'a>(catalog: &'a ModelCatalog, id: &str) -> &'a ProviderInfo {
    catalog
        .get_provider(id)
        .unwrap_or_else(|| panic!("missing provider {id}"))
}

fn model<'a>(catalog: &'a ModelCatalog, id_or_alias: &str) -> &'a ModelCatalogEntry {
    catalog
        .find_model(id_or_alias)
        .unwrap_or_else(|| panic!("missing model or alias {id_or_alias}"))
}

fn assert_price(catalog: &ModelCatalog, id_or_alias: &str, input: f64, output: f64) {
    let (actual_input, actual_output) = catalog
        .pricing(id_or_alias)
        .unwrap_or_else(|| panic!("missing pricing for {id_or_alias}"));

    assert!((actual_input - input).abs() < f64::EPSILON);
    assert!((actual_output - output).abs() < f64::EPSILON);
}

fn assert_provider_count(catalog: &ModelCatalog, provider_id: &str, expected: usize) {
    let models = catalog.models_by_provider(provider_id);
    let provider = provider(catalog, provider_id);

    assert_eq!(
        models.len(),
        expected,
        "unexpected model count for {provider_id}"
    );
    assert_eq!(provider.model_count, expected);
    assert!(models.iter().all(|model| model.provider == provider_id));
}

#[test]
fn principal_remote_providers_keep_auth_and_base_url_contracts() {
    let catalog = ModelCatalog::new();
    let expected = [
        ("codex", "", CODEX_BASE_URL),
        ("openai", "OPENAI_API_KEY", OPENAI_BASE_URL),
        ("anthropic", "ANTHROPIC_API_KEY", ANTHROPIC_BASE_URL),
        ("gemini", "GEMINI_API_KEY", GEMINI_BASE_URL),
        ("qwen", "DASHSCOPE_API_KEY", QWEN_BASE_URL),
    ];

    for (id, api_key_env, base_url) in expected {
        let provider = provider(&catalog, id);
        assert!(provider.key_required, "{id} must keep explicit auth gating");
        assert_eq!(provider.auth_status, AuthStatus::Missing);
        assert_eq!(provider.api_key_env, api_key_env);
        assert_eq!(provider.base_url, base_url);
        assert!(provider.model_count > 0, "{id} should expose models");
    }
}

#[test]
fn principal_provider_counts_match_public_catalog() {
    let catalog = ModelCatalog::new();

    for (provider_id, expected) in [
        ("openai", 16),
        ("anthropic", 7),
        ("gemini", 10),
        ("qwen", 11),
        ("claude-code", 3),
        ("qwen-code", 3),
        ("ollama", 6),
        ("vllm", 1),
        ("lmstudio", 1),
    ] {
        assert_provider_count(&catalog, provider_id, expected);
    }

    let codex_models = catalog.models_by_provider("codex");
    let codex = provider(&catalog, "codex");
    assert_eq!(codex.model_count, codex_models.len());
    assert!(!codex_models.is_empty());
    assert!(codex_models
        .iter()
        .all(|model| model.input_cost_per_m == 0.0));
    assert!(codex_models
        .iter()
        .all(|model| model.output_cost_per_m == 0.0));
}

#[test]
fn principal_aliases_resolve_to_lookupable_priced_models() {
    let catalog = ModelCatalog::new();

    let openai = model(&catalog, "gpt5");
    assert_eq!(openai.id, "gpt-5.2");
    assert_eq!(openai.provider, "openai");
    assert_eq!(openai.tier, ModelTier::Frontier);
    assert!(openai.supports_tools);
    assert!(openai.supports_vision);
    assert!(openai.supports_streaming);
    assert_price(&catalog, "gpt5", 1.75, 14.0);

    let anthropic = model(&catalog, "sonnet");
    assert_eq!(anthropic.id, "claude-sonnet-4-6");
    assert_eq!(anthropic.provider, "anthropic");
    assert_eq!(anthropic.tier, ModelTier::Smart);
    assert!(anthropic.supports_tools);
    assert!(anthropic.supports_vision);
    assert!(anthropic.supports_streaming);
    assert_price(&catalog, "sonnet", 3.0, 15.0);

    let gemini = model(&catalog, "gemini-pro");
    assert_eq!(gemini.id, "gemini-3.1-pro-preview");
    assert_eq!(gemini.provider, "gemini");
    assert_eq!(gemini.tier, ModelTier::Frontier);
    assert!(gemini.supports_tools);
    assert!(gemini.supports_vision);
    assert!(gemini.supports_streaming);
    assert_price(&catalog, "gemini-pro", 2.50, 15.0);

    let qwen = model(&catalog, "qwen");
    assert_eq!(qwen.id, "qwen-plus");
    assert_eq!(qwen.provider, "qwen");
    assert_eq!(qwen.tier, ModelTier::Smart);
    assert!(qwen.supports_tools);
    assert!(!qwen.supports_vision);
    assert!(qwen.supports_streaming);
    assert_price(&catalog, "qwen", 0.80, 2.00);
}

#[test]
fn codex_alias_uses_codex_provider_without_legacy_openai_fallback() {
    let catalog = ModelCatalog::new();
    let codex = model(&catalog, "codex");

    assert_eq!(codex.provider, "codex");
    assert!(codex.id.starts_with("codex/"));
    assert_ne!(codex.id, "codex/o4-mini");
    assert_eq!(catalog.resolve_alias("codex"), Some(codex.id.as_str()));
    assert!(codex.supports_tools);
    assert!(codex.supports_streaming);
    assert_eq!(codex.input_cost_per_m, 0.0);
    assert_eq!(codex.output_cost_per_m, 0.0);
}

#[test]
fn cli_provider_models_resolve_without_external_api_keys() {
    let catalog = ModelCatalog::new();

    let claude_code = provider(&catalog, "claude-code");
    assert!(!claude_code.key_required);
    assert_eq!(claude_code.auth_status, AuthStatus::NotRequired);
    assert!(claude_code.api_key_env.is_empty());
    assert!(claude_code.base_url.is_empty());

    let claude_sonnet = model(&catalog, "claude-code");
    assert_eq!(claude_sonnet.id, "claude-code/sonnet");
    assert_eq!(claude_sonnet.provider, "claude-code");
    assert_eq!(claude_sonnet.tier, ModelTier::Smart);
    assert!(!claude_sonnet.supports_tools);
    assert!(claude_sonnet.supports_streaming);
    assert_price(&catalog, "claude-code", 3.0, 15.0);

    let qwen_code = provider(&catalog, "qwen-code");
    assert!(!qwen_code.key_required);
    assert_eq!(qwen_code.auth_status, AuthStatus::NotRequired);
    assert!(qwen_code.api_key_env.is_empty());
    assert!(qwen_code.base_url.is_empty());

    let qwen_coder = model(&catalog, "qwen-code");
    assert_eq!(qwen_coder.id, "qwen-code/qwen3-coder");
    assert_eq!(qwen_coder.provider, "qwen-code");
    assert_eq!(qwen_coder.tier, ModelTier::Smart);
    assert!(!qwen_coder.supports_tools);
    assert!(qwen_coder.supports_streaming);
    assert_price(&catalog, "qwen-code", 0.0, 0.0);
}

#[test]
fn local_provider_contracts_remain_available_without_auth() {
    let mut catalog = ModelCatalog::new();
    catalog.detect_auth();

    for (provider_id, base_url) in [
        ("ollama", OLLAMA_BASE_URL),
        ("vllm", VLLM_BASE_URL),
        ("lmstudio", LMSTUDIO_BASE_URL),
    ] {
        let provider = provider(&catalog, provider_id);
        assert!(!provider.key_required);
        assert_eq!(provider.auth_status, AuthStatus::NotRequired);
        assert_eq!(provider.base_url, base_url);
        assert!(provider.model_count > 0);
    }

    let available = catalog.available_models();
    for provider_id in ["ollama", "vllm", "lmstudio"] {
        assert!(
            available.iter().any(|model| model.provider == provider_id),
            "{provider_id} models should be available without API keys"
        );
    }
}
