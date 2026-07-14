use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct TogetherModelRow {
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

fn together_model(row: &TogetherModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "together".into(),
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

pub(crate) fn together_models() -> Vec<ModelCatalogEntry> {
    TOGETHER_MODEL_ROWS.iter().map(together_model).collect()
}

const TOGETHER_MODEL_ROWS: &[TogetherModelRow] = &[
    TogetherModelRow {
        id: "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo",
        display_name: "Llama 3.1 405B (Together)",
        tier: ModelTier::Frontier,
        context_window: 130_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 3.50,
        output_cost_per_m: 3.50,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        display_name: "Llama 3.3 70B (Together)",
        tier: ModelTier::Smart,
        context_window: 128_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.88,
        output_cost_per_m: 0.88,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8",
        display_name: "Llama 4 Maverick (Together)",
        tier: ModelTier::Smart,
        context_window: 1_048_576,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.27,
        output_cost_per_m: 0.35,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "meta-llama/Llama-4-Scout-17B-16E-Instruct",
        display_name: "Llama 4 Scout (Together)",
        tier: ModelTier::Balanced,
        context_window: 512_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.18,
        output_cost_per_m: 0.30,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "deepseek-ai/DeepSeek-R1",
        display_name: "DeepSeek R1 (Together)",
        tier: ModelTier::Frontier,
        context_window: 64_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 3.00,
        output_cost_per_m: 7.00,
        supports_tools: false,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "deepseek-ai/DeepSeek-V3",
        display_name: "DeepSeek V3 (Together)",
        tier: ModelTier::Smart,
        context_window: 64_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.90,
        output_cost_per_m: 0.90,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "Qwen/Qwen2.5-72B-Instruct-Turbo",
        display_name: "Qwen 2.5 72B (Together)",
        tier: ModelTier::Smart,
        context_window: 32_768,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.20,
        output_cost_per_m: 0.60,
        supports_tools: true,
        supports_vision: false,
        aliases: &[],
    },
    TogetherModelRow {
        id: "mistralai/Mixtral-8x22B-Instruct-v0.1",
        display_name: "Mixtral 8x22B (Together)",
        tier: ModelTier::Balanced,
        context_window: 65_536,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.60,
        output_cost_per_m: 0.60,
        supports_tools: true,
        supports_vision: false,
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
            .unwrap_or_else(|| panic!("missing Together model {id}"))
    }

    #[test]
    fn together_models_count_is_stable() {
        let models = together_models();

        assert_eq!(models.len(), 8);
        assert!(models.iter().all(|model| model.provider == "together"));
        assert!(models.iter().all(|model| model.aliases.is_empty()));
    }

    #[test]
    fn together_models_keep_public_order() {
        let ids: Vec<_> = together_models()
            .into_iter()
            .map(|model| model.id)
            .collect();

        assert_eq!(
            ids,
            vec![
                "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo",
                "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8",
                "meta-llama/Llama-4-Scout-17B-16E-Instruct",
                "deepseek-ai/DeepSeek-R1",
                "deepseek-ai/DeepSeek-V3",
                "Qwen/Qwen2.5-72B-Instruct-Turbo",
                "mistralai/Mixtral-8x22B-Instruct-v0.1",
            ]
        );
    }

    #[test]
    fn together_llama_pricing_and_capabilities_are_preserved() {
        let models = together_models();
        let llama405 = model(&models, "meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo");
        let llama70 = model(&models, "meta-llama/Llama-3.3-70B-Instruct-Turbo");
        let scout = model(&models, "meta-llama/Llama-4-Scout-17B-16E-Instruct");

        assert_eq!(llama405.tier, ModelTier::Frontier);
        assert_eq!(llama405.context_window, 130_000);
        assert_eq!(llama405.max_output_tokens, 4_096);
        assert_eq!(llama405.input_cost_per_m, 3.50);
        assert_eq!(llama405.output_cost_per_m, 3.50);

        assert_eq!(llama70.tier, ModelTier::Smart);
        assert_eq!(llama70.context_window, 128_000);
        assert_eq!(llama70.input_cost_per_m, 0.88);
        assert_eq!(llama70.output_cost_per_m, 0.88);

        assert_eq!(scout.tier, ModelTier::Balanced);
        assert_eq!(scout.context_window, 512_000);
        assert_eq!(scout.input_cost_per_m, 0.18);
        assert_eq!(scout.output_cost_per_m, 0.30);

        for model in [llama405, llama70, scout] {
            assert!(model.supports_tools);
            assert!(!model.supports_vision);
            assert!(model.supports_streaming);
        }
    }

    #[test]
    fn together_reasoning_and_qwen_ids_stay_available() {
        let models = together_models();
        let deepseek_r1 = model(&models, "deepseek-ai/DeepSeek-R1");
        let deepseek_v3 = model(&models, "deepseek-ai/DeepSeek-V3");
        let qwen = model(&models, "Qwen/Qwen2.5-72B-Instruct-Turbo");

        assert_eq!(deepseek_r1.tier, ModelTier::Frontier);
        assert!(!deepseek_r1.supports_tools);
        assert_eq!(deepseek_r1.input_cost_per_m, 3.00);
        assert_eq!(deepseek_r1.output_cost_per_m, 7.00);

        assert_eq!(deepseek_v3.tier, ModelTier::Smart);
        assert!(deepseek_v3.supports_tools);
        assert_eq!(deepseek_v3.input_cost_per_m, 0.90);
        assert_eq!(deepseek_v3.output_cost_per_m, 0.90);

        assert_eq!(qwen.context_window, 32_768);
        assert_eq!(qwen.input_cost_per_m, 0.20);
        assert_eq!(qwen.output_cost_per_m, 0.60);
    }

    #[test]
    fn together_maverick_and_mixtral_ids_stay_available() {
        let models = together_models();
        let maverick = model(&models, "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8");
        let mixtral = model(&models, "mistralai/Mixtral-8x22B-Instruct-v0.1");

        assert_eq!(maverick.tier, ModelTier::Smart);
        assert_eq!(maverick.context_window, 1_048_576);
        assert_eq!(maverick.max_output_tokens, 8_192);
        assert_eq!(maverick.input_cost_per_m, 0.27);
        assert_eq!(maverick.output_cost_per_m, 0.35);

        assert_eq!(mixtral.tier, ModelTier::Balanced);
        assert_eq!(mixtral.context_window, 65_536);
        assert_eq!(mixtral.max_output_tokens, 4_096);
        assert_eq!(mixtral.input_cost_per_m, 0.60);
        assert_eq!(mixtral.output_cost_per_m, 0.60);
    }
}
