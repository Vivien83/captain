use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn fireworks_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "accounts/fireworks/models/llama-v3p1-405b-instruct".into(),
            display_name: "Llama 3.1 405B (Fireworks)".into(),
            provider: "fireworks".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 3.00,
            output_cost_per_m: 3.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
            display_name: "Llama 3.3 70B (Fireworks)".into(),
            provider: "fireworks".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.90,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "accounts/fireworks/models/deepseek-r1".into(),
            display_name: "DeepSeek R1 (Fireworks)".into(),
            provider: "fireworks".into(),
            tier: ModelTier::Frontier,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 3.00,
            output_cost_per_m: 8.00,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "accounts/fireworks/models/deepseek-v3".into(),
            display_name: "DeepSeek V3 (Fireworks)".into(),
            provider: "fireworks".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.90,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "accounts/fireworks/models/mixtral-8x22b-instruct".into(),
            display_name: "Mixtral 8x22B (Fireworks)".into(),
            provider: "fireworks".into(),
            tier: ModelTier::Balanced,
            context_window: 65_536,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.90,
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
            .unwrap_or_else(|| panic!("missing Fireworks model {id}"))
    }

    #[test]
    fn fireworks_models_count_is_stable() {
        let models = fireworks_models();

        assert_eq!(models.len(), 5);
        assert!(models.iter().all(|model| model.provider == "fireworks"));
        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn fireworks_llama_pricing_and_capabilities_are_preserved() {
        let models = fireworks_models();
        let llama405 = model(
            &models,
            "accounts/fireworks/models/llama-v3p1-405b-instruct",
        );
        let llama70 = model(&models, "accounts/fireworks/models/llama-v3p3-70b-instruct");

        assert_eq!(llama405.tier, ModelTier::Frontier);
        assert_eq!(llama405.context_window, 131_072);
        assert_eq!(llama405.max_output_tokens, 16_384);
        assert_eq!(llama405.input_cost_per_m, 3.00);
        assert_eq!(llama405.output_cost_per_m, 3.00);

        assert_eq!(llama70.tier, ModelTier::Smart);
        assert_eq!(llama70.context_window, 131_072);
        assert_eq!(llama70.max_output_tokens, 16_384);
        assert_eq!(llama70.input_cost_per_m, 0.90);
        assert_eq!(llama70.output_cost_per_m, 0.90);

        for model in [llama405, llama70] {
            assert!(model.supports_tools);
            assert!(!model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn fireworks_deepseek_ids_stay_available() {
        let models = fireworks_models();
        let r1 = model(&models, "accounts/fireworks/models/deepseek-r1");
        let v3 = model(&models, "accounts/fireworks/models/deepseek-v3");

        assert_eq!(r1.tier, ModelTier::Frontier);
        assert_eq!(r1.context_window, 64_000);
        assert_eq!(r1.input_cost_per_m, 3.00);
        assert_eq!(r1.output_cost_per_m, 8.00);
        assert!(!r1.supports_tools);

        assert_eq!(v3.tier, ModelTier::Smart);
        assert_eq!(v3.context_window, 64_000);
        assert_eq!(v3.input_cost_per_m, 0.90);
        assert_eq!(v3.output_cost_per_m, 0.90);
        assert!(v3.supports_tools);
    }

    #[test]
    fn fireworks_mixtral_id_stays_available() {
        let models = fireworks_models();
        let mixtral = model(&models, "accounts/fireworks/models/mixtral-8x22b-instruct");

        assert_eq!(mixtral.tier, ModelTier::Balanced);
        assert_eq!(mixtral.context_window, 65_536);
        assert_eq!(mixtral.max_output_tokens, 4_096);
        assert_eq!(mixtral.input_cost_per_m, 0.90);
        assert_eq!(mixtral.output_cost_per_m, 0.90);
        assert!(mixtral.supports_tools);
        assert!(!mixtral.supports_vision);
        assert!(mixtral.supports_streaming);
    }
}
