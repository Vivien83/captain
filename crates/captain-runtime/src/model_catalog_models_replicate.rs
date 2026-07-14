use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn replicate_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "replicate/meta-llama-3.3-70b-instruct".into(),
            display_name: "Llama 3.3 70B (Replicate)".into(),
            provider: "replicate".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.40,
            output_cost_per_m: 0.40,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "replicate/deepseek-r1".into(),
            display_name: "DeepSeek R1 (Replicate)".into(),
            provider: "replicate".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.40,
            output_cost_per_m: 0.40,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "replicate/mistral-7b-instruct".into(),
            display_name: "Mistral 7B (Replicate)".into(),
            provider: "replicate".into(),
            tier: ModelTier::Fast,
            context_window: 32_768,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.05,
            output_cost_per_m: 0.25,
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
            .unwrap_or_else(|| panic!("missing Replicate model {id}"))
    }

    #[test]
    fn replicate_models_count_is_stable() {
        let models = replicate_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "replicate"));
        assert!(models.iter().all(|model| !model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn replicate_models_keep_empty_aliases() {
        let models = replicate_models();

        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn replicate_llama_and_deepseek_keep_pricing_and_windows() {
        let models = replicate_models();
        let llama = model(&models, "replicate/meta-llama-3.3-70b-instruct");
        let deepseek = model(&models, "replicate/deepseek-r1");

        assert_eq!(llama.display_name, "Llama 3.3 70B (Replicate)");
        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.context_window, 128_000);
        assert_eq!(llama.max_output_tokens, 4_096);
        assert_eq!(llama.input_cost_per_m, 0.40);
        assert_eq!(llama.output_cost_per_m, 0.40);

        assert_eq!(deepseek.display_name, "DeepSeek R1 (Replicate)");
        assert_eq!(deepseek.tier, ModelTier::Smart);
        assert_eq!(deepseek.context_window, 64_000);
        assert_eq!(deepseek.max_output_tokens, 4_096);
        assert_eq!(deepseek.input_cost_per_m, 0.40);
        assert_eq!(deepseek.output_cost_per_m, 0.40);
    }

    #[test]
    fn replicate_mistral_id_stays_available() {
        let models = replicate_models();
        let mistral = model(&models, "replicate/mistral-7b-instruct");

        assert_eq!(mistral.display_name, "Mistral 7B (Replicate)");
        assert_eq!(mistral.tier, ModelTier::Fast);
        assert_eq!(mistral.context_window, 32_768);
        assert_eq!(mistral.max_output_tokens, 4_096);
        assert_eq!(mistral.input_cost_per_m, 0.05);
        assert_eq!(mistral.output_cost_per_m, 0.25);
    }
}
