use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn codex_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "codex/gpt-5.5".into(),
            display_name: "GPT-5.5 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Frontier,
            context_window: 272_000,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex".into(), "codex-5.5".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-5.4".into(),
            display_name: "GPT-5.4 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-5.4".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-5.3-codex".into(),
            display_name: "GPT-5.3 Codex (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Smart,
            context_window: 272_000,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-5.3".into(), "codex-5.3-codex".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-5.3-codex-spark".into(),
            display_name: "GPT-5.3 Codex Spark (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Fast,
            context_window: 128_000,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["codex-5.3-spark".into(), "codex-5.3-codex-spark".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-5.2".into(),
            display_name: "GPT-5.2 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Balanced,
            context_window: 272_000,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-5.2".into()],
        },
        ModelCatalogEntry {
            id: "codex/gpt-4.1".into(),
            display_name: "GPT-4.1 (Codex)".into(),
            provider: "codex".into(),
            tier: ModelTier::Frontier,
            context_window: 1_047_576,
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["codex-4.1".into()],
        },
    ]
}

pub(crate) fn codex_static_model_choices() -> Vec<(String, String)> {
    codex_models()
        .into_iter()
        .filter(|model| model.id != "codex/gpt-4.1")
        .map(|model| {
            let slug = model
                .id
                .strip_prefix("codex/")
                .unwrap_or(&model.id)
                .to_string();
            (slug, model.display_name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(models: &'a [ModelCatalogEntry], id: &str) -> &'a ModelCatalogEntry {
        models
            .iter()
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("missing Codex model {id}"))
    }

    #[test]
    fn codex_models_count_and_provider_are_stable() {
        let models = codex_models();

        assert_eq!(models.len(), 6);
        assert!(models.iter().all(|model| model.provider == "codex"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.input_cost_per_m == 0.0));
        assert!(models.iter().all(|model| model.output_cost_per_m == 0.0));
        assert_eq!(
            models
                .iter()
                .filter(|model| !model.supports_vision)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex/gpt-5.3-codex-spark"]
        );
    }

    #[test]
    fn codex_models_keep_alias_contract() {
        let models = codex_models();

        assert_eq!(
            model(&models, "codex/gpt-5.5").aliases,
            vec!["codex".to_string(), "codex-5.5".to_string()]
        );
        assert_eq!(
            model(&models, "codex/gpt-5.3-codex").aliases,
            vec!["codex-5.3".to_string(), "codex-5.3-codex".to_string()]
        );
        assert_eq!(
            model(&models, "codex/gpt-5.3-codex-spark").aliases,
            vec![
                "codex-5.3-spark".to_string(),
                "codex-5.3-codex-spark".to_string()
            ]
        );
        assert_eq!(
            model(&models, "codex/gpt-4.1").aliases,
            vec!["codex-4.1".to_string()]
        );
    }

    #[test]
    fn codex_frontier_models_keep_contract() {
        let models = codex_models();
        let gpt_55 = model(&models, "codex/gpt-5.5");
        let gpt_54 = model(&models, "codex/gpt-5.4");
        let gpt_41 = model(&models, "codex/gpt-4.1");

        assert_eq!(gpt_55.display_name, "GPT-5.5 (Codex)");
        assert_eq!(gpt_55.tier, ModelTier::Frontier);
        assert_eq!(gpt_55.context_window, 272_000);
        assert_eq!(gpt_55.max_output_tokens, 32_768);

        assert_eq!(gpt_54.display_name, "GPT-5.4 (Codex)");
        assert_eq!(gpt_54.tier, ModelTier::Frontier);
        assert_eq!(gpt_54.context_window, 1_047_576);
        assert_eq!(gpt_54.max_output_tokens, 32_768);

        assert_eq!(gpt_41.display_name, "GPT-4.1 (Codex)");
        assert_eq!(gpt_41.tier, ModelTier::Frontier);
        assert_eq!(gpt_41.context_window, 1_047_576);
        assert_eq!(gpt_41.max_output_tokens, 32_768);
    }

    #[test]
    fn codex_balanced_smart_and_fast_models_keep_contract() {
        let models = codex_models();
        let gpt_53 = model(&models, "codex/gpt-5.3-codex");
        let spark = model(&models, "codex/gpt-5.3-codex-spark");
        let gpt_52 = model(&models, "codex/gpt-5.2");

        assert_eq!(gpt_53.display_name, "GPT-5.3 Codex (Codex)");
        assert_eq!(gpt_53.tier, ModelTier::Smart);
        assert_eq!(gpt_53.context_window, 272_000);
        assert!(gpt_53.supports_vision);

        assert_eq!(spark.display_name, "GPT-5.3 Codex Spark (Codex)");
        assert_eq!(spark.tier, ModelTier::Fast);
        assert_eq!(spark.context_window, 128_000);
        assert!(!spark.supports_vision);

        assert_eq!(gpt_52.display_name, "GPT-5.2 (Codex)");
        assert_eq!(gpt_52.tier, ModelTier::Balanced);
        assert_eq!(gpt_52.context_window, 272_000);
        assert!(gpt_52.supports_vision);
    }

    #[test]
    fn codex_static_model_choices_preserve_current_default_list() {
        assert_eq!(
            codex_static_model_choices(),
            vec![
                ("gpt-5.5".to_string(), "GPT-5.5 (Codex)".to_string()),
                ("gpt-5.4".to_string(), "GPT-5.4 (Codex)".to_string()),
                (
                    "gpt-5.3-codex".to_string(),
                    "GPT-5.3 Codex (Codex)".to_string()
                ),
                (
                    "gpt-5.3-codex-spark".to_string(),
                    "GPT-5.3 Codex Spark (Codex)".to_string()
                ),
                ("gpt-5.2".to_string(), "GPT-5.2 (Codex)".to_string())
            ]
        );
    }
}
