use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn sambanova_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "sambanova/llama-3.3-70b".into(),
            display_name: "Llama 3.3 70B (SambaNova)".into(),
            provider: "sambanova".into(),
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
            id: "sambanova/deepseek-r1".into(),
            display_name: "DeepSeek R1 (SambaNova)".into(),
            provider: "sambanova".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.06,
            output_cost_per_m: 0.06,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "sambanova/qwen-2.5-72b".into(),
            display_name: "Qwen 2.5 72B (SambaNova)".into(),
            provider: "sambanova".into(),
            tier: ModelTier::Smart,
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
            .unwrap_or_else(|| panic!("missing SambaNova model {id}"))
    }

    #[test]
    fn sambanova_models_count_is_stable() {
        let models = sambanova_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "sambanova"));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn sambanova_models_keep_tool_support_contract() {
        let models = sambanova_models();

        assert!(model(&models, "sambanova/llama-3.3-70b").supports_tools);
        assert!(!model(&models, "sambanova/deepseek-r1").supports_tools);
        assert!(model(&models, "sambanova/qwen-2.5-72b").supports_tools);
    }

    #[test]
    fn sambanova_llama_and_deepseek_keep_pricing_and_windows() {
        let models = sambanova_models();
        let llama = model(&models, "sambanova/llama-3.3-70b");
        let deepseek = model(&models, "sambanova/deepseek-r1");

        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.context_window, 128_000);
        assert_eq!(llama.max_output_tokens, 8_192);
        assert_eq!(llama.input_cost_per_m, 0.06);
        assert_eq!(llama.output_cost_per_m, 0.06);

        assert_eq!(deepseek.tier, ModelTier::Smart);
        assert_eq!(deepseek.context_window, 64_000);
        assert_eq!(deepseek.max_output_tokens, 8_192);
        assert_eq!(deepseek.input_cost_per_m, 0.06);
        assert_eq!(deepseek.output_cost_per_m, 0.06);
    }

    #[test]
    fn sambanova_qwen_id_stays_available() {
        let models = sambanova_models();
        let qwen = model(&models, "sambanova/qwen-2.5-72b");

        assert_eq!(qwen.display_name, "Qwen 2.5 72B (SambaNova)");
        assert_eq!(qwen.tier, ModelTier::Smart);
        assert_eq!(qwen.context_window, 32_768);
        assert_eq!(qwen.max_output_tokens, 8_192);
        assert_eq!(qwen.input_cost_per_m, 0.06);
        assert_eq!(qwen.output_cost_per_m, 0.06);
    }
}
