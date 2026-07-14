use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn deepseek_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "deepseek-chat".into(),
            display_name: "DeepSeek V3".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.27,
            output_cost_per_m: 1.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["deepseek".into(), "deepseek-v3".into()],
        },
        ModelCatalogEntry {
            id: "deepseek-reasoner".into(),
            display_name: "DeepSeek R1".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Frontier,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.55,
            output_cost_per_m: 2.19,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["deepseek-r1".into()],
        },
        ModelCatalogEntry {
            id: "deepseek-coder".into(),
            display_name: "DeepSeek Coder V2".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.14,
            output_cost_per_m: 0.28,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "deepseek-chat-v3-0324".into(),
            display_name: "DeepSeek V3 0324".into(),
            provider: "deepseek".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.27,
            output_cost_per_m: 1.10,
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
            .unwrap_or_else(|| panic!("missing DeepSeek model {id}"))
    }

    #[test]
    fn deepseek_models_count_is_stable() {
        let models = deepseek_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "deepseek"));
    }

    #[test]
    fn deepseek_models_keep_primary_aliases() {
        let models = deepseek_models();

        assert_eq!(
            model(&models, "deepseek-chat").aliases.as_slice(),
            ["deepseek", "deepseek-v3"]
        );
        assert_eq!(
            model(&models, "deepseek-reasoner").aliases.as_slice(),
            ["deepseek-r1"]
        );
    }

    #[test]
    fn deepseek_pricing_and_capabilities_are_preserved() {
        let models = deepseek_models();
        let chat = model(&models, "deepseek-chat");
        let reasoner = model(&models, "deepseek-reasoner");
        let coder = model(&models, "deepseek-coder");

        assert_eq!(chat.tier, ModelTier::Smart);
        assert_eq!(chat.context_window, 64_000);
        assert_eq!(chat.max_output_tokens, 8_192);
        assert_eq!(chat.input_cost_per_m, 0.27);
        assert_eq!(chat.output_cost_per_m, 1.10);
        assert!(chat.supports_tools);
        assert!(!chat.supports_vision);
        assert!(chat.supports_streaming);

        assert_eq!(reasoner.tier, ModelTier::Frontier);
        assert_eq!(reasoner.context_window, 64_000);
        assert_eq!(reasoner.max_output_tokens, 8_192);
        assert_eq!(reasoner.input_cost_per_m, 0.55);
        assert_eq!(reasoner.output_cost_per_m, 2.19);
        assert!(!reasoner.supports_tools);
        assert!(!reasoner.supports_vision);
        assert!(reasoner.supports_streaming);

        assert_eq!(coder.tier, ModelTier::Balanced);
        assert_eq!(coder.context_window, 128_000);
        assert_eq!(coder.max_output_tokens, 8_192);
        assert_eq!(coder.input_cost_per_m, 0.14);
        assert_eq!(coder.output_cost_per_m, 0.28);
        assert!(coder.supports_tools);
        assert!(!coder.supports_vision);
        assert!(coder.supports_streaming);
    }

    #[test]
    fn deepseek_versioned_ids_stay_available() {
        let models = deepseek_models();

        assert!(model(&models, "deepseek-coder").aliases.is_empty());
        assert!(model(&models, "deepseek-chat-v3-0324").aliases.is_empty());
        assert_eq!(
            model(&models, "deepseek-chat-v3-0324").tier,
            ModelTier::Smart
        );
    }
}
