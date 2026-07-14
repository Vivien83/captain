use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn ollama_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "llama3.2".into(),
            display_name: "Llama 3.2 (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "llama3.1".into(),
            display_name: "Llama 3.1 (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "mistral:latest".into(),
            display_name: "Mistral (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 32_768,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "qwen2.5".into(),
            display_name: "Qwen 2.5 (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 32_768,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "phi3".into(),
            display_name: "Phi-3 (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "deepseek-r1:latest".into(),
            display_name: "DeepSeek R1 (Ollama)".into(),
            provider: "ollama".into(),
            tier: ModelTier::Local,
            context_window: 64_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
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
            .unwrap_or_else(|| panic!("missing Ollama model {id}"))
    }

    #[test]
    fn ollama_models_count_is_stable() {
        let models = ollama_models();

        assert_eq!(models.len(), 6);
        assert!(models.iter().all(|model| model.provider == "ollama"));
        assert!(models.iter().all(|model| model.tier == ModelTier::Local));
        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn ollama_models_are_free_local_streaming_models() {
        let models = ollama_models();

        for model in models {
            assert_eq!(model.input_cost_per_m, 0.0);
            assert_eq!(model.output_cost_per_m, 0.0);
            assert!(!model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn ollama_llama_and_qwen_models_keep_context_and_tools() {
        let models = ollama_models();
        let llama32 = model(&models, "llama3.2");
        let llama31 = model(&models, "llama3.1");
        let qwen = model(&models, "qwen2.5");

        assert_eq!(llama32.context_window, 128_000);
        assert_eq!(llama32.max_output_tokens, 4_096);
        assert!(llama32.supports_tools);

        assert_eq!(llama31.context_window, 128_000);
        assert_eq!(llama31.max_output_tokens, 4_096);
        assert!(llama31.supports_tools);

        assert_eq!(qwen.context_window, 32_768);
        assert_eq!(qwen.max_output_tokens, 4_096);
        assert!(qwen.supports_tools);
    }

    #[test]
    fn ollama_mistral_phi_and_deepseek_ids_stay_available() {
        let models = ollama_models();
        let mistral = model(&models, "mistral:latest");
        let phi = model(&models, "phi3");
        let deepseek = model(&models, "deepseek-r1:latest");

        assert_eq!(mistral.context_window, 32_768);
        assert!(mistral.supports_tools);

        assert_eq!(phi.context_window, 128_000);
        assert!(!phi.supports_tools);

        assert_eq!(deepseek.context_window, 64_000);
        assert!(!deepseek.supports_tools);
    }
}
