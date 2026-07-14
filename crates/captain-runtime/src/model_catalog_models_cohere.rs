use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn cohere_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "command-r-plus".into(),
            display_name: "Command R+".into(),
            provider: "cohere".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["command-r".into()],
        },
        ModelCatalogEntry {
            id: "command-r-08-2024".into(),
            display_name: "Command R (Aug 2024)".into(),
            provider: "cohere".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.15,
            output_cost_per_m: 0.60,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "command-a".into(),
            display_name: "Command A".into(),
            provider: "cohere".into(),
            tier: ModelTier::Smart,
            context_window: 256_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 2.50,
            output_cost_per_m: 10.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "command-light".into(),
            display_name: "Command Light".into(),
            provider: "cohere".into(),
            tier: ModelTier::Fast,
            context_window: 4_096,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.60,
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
            .unwrap_or_else(|| panic!("missing Cohere model {id}"))
    }

    #[test]
    fn cohere_models_count_is_stable() {
        let models = cohere_models();

        assert_eq!(models.len(), 4);
        assert!(models.iter().all(|model| model.provider == "cohere"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn cohere_models_keep_primary_alias() {
        let models = cohere_models();

        assert_eq!(
            model(&models, "command-r-plus").aliases.as_slice(),
            ["command-r"]
        );
        assert!(model(&models, "command-r-08-2024").aliases.is_empty());
        assert!(model(&models, "command-a").aliases.is_empty());
        assert!(model(&models, "command-light").aliases.is_empty());
    }

    #[test]
    fn cohere_smart_models_keep_pricing_and_windows() {
        let models = cohere_models();
        let command_r_plus = model(&models, "command-r-plus");
        let command_a = model(&models, "command-a");

        assert_eq!(command_r_plus.display_name, "Command R+");
        assert_eq!(command_r_plus.tier, ModelTier::Smart);
        assert_eq!(command_r_plus.context_window, 128_000);
        assert_eq!(command_r_plus.max_output_tokens, 4_096);
        assert_eq!(command_r_plus.input_cost_per_m, 2.50);
        assert_eq!(command_r_plus.output_cost_per_m, 10.0);

        assert_eq!(command_a.display_name, "Command A");
        assert_eq!(command_a.tier, ModelTier::Smart);
        assert_eq!(command_a.context_window, 256_000);
        assert_eq!(command_a.max_output_tokens, 8_192);
        assert_eq!(command_a.input_cost_per_m, 2.50);
        assert_eq!(command_a.output_cost_per_m, 10.0);
    }

    #[test]
    fn cohere_balanced_and_fast_ids_stay_available() {
        let models = cohere_models();
        let command_r = model(&models, "command-r-08-2024");
        let command_light = model(&models, "command-light");

        assert_eq!(command_r.display_name, "Command R (Aug 2024)");
        assert_eq!(command_r.tier, ModelTier::Balanced);
        assert_eq!(command_r.context_window, 128_000);
        assert_eq!(command_r.max_output_tokens, 4_096);
        assert_eq!(command_r.input_cost_per_m, 0.15);
        assert_eq!(command_r.output_cost_per_m, 0.60);

        assert_eq!(command_light.display_name, "Command Light");
        assert_eq!(command_light.tier, ModelTier::Fast);
        assert_eq!(command_light.context_window, 4_096);
        assert_eq!(command_light.max_output_tokens, 4_096);
        assert_eq!(command_light.input_cost_per_m, 0.30);
        assert_eq!(command_light.output_cost_per_m, 0.60);
    }
}
