use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn qianfan_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "ernie-4.5-8k".into(),
            display_name: "ERNIE 4.5 8K".into(),
            provider: "qianfan".into(),
            tier: ModelTier::Smart,
            context_window: 8_192,
            max_output_tokens: 4_096,
            input_cost_per_m: 2.00,
            output_cost_per_m: 6.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["ernie".into()],
        },
        ModelCatalogEntry {
            id: "ernie-4.0-turbo-8k".into(),
            display_name: "ERNIE 4.0 Turbo 8K".into(),
            provider: "qianfan".into(),
            tier: ModelTier::Balanced,
            context_window: 8_192,
            max_output_tokens: 4_096,
            input_cost_per_m: 1.00,
            output_cost_per_m: 3.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "ernie-speed-128k".into(),
            display_name: "ERNIE Speed 128K".into(),
            provider: "qianfan".into(),
            tier: ModelTier::Fast,
            context_window: 131_072,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
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
            .unwrap_or_else(|| panic!("missing Qianfan model {id}"))
    }

    #[test]
    fn qianfan_models_count_is_stable() {
        let models = qianfan_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "qianfan"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| !model.supports_vision));
    }

    #[test]
    fn qianfan_models_keep_alias_contract() {
        let models = qianfan_models();

        assert_eq!(
            model(&models, "ernie-4.5-8k").aliases,
            vec!["ernie".to_string()]
        );
        assert!(model(&models, "ernie-4.0-turbo-8k").aliases.is_empty());
        assert!(model(&models, "ernie-speed-128k").aliases.is_empty());
    }

    #[test]
    fn qianfan_smart_and_balanced_models_keep_contract() {
        let models = qianfan_models();
        let smart = model(&models, "ernie-4.5-8k");
        let balanced = model(&models, "ernie-4.0-turbo-8k");

        assert_eq!(smart.display_name, "ERNIE 4.5 8K");
        assert_eq!(smart.tier, ModelTier::Smart);
        assert_eq!(smart.context_window, 8_192);
        assert_eq!(smart.max_output_tokens, 4_096);
        assert_eq!(smart.input_cost_per_m, 2.00);
        assert_eq!(smart.output_cost_per_m, 6.00);

        assert_eq!(balanced.display_name, "ERNIE 4.0 Turbo 8K");
        assert_eq!(balanced.tier, ModelTier::Balanced);
        assert_eq!(balanced.context_window, 8_192);
        assert_eq!(balanced.max_output_tokens, 4_096);
        assert_eq!(balanced.input_cost_per_m, 1.00);
        assert_eq!(balanced.output_cost_per_m, 3.00);
    }

    #[test]
    fn qianfan_speed_model_keeps_free_long_context_contract() {
        let models = qianfan_models();
        let model = model(&models, "ernie-speed-128k");

        assert_eq!(model.display_name, "ERNIE Speed 128K");
        assert_eq!(model.tier, ModelTier::Fast);
        assert_eq!(model.context_window, 131_072);
        assert_eq!(model.max_output_tokens, 4_096);
        assert_eq!(model.input_cost_per_m, 0.0);
        assert_eq!(model.output_cost_per_m, 0.0);
    }
}
