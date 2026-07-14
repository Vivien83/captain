use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn volcengine_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "doubao-seed-1-6-251015".into(),
            display_name: "Doubao Seed 1.6 Pro".into(),
            provider: "volcengine".into(),
            tier: ModelTier::Smart,
            context_window: 262_144,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.80,
            output_cost_per_m: 2.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["doubao".into(), "doubao-pro".into()],
        },
        ModelCatalogEntry {
            id: "doubao-seed-2-0-lite".into(),
            display_name: "Doubao Seed 2.0 Lite".into(),
            provider: "volcengine".into(),
            tier: ModelTier::Balanced,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.60,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["doubao-lite".into()],
        },
        ModelCatalogEntry {
            id: "doubao-seed-2-0-mini".into(),
            display_name: "Doubao Seed 2.0 Mini".into(),
            provider: "volcengine".into(),
            tier: ModelTier::Fast,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["doubao-mini".into()],
        },
        ModelCatalogEntry {
            id: "doubao-seed-code".into(),
            display_name: "Doubao Seed Code".into(),
            provider: "volcengine".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.50,
            output_cost_per_m: 1.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["doubao-code".into()],
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
            .unwrap_or_else(|| panic!("missing Volcengine model {id}"))
    }

    #[test]
    fn volcengine_models_count_is_stable() {
        let models = volcengine_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "volcengine"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| !model.supports_vision));
    }

    #[test]
    fn volcengine_models_keep_alias_contract() {
        let models = volcengine_models();

        assert_eq!(
            model(&models, "doubao-seed-1-6-251015").aliases,
            vec!["doubao".to_string(), "doubao-pro".to_string()]
        );
        assert_eq!(
            model(&models, "doubao-seed-2-0-lite").aliases,
            vec!["doubao-lite".to_string()]
        );
        assert_eq!(
            model(&models, "doubao-seed-2-0-mini").aliases,
            vec!["doubao-mini".to_string()]
        );
        assert_eq!(
            model(&models, "doubao-seed-code").aliases,
            vec!["doubao-code".to_string()]
        );
    }

    #[test]
    fn volcengine_seed_pro_keeps_contract() {
        let models = volcengine_models();
        let model = model(&models, "doubao-seed-1-6-251015");

        assert_eq!(model.display_name, "Doubao Seed 1.6 Pro");
        assert_eq!(model.tier, ModelTier::Smart);
        assert_eq!(model.context_window, 262_144);
        assert_eq!(model.max_output_tokens, 16_384);
        assert_eq!(model.input_cost_per_m, 0.80);
        assert_eq!(model.output_cost_per_m, 2.00);
    }

    #[test]
    fn volcengine_lite_mini_and_code_keep_contracts() {
        let models = volcengine_models();
        let lite = model(&models, "doubao-seed-2-0-lite");
        let mini = model(&models, "doubao-seed-2-0-mini");
        let code = model(&models, "doubao-seed-code");

        assert_eq!(lite.display_name, "Doubao Seed 2.0 Lite");
        assert_eq!(lite.tier, ModelTier::Balanced);
        assert_eq!(lite.context_window, 131_072);
        assert_eq!(lite.max_output_tokens, 16_384);
        assert_eq!(lite.input_cost_per_m, 0.30);
        assert_eq!(lite.output_cost_per_m, 0.60);

        assert_eq!(mini.display_name, "Doubao Seed 2.0 Mini");
        assert_eq!(mini.tier, ModelTier::Fast);
        assert_eq!(mini.context_window, 131_072);
        assert_eq!(mini.max_output_tokens, 16_384);
        assert_eq!(mini.input_cost_per_m, 0.10);
        assert_eq!(mini.output_cost_per_m, 0.10);

        assert_eq!(code.display_name, "Doubao Seed Code");
        assert_eq!(code.tier, ModelTier::Smart);
        assert_eq!(code.context_window, 131_072);
        assert_eq!(code.max_output_tokens, 16_384);
        assert_eq!(code.input_cost_per_m, 0.50);
        assert_eq!(code.output_cost_per_m, 1.00);
    }
}
