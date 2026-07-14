use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn zhipu_coding_models() -> Vec<ModelCatalogEntry> {
    vec![ModelCatalogEntry {
        id: "codegeex-4".into(),
        display_name: "CodeGeeX 4".into(),
        provider: "zhipu_coding".into(),
        tier: ModelTier::Smart,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.10,
        output_cost_per_m: 0.10,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        aliases: vec!["codegeex".into()],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zhipu_coding_models_count_is_stable() {
        let models = zhipu_coding_models();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "codegeex-4");
    }

    #[test]
    fn zhipu_coding_model_keeps_alias_and_provider() {
        let model = &zhipu_coding_models()[0];

        assert_eq!(model.display_name, "CodeGeeX 4");
        assert_eq!(model.provider, "zhipu_coding");
        assert_eq!(model.aliases, vec!["codegeex".to_string()]);
    }

    #[test]
    fn zhipu_coding_model_keeps_pricing_and_capabilities() {
        let model = &zhipu_coding_models()[0];

        assert_eq!(model.tier, ModelTier::Smart);
        assert_eq!(model.context_window, 131_072);
        assert_eq!(model.max_output_tokens, 8_192);
        assert_eq!(model.input_cost_per_m, 0.10);
        assert_eq!(model.output_cost_per_m, 0.10);
        assert!(model.supports_tools);
        assert!(!model.supports_vision);
        assert!(model.supports_streaming);
    }
}
