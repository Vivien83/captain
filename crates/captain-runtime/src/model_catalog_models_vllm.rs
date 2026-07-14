use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn vllm_models() -> Vec<ModelCatalogEntry> {
    vec![ModelCatalogEntry {
        id: "vllm-local".into(),
        display_name: "vLLM Local Model".into(),
        provider: "vllm".into(),
        tier: ModelTier::Local,
        context_window: 32_768,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.0,
        output_cost_per_m: 0.0,
        supports_tools: true,
        supports_vision: false,
        supports_streaming: true,
        aliases: vec![],
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vllm_models_count_is_stable() {
        let models = vllm_models();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].provider, "vllm");
        assert_eq!(models[0].id, "vllm-local");
        assert!(models[0].aliases.is_empty());
    }

    #[test]
    fn vllm_local_contract_is_preserved() {
        let model = &vllm_models()[0];

        assert_eq!(model.display_name, "vLLM Local Model");
        assert_eq!(model.tier, ModelTier::Local);
        assert_eq!(model.context_window, 32_768);
        assert_eq!(model.max_output_tokens, 4_096);
        assert_eq!(model.input_cost_per_m, 0.0);
        assert_eq!(model.output_cost_per_m, 0.0);
        assert!(model.supports_tools);
        assert!(!model.supports_vision);
        assert!(model.supports_streaming);
    }
}
