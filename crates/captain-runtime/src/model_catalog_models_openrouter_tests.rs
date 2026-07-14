use crate::model_catalog_models_openrouter::{
    openrouter_models_after_mistral, openrouter_models_before_mistral,
};
use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

fn all_models() -> Vec<ModelCatalogEntry> {
    let mut models = openrouter_models_before_mistral();
    models.extend(openrouter_models_after_mistral());
    models
}

fn model<'a>(models: &'a [ModelCatalogEntry], id: &str) -> &'a ModelCatalogEntry {
    models
        .iter()
        .find(|model| model.id == id)
        .unwrap_or_else(|| panic!("missing OpenRouter model {id}"))
}

#[test]
fn openrouter_model_counts_and_order_slots_are_stable() {
    let before_mistral = openrouter_models_before_mistral();
    let after_mistral = openrouter_models_after_mistral();

    assert_eq!(before_mistral.len(), 24);
    assert_eq!(after_mistral.len(), 8);
    assert_eq!(
        before_mistral.first().unwrap().id,
        "openrouter/google/gemini-2.5-flash"
    );
    assert_eq!(
        before_mistral.last().unwrap().id,
        "openrouter/x-ai/grok-4-fast"
    );
    assert_eq!(after_mistral.first().unwrap().id, "xiaomi/mimo-v2-flash");
    assert_eq!(after_mistral.last().unwrap().id, "qwen/qwen3.6-plus:free");
}

#[test]
fn openrouter_models_keep_provider_and_capabilities() {
    let models = all_models();

    assert_eq!(models.len(), 32);
    assert!(models.iter().all(|model| model.provider == "openrouter"));
    assert_eq!(
        models
            .iter()
            .filter(|model| model.supports_vision)
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "openrouter/google/gemini-2.5-flash",
            "openrouter/anthropic/claude-sonnet-4",
            "openrouter/openai/gpt-4o",
            "openrouter/google/gemini-2.5-pro",
            "openrouter/anthropic/claude-haiku-4.5",
            "openrouter/anthropic/claude-sonnet-4.6",
            "openrouter/google/gemini-2.5-flash",
            "xiaomi/mimo-v2-omni",
            "moonshotai/kimi-k2.6",
            "qwen/qwen3.6-plus:free",
        ]
    );
}

#[test]
fn openrouter_duplicate_gemini_flash_contract_is_preserved() {
    let models = all_models();
    let gemini_flash = models
        .iter()
        .filter(|model| model.id == "openrouter/google/gemini-2.5-flash")
        .collect::<Vec<_>>();

    assert_eq!(gemini_flash.len(), 2);
    assert_eq!(gemini_flash[0].tier, ModelTier::Smart);
    assert_eq!(gemini_flash[0].max_output_tokens, 65_536);
    assert_eq!(gemini_flash[0].input_cost_per_m, 0.15);
    assert_eq!(gemini_flash[0].output_cost_per_m, 0.60);
    assert_eq!(gemini_flash[1].tier, ModelTier::Fast);
    assert_eq!(gemini_flash[1].max_output_tokens, 8_192);
    assert_eq!(gemini_flash[1].input_cost_per_m, 0.075);
    assert_eq!(gemini_flash[1].output_cost_per_m, 0.30);
}

#[test]
fn openrouter_free_models_keep_zero_cost_contract() {
    let models = all_models();
    let free_ids = [
        "openrouter/google/gemma-2-9b-it:free",
        "openrouter/meta-llama/llama-3.1-8b-instruct:free",
        "openrouter/qwen/qwen-2.5-7b-instruct:free",
        "openrouter/mistralai/mistral-7b-instruct:free",
        "openrouter/huggingfaceh4/zephyr-7b-beta:free",
        "openrouter/deepseek/deepseek-r1:free",
        "qwen/qwen3.6-plus:free",
    ];

    for id in free_ids {
        let entry = model(&models, id);
        assert_eq!(entry.input_cost_per_m, 0.0);
        assert_eq!(entry.output_cost_per_m, 0.0);
        assert!(entry.supports_streaming);
    }
}

#[test]
fn openrouter_extended_aliases_are_preserved() {
    let models = all_models();

    assert_eq!(
        model(&models, "xiaomi/mimo-v2-pro").aliases,
        vec!["mimo-pro".to_string(), "mimo".to_string()]
    );
    assert_eq!(
        model(&models, "xiaomi/mimo-v2.5-pro").aliases,
        vec!["mimo-2.5-pro".to_string(), "mimo2.5".to_string()]
    );
    assert_eq!(
        model(&models, "deepseek/deepseek-v4-pro").aliases,
        vec!["deepseek-v4-pro".to_string(), "deepseek-v4".to_string()]
    );
    assert_eq!(
        model(&models, "moonshotai/kimi-k2.6").aliases,
        vec!["kimi-k2.6-openrouter".to_string()]
    );
    assert_eq!(
        model(&models, "qwen/qwen3.6-plus:free").aliases,
        vec!["qwen3.6-plus".to_string()]
    );
}
