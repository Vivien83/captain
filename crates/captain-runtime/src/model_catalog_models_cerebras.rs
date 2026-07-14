use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn cerebras_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "cerebras/llama3.3-70b".into(),
            display_name: "Llama 3.3 70B (Cerebras)".into(),
            provider: "cerebras".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.06,
            output_cost_per_m: 0.06,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "cerebras/llama3.1-8b".into(),
            display_name: "Llama 3.1 8B (Cerebras)".into(),
            provider: "cerebras".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.01,
            output_cost_per_m: 0.01,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "cerebras/llama-4-scout-17b".into(),
            display_name: "Llama 4 Scout (Cerebras)".into(),
            provider: "cerebras".into(),
            tier: ModelTier::Smart,
            context_window: 512_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "cerebras/qwen-2.5-32b".into(),
            display_name: "Qwen 2.5 32B (Cerebras)".into(),
            provider: "cerebras".into(),
            tier: ModelTier::Balanced,
            context_window: 32_768,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.06,
            output_cost_per_m: 0.06,
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
            .unwrap_or_else(|| panic!("missing Cerebras model {id}"))
    }

    #[test]
    fn cerebras_models_count_is_stable() {
        let models = cerebras_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "cerebras"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn cerebras_models_keep_empty_aliases() {
        let models = cerebras_models();

        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn cerebras_llama_models_keep_pricing_and_windows() {
        let models = cerebras_models();
        let llama_70b = model(&models, "cerebras/llama3.3-70b");
        let llama_8b = model(&models, "cerebras/llama3.1-8b");

        assert_eq!(llama_70b.tier, ModelTier::Balanced);
        assert_eq!(llama_70b.context_window, 128_000);
        assert_eq!(llama_70b.max_output_tokens, 8_192);
        assert_eq!(llama_70b.input_cost_per_m, 0.06);
        assert_eq!(llama_70b.output_cost_per_m, 0.06);

        assert_eq!(llama_8b.tier, ModelTier::Fast);
        assert_eq!(llama_8b.context_window, 128_000);
        assert_eq!(llama_8b.max_output_tokens, 8_192);
        assert_eq!(llama_8b.input_cost_per_m, 0.01);
        assert_eq!(llama_8b.output_cost_per_m, 0.01);
    }

    #[test]
    fn cerebras_scout_and_qwen_ids_stay_available() {
        let models = cerebras_models();
        let scout = model(&models, "cerebras/llama-4-scout-17b");
        let qwen = model(&models, "cerebras/qwen-2.5-32b");

        assert_eq!(scout.display_name, "Llama 4 Scout (Cerebras)");
        assert_eq!(scout.tier, ModelTier::Smart);
        assert_eq!(scout.context_window, 512_000);
        assert_eq!(scout.input_cost_per_m, 0.10);
        assert_eq!(scout.output_cost_per_m, 0.10);

        assert_eq!(qwen.display_name, "Qwen 2.5 32B (Cerebras)");
        assert_eq!(qwen.tier, ModelTier::Balanced);
        assert_eq!(qwen.context_window, 32_768);
        assert_eq!(qwen.input_cost_per_m, 0.06);
        assert_eq!(qwen.output_cost_per_m, 0.06);
    }
}
