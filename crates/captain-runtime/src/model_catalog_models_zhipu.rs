use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn zhipu_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "glm-4-plus".into(),
            display_name: "GLM-4 Plus".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.60,
            output_cost_per_m: 2.20,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["glm".into()],
        },
        ModelCatalogEntry {
            id: "glm-4-flash".into(),
            display_name: "GLM-4 Flash".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Fast,
            context_window: 131_072,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "glm-4v-plus".into(),
            display_name: "GLM-4V Plus".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Smart,
            context_window: 8_192,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.60,
            output_cost_per_m: 2.20,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "glm-4-long".into(),
            display_name: "GLM-4 Long".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Balanced,
            context_window: 1_000_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "glm-5-20250605".into(),
            display_name: "GLM-5".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.00,
            output_cost_per_m: 3.20,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["glm-5".into()],
        },
        ModelCatalogEntry {
            id: "glm-4.7".into(),
            display_name: "GLM-4.7".into(),
            provider: "zhipu".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.60,
            output_cost_per_m: 2.20,
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
            .unwrap_or_else(|| panic!("missing Zhipu model {id}"))
    }

    #[test]
    fn zhipu_models_count_is_stable() {
        let models = zhipu_models();

        assert_eq!(models.len(), 6);
        assert!(models.iter().all(|model| model.provider == "zhipu"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn zhipu_models_keep_primary_aliases() {
        let models = zhipu_models();

        assert_eq!(
            model(&models, "glm-4-plus").aliases,
            vec!["glm".to_string()]
        );
        assert_eq!(
            model(&models, "glm-5-20250605").aliases,
            vec!["glm-5".to_string()]
        );
    }

    #[test]
    fn zhipu_vision_models_keep_capability_contract() {
        let models = zhipu_models();
        let glm4v = model(&models, "glm-4v-plus");
        let glm5 = model(&models, "glm-5-20250605");
        let glm47 = model(&models, "glm-4.7");

        assert_eq!(glm4v.tier, ModelTier::Smart);
        assert_eq!(glm4v.context_window, 8_192);
        assert_eq!(glm4v.max_output_tokens, 4_096);
        assert_eq!(glm4v.input_cost_per_m, 0.60);
        assert_eq!(glm4v.output_cost_per_m, 2.20);
        assert!(glm4v.supports_vision);

        assert_eq!(glm5.tier, ModelTier::Frontier);
        assert_eq!(glm5.context_window, 131_072);
        assert_eq!(glm5.max_output_tokens, 16_384);
        assert_eq!(glm5.input_cost_per_m, 1.00);
        assert_eq!(glm5.output_cost_per_m, 3.20);
        assert!(glm5.supports_vision);

        assert_eq!(glm47.tier, ModelTier::Smart);
        assert_eq!(glm47.context_window, 131_072);
        assert_eq!(glm47.max_output_tokens, 16_384);
        assert_eq!(glm47.input_cost_per_m, 0.60);
        assert_eq!(glm47.output_cost_per_m, 2.20);
        assert!(glm47.supports_vision);
    }

    #[test]
    fn zhipu_text_models_keep_pricing_and_windows() {
        let models = zhipu_models();
        let plus = model(&models, "glm-4-plus");
        let flash = model(&models, "glm-4-flash");
        let long = model(&models, "glm-4-long");

        assert_eq!(plus.tier, ModelTier::Smart);
        assert_eq!(plus.context_window, 131_072);
        assert_eq!(plus.max_output_tokens, 8_192);
        assert_eq!(plus.input_cost_per_m, 0.60);
        assert_eq!(plus.output_cost_per_m, 2.20);
        assert!(!plus.supports_vision);

        assert_eq!(flash.tier, ModelTier::Fast);
        assert_eq!(flash.context_window, 131_072);
        assert_eq!(flash.max_output_tokens, 8_192);
        assert_eq!(flash.input_cost_per_m, 0.0);
        assert_eq!(flash.output_cost_per_m, 0.0);
        assert!(!flash.supports_vision);

        assert_eq!(long.tier, ModelTier::Balanced);
        assert_eq!(long.context_window, 1_000_000);
        assert_eq!(long.max_output_tokens, 8_192);
        assert_eq!(long.input_cost_per_m, 0.10);
        assert_eq!(long.output_cost_per_m, 0.10);
        assert!(!long.supports_vision);
    }
}
