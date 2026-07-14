use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn nvidia_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "nvidia/llama-3.1-nemotron-70b-instruct".into(),
            display_name: "Nemotron 70B Instruct (NVIDIA NIM)".into(),
            provider: "nvidia".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.88,
            output_cost_per_m: 0.88,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["nemotron-70b".into()],
        },
        ModelCatalogEntry {
            id: "meta/llama-3.1-405b-instruct".into(),
            display_name: "Llama 3.1 405B Instruct (NVIDIA NIM)".into(),
            provider: "nvidia".into(),
            tier: ModelTier::Frontier,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 5.00,
            output_cost_per_m: 16.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "meta/llama-3.1-70b-instruct".into(),
            display_name: "Llama 3.1 70B Instruct (NVIDIA NIM)".into(),
            provider: "nvidia".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.88,
            output_cost_per_m: 0.88,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "mistralai/mistral-large-latest".into(),
            display_name: "Mistral Large (NVIDIA NIM)".into(),
            provider: "nvidia".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 2.00,
            output_cost_per_m: 6.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "nvidia/nemotron-4-340b-instruct".into(),
            display_name: "Nemotron 4 340B Instruct (NVIDIA NIM)".into(),
            provider: "nvidia".into(),
            tier: ModelTier::Frontier,
            context_window: 4_096,
            max_output_tokens: 4_096,
            input_cost_per_m: 4.20,
            output_cost_per_m: 4.20,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["nemotron-340b".into()],
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
            .unwrap_or_else(|| panic!("missing NVIDIA NIM model {id}"))
    }

    #[test]
    fn nvidia_models_count_is_stable() {
        let models = nvidia_models();

        assert_eq!(models.len(), 5);
        assert!(models.iter().all(|model| model.provider == "nvidia"));
    }

    #[test]
    fn nvidia_models_keep_primary_aliases() {
        let models = nvidia_models();

        assert_eq!(
            model(&models, "nvidia/llama-3.1-nemotron-70b-instruct")
                .aliases
                .as_slice(),
            ["nemotron-70b"]
        );
        assert_eq!(
            model(&models, "nvidia/nemotron-4-340b-instruct")
                .aliases
                .as_slice(),
            ["nemotron-340b"]
        );
    }

    #[test]
    fn nvidia_llama_pricing_and_capabilities_are_preserved() {
        let models = nvidia_models();
        let nemotron70 = model(&models, "nvidia/llama-3.1-nemotron-70b-instruct");
        let llama405 = model(&models, "meta/llama-3.1-405b-instruct");
        let llama70 = model(&models, "meta/llama-3.1-70b-instruct");

        assert_eq!(nemotron70.tier, ModelTier::Smart);
        assert_eq!(nemotron70.context_window, 128_000);
        assert_eq!(nemotron70.max_output_tokens, 4_096);
        assert_eq!(nemotron70.input_cost_per_m, 0.88);
        assert_eq!(nemotron70.output_cost_per_m, 0.88);

        assert_eq!(llama405.tier, ModelTier::Frontier);
        assert_eq!(llama405.context_window, 128_000);
        assert_eq!(llama405.input_cost_per_m, 5.00);
        assert_eq!(llama405.output_cost_per_m, 16.00);

        assert_eq!(llama70.tier, ModelTier::Balanced);
        assert_eq!(llama70.context_window, 128_000);
        assert_eq!(llama70.input_cost_per_m, 0.88);
        assert_eq!(llama70.output_cost_per_m, 0.88);

        for model in [nemotron70, llama405, llama70] {
            assert!(model.supports_tools);
            assert!(!model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn nvidia_mistral_and_nemotron_ids_stay_available() {
        let models = nvidia_models();
        let mistral = model(&models, "mistralai/mistral-large-latest");
        let nemotron340 = model(&models, "nvidia/nemotron-4-340b-instruct");

        assert_eq!(mistral.tier, ModelTier::Smart);
        assert_eq!(mistral.context_window, 128_000);
        assert_eq!(mistral.max_output_tokens, 4_096);
        assert_eq!(mistral.input_cost_per_m, 2.00);
        assert_eq!(mistral.output_cost_per_m, 6.00);

        assert_eq!(nemotron340.tier, ModelTier::Frontier);
        assert_eq!(nemotron340.context_window, 4_096);
        assert_eq!(nemotron340.max_output_tokens, 4_096);
        assert_eq!(nemotron340.input_cost_per_m, 4.20);
        assert_eq!(nemotron340.output_cost_per_m, 4.20);
    }
}
