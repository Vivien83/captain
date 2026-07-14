use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn moonshot_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "moonshot-v1-128k".into(),
            display_name: "Moonshot V1 128K".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.80,
            output_cost_per_m: 0.80,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "moonshot-v1-32k".into(),
            display_name: "Moonshot V1 32K".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Balanced,
            context_window: 32_768,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.30,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "moonshot-v1-8k".into(),
            display_name: "Moonshot V1 8K".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Fast,
            context_window: 8_192,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "kimi-k2".into(),
            display_name: "Kimi K2".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "kimi-k2.5".into(),
            display_name: "Kimi K2.5".into(),
            provider: "moonshot".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["kimi-k2.5-0711".into()],
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
            .unwrap_or_else(|| panic!("missing Moonshot model {id}"))
    }

    #[test]
    fn moonshot_models_count_is_stable() {
        let models = moonshot_models();

        assert_eq!(models.len(), 5);
        assert!(models.iter().all(|model| model.provider == "moonshot"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn moonshot_legacy_models_keep_windows_and_pricing() {
        let models = moonshot_models();
        let large = model(&models, "moonshot-v1-128k");
        let medium = model(&models, "moonshot-v1-32k");
        let small = model(&models, "moonshot-v1-8k");

        assert_eq!(large.tier, ModelTier::Smart);
        assert_eq!(large.context_window, 131_072);
        assert_eq!(large.max_output_tokens, 8_192);
        assert_eq!(large.input_cost_per_m, 0.80);
        assert_eq!(large.output_cost_per_m, 0.80);
        assert!(!large.supports_vision);
        assert!(large.aliases.is_empty());

        assert_eq!(medium.tier, ModelTier::Balanced);
        assert_eq!(medium.context_window, 32_768);
        assert_eq!(medium.max_output_tokens, 8_192);
        assert_eq!(medium.input_cost_per_m, 0.30);
        assert_eq!(medium.output_cost_per_m, 0.30);
        assert!(!medium.supports_vision);
        assert!(medium.aliases.is_empty());

        assert_eq!(small.tier, ModelTier::Fast);
        assert_eq!(small.context_window, 8_192);
        assert_eq!(small.max_output_tokens, 4_096);
        assert_eq!(small.input_cost_per_m, 0.10);
        assert_eq!(small.output_cost_per_m, 0.10);
        assert!(!small.supports_vision);
        assert!(small.aliases.is_empty());
    }

    #[test]
    fn moonshot_kimi_models_keep_frontier_contract() {
        let models = moonshot_models();
        let kimi_k2 = model(&models, "kimi-k2");
        let kimi_k25 = model(&models, "kimi-k2.5");

        for model in [kimi_k2, kimi_k25] {
            assert_eq!(model.tier, ModelTier::Frontier);
            assert_eq!(model.context_window, 131_072);
            assert_eq!(model.max_output_tokens, 16_384);
            assert_eq!(model.input_cost_per_m, 2.00);
            assert_eq!(model.output_cost_per_m, 8.00);
            assert!(model.supports_vision);
        }
    }

    #[test]
    fn moonshot_kimi_models_keep_alias_contract() {
        let models = moonshot_models();

        assert!(model(&models, "kimi-k2").aliases.is_empty());
        assert_eq!(
            model(&models, "kimi-k2.5").aliases,
            vec!["kimi-k2.5-0711".to_string()]
        );
    }
}
