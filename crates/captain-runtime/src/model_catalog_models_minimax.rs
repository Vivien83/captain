use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

pub(crate) fn minimax_models() -> Vec<ModelCatalogEntry> {
    vec![
        ModelCatalogEntry {
            id: "minimax-text-01".into(),
            display_name: "MiniMax Text 01".into(),
            provider: "minimax".into(),
            tier: ModelTier::Smart,
            context_window: 1_048_576,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.00,
            output_cost_per_m: 3.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["minimax".into()],
        },
        ModelCatalogEntry {
            id: "MiniMax-M2.5".into(),
            display_name: "MiniMax M2.5".into(),
            provider: "minimax".into(),
            tier: ModelTier::Frontier,
            context_window: 1_048_576,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.10,
            output_cost_per_m: 4.40,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["minimax-m2.5".into()],
        },
        ModelCatalogEntry {
            id: "MiniMax-M2.5-highspeed".into(),
            display_name: "MiniMax M2.5 Highspeed".into(),
            provider: "minimax".into(),
            tier: ModelTier::Smart,
            context_window: 1_048_576,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.80,
            output_cost_per_m: 3.20,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["minimax-m2.5-highspeed".into(), "m2.5-highspeed".into()],
        },
        ModelCatalogEntry {
            id: "MiniMax-M2.1".into(),
            display_name: "MiniMax M2.1".into(),
            provider: "minimax".into(),
            tier: ModelTier::Smart,
            context_window: 1_048_576,
            max_output_tokens: 16_384,
            input_cost_per_m: 1.00,
            output_cost_per_m: 3.00,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec!["minimax-m2.1".into()],
        },
        ModelCatalogEntry {
            id: "abab6.5-chat".into(),
            display_name: "ABAB 6.5 Chat".into(),
            provider: "minimax".into(),
            tier: ModelTier::Balanced,
            context_window: 245_760,
            max_output_tokens: 8_192,
            input_cost_per_m: 0.50,
            output_cost_per_m: 1.50,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            aliases: vec![],
        },
        ModelCatalogEntry {
            id: "abab7-chat".into(),
            display_name: "ABAB 7 Chat".into(),
            provider: "minimax".into(),
            tier: ModelTier::Smart,
            context_window: 524_288,
            max_output_tokens: 16_384,
            input_cost_per_m: 0.80,
            output_cost_per_m: 2.40,
            supports_tools: true,
            supports_vision: true,
            supports_streaming: true,
            aliases: vec!["abab7".into()],
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
            .unwrap_or_else(|| panic!("missing MiniMax model {id}"))
    }

    #[test]
    fn minimax_models_count_is_stable() {
        let models = minimax_models();

        assert_eq!(models.len(), 6);
        assert!(models.iter().all(|model| model.provider == "minimax"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn minimax_models_keep_primary_aliases() {
        let models = minimax_models();

        assert_eq!(
            model(&models, "minimax-text-01").aliases,
            vec!["minimax".to_string()]
        );
        assert_eq!(
            model(&models, "MiniMax-M2.5").aliases,
            vec!["minimax-m2.5".to_string()]
        );
        assert_eq!(
            model(&models, "MiniMax-M2.5-highspeed").aliases,
            vec![
                "minimax-m2.5-highspeed".to_string(),
                "m2.5-highspeed".to_string()
            ]
        );
        assert_eq!(
            model(&models, "abab7-chat").aliases,
            vec!["abab7".to_string()]
        );
    }

    #[test]
    fn minimax_m2_models_keep_pricing_windows_and_vision() {
        let models = minimax_models();
        let m25 = model(&models, "MiniMax-M2.5");
        let highspeed = model(&models, "MiniMax-M2.5-highspeed");
        let m21 = model(&models, "MiniMax-M2.1");

        assert_eq!(m25.tier, ModelTier::Frontier);
        assert_eq!(m25.context_window, 1_048_576);
        assert_eq!(m25.max_output_tokens, 16_384);
        assert_eq!(m25.input_cost_per_m, 1.10);
        assert_eq!(m25.output_cost_per_m, 4.40);
        assert!(m25.supports_vision);

        assert_eq!(highspeed.tier, ModelTier::Smart);
        assert_eq!(highspeed.context_window, 1_048_576);
        assert_eq!(highspeed.input_cost_per_m, 0.80);
        assert_eq!(highspeed.output_cost_per_m, 3.20);
        assert!(highspeed.supports_vision);

        assert_eq!(m21.tier, ModelTier::Smart);
        assert_eq!(m21.context_window, 1_048_576);
        assert_eq!(m21.input_cost_per_m, 1.00);
        assert_eq!(m21.output_cost_per_m, 3.00);
        assert!(!m21.supports_vision);
    }

    #[test]
    fn minimax_text_and_abab_models_keep_contract() {
        let models = minimax_models();
        let text = model(&models, "minimax-text-01");
        let abab65 = model(&models, "abab6.5-chat");
        let abab7 = model(&models, "abab7-chat");

        assert_eq!(text.tier, ModelTier::Smart);
        assert_eq!(text.context_window, 1_048_576);
        assert_eq!(text.input_cost_per_m, 1.00);
        assert_eq!(text.output_cost_per_m, 3.00);
        assert!(!text.supports_vision);

        assert_eq!(abab65.tier, ModelTier::Balanced);
        assert_eq!(abab65.context_window, 245_760);
        assert_eq!(abab65.max_output_tokens, 8_192);
        assert_eq!(abab65.input_cost_per_m, 0.50);
        assert_eq!(abab65.output_cost_per_m, 1.50);
        assert!(!abab65.supports_vision);

        assert_eq!(abab7.tier, ModelTier::Smart);
        assert_eq!(abab7.context_window, 524_288);
        assert_eq!(abab7.max_output_tokens, 16_384);
        assert_eq!(abab7.input_cost_per_m, 0.80);
        assert_eq!(abab7.output_cost_per_m, 2.40);
        assert!(abab7.supports_vision);
    }
}
