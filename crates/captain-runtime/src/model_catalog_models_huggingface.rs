use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn huggingface_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "hf/meta-llama/Llama-3.3-70B-Instruct".into(),
            display_name: "Llama 3.3 70B (HF)".into(),
            provider: "huggingface".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.30,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "hf/deepseek-ai/DeepSeek-R1".into(),
            display_name: "DeepSeek R1 (HF)".into(),
            provider: "huggingface".into(),
            tier: ModelTier::Smart,
            context_window: 64_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.30,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "hf/Qwen/Qwen2.5-72B-Instruct".into(),
            display_name: "Qwen 2.5 72B (HF)".into(),
            provider: "huggingface".into(),
            tier: ModelTier::Balanced,
            context_window: 32_768,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.30,
            output_cost_per_m: 0.30,
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
            .unwrap_or_else(|| panic!("missing Hugging Face model {id}"))
    }

    #[test]
    fn huggingface_models_count_is_stable() {
        let models = huggingface_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "huggingface"));
        assert!(models.iter().all(|model| !model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn huggingface_models_keep_empty_aliases() {
        let models = huggingface_models();

        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn huggingface_llama_and_deepseek_keep_pricing_and_windows() {
        let models = huggingface_models();
        let llama = model(&models, "hf/meta-llama/Llama-3.3-70B-Instruct");
        let deepseek = model(&models, "hf/deepseek-ai/DeepSeek-R1");

        assert_eq!(llama.display_name, "Llama 3.3 70B (HF)");
        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.context_window, 128_000);
        assert_eq!(llama.max_output_tokens, 4_096);
        assert_eq!(llama.input_cost_per_m, 0.30);
        assert_eq!(llama.output_cost_per_m, 0.30);

        assert_eq!(deepseek.display_name, "DeepSeek R1 (HF)");
        assert_eq!(deepseek.tier, ModelTier::Smart);
        assert_eq!(deepseek.context_window, 64_000);
        assert_eq!(deepseek.max_output_tokens, 4_096);
        assert_eq!(deepseek.input_cost_per_m, 0.30);
        assert_eq!(deepseek.output_cost_per_m, 0.30);
    }

    #[test]
    fn huggingface_qwen_id_stays_available() {
        let models = huggingface_models();
        let qwen = model(&models, "hf/Qwen/Qwen2.5-72B-Instruct");

        assert_eq!(qwen.display_name, "Qwen 2.5 72B (HF)");
        assert_eq!(qwen.tier, ModelTier::Balanced);
        assert_eq!(qwen.context_window, 32_768);
        assert_eq!(qwen.max_output_tokens, 4_096);
        assert_eq!(qwen.input_cost_per_m, 0.30);
        assert_eq!(qwen.output_cost_per_m, 0.30);
    }
}
