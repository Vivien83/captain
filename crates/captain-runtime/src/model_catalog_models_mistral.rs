use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn mistral_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "mistral-large-latest".into(),
            display_name: "Mistral Large".into(),
            provider: "mistral".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 2.00,
            output_cost_per_m: 6.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["mistral".into()],
        },
        ModelCatalogEntry {
            id: "mistral-medium-latest".into(),
            display_name: "Mistral Medium".into(),
            provider: "mistral".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 2.70,
            output_cost_per_m: 8.10,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "mistral-small-latest".into(),
            display_name: "Mistral Small".into(),
            provider: "mistral".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.30,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "codestral-latest".into(),
            display_name: "Codestral".into(),
            provider: "mistral".into(),
            tier: ModelTier::Smart,
            context_window: 32_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.90,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["codestral".into()],
        },
        ModelCatalogEntry {
            id: "open-mistral-nemo".into(),
            display_name: "Mistral Nemo".into(),
            provider: "mistral".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.15,
            output_cost_per_m: 0.15,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["mistral-nemo".into()],
        },
        ModelCatalogEntry {
            id: "pixtral-large-latest".into(),
            display_name: "Pixtral Large".into(),
            provider: "mistral".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 2.00,
            output_cost_per_m: 6.00,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["pixtral".into()],
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
            .unwrap_or_else(|| panic!("missing Mistral model {id}"))
    }

    #[test]
    fn mistral_models_count_is_stable() {
        let models = mistral_models();

        assert_eq!(models.len(), 6);
        assert!(models.iter().all(|model| model.provider == "mistral"));
    }

    #[test]
    fn mistral_models_keep_primary_aliases() {
        let models = mistral_models();

        assert_eq!(
            model(&models, "mistral-large-latest").aliases.as_slice(),
            ["mistral"]
        );
        assert_eq!(
            model(&models, "codestral-latest").aliases.as_slice(),
            ["codestral"]
        );
        assert_eq!(
            model(&models, "pixtral-large-latest").aliases.as_slice(),
            ["pixtral"]
        );
    }

    #[test]
    fn mistral_pricing_and_capabilities_are_preserved() {
        let models = mistral_models();
        let large = model(&models, "mistral-large-latest");
        let medium = model(&models, "mistral-medium-latest");
        let pixtral = model(&models, "pixtral-large-latest");

        assert_eq!(large.tier, ModelTier::Smart);
        assert_eq!(large.context_window, 128_000);
        assert_eq!(large.max_output_tokens, 8_192);
        assert_eq!(large.input_cost_per_m, 2.00);
        assert_eq!(large.output_cost_per_m, 6.00);

        assert_eq!(medium.tier, ModelTier::Balanced);
        assert_eq!(medium.context_window, 128_000);
        assert_eq!(medium.input_cost_per_m, 2.70);
        assert_eq!(medium.output_cost_per_m, 8.10);

        assert_eq!(pixtral.tier, ModelTier::Smart);
        assert_eq!(pixtral.context_window, 128_000);
        assert_eq!(pixtral.input_cost_per_m, 2.00);
        assert_eq!(pixtral.output_cost_per_m, 6.00);
        assert!(pixtral.supports_vision);

        for model in [large, medium, pixtral] {
            assert!(model.supports_tools);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn mistral_small_and_code_ids_stay_available() {
        let models = mistral_models();
        let small = model(&models, "mistral-small-latest");
        let nemo = model(&models, "open-mistral-nemo");

        assert_eq!(small.tier, ModelTier::Fast);
        assert!(small.aliases.is_empty());
        assert_eq!(small.input_cost_per_m, 0.10);
        assert_eq!(small.output_cost_per_m, 0.30);

        assert_eq!(nemo.tier, ModelTier::Fast);
        assert_eq!(nemo.aliases.as_slice(), ["mistral-nemo"]);
        assert_eq!(nemo.input_cost_per_m, 0.15);
        assert_eq!(nemo.output_cost_per_m, 0.15);

        assert_eq!(model(&models, "codestral-latest").context_window, 32_000);
    }
}
