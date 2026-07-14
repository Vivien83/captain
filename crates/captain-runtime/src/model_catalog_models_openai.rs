use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct OpenAiModelRow {
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

fn openai_model(row: &OpenAiModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "openai".into(),
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

pub(crate) fn openai_models() -> Vec<ModelCatalogEntry> {
    OPENAI_MODEL_ROWS.iter().map(openai_model).collect()
}

const OPENAI_MODEL_ROWS: &[OpenAiModelRow] = &[
    OpenAiModelRow {
        id: "gpt-4o",
        display_name: "GPT-4o",
        tier: ModelTier::Smart,
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_per_m: 2.50,
        output_cost_per_m: 10.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["gpt4", "gpt4o"],
    },
    OpenAiModelRow {
        id: "gpt-4o-mini",
        display_name: "GPT-4o Mini",
        tier: ModelTier::Fast,
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_per_m: 0.15,
        output_cost_per_m: 0.60,
        supports_tools: true,
        supports_vision: true,
        aliases: &["gpt4-mini"],
    },
    OpenAiModelRow {
        id: "gpt-4.1",
        display_name: "GPT-4.1",
        tier: ModelTier::Frontier,
        context_window: 1_047_576,
        max_output_tokens: 32_768,
        input_cost_per_m: 2.00,
        output_cost_per_m: 8.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-4.1-mini",
        display_name: "GPT-4.1 Mini",
        tier: ModelTier::Balanced,
        context_window: 1_047_576,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.40,
        output_cost_per_m: 1.60,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-4.1-nano",
        display_name: "GPT-4.1 Nano",
        tier: ModelTier::Fast,
        context_window: 1_047_576,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.10,
        output_cost_per_m: 0.40,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "o3",
        display_name: "o3",
        tier: ModelTier::Frontier,
        context_window: 200_000,
        max_output_tokens: 100_000,
        input_cost_per_m: 2.00,
        output_cost_per_m: 8.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "o3-mini",
        display_name: "o3-mini",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 100_000,
        input_cost_per_m: 1.10,
        output_cost_per_m: 4.40,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "o4-mini",
        display_name: "o4-mini",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 100_000,
        input_cost_per_m: 1.10,
        output_cost_per_m: 4.40,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-4-turbo",
        display_name: "GPT-4 Turbo",
        tier: ModelTier::Smart,
        context_window: 128_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 10.00,
        output_cost_per_m: 30.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-3.5-turbo",
        display_name: "GPT-3.5 Turbo",
        tier: ModelTier::Fast,
        context_window: 16_385,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.50,
        output_cost_per_m: 1.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-5",
        display_name: "GPT-5",
        tier: ModelTier::Frontier,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 1.25,
        output_cost_per_m: 10.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-5-mini",
        display_name: "GPT-5 Mini",
        tier: ModelTier::Balanced,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 0.25,
        output_cost_per_m: 2.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["gpt5-mini"],
    },
    OpenAiModelRow {
        id: "gpt-5-nano",
        display_name: "GPT-5 Nano",
        tier: ModelTier::Fast,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 0.05,
        output_cost_per_m: 0.40,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-5.1",
        display_name: "GPT-5.1",
        tier: ModelTier::Frontier,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 1.25,
        output_cost_per_m: 10.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    OpenAiModelRow {
        id: "gpt-5.2",
        display_name: "GPT-5.2",
        tier: ModelTier::Frontier,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 1.75,
        output_cost_per_m: 14.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["gpt5"],
    },
    OpenAiModelRow {
        id: "gpt-5.2-pro",
        display_name: "GPT-5.2 Pro",
        tier: ModelTier::Frontier,
        context_window: 400_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 1.75,
        output_cost_per_m: 14.0,
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
            .unwrap_or_else(|| panic!("missing OpenAI model {id}"))
    }

    #[test]
    fn openai_models_count_is_stable() {
        let models = openai_models();

        assert_eq!(models.len(), 16);
        assert!(models.iter().all(|model| model.provider == "openai"));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn openai_models_order_is_stable() {
        let models = openai_models();
        let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(
            ids,
            vec![
                "gpt-4o",
                "gpt-4o-mini",
                "gpt-4.1",
                "gpt-4.1-mini",
                "gpt-4.1-nano",
                "o3",
                "o3-mini",
                "o4-mini",
                "gpt-4-turbo",
                "gpt-3.5-turbo",
                "gpt-5",
                "gpt-5-mini",
                "gpt-5-nano",
                "gpt-5.1",
                "gpt-5.2",
                "gpt-5.2-pro",
            ]
        );
    }

    #[test]
    fn openai_models_keep_primary_aliases() {
        let models = openai_models();

        assert_eq!(
            model(&models, "gpt-4o").aliases.as_slice(),
            ["gpt4", "gpt4o"]
        );
        assert_eq!(
            model(&models, "gpt-4o-mini").aliases.as_slice(),
            ["gpt4-mini"]
        );
        assert_eq!(
            model(&models, "gpt-5-mini").aliases.as_slice(),
            ["gpt5-mini"]
        );
        assert_eq!(model(&models, "gpt-5.2").aliases.as_slice(), ["gpt5"]);
    }

    #[test]
    fn openai_pricing_and_capabilities_are_preserved() {
        let models = openai_models();
        let gpt52 = model(&models, "gpt-5.2");
        let gpt41 = model(&models, "gpt-4.1");
        let o3 = model(&models, "o3");

        assert_eq!(gpt52.tier, ModelTier::Frontier);
        assert_eq!(gpt52.context_window, 400_000);
        assert_eq!(gpt52.max_output_tokens, 128_000);
        assert_eq!(gpt52.input_cost_per_m, 1.75);
        assert_eq!(gpt52.output_cost_per_m, 14.0);

        assert_eq!(gpt41.tier, ModelTier::Frontier);
        assert_eq!(gpt41.context_window, 1_047_576);
        assert_eq!(gpt41.max_output_tokens, 32_768);
        assert_eq!(gpt41.input_cost_per_m, 2.00);
        assert_eq!(gpt41.output_cost_per_m, 8.00);

        assert_eq!(o3.tier, ModelTier::Frontier);
        assert_eq!(o3.context_window, 200_000);
        assert_eq!(o3.max_output_tokens, 100_000);
        assert_eq!(o3.input_cost_per_m, 2.00);
        assert_eq!(o3.output_cost_per_m, 8.00);

        for model in [gpt52, gpt41, o3] {
            assert!(model.supports_tools);
            assert!(model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn openai_legacy_and_reasoning_ids_stay_available() {
        let models = openai_models();

        for id in ["gpt-4-turbo", "gpt-3.5-turbo", "o3-mini", "o4-mini"] {
            assert!(model(&models, id).aliases.is_empty());
        }
        assert!(!model(&models, "o3-mini").supports_vision);
        assert!(model(&models, "o4-mini").supports_vision);
    }
}
