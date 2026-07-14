use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct GroqModelRow {
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

fn groq_model(row: &GroqModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "groq".into(),
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

pub(crate) fn groq_models() -> Vec<ModelCatalogEntry> {
    GROQ_MODEL_ROWS.iter().map(groq_model).collect()
}

const GROQ_MODEL_ROWS: &[GroqModelRow] = &[
    GroqModelRow {
        id: "llama-3.3-70b-versatile",
        display_name: "Llama 3.3 70B",
        tier: ModelTier::Balanced,
        context_window: 128_000,
        max_output_tokens: 32_768,
        input_cost_per_m: 0.059,
        output_cost_per_m: 0.079,
        supports_tools: true,
        supports_vision: false,
        aliases: &["llama", "llama-70b"],
    },
    GroqModelRow {
        id: "llama-3.1-8b-instant",
        display_name: "Llama 3.1 8B",
        tier: ModelTier::Fast,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.05,
        output_cost_per_m: 0.08,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    GroqModelRow {
        id: "llama-3.2-90b-vision-preview",
        display_name: "Llama 3.2 90B Vision",
        tier: ModelTier::Smart,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.90,
        output_cost_per_m: 0.90,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    GroqModelRow {
        id: "llama-3.2-11b-vision-preview",
        display_name: "Llama 3.2 11B Vision",
        tier: ModelTier::Balanced,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.18,
        output_cost_per_m: 0.18,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    GroqModelRow {
        id: "llama-3.2-3b-preview",
        display_name: "Llama 3.2 3B",
        tier: ModelTier::Fast,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.06,
        output_cost_per_m: 0.06,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    GroqModelRow {
        id: "llama-3.2-1b-preview",
        display_name: "Llama 3.2 1B",
        tier: ModelTier::Fast,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.04,
        output_cost_per_m: 0.04,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    GroqModelRow {
        id: "mixtral-8x7b-32768",
        display_name: "Mixtral 8x7B",
        tier: ModelTier::Balanced,
        context_window: 32_768,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.024,
        output_cost_per_m: 0.024,
        supports_tools: true,
        supports_vision: false,
        aliases: &["mixtral"],
    },
    GroqModelRow {
        id: "gemma2-9b-it",
        display_name: "Gemma 2 9B",
        tier: ModelTier::Fast,
        context_window: 8_192,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.02,
        output_cost_per_m: 0.02,
        supports_tools: false,
        supports_vision: false,
        aliases: &[],
    },
    GroqModelRow {
        id: "qwen-qwq-32b",
        display_name: "Qwen QWQ 32B",
        tier: ModelTier::Balanced,
        context_window: 128_000,
        max_output_tokens: 16_384,
        input_cost_per_m: 0.20,
        output_cost_per_m: 0.20,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    GroqModelRow {
        id: "meta-llama/llama-4-scout-17b-16e-instruct",
        display_name: "Llama 4 Scout 17B",
        tier: ModelTier::Balanced,
        context_window: 128_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.11,
        output_cost_per_m: 0.34,
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
            .unwrap_or_else(|| panic!("missing Groq model {id}"))
    }

    #[test]
    fn groq_models_count_is_stable() {
        let models = groq_models();

        assert_eq!(models.len(), 10);
        assert!(models.iter().all(|model| model.provider == "groq"));
    }

    #[test]
    fn groq_models_keep_public_order() {
        let ids: Vec<_> = groq_models().into_iter().map(|model| model.id).collect();

        assert_eq!(
            ids,
            vec![
                "llama-3.3-70b-versatile",
                "llama-3.1-8b-instant",
                "llama-3.2-90b-vision-preview",
                "llama-3.2-11b-vision-preview",
                "llama-3.2-3b-preview",
                "llama-3.2-1b-preview",
                "mixtral-8x7b-32768",
                "gemma2-9b-it",
                "qwen-qwq-32b",
                "meta-llama/llama-4-scout-17b-16e-instruct",
            ]
        );
    }

    #[test]
    fn groq_models_keep_primary_aliases() {
        let models = groq_models();

        assert_eq!(
            model(&models, "llama-3.3-70b-versatile").aliases.as_slice(),
            ["llama", "llama-70b"]
        );
        assert_eq!(
            model(&models, "mixtral-8x7b-32768").aliases.as_slice(),
            ["mixtral"]
        );
    }

    #[test]
    fn groq_pricing_and_capabilities_are_preserved() {
        let models = groq_models();
        let llama70 = model(&models, "llama-3.3-70b-versatile");
        let vision = model(&models, "llama-3.2-90b-vision-preview");
        let scout = model(&models, "meta-llama/llama-4-scout-17b-16e-instruct");

        assert_eq!(llama70.tier, ModelTier::Balanced);
        assert_eq!(llama70.context_window, 128_000);
        assert_eq!(llama70.max_output_tokens, 32_768);
        assert_eq!(llama70.input_cost_per_m, 0.059);
        assert_eq!(llama70.output_cost_per_m, 0.079);
        assert!(llama70.supports_tools);
        assert!(!llama70.supports_vision);
        assert!(llama70.supports_streaming);

        assert_eq!(vision.tier, ModelTier::Smart);
        assert_eq!(vision.max_output_tokens, 8_192);
        assert_eq!(vision.input_cost_per_m, 0.90);
        assert_eq!(vision.output_cost_per_m, 0.90);
        assert!(vision.supports_tools);
        assert!(vision.supports_vision);
        assert!(vision.supports_streaming);

        assert_eq!(scout.tier, ModelTier::Balanced);
        assert_eq!(scout.max_output_tokens, 8_192);
        assert_eq!(scout.input_cost_per_m, 0.11);
        assert_eq!(scout.output_cost_per_m, 0.34);
        assert!(scout.supports_tools);
        assert!(scout.supports_vision);
        assert!(scout.supports_streaming);
    }

    #[test]
    fn groq_small_and_mixed_ids_stay_available() {
        let models = groq_models();

        for id in [
            "llama-3.1-8b-instant",
            "llama-3.2-3b-preview",
            "llama-3.2-1b-preview",
            "gemma2-9b-it",
            "qwen-qwq-32b",
        ] {
            assert!(model(&models, id).aliases.is_empty());
        }

        assert_eq!(
            model(&models, "llama-3.2-11b-vision-preview").tier,
            ModelTier::Balanced
        );
    }
}
