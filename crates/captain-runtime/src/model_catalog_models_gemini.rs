use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct GeminiModelRow {
    id: &'static str,
    display_name: &'static str,
    tier: ModelTier,
    context_window: u64,
    max_output_tokens: u64,
    input_cost_per_m: f64,
    output_cost_per_m: f64,
    aliases: &'static [&'static str],
}

fn gemini_model(row: &GeminiModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "gemini".into(),
        tier: row.tier,
        context_window: row.context_window,
        max_output_tokens: row.max_output_tokens,
        input_cost_per_m: row.input_cost_per_m,
        output_cost_per_m: row.output_cost_per_m,
        supports_tools: true,
        supports_vision: true,
        supports_streaming: true,
        aliases: row.aliases.iter().map(|alias| (*alias).into()).collect(),
    }
}

pub(crate) fn gemini_models() -> Vec<ModelCatalogEntry> {
    GEMINI_MODEL_ROWS.iter().map(gemini_model).collect()
}

const GEMINI_MODEL_ROWS: &[GeminiModelRow] = &[
    GeminiModelRow {
        id: "gemini-3.1-pro-preview",
        display_name: "Gemini 3.1 Pro Preview",
        tier: ModelTier::Frontier,
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_per_m: 2.50,
        output_cost_per_m: 15.0,
        aliases: &["gemini-pro"],
    },
    GeminiModelRow {
        id: "gemini-3-flash-preview",
        display_name: "Gemini 3 Flash Preview",
        tier: ModelTier::Smart,
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_per_m: 0.15,
        output_cost_per_m: 0.60,
        aliases: &["gemini-flash"],
    },
    GeminiModelRow {
        id: "gemini-3.1-flash-lite-preview",
        display_name: "Gemini 3.1 Flash Lite Preview",
        tier: ModelTier::Fast,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.04,
        output_cost_per_m: 0.15,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-2.5-flash-lite",
        display_name: "Gemini 2.5 Flash Lite",
        tier: ModelTier::Fast,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.04,
        output_cost_per_m: 0.15,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-2.5-pro",
        display_name: "Gemini 2.5 Pro",
        tier: ModelTier::Frontier,
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_per_m: 1.25,
        output_cost_per_m: 10.0,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-2.5-flash",
        display_name: "Gemini 2.5 Flash",
        tier: ModelTier::Smart,
        context_window: 1_048_576,
        max_output_tokens: 65_536,
        input_cost_per_m: 0.15,
        output_cost_per_m: 0.60,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-2.0-flash",
        display_name: "Gemini 2.0 Flash",
        tier: ModelTier::Fast,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.10,
        output_cost_per_m: 0.40,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-2.0-flash-lite",
        display_name: "Gemini 2.0 Flash Lite",
        tier: ModelTier::Fast,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.075,
        output_cost_per_m: 0.30,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-1.5-pro",
        display_name: "Gemini 1.5 Pro",
        tier: ModelTier::Smart,
        context_window: 2_097_152,
        max_output_tokens: 8_192,
        input_cost_per_m: 1.25,
        output_cost_per_m: 5.00,
        aliases: &[],
    },
    GeminiModelRow {
        id: "gemini-1.5-flash",
        display_name: "Gemini 1.5 Flash",
        tier: ModelTier::Fast,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.075,
        output_cost_per_m: 0.30,
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
            .unwrap_or_else(|| panic!("missing Gemini model {id}"))
    }

    #[test]
    fn gemini_models_count_is_stable() {
        let models = gemini_models();

        assert_eq!(models.len(), 10);
        assert!(models.iter().all(|model| model.provider == "gemini"));
    }

    #[test]
    fn gemini_models_keep_public_order() {
        let ids: Vec<_> = gemini_models().into_iter().map(|model| model.id).collect();

        assert_eq!(
            ids,
            vec![
                "gemini-3.1-pro-preview",
                "gemini-3-flash-preview",
                "gemini-3.1-flash-lite-preview",
                "gemini-2.5-flash-lite",
                "gemini-2.5-pro",
                "gemini-2.5-flash",
                "gemini-2.0-flash",
                "gemini-2.0-flash-lite",
                "gemini-1.5-pro",
                "gemini-1.5-flash",
            ]
        );
    }

    #[test]
    fn gemini_models_keep_primary_aliases() {
        let models = gemini_models();

        assert_eq!(
            model(&models, "gemini-3.1-pro-preview").aliases.as_slice(),
            ["gemini-pro"]
        );
        assert_eq!(
            model(&models, "gemini-3-flash-preview").aliases.as_slice(),
            ["gemini-flash"]
        );
    }

    #[test]
    fn gemini_pricing_and_capabilities_are_preserved() {
        let models = gemini_models();
        let pro31 = model(&models, "gemini-3.1-pro-preview");
        let pro25 = model(&models, "gemini-2.5-pro");
        let pro15 = model(&models, "gemini-1.5-pro");

        assert_eq!(pro31.tier, ModelTier::Frontier);
        assert_eq!(pro31.context_window, 1_048_576);
        assert_eq!(pro31.max_output_tokens, 65_536);
        assert_eq!(pro31.input_cost_per_m, 2.50);
        assert_eq!(pro31.output_cost_per_m, 15.0);

        assert_eq!(pro25.tier, ModelTier::Frontier);
        assert_eq!(pro25.context_window, 1_048_576);
        assert_eq!(pro25.max_output_tokens, 65_536);
        assert_eq!(pro25.input_cost_per_m, 1.25);
        assert_eq!(pro25.output_cost_per_m, 10.0);

        assert_eq!(pro15.tier, ModelTier::Smart);
        assert_eq!(pro15.context_window, 2_097_152);
        assert_eq!(pro15.max_output_tokens, 8_192);
        assert_eq!(pro15.input_cost_per_m, 1.25);
        assert_eq!(pro15.output_cost_per_m, 5.00);

        for model in [pro31, pro25, pro15] {
            assert!(model.supports_tools);
            assert!(model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn gemini_legacy_flash_ids_stay_available() {
        let models = gemini_models();

        for id in [
            "gemini-3.1-flash-lite-preview",
            "gemini-2.5-flash-lite",
            "gemini-2.5-flash",
            "gemini-2.0-flash",
            "gemini-2.0-flash-lite",
            "gemini-1.5-flash",
        ] {
            assert!(model(&models, id).aliases.is_empty());
            assert!(model(&models, id).supports_vision);
        }
    }
}
