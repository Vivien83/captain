use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct QwenModelRow {
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

fn qwen_model(row: &QwenModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "qwen".into(),
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

pub(crate) fn qwen_models() -> Vec<ModelCatalogEntry> {
    QWEN_MODEL_ROWS.iter().map(qwen_model).collect()
}

const QWEN_MODEL_ROWS: &[QwenModelRow] = &[
    QwenModelRow {
        id: "qwen-max",
        display_name: "Qwen Max",
        tier: ModelTier::Frontier,
        context_window: 32_768,
        max_output_tokens: 8_192,
        input_cost_per_m: 4.00,
        output_cost_per_m: 12.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-plus",
        display_name: "Qwen Plus",
        tier: ModelTier::Smart,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.80,
        output_cost_per_m: 2.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &["qwen"],
    },
    QwenModelRow {
        id: "qwen-turbo",
        display_name: "Qwen Turbo",
        tier: ModelTier::Fast,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.30,
        output_cost_per_m: 0.60,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-vl-plus",
        display_name: "Qwen VL Plus",
        tier: ModelTier::Smart,
        context_window: 32_768,
        max_output_tokens: 8_192,
        input_cost_per_m: 1.50,
        output_cost_per_m: 4.50,
        supports_tools: false,
        supports_vision: true,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-coder-plus",
        display_name: "Qwen Coder Plus",
        tier: ModelTier::Smart,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.80,
        output_cost_per_m: 2.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-long",
        display_name: "Qwen Long",
        tier: ModelTier::Balanced,
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.50,
        output_cost_per_m: 2.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen3-235b-a22b",
        display_name: "Qwen3 235B",
        tier: ModelTier::Frontier,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 4.00,
        output_cost_per_m: 12.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &["qwen3"],
    },
    QwenModelRow {
        id: "qwen3-30b-a3b",
        display_name: "Qwen3 30B",
        tier: ModelTier::Fast,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.30,
        output_cost_per_m: 0.60,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-coder-plus-latest",
        display_name: "Qwen Coder Plus (Latest)",
        tier: ModelTier::Smart,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.80,
        output_cost_per_m: 2.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &["qwen-coder"],
    },
    QwenModelRow {
        id: "qwen2.5-coder-32b-instruct",
        display_name: "Qwen 2.5 Coder 32B",
        tier: ModelTier::Balanced,
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.80,
        output_cost_per_m: 2.00,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    QwenModelRow {
        id: "qwen-vl-max",
        display_name: "Qwen VL Max",
        tier: ModelTier::Frontier,
        context_window: 32_768,
        max_output_tokens: 8_192,
        input_cost_per_m: 3.00,
        output_cost_per_m: 9.00,
        supports_tools: false,
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
            .unwrap_or_else(|| panic!("missing Qwen model {id}"))
    }

    #[test]
    fn qwen_models_count_is_stable() {
        let models = qwen_models();

        assert_eq!(models.len(), 11);
        assert!(models.iter().all(|model| model.provider == "qwen"));
        assert!(models.iter().all(|model| model.supports_streaming));
    }

    #[test]
    fn qwen_models_keep_public_order() {
        let ids: Vec<_> = qwen_models().into_iter().map(|model| model.id).collect();

        assert_eq!(
            ids,
            vec![
                "qwen-max",
                "qwen-plus",
                "qwen-turbo",
                "qwen-vl-plus",
                "qwen-coder-plus",
                "qwen-long",
                "qwen3-235b-a22b",
                "qwen3-30b-a3b",
                "qwen-coder-plus-latest",
                "qwen2.5-coder-32b-instruct",
                "qwen-vl-max",
            ]
        );
    }

    #[test]
    fn qwen_models_keep_primary_aliases() {
        let models = qwen_models();

        assert_eq!(
            model(&models, "qwen-plus").aliases,
            vec!["qwen".to_string()]
        );
        assert_eq!(
            model(&models, "qwen3-235b-a22b").aliases,
            vec!["qwen3".to_string()]
        );
        assert_eq!(
            model(&models, "qwen-coder-plus-latest").aliases,
            vec!["qwen-coder".to_string()]
        );
    }

    #[test]
    fn qwen_vision_models_keep_capability_contract() {
        let models = qwen_models();
        let vl_plus = model(&models, "qwen-vl-plus");
        let vl_max = model(&models, "qwen-vl-max");

        assert!(!vl_plus.supports_tools);
        assert!(vl_plus.supports_vision);
        assert_eq!(vl_plus.context_window, 32_768);
        assert_eq!(vl_plus.input_cost_per_m, 1.50);
        assert_eq!(vl_plus.output_cost_per_m, 4.50);

        assert!(!vl_max.supports_tools);
        assert!(vl_max.supports_vision);
        assert_eq!(vl_max.tier, ModelTier::Frontier);
        assert_eq!(vl_max.context_window, 32_768);
        assert_eq!(vl_max.input_cost_per_m, 3.00);
        assert_eq!(vl_max.output_cost_per_m, 9.00);
    }

    #[test]
    fn qwen_core_and_coder_models_keep_pricing_and_windows() {
        let models = qwen_models();
        let max = model(&models, "qwen-max");
        let plus = model(&models, "qwen-plus");
        let long = model(&models, "qwen-long");
        let coder = model(&models, "qwen-coder-plus-latest");

        assert_eq!(max.tier, ModelTier::Frontier);
        assert_eq!(max.context_window, 32_768);
        assert_eq!(max.input_cost_per_m, 4.00);
        assert_eq!(max.output_cost_per_m, 12.00);

        assert_eq!(plus.tier, ModelTier::Smart);
        assert_eq!(plus.context_window, 131_072);
        assert_eq!(plus.input_cost_per_m, 0.80);
        assert_eq!(plus.output_cost_per_m, 2.00);

        assert_eq!(long.tier, ModelTier::Balanced);
        assert_eq!(long.context_window, 1_000_000);
        assert_eq!(long.input_cost_per_m, 0.50);
        assert_eq!(long.output_cost_per_m, 2.00);

        assert_eq!(coder.display_name, "Qwen Coder Plus (Latest)");
        assert_eq!(coder.context_window, 131_072);
        assert_eq!(coder.input_cost_per_m, 0.80);
        assert_eq!(coder.output_cost_per_m, 2.00);
    }
}
