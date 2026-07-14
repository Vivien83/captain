use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn github_copilot_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "copilot/gpt-4o".into(),
            display_name: "GPT-4o (Copilot)".into(),
            provider: "github-copilot".into(),
            tier: ModelTier::Smart,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["copilot-gpt4o".into()],
        },
        ModelCatalogEntry {
            id: "copilot/gpt-4".into(),
            display_name: "GPT-4 (Copilot)".into(),
            provider: "github-copilot".into(),
            tier: ModelTier::Frontier,
            context_window: 128_000,
            max_output_tokens: 4_096,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["copilot-gpt4".into()],
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
            .unwrap_or_else(|| panic!("missing GitHub Copilot model {id}"))
    }

    #[test]
    fn github_copilot_models_count_is_stable() {
        let models = github_copilot_models();

        assert_eq!(models.len(), 2);
        assert!(models
            .iter()
            .all(|model| model.provider == "github-copilot"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn github_copilot_models_are_free_for_subscribers() {
        let models = github_copilot_models();

        assert!(models.iter().all(|model| model.input_cost_per_m == 0.0));
        assert!(models.iter().all(|model| model.output_cost_per_m == 0.0));
    }

    #[test]
    fn github_copilot_models_keep_primary_aliases() {
        let models = github_copilot_models();

        assert_eq!(
            model(&models, "copilot/gpt-4o").aliases,
            vec!["copilot-gpt4o".to_string()]
        );
        assert_eq!(
            model(&models, "copilot/gpt-4").aliases,
            vec!["copilot-gpt4".to_string()]
        );
    }

    #[test]
    fn github_copilot_capabilities_are_preserved() {
        let models = github_copilot_models();
        let gpt_4o = model(&models, "copilot/gpt-4o");
        let gpt_4 = model(&models, "copilot/gpt-4");

        assert_eq!(gpt_4o.display_name, "GPT-4o (Copilot)");
        assert_eq!(gpt_4o.tier, ModelTier::Smart);
        assert_eq!(gpt_4o.context_window, 128_000);
        assert_eq!(gpt_4o.max_output_tokens, 4_096);
        assert!(gpt_4o.supports_vision);

        assert_eq!(gpt_4.display_name, "GPT-4 (Copilot)");
        assert_eq!(gpt_4.tier, ModelTier::Frontier);
        assert_eq!(gpt_4.context_window, 128_000);
        assert_eq!(gpt_4.max_output_tokens, 4_096);
        assert!(!gpt_4.supports_vision);
    }
}
