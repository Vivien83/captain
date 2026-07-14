use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn claude_code_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "claude-code/opus".into(),
            display_name: "Claude Opus (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Frontier,
            context_window: 200_000,
            max_output_tokens: 128_000,
            input_cost_per_m: 5.0,
            output_cost_per_m: 25.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code-opus".into()],
        },
        ModelCatalogEntry {
            id: "claude-code/sonnet".into(),
            display_name: "Claude Sonnet (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Smart,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_cost_per_m: 3.0,
            output_cost_per_m: 15.0,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code".into(), "claude-code-sonnet".into()],
        },
        ModelCatalogEntry {
            id: "claude-code/haiku".into(),
            display_name: "Claude Haiku (CLI)".into(),
            provider: "claude-code".into(),
            tier: ModelTier::Fast,
            context_window: 200_000,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.25,
            output_cost_per_m: 1.25,
            supports_tools: false,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["claude-code-haiku".into()],
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
            .unwrap_or_else(|| panic!("missing Claude Code model {id}"))
    }

    #[test]
    fn claude_code_models_count_and_provider_are_stable() {
        let models = claude_code_models();

        assert_eq!(models.len(), 3);
        assert!(models.iter().all(|model| model.provider == "claude-code"));
        assert!(models.iter().all(|model| !model.supports_tools));
        assert!(models.iter().all(|model| !model.supports_vision));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn claude_code_models_keep_alias_contract() {
        let models = claude_code_models();

        assert_eq!(
            model(&models, "claude-code/opus").aliases,
            vec!["claude-code-opus".to_string()]
        );
        assert_eq!(
            model(&models, "claude-code/sonnet").aliases,
            vec!["claude-code".to_string(), "claude-code-sonnet".to_string()]
        );
        assert_eq!(
            model(&models, "claude-code/haiku").aliases,
            vec!["claude-code-haiku".to_string()]
        );
    }

    #[test]
    fn claude_code_frontier_model_keeps_contract() {
        let models = claude_code_models();
        let opus = model(&models, "claude-code/opus");

        assert_eq!(opus.display_name, "Claude Opus (CLI)");
        assert_eq!(opus.tier, ModelTier::Frontier);
        assert_eq!(opus.context_window, 200_000);
        assert_eq!(opus.max_output_tokens, 128_000);
        assert_eq!(opus.input_cost_per_m, 5.0);
        assert_eq!(opus.output_cost_per_m, 25.0);
    }

    #[test]
    fn claude_code_smart_and_fast_models_keep_contract() {
        let models = claude_code_models();
        let sonnet = model(&models, "claude-code/sonnet");
        let haiku = model(&models, "claude-code/haiku");

        assert_eq!(sonnet.display_name, "Claude Sonnet (CLI)");
        assert_eq!(sonnet.tier, ModelTier::Smart);
        assert_eq!(sonnet.context_window, 200_000);
        assert_eq!(sonnet.max_output_tokens, 64_000);
        assert_eq!(sonnet.input_cost_per_m, 3.0);
        assert_eq!(sonnet.output_cost_per_m, 15.0);

        assert_eq!(haiku.display_name, "Claude Haiku (CLI)");
        assert_eq!(haiku.tier, ModelTier::Fast);
        assert_eq!(haiku.context_window, 200_000);
        assert_eq!(haiku.max_output_tokens, 8_192);
        assert_eq!(haiku.input_cost_per_m, 0.25);
        assert_eq!(haiku.output_cost_per_m, 1.25);
    }
}
