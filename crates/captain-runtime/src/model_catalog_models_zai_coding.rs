use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn zai_coding_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "glm-5-coding".into(),
            display_name: "GLM-5 Coding".into(),
            provider: "zai_coding".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 2.00,
            output_cost_per_m: 8.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["glm-5-code".into(), "glm-coding".into()],
        },
        ModelCatalogEntry {
            id: "glm-4.7-coding".into(),
            display_name: "GLM-4.7 Coding".into(),
            provider: "zai_coding".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.50,
            output_cost_per_m: 5.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["glm-4.7-code".into()],
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
            .unwrap_or_else(|| panic!("missing Z.AI Coding model {id}"))
    }

    #[test]
    fn zai_coding_models_count_is_stable() {
        let models = zai_coding_models();

        assert_eq!(models.len(), 2);
        assert!(models.iter().all(|model| model.provider == "zai_coding"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| !model.supports_vision));
    }

    #[test]
    fn zai_coding_models_keep_primary_aliases() {
        let models = zai_coding_models();

        assert_eq!(
            model(&models, "glm-5-coding").aliases,
            vec!["glm-5-code".to_string(), "glm-coding".to_string()]
        );
        assert_eq!(
            model(&models, "glm-4.7-coding").aliases,
            vec!["glm-4.7-code".to_string()]
        );
    }

    #[test]
    fn zai_coding_frontier_model_keeps_contract() {
        let models = zai_coding_models();
        let model = model(&models, "glm-5-coding");

        assert_eq!(model.display_name, "GLM-5 Coding");
        assert_eq!(model.tier, ModelTier::Frontier);
        assert_eq!(model.context_window, 131_072);
        assert_eq!(model.max_output_tokens, 16_384);
        assert_eq!(model.input_cost_per_m, 2.00);
        assert_eq!(model.output_cost_per_m, 8.00);
    }

    #[test]
    fn zai_coding_smart_model_keeps_contract() {
        let models = zai_coding_models();
        let model = model(&models, "glm-4.7-coding");

        assert_eq!(model.display_name, "GLM-4.7 Coding");
        assert_eq!(model.tier, ModelTier::Smart);
        assert_eq!(model.context_window, 131_072);
        assert_eq!(model.max_output_tokens, 16_384);
        assert_eq!(model.input_cost_per_m, 1.50);
        assert_eq!(model.output_cost_per_m, 5.00);
    }
}
