use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct XaiModelRow {
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

fn xai_model(row: &XaiModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "xai".into(),
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

pub(crate) fn xai_models() -> Vec<ModelCatalogEntry> {
    XAI_MODEL_ROWS.iter().map(xai_model).collect()
}

const XAI_MODEL_ROWS: &[XaiModelRow] = &[
    XaiModelRow {
        id: "grok-4-0709",
        display_name: "Grok 4",
        tier: ModelTier::Frontier,
        context_window: 256_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["grok", "grok-4"],
    },
    XaiModelRow {
        id: "grok-4-fast-reasoning",
        display_name: "Grok 4 Fast Reasoning",
        tier: ModelTier::Smart,
        context_window: 256_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 1.0,
        output_cost_per_m: 5.0,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    XaiModelRow {
        id: "grok-4-fast-non-reasoning",
        display_name: "Grok 4 Fast Non-Reasoning",
        tier: ModelTier::Smart,
        context_window: 256_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 1.0,
        output_cost_per_m: 5.0,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    XaiModelRow {
        id: "grok-4-1-fast-reasoning",
        display_name: "Grok 4.1 Fast Reasoning",
        tier: ModelTier::Fast,
        context_window: 2_000_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.20,
        output_cost_per_m: 0.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &["grok-fast"],
    },
    XaiModelRow {
        id: "grok-4-1-fast-non-reasoning",
        display_name: "Grok 4.1 Fast Non-Reasoning",
        tier: ModelTier::Fast,
        context_window: 2_000_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.20,
        output_cost_per_m: 0.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    XaiModelRow {
        id: "grok-3",
        display_name: "Grok 3",
        tier: ModelTier::Frontier,
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_per_m: 3.0,
        output_cost_per_m: 15.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &["grok3"],
    },
    XaiModelRow {
        id: "grok-3-mini",
        display_name: "Grok 3 Mini",
        tier: ModelTier::Balanced,
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.30,
        output_cost_per_m: 0.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    XaiModelRow {
        id: "grok-2",
        display_name: "Grok 2",
        tier: ModelTier::Smart,
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_per_m: 2.0,
        output_cost_per_m: 10.0,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    XaiModelRow {
        id: "grok-2-mini",
        display_name: "Grok 2 Mini",
        tier: ModelTier::Fast,
        context_window: 131_072,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.30,
        output_cost_per_m: 0.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &["grok-mini"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(models: &'a [ModelCatalogEntry], id: &str) -> &'a ModelCatalogEntry {
        models
            .iter()
            .find(|model| model.id == id)
            .unwrap_or_else(|| panic!("missing xAI model {id}"))
    }

    #[test]
    fn xai_models_count_is_stable() {
        let models = xai_models();

        assert_eq!(models.len(), 9);
        assert!(models.iter().all(|model| model.provider == "xai"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn xai_models_keep_public_order() {
        let ids: Vec<_> = xai_models().into_iter().map(|model| model.id).collect();

        assert_eq!(
            ids,
            vec![
                "grok-4-0709",
                "grok-4-fast-reasoning",
                "grok-4-fast-non-reasoning",
                "grok-4-1-fast-reasoning",
                "grok-4-1-fast-non-reasoning",
                "grok-3",
                "grok-3-mini",
                "grok-2",
                "grok-2-mini",
            ]
        );
    }

    #[test]
    fn xai_models_keep_primary_aliases() {
        let models = xai_models();

        assert_eq!(
            model(&models, "grok-4-0709").aliases,
            vec!["grok".to_string(), "grok-4".to_string()]
        );
        assert_eq!(
            model(&models, "grok-4-1-fast-reasoning").aliases,
            vec!["grok-fast".to_string()]
        );
        assert_eq!(
            model(&models, "grok-2-mini").aliases,
            vec!["grok-mini".to_string()]
        );
    }

    #[test]
    fn xai_frontier_models_keep_pricing_and_vision() {
        let models = xai_models();
        let grok_4 = model(&models, "grok-4-0709");
        let grok_3 = model(&models, "grok-3");

        assert_eq!(grok_4.tier, ModelTier::Frontier);
        assert_eq!(grok_4.context_window, 256_000);
        assert_eq!(grok_4.max_output_tokens, 32_768);
        assert_eq!(grok_4.input_cost_per_m, 3.0);
        assert_eq!(grok_4.output_cost_per_m, 15.0);
        assert!(grok_4.supports_vision);

        assert_eq!(grok_3.tier, ModelTier::Frontier);
        assert_eq!(grok_3.context_window, 131_072);
        assert_eq!(grok_3.input_cost_per_m, 3.0);
        assert_eq!(grok_3.output_cost_per_m, 15.0);
        assert!(grok_3.supports_vision);
    }

    #[test]
    fn xai_fast_and_legacy_ids_stay_available() {
        let models = xai_models();
        let fast_reasoning = model(&models, "grok-4-1-fast-reasoning");
        let fast_non_reasoning = model(&models, "grok-4-1-fast-non-reasoning");
        let legacy = model(&models, "grok-2-mini");

        assert_eq!(fast_reasoning.tier, ModelTier::Fast);
        assert_eq!(fast_reasoning.context_window, 2_000_000);
        assert_eq!(fast_reasoning.input_cost_per_m, 0.20);
        assert_eq!(fast_reasoning.output_cost_per_m, 0.50);
        assert!(!fast_reasoning.supports_vision);

        assert_eq!(fast_non_reasoning.tier, ModelTier::Fast);
        assert_eq!(fast_non_reasoning.context_window, 2_000_000);
        assert_eq!(fast_non_reasoning.input_cost_per_m, 0.20);
        assert_eq!(fast_non_reasoning.output_cost_per_m, 0.50);
        assert!(!fast_non_reasoning.supports_vision);

        assert_eq!(legacy.display_name, "Grok 2 Mini");
        assert_eq!(legacy.tier, ModelTier::Fast);
        assert_eq!(legacy.context_window, 131_072);
        assert_eq!(legacy.input_cost_per_m, 0.30);
        assert_eq!(legacy.output_cost_per_m, 0.50);
    }
}
