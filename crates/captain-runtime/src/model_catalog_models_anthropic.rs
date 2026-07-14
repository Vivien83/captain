use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct AnthropicModelRow {
    id: &'static str,
    display_name: &'static str,
    tier: ModelTier,
    context_window: u64,
    max_output_tokens: u64,
    input_cost_per_m: f64,
    output_cost_per_m: f64,
    supports_tools: bool,
    supports_vision: bool,
    aliases: &'static [&'static str],
}

fn anthropic_model(row: &AnthropicModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "anthropic".into(),
        tier: row.tier,
        context_window: row.context_window,
        max_output_tokens: row.max_output_tokens,
        input_cost_per_m: row.input_cost_per_m,
        output_cost_per_m: row.output_cost_per_m,
        supports_tools: row.supports_tools,
        supports_vision: row.supports_vision,
        supports_streaming: true,
        aliases: row.aliases.iter().map(|alias| (*alias).into()).collect(),
    }
}

pub(crate) fn anthropic_models() -> Vec<ModelCatalogEntry> {
    ANTHROPIC_MODEL_ROWS.iter().map(anthropic_model).collect()
}

const ANTHROPIC_MODEL_ROWS: &[AnthropicModelRow] = &[
    AnthropicModelRow {
        id: "claude-opus-4-6",
        display_name: "Claude Opus 4.6",
        tier: ModelTier::Frontier,
        context_window: 200_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 5.0,
        output_cost_per_m: 25.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["opus", "claude-opus"],
    },
    AnthropicModelRow {
        id: "claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["sonnet", "claude-sonnet"],
    },
    AnthropicModelRow {
        id: "claude-opus-4-20250514",
        display_name: "Claude Opus 4",
        tier: ModelTier::Frontier,
        context_window: 200_000,
        max_output_tokens: 32_000,
        input_cost_per_m: 15.0,
        output_cost_per_m: 75.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    AnthropicModelRow {
        id: "claude-sonnet-4-20250514",
        display_name: "Claude Sonnet 4",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    AnthropicModelRow {
        id: "claude-haiku-4-5-20251001",
        display_name: "Claude Haiku 4.5",
        tier: ModelTier::Fast,
        context_window: 200_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.25,
        output_cost_per_m: 1.25,
        supports_tools: true,
        supports_vision: true,
        aliases: &["haiku", "claude-haiku"],
    },
    AnthropicModelRow {
        id: "claude-sonnet-4-5-20250514",
        display_name: "Claude Sonnet 4.5",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    AnthropicModelRow {
        id: "claude-3-5-sonnet-20241022",
        display_name: "Claude 3.5 Sonnet",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(models: &'a [ModelCatalogEntry], id: &str) -> &'a ModelCatalogEntry {
        models
            .iter()
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("missing Anthropic model {id}"))
    }

    #[test]
    fn anthropic_models_count_is_stable() {
        let models = anthropic_models();

        assert_eq!(models.len(), 7);
        assert!(models.iter().all(|model| model.provider == "anthropic"));
    }

    #[test]
    fn anthropic_models_keep_public_order() {
        let ids: Vec<_> = anthropic_models()
            .into_iter()
            .map(|model| model.id)
            .collect();

        assert_eq!(
            ids,
            vec![
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                "claude-opus-4-20250514",
                "claude-sonnet-4-20250514",
                "claude-haiku-4-5-20251001",
                "claude-sonnet-4-5-20250514",
                "claude-3-5-sonnet-20241022",
            ]
        );
    }

    #[test]
    fn anthropic_models_keep_primary_aliases() {
        let models = anthropic_models();

        assert_eq!(
            model(&models, "claude-opus-4-6").aliases.as_slice(),
            ["opus", "claude-opus"]
        );
        assert_eq!(
            model(&models, "claude-sonnet-4-6").aliases.as_slice(),
            ["sonnet", "claude-sonnet"]
        );
        assert_eq!(
            model(&models, "claude-haiku-4-5-20251001")
                .aliases
                .as_slice(),
            ["haiku", "claude-haiku"]
        );
    }

    #[test]
    fn anthropic_pricing_and_capabilities_are_preserved() {
        let models = anthropic_models();
        let opus = model(&models, "claude-opus-4-6");
        let sonnet = model(&models, "claude-sonnet-4-6");
        let haiku = model(&models, "claude-haiku-4-5-20251001");

        assert_eq!(opus.tier, ModelTier::Frontier);
        assert_eq!(opus.context_window, 200_000);
        assert_eq!(opus.max_output_tokens, 128_000);
        assert_eq!(opus.input_cost_per_m, 5.0);
        assert_eq!(opus.output_cost_per_m, 25.0);

        assert_eq!(sonnet.tier, ModelTier::Smart);
        assert_eq!(sonnet.max_output_tokens, 64_000);
        assert_eq!(sonnet.input_cost_per_m, 3.0);
        assert_eq!(sonnet.output_cost_per_m, 15.0);

        assert_eq!(haiku.tier, ModelTier::Fast);
        assert_eq!(haiku.max_output_tokens, 8_192);
        assert_eq!(haiku.input_cost_per_m, 0.25);
        assert_eq!(haiku.output_cost_per_m, 1.25);

        for model in [opus, sonnet, haiku] {
            assert!(model.supports_tools);
            assert!(model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn anthropic_legacy_ids_stay_available() {
        let models = anthropic_models();

        for id in [
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-sonnet-4-5-20250514",
            "claude-3-5-sonnet-20241022",
        ] {
            assert!(model(&models, id).aliases.is_empty());
        }
    }
}
