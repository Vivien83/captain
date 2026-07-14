use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn perplexity_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "sonar-pro".into(),
            display_name: "Sonar Pro".into(),
            provider: "perplexity".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["sonar".into()],
        },
        ModelCatalogEntry {
            id: "sonar-reasoning-pro".into(),
            display_name: "Sonar Reasoning Pro".into(),
            provider: "perplexity".into(),
            tier: ModelTier::Frontier,
            context_window: 200_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 2.0,
            output_cost_per_m: 8.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "sonar-reasoning".into(),
            display_name: "Sonar Reasoning".into(),
            provider: "perplexity".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 1.0,
            output_cost_per_m: 5.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "sonar-basic".into(),
            display_name: "Sonar".into(),
            provider: "perplexity".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 1.0,
            output_cost_per_m: 5.0,
            supports_tools: false,
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
            .unwrap_or_else(|| panic!("missing Perplexity model {id}"))
    }

    #[test]
    fn perplexity_models_count_is_stable() {
        let models = perplexity_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "perplexity"));
        assert!(models.iter().all(|model| !model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn perplexity_models_keep_primary_alias() {
        let models = perplexity_models();

        assert_eq!(model(&models, "sonar-pro").aliases.as_slice(), ["sonar"]);
        assert!(model(&models, "sonar-reasoning-pro").aliases.is_empty());
        assert!(model(&models, "sonar-reasoning").aliases.is_empty());
        assert!(model(&models, "sonar-basic").aliases.is_empty());
    }

    #[test]
    fn perplexity_pro_models_keep_pricing_and_windows() {
        let models = perplexity_models();
        let sonar_pro = model(&models, "sonar-pro");
        let reasoning_pro = model(&models, "sonar-reasoning-pro");

        assert_eq!(sonar_pro.tier, ModelTier::Smart);
        assert_eq!(sonar_pro.context_window, 200_000);
        assert_eq!(sonar_pro.max_output_tokens, 8_192);
        assert_eq!(sonar_pro.input_cost_per_m, 3.0);
        assert_eq!(sonar_pro.output_cost_per_m, 15.0);

        assert_eq!(reasoning_pro.tier, ModelTier::Frontier);
        assert_eq!(reasoning_pro.context_window, 200_000);
        assert_eq!(reasoning_pro.max_output_tokens, 8_192);
        assert_eq!(reasoning_pro.input_cost_per_m, 2.0);
        assert_eq!(reasoning_pro.output_cost_per_m, 8.0);
    }

    #[test]
    fn perplexity_reasoning_and_basic_ids_stay_available() {
        let models = perplexity_models();
        let reasoning = model(&models, "sonar-reasoning");
        let basic = model(&models, "sonar-basic");

        assert_eq!(reasoning.display_name, "Sonar Reasoning");
        assert_eq!(reasoning.tier, ModelTier::Balanced);
        assert_eq!(reasoning.context_window, 128_000);
        assert_eq!(reasoning.input_cost_per_m, 1.0);
        assert_eq!(reasoning.output_cost_per_m, 5.0);

        assert_eq!(basic.display_name, "Sonar");
        assert_eq!(basic.tier, ModelTier::Fast);
        assert_eq!(basic.context_window, 128_000);
        assert_eq!(basic.input_cost_per_m, 1.0);
        assert_eq!(basic.output_cost_per_m, 5.0);
    }
}
