use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn ai21_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "jamba-1.5-large".into(),
            display_name: "Jamba 1.5 Large".into(),
            provider: "ai21".into(),
            tier: ModelTier::Smart,
            context_window: 256_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 2.0,
            output_cost_per_m: 8.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["jamba".into()],
        },
        ModelCatalogEntry {
            id: "jamba-1.5-mini".into(),
            display_name: "Jamba 1.5 Mini".into(),
            provider: "ai21".into(),
            tier: ModelTier::Fast,
            context_window: 256_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.20,
            output_cost_per_m: 0.40,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "jamba-instruct".into(),
            display_name: "Jamba Instruct".into(),
            provider: "ai21".into(),
            tier: ModelTier::Balanced,
            context_window: 256_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.50,
            output_cost_per_m: 0.70,
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
            .unwrap_or_else(|| panic!("missing AI21 model {id}"))
    }

    #[test]
    fn ai21_models_count_is_stable() {
        let models = ai21_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "ai21"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.context_window == 256_000));
    }

    #[test]
    fn ai21_models_keep_primary_alias() {
        let models = ai21_models();

        assert_eq!(
            model(&models, "jamba-1.5-large").aliases.as_slice(),
            ["jamba"]
        );
        assert!(model(&models, "jamba-1.5-mini").aliases.is_empty());
        assert!(model(&models, "jamba-instruct").aliases.is_empty());
    }

    #[test]
    fn ai21_large_model_keeps_pricing_and_window() {
        let models = ai21_models();
        let large = model(&models, "jamba-1.5-large");

        assert_eq!(large.display_name, "Jamba 1.5 Large");
        assert_eq!(large.tier, ModelTier::Smart);
        assert_eq!(large.context_window, 256_000);
        assert_eq!(large.max_output_tokens, 4_096);
        assert_eq!(large.input_cost_per_m, 2.0);
        assert_eq!(large.output_cost_per_m, 8.0);
    }

    #[test]
    fn ai21_mini_and_instruct_ids_stay_available() {
        let models = ai21_models();
        let mini = model(&models, "jamba-1.5-mini");
        let instruct = model(&models, "jamba-instruct");

        assert_eq!(mini.display_name, "Jamba 1.5 Mini");
        assert_eq!(mini.tier, ModelTier::Fast);
        assert_eq!(mini.max_output_tokens, 4_096);
        assert_eq!(mini.input_cost_per_m, 0.20);
        assert_eq!(mini.output_cost_per_m, 0.40);

        assert_eq!(instruct.display_name, "Jamba Instruct");
        assert_eq!(instruct.tier, ModelTier::Balanced);
        assert_eq!(instruct.max_output_tokens, 4_096);
        assert_eq!(instruct.input_cost_per_m, 0.50);
        assert_eq!(instruct.output_cost_per_m, 0.70);
    }
}
