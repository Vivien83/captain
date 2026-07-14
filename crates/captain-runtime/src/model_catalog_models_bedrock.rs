use captain_types::model_catalog::{ModelCatalogEntry, ModelTier};

struct BedrockModelRow {
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

fn bedrock_model(row: &BedrockModelRow) -> ModelCatalogEntry {
    ModelCatalogEntry {
        id: row.id.into(),
        display_name: row.display_name.into(),
        provider: "bedrock".into(),
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

pub(crate) fn bedrock_models() -> Vec<ModelCatalogEntry> {
    BEDROCK_MODEL_ROWS.iter().map(bedrock_model).collect()
}

const BEDROCK_MODEL_ROWS: &[BedrockModelRow] = &[
    BedrockModelRow {
        id: "bedrock/anthropic.claude-opus-4-6",
        display_name: "Claude Opus 4.6 (Bedrock)",
        tier: ModelTier::Frontier,
        context_window: 200_000,
        max_output_tokens: 128_000,
        input_cost_per_m: 5.00,
        output_cost_per_m: 25.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/anthropic.claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6 (Bedrock)",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 3.00,
        output_cost_per_m: 15.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/anthropic.claude-opus-4-20250514",
        display_name: "Claude Opus 4 (Bedrock)",
        tier: ModelTier::Frontier,
        context_window: 200_000,
        max_output_tokens: 32_000,
        input_cost_per_m: 15.00,
        output_cost_per_m: 75.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/anthropic.claude-sonnet-4-20250514",
        display_name: "Claude Sonnet 4 (Bedrock)",
        tier: ModelTier::Smart,
        context_window: 200_000,
        max_output_tokens: 64_000,
        input_cost_per_m: 3.00,
        output_cost_per_m: 15.00,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/anthropic.claude-haiku-4-5-20251001",
        display_name: "Claude Haiku 4.5 (Bedrock)",
        tier: ModelTier::Fast,
        context_window: 200_000,
        max_output_tokens: 8_192,
        input_cost_per_m: 0.25,
        output_cost_per_m: 1.25,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/amazon.nova-pro-v1:0",
        display_name: "Amazon Nova Pro (Bedrock)",
        tier: ModelTier::Smart,
        context_window: 300_000,
        max_output_tokens: 5_120,
        input_cost_per_m: 0.80,
        output_cost_per_m: 3.20,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/amazon.nova-lite-v1:0",
        display_name: "Amazon Nova Lite (Bedrock)",
        tier: ModelTier::Fast,
        context_window: 300_000,
        max_output_tokens: 5_120,
        input_cost_per_m: 0.06,
        output_cost_per_m: 0.24,
        supports_tools: true,
        supports_vision: true,
        aliases: &[],
    },
    BedrockModelRow {
        id: "bedrock/meta.llama3-3-70b-instruct-v1:0",
        display_name: "Llama 3.3 70B (Bedrock)",
        tier: ModelTier::Balanced,
        context_window: 128_000,
        max_output_tokens: 4_096,
        input_cost_per_m: 0.72,
        output_cost_per_m: 0.72,
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
            .unwrap_or_else(|| panic!("missing Bedrock model {id}"))
    }

    #[test]
    fn bedrock_models_count_and_provider_are_stable() {
        let models = bedrock_models();

        assert_eq!(models.len(), 8);
        assert!(models.iter().all(|model| model.provider == "bedrock"));
        assert!(models.iter().all(|model| model.supports_tools));
        assert!(models.iter().all(|model| model.supports_streaming));
        assert!(models.iter().all(|model| model.aliases.is_empty()));
        assert_eq!(
            models
                .iter()
                .filter(|model| !model.supports_vision)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["bedrock/meta.llama3-3-70b-instruct-v1:0"]
        );
    }

    #[test]
    fn bedrock_models_keep_public_order() {
        let ids: Vec<_> = bedrock_models().into_iter().map(|model| model.id).collect();

        assert_eq!(
            ids,
            vec![
                "bedrock/anthropic.claude-opus-4-6",
                "bedrock/anthropic.claude-sonnet-4-6",
                "bedrock/anthropic.claude-opus-4-20250514",
                "bedrock/anthropic.claude-sonnet-4-20250514",
                "bedrock/anthropic.claude-haiku-4-5-20251001",
                "bedrock/amazon.nova-pro-v1:0",
                "bedrock/amazon.nova-lite-v1:0",
                "bedrock/meta.llama3-3-70b-instruct-v1:0",
            ]
        );
    }

    #[test]
    fn bedrock_claude_46_models_keep_contract() {
        let models = bedrock_models();
        let opus = model(&models, "bedrock/anthropic.claude-opus-4-6");
        let sonnet = model(&models, "bedrock/anthropic.claude-sonnet-4-6");

        assert_eq!(opus.display_name, "Claude Opus 4.6 (Bedrock)");
        assert_eq!(opus.tier, ModelTier::Frontier);
        assert_eq!(opus.context_window, 200_000);
        assert_eq!(opus.max_output_tokens, 128_000);
        assert_eq!(opus.input_cost_per_m, 5.00);
        assert_eq!(opus.output_cost_per_m, 25.00);

        assert_eq!(sonnet.display_name, "Claude Sonnet 4.6 (Bedrock)");
        assert_eq!(sonnet.tier, ModelTier::Smart);
        assert_eq!(sonnet.context_window, 200_000);
        assert_eq!(sonnet.max_output_tokens, 64_000);
        assert_eq!(sonnet.input_cost_per_m, 3.00);
        assert_eq!(sonnet.output_cost_per_m, 15.00);
    }

    #[test]
    fn bedrock_legacy_claude_models_keep_contract() {
        let models = bedrock_models();
        let opus = model(&models, "bedrock/anthropic.claude-opus-4-20250514");
        let sonnet = model(&models, "bedrock/anthropic.claude-sonnet-4-20250514");
        let haiku = model(&models, "bedrock/anthropic.claude-haiku-4-5-20251001");

        assert_eq!(opus.display_name, "Claude Opus 4 (Bedrock)");
        assert_eq!(opus.tier, ModelTier::Frontier);
        assert_eq!(opus.max_output_tokens, 32_000);
        assert_eq!(opus.input_cost_per_m, 15.00);
        assert_eq!(opus.output_cost_per_m, 75.00);

        assert_eq!(sonnet.display_name, "Claude Sonnet 4 (Bedrock)");
        assert_eq!(sonnet.tier, ModelTier::Smart);
        assert_eq!(sonnet.max_output_tokens, 64_000);
        assert_eq!(sonnet.input_cost_per_m, 3.00);
        assert_eq!(sonnet.output_cost_per_m, 15.00);

        assert_eq!(haiku.display_name, "Claude Haiku 4.5 (Bedrock)");
        assert_eq!(haiku.tier, ModelTier::Fast);
        assert_eq!(haiku.context_window, 200_000);
        assert_eq!(haiku.max_output_tokens, 8_192);
        assert_eq!(haiku.input_cost_per_m, 0.25);
        assert_eq!(haiku.output_cost_per_m, 1.25);
    }

    #[test]
    fn bedrock_nova_and_llama_models_keep_contract() {
        let models = bedrock_models();
        let nova_pro = model(&models, "bedrock/amazon.nova-pro-v1:0");
        let nova_lite = model(&models, "bedrock/amazon.nova-lite-v1:0");
        let llama = model(&models, "bedrock/meta.llama3-3-70b-instruct-v1:0");

        assert_eq!(nova_pro.display_name, "Amazon Nova Pro (Bedrock)");
        assert_eq!(nova_pro.tier, ModelTier::Smart);
        assert_eq!(nova_pro.context_window, 300_000);
        assert_eq!(nova_pro.max_output_tokens, 5_120);
        assert_eq!(nova_pro.input_cost_per_m, 0.80);
        assert_eq!(nova_pro.output_cost_per_m, 3.20);

        assert_eq!(nova_lite.display_name, "Amazon Nova Lite (Bedrock)");
        assert_eq!(nova_lite.tier, ModelTier::Fast);
        assert_eq!(nova_lite.context_window, 300_000);
        assert_eq!(nova_lite.max_output_tokens, 5_120);
        assert_eq!(nova_lite.input_cost_per_m, 0.06);
        assert_eq!(nova_lite.output_cost_per_m, 0.24);

        assert_eq!(llama.display_name, "Llama 3.3 70B (Bedrock)");
        assert_eq!(llama.tier, ModelTier::Balanced);
        assert_eq!(llama.context_window, 128_000);
        assert_eq!(llama.max_output_tokens, 4_096);
        assert_eq!(llama.input_cost_per_m, 0.72);
        assert_eq!(llama.output_cost_per_m, 0.72);
        assert!(!llama.supports_vision);
    }
}
