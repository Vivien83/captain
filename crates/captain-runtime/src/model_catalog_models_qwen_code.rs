use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn qwen_code_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "qwen-code/qwen-coder-plus".into(),
            display_name: "Qwen Coder Plus (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Frontier,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen-coder-plus".into()],
        },
        ModelCatalogEntry {
            id: "qwen-code/qwen3-coder".into(),
            display_name: "Qwen3 Coder (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Smart,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwen-code".into(), "qwen-coder".into()],
        },
        ModelCatalogEntry {
            id: "qwen-code/qwq-32b".into(),
            display_name: "QwQ 32B (CLI)".into(),
            provider: "qwen-code".into(),
            tier: ModelTier::Balanced,
            context_window: 131_072,
            max_output_tokens: 65_536,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["qwq".into()],
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
            .unwrap_or_else(|| panic!("missing Qwen Code model {id}"))
    }

    #[test]
    fn qwen_code_models_count_and_provider_are_stable() {
        let models = qwen_code_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "qwen-code"));
        assert!(models.iter().all(|model| !model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.input_cost_per_m == 0.0));
        assert!(models.iter().all(|model| model.output_cost_per_m == 0.0));
    }

    #[test]
    fn qwen_code_models_keep_alias_contract() {
        let models = qwen_code_models();

        assert_eq!(
            model(&models, "qwen-code/qwen-coder-plus").aliases,
            vec!["qwen-coder-plus".to_string()]
        );
        assert_eq!(
            model(&models, "qwen-code/qwen3-coder").aliases,
            vec!["qwen-code".to_string(), "qwen-coder".to_string()]
        );
        assert_eq!(
            model(&models, "qwen-code/qwq-32b").aliases,
            vec!["qwq".to_string()]
        );
    }

    #[test]
    fn qwen_code_frontier_model_keeps_contract() {
        let models = qwen_code_models();
        let plus = model(&models, "qwen-code/qwen-coder-plus");

        assert_eq!(plus.display_name, "Qwen Coder Plus (CLI)");
        assert_eq!(plus.tier, ModelTier::Frontier);
        assert_eq!(plus.context_window, 131_072);
        assert_eq!(plus.max_output_tokens, 65_536);
    }

    #[test]
    fn qwen_code_smart_and_balanced_models_keep_contract() {
        let models = qwen_code_models();
        let coder = model(&models, "qwen-code/qwen3-coder");
        let qwq = model(&models, "qwen-code/qwq-32b");

        assert_eq!(coder.display_name, "Qwen3 Coder (CLI)");
        assert_eq!(coder.tier, ModelTier::Smart);
        assert_eq!(coder.context_window, 131_072);
        assert_eq!(coder.max_output_tokens, 65_536);

        assert_eq!(qwq.display_name, "QwQ 32B (CLI)");
        assert_eq!(qwq.tier, ModelTier::Balanced);
        assert_eq!(qwq.context_window, 131_072);
        assert_eq!(qwq.max_output_tokens, 65_536);
    }
}
