use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn chutes_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "chutes/deepseek-ai/DeepSeek-V3".into(),
            display_name: "DeepSeek V3 (Chutes)".into(),
            provider: "chutes".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.25,
            output_cost_per_m: 0.35,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["chutes-deepseek-v3".into()],
        },
        ModelCatalogEntry {
            id: "chutes/deepseek-ai/DeepSeek-R1".into(),
            display_name: "DeepSeek R1 (Chutes)".into(),
            provider: "chutes".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.55,
            output_cost_per_m: 2.19,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["chutes-deepseek-r1".into()],
        },
        ModelCatalogEntry {
            id: "chutes/meta-llama/Llama-4-Maverick-17B-128E-Instruct".into(),
            display_name: "Llama 4 Maverick (Chutes)".into(),
            provider: "chutes".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.20,
            output_cost_per_m: 0.30,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["chutes-llama-maverick".into()],
        },
        ModelCatalogEntry {
            id: "chutes/Qwen/Qwen3-235B-A22B".into(),
            display_name: "Qwen3 235B (Chutes)".into(),
            provider: "chutes".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.25,
            output_cost_per_m: 0.35,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["chutes-qwen3".into()],
        },
        ModelCatalogEntry {
            id: "chutes/meta-llama/Llama-3.3-70B-Instruct".into(),
            display_name: "Llama 3.3 70B (Chutes)".into(),
            provider: "chutes".into(),
            tier: ModelTier::Balanced,
            context_window: 128_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.10,
            output_cost_per_m: 0.15,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["chutes-llama-70b".into()],
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
            .unwrap_or_else(|| panic!("missing Chutes model {id}"))
    }

    #[test]
    fn chutes_models_count_and_provider_are_stable() {
        let models = chutes_models();

        assert_eq!(models.len(), 5);
        assert!(models.iter().all(|model| model.provider == "chutes"));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert_eq!(
            models
                .iter()
                .filter(|model| !model.supports_tools)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["chutes/deepseek-ai/DeepSeek-R1"]
        );
    }

    #[test]
    fn chutes_models_keep_alias_contract() {
        let models = chutes_models();

        assert_eq!(
            model(&models, "chutes/deepseek-ai/DeepSeek-V3").aliases,
            vec!["chutes-deepseek-v3".to_string()]
        );
        assert_eq!(
            model(&models, "chutes/deepseek-ai/DeepSeek-R1").aliases,
            vec!["chutes-deepseek-r1".to_string()]
        );
        assert_eq!(
            model(
                &models,
                "chutes/meta-llama/Llama-4-Maverick-17B-128E-Instruct"
            )
            .aliases,
            vec!["chutes-llama-maverick".to_string()]
        );
        assert_eq!(
            model(&models, "chutes/Qwen/Qwen3-235B-A22B").aliases,
            vec!["chutes-qwen3".to_string()]
        );
        assert_eq!(
            model(&models, "chutes/meta-llama/Llama-3.3-70B-Instruct").aliases,
            vec!["chutes-llama-70b".to_string()]
        );
    }

    #[test]
    fn chutes_deepseek_models_keep_contract() {
        let models = chutes_models();
        let v3 = model(&models, "chutes/deepseek-ai/DeepSeek-V3");
        let r1 = model(&models, "chutes/deepseek-ai/DeepSeek-R1");

        assert_eq!(v3.display_name, "DeepSeek V3 (Chutes)");
        assert_eq!(v3.tier, ModelTier::Smart);
        assert_eq!(v3.context_window, 128_000);
        assert_eq!(v3.max_output_tokens, 8_192);
        assert_eq!(v3.input_cost_per_m, 0.25);
        assert_eq!(v3.output_cost_per_m, 0.35);
        assert!(v3.supports_tools);

        assert_eq!(r1.display_name, "DeepSeek R1 (Chutes)");
        assert_eq!(r1.tier, ModelTier::Smart);
        assert_eq!(r1.context_window, 128_000);
        assert_eq!(r1.max_output_tokens, 8_192);
        assert_eq!(r1.input_cost_per_m, 0.55);
        assert_eq!(r1.output_cost_per_m, 2.19);
        assert!(!r1.supports_tools);
    }

    #[test]
    fn chutes_llama_and_qwen_models_keep_contract() {
        let models = chutes_models();
        let maverick = model(
            &models,
            "chutes/meta-llama/Llama-4-Maverick-17B-128E-Instruct",
        );
        let qwen = model(&models, "chutes/Qwen/Qwen3-235B-A22B");
        let llama = model(&models, "chutes/meta-llama/Llama-3.3-70B-Instruct");

        assert_eq!(maverick.display_name, "Llama 4 Maverick (Chutes)");
        assert_eq!(maverick.tier, ModelTier::Balanced);
        assert_eq!(maverick.input_cost_per_m, 0.20);
        assert_eq!(maverick.output_cost_per_m, 0.30);

        assert_eq!(qwen.display_name, "Qwen3 235B (Chutes)");
        assert_eq!(qwen.tier, ModelTier::Smart);
        assert_eq!(qwen.input_cost_per_m, 0.25);
        assert_eq!(qwen.output_cost_per_m, 0.35);

        assert_eq!(llama.display_name, "Llama 3.3 70B (Chutes)");
        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.input_cost_per_m, 0.10);
        assert_eq!(llama.output_cost_per_m, 0.15);
    }
}
