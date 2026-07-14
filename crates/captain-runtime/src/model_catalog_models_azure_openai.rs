use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn azure_openai_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "azure/gpt-4o".into(),
            display_name: "GPT-4o (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4o-mini".into(),
            display_name: "GPT-4o Mini (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.15,
            output_cost_per_m: 0.60,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4.1".into(),
            display_name: "GPT-4.1 (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "azure/gpt-4.1-mini".into(),
            display_name: "GPT-4.1 Mini (Azure)".into(),
            provider: "azure".into(),
            tier: ModelTier::Fast,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.40,
            output_cost_per_m: 1.60,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(models: &'a [ModelCatalogEntry], id: &str) -> &'a ModelCatalogEntry {
        models
            .iter()
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("missing Azure OpenAI model {id}"))
    }

    #[test]
    fn azure_openai_models_count_is_stable() {
        let models = azure_openai_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "azure"));
    }

    #[test]
    fn azure_openai_models_have_no_aliases() {
        let models = azure_openai_models();

        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn azure_openai_pricing_and_capabilities_are_preserved() {
        let models = azure_openai_models();
        let gpt4o = model(&models, "azure/gpt-4o");
        let gpt41 = model(&models, "azure/gpt-4.1");

        assert_eq!(gpt4o.tier, ModelTier::Smart);
        assert_eq!(gpt4o.context_window, 128_000);
        assert_eq!(gpt4o.max_output_tokens, 16_384);
        assert_eq!(gpt4o.input_cost_per_m, 2.50);
        assert_eq!(gpt4o.output_cost_per_m, 10.0);

        assert_eq!(gpt41.tier, ModelTier::Frontier);
        assert_eq!(gpt41.context_window, 1_047_576);
        assert_eq!(gpt41.max_output_tokens, 32_768);
        assert_eq!(gpt41.input_cost_per_m, 2.00);
        assert_eq!(gpt41.output_cost_per_m, 8.00);

        for model in [gpt4o, gpt41] {
            assert!(model.supports_tools);
            assert!(model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn azure_openai_mini_ids_stay_available() {
        let models = azure_openai_models();
        let gpt4o_mini = model(&models, "azure/gpt-4o-mini");
        let gpt41_mini = model(&models, "azure/gpt-4.1-mini");

        assert_eq!(gpt4o_mini.tier, ModelTier::Fast);
        assert_eq!(gpt4o_mini.context_window, 128_000);
        assert_eq!(gpt4o_mini.max_output_tokens, 16_384);
        assert_eq!(gpt4o_mini.input_cost_per_m, 0.15);
        assert_eq!(gpt4o_mini.output_cost_per_m, 0.60);

        assert_eq!(gpt41_mini.tier, ModelTier::Fast);
        assert_eq!(gpt41_mini.context_window, 1_047_576);
        assert_eq!(gpt41_mini.max_output_tokens, 32_768);
        assert_eq!(gpt41_mini.input_cost_per_m, 0.40);
        assert_eq!(gpt41_mini.output_cost_per_m, 1.60);
    }
}
