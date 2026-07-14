use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn venice_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "venice-uncensored".into(),
            display_name: "Venice Uncensored".into(),
            provider: "venice".into(),
            tier: ModelTier::Fast,
            context_window: 32_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.20,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["venice".into()],
        },
        ModelCatalogEntry {
            id: "llama-3.3-70b".into(),
            display_name: "Llama 3.3 70B (Venice)".into(),
            provider: "venice".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.20,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "qwen3-235b-a22b-instruct-2507".into(),
            display_name: "Qwen3 235B A22B (Venice)".into(),
            provider: "venice".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.20,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
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
            .unwrap_or_else(|| panic!("missing Venice model {id}"))
    }

    #[test]
    fn venice_models_count_and_provider_are_stable() {
        let models = venice_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "venice"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.output_cost_per_m == 0.90));
    }

    #[test]
    fn venice_models_keep_alias_contract() {
        let models = venice_models();

        assert_eq!(
            model(&models, "venice-uncensored").aliases,
            vec!["venice".to_string()]
        );
        assert!(model(&models, "llama-3.3-70b").aliases.is_empty());
        assert!(model(&models, "qwen3-235b-a22b-instruct-2507")
            .aliases
            .is_empty());
    }

    #[test]
    fn venice_uncensored_model_keeps_contract() {
        let models = venice_models();
        let uncensored = model(&models, "venice-uncensored");

        assert_eq!(uncensored.display_name, "Venice Uncensored");
        assert_eq!(uncensored.tier, ModelTier::Fast);
        assert_eq!(uncensored.context_window, 32_000);
        assert_eq!(uncensored.max_output_tokens, 8_192);
        assert_eq!(uncensored.input_cost_per_m, 0.20);
        assert_eq!(uncensored.output_cost_per_m, 0.90);
    }

    #[test]
    fn venice_llama_and_qwen_models_keep_contract() {
        let models = venice_models();
        let llama = model(&models, "llama-3.3-70b");
        let qwen = model(&models, "qwen3-235b-a22b-instruct-2507");

        assert_eq!(llama.display_name, "Llama 3.3 70B (Venice)");
        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.context_window, 128_000);
        assert_eq!(llama.max_output_tokens, 8_192);
        assert_eq!(llama.input_cost_per_m, 0.20);
        assert_eq!(llama.output_cost_per_m, 0.90);

        assert_eq!(qwen.display_name, "Qwen3 235B A22B (Venice)");
        assert_eq!(qwen.tier, ModelTier::Smart);
        assert_eq!(qwen.context_window, 128_000);
        assert_eq!(qwen.max_output_tokens, 8_192);
        assert_eq!(qwen.input_cost_per_m, 0.20);
        assert_eq!(qwen.output_cost_per_m, 0.90);
    }
}
