use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn kimi_coding_models() -> Vec<ModelCatalogEntry> {
    vec![ModelCatalogEntry {
        id: "kimi-for-coding".into(),
        display_name: "Kimi For Coding".into(),
        provider: "kimi_coding".into(),
        tier: ModelTier::Frontier,
        context_window: 262_144,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: true,
        supports_streaming: true,
        aliases: vec![],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelCatalogEntry {
        kimi_coding_models()
            .into_iter()
            .next()
            .expect("missing Kimi Coding model")
    }

    #[test]
    fn kimi_coding_models_count_is_stable() {
        let models = kimi_coding_models();

        assert_eq!(models.len(), 1);
        assert!(models.iter().all(|model| model.provider == "kimi_coding"));
    }

    #[test]
    fn kimi_coding_model_keeps_identity_and_tier() {
        let model = model();

        assert_eq!(model.id, "kimi-for-coding");
        assert_eq!(model.display_name, "Kimi For Coding");
        assert_eq!(model.tier, ModelTier::Frontier);
    }

    #[test]
    fn kimi_coding_model_keeps_window_and_cost_contract() {
        let model = model();

        assert_eq!(model.context_window, 262_144);
        assert_eq!(model.max_output_tokens, 32_768);
        assert_eq!(model.input_cost_per_m, 0.0);
        assert_eq!(model.output_cost_per_m, 0.0);
    }

    #[test]
    fn kimi_coding_model_keeps_capability_contract() {
        let model = model();

        assert!(model.supports_tools);
        assert!(model.supports_vision);
        assert!(model.supports_streaming);
        assert!(model.aliases.is_empty());
    }
}
