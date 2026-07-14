use captain_types::model_catalog::{
    AuthStatus, ProviderInfo, AI21_BASE_URL, ANTHROPIC_BASE_URL, AZURE_OPENAI_BASE_URL,
    BEDROCK_BASE_URL, CEREBRAS_BASE_URL, CHUTES_BASE_URL, CODEX_BASE_URL, COHERE_BASE_URL,
    DEEPSEEK_BASE_URL, FIREWORKS_BASE_URL, GEMINI_BASE_URL, GITHUB_COPILOT_BASE_URL, GROQ_BASE_URL,
    HUGGINGFACE_BASE_URL, KIMI_CODING_BASE_URL, LEMONADE_BASE_URL, LMSTUDIO_BASE_URL,
    MINIMAX_BASE_URL, MISTRAL_BASE_URL, MOONSHOT_BASE_URL, NVIDIA_NIM_BASE_URL, OLLAMA_BASE_URL,
    OPENAI_BASE_URL, OPENROUTER_BASE_URL, PERPLEXITY_BASE_URL, QIANFAN_BASE_URL, QWEN_BASE_URL,
    REPLICATE_BASE_URL, SAMBANOVA_BASE_URL, TOGETHER_BASE_URL, VENICE_BASE_URL, VLLM_BASE_URL,
    VOLCENGINE_BASE_URL, VOLCENGINE_CODING_BASE_URL, XAI_BASE_URL, ZAI_BASE_URL,
    ZAI_CODING_BASE_URL, ZHIPU_BASE_URL, ZHIPU_CODING_BASE_URL,
};

fn provider_info(
    (id, display_name, api_key_env, base_url, key_required): (&str, &str, &str, &str, bool),
) -> ProviderInfo {
    ProviderInfo {
        id: id.into(),
        display_name: display_name.into(),
        api_key_env: api_key_env.into(),
        base_url: base_url.into(),
        key_required,
        auth_status: if key_required {
            AuthStatus::Missing
        } else {
            AuthStatus::NotRequired
        },
        model_count: 0,
    }
}

const BUILTIN_PROVIDER_ROWS: &[(&str, &str, &str, &str, bool)] = &[
    (
        "anthropic",
        "Anthropic",
        "ANTHROPIC_API_KEY",
        ANTHROPIC_BASE_URL,
        true,
    ),
    ("openai", "OpenAI", "OPENAI_API_KEY", OPENAI_BASE_URL, true),
    (
        "gemini",
        "Google Gemini",
        "GEMINI_API_KEY",
        GEMINI_BASE_URL,
        true,
    ),
    (
        "deepseek",
        "DeepSeek",
        "DEEPSEEK_API_KEY",
        DEEPSEEK_BASE_URL,
        true,
    ),
    ("groq", "Groq", "GROQ_API_KEY", GROQ_BASE_URL, true),
    (
        "openrouter",
        "OpenRouter",
        "OPENROUTER_API_KEY",
        OPENROUTER_BASE_URL,
        true,
    ),
    (
        "mistral",
        "Mistral AI",
        "MISTRAL_API_KEY",
        MISTRAL_BASE_URL,
        true,
    ),
    (
        "together",
        "Together AI",
        "TOGETHER_API_KEY",
        TOGETHER_BASE_URL,
        true,
    ),
    (
        "fireworks",
        "Fireworks AI",
        "FIREWORKS_API_KEY",
        FIREWORKS_BASE_URL,
        true,
    ),
    ("ollama", "Ollama", "OLLAMA_API_KEY", OLLAMA_BASE_URL, false),
    ("vllm", "vLLM", "VLLM_API_KEY", VLLM_BASE_URL, false),
    (
        "lmstudio",
        "LM Studio",
        "LMSTUDIO_API_KEY",
        LMSTUDIO_BASE_URL,
        false,
    ),
    (
        "lemonade",
        "Lemonade",
        "LEMONADE_API_KEY",
        LEMONADE_BASE_URL,
        false,
    ),
    (
        "perplexity",
        "Perplexity AI",
        "PERPLEXITY_API_KEY",
        PERPLEXITY_BASE_URL,
        true,
    ),
    ("cohere", "Cohere", "COHERE_API_KEY", COHERE_BASE_URL, true),
    ("ai21", "AI21 Labs", "AI21_API_KEY", AI21_BASE_URL, true),
    (
        "cerebras",
        "Cerebras",
        "CEREBRAS_API_KEY",
        CEREBRAS_BASE_URL,
        true,
    ),
    (
        "sambanova",
        "SambaNova",
        "SAMBANOVA_API_KEY",
        SAMBANOVA_BASE_URL,
        true,
    ),
    (
        "huggingface",
        "Hugging Face",
        "HF_API_KEY",
        HUGGINGFACE_BASE_URL,
        true,
    ),
    ("xai", "xAI", "XAI_API_KEY", XAI_BASE_URL, true),
    (
        "replicate",
        "Replicate",
        "REPLICATE_API_TOKEN",
        REPLICATE_BASE_URL,
        true,
    ),
    (
        "github-copilot",
        "GitHub Copilot",
        "GITHUB_TOKEN",
        GITHUB_COPILOT_BASE_URL,
        true,
    ),
    (
        "chutes",
        "Chutes.ai",
        "CHUTES_API_KEY",
        CHUTES_BASE_URL,
        true,
    ),
    (
        "venice",
        "Venice.ai",
        "VENICE_API_KEY",
        VENICE_BASE_URL,
        true,
    ),
    (
        "nvidia",
        "NVIDIA NIM",
        "NVIDIA_API_KEY",
        NVIDIA_NIM_BASE_URL,
        true,
    ),
    (
        "qwen",
        "Qwen (Alibaba)",
        "DASHSCOPE_API_KEY",
        QWEN_BASE_URL,
        true,
    ),
    (
        "minimax",
        "MiniMax",
        "MINIMAX_API_KEY",
        MINIMAX_BASE_URL,
        true,
    ),
    (
        "zhipu",
        "Zhipu AI (GLM)",
        "ZHIPU_API_KEY",
        ZHIPU_BASE_URL,
        true,
    ),
    (
        "zhipu_coding",
        "Zhipu Coding (CodeGeeX)",
        "ZHIPU_API_KEY",
        ZHIPU_CODING_BASE_URL,
        true,
    ),
    ("zai", "Z.AI", "ZHIPU_API_KEY", ZAI_BASE_URL, true),
    (
        "zai_coding",
        "Z.AI Coding",
        "ZHIPU_API_KEY",
        ZAI_CODING_BASE_URL,
        true,
    ),
    (
        "moonshot",
        "Moonshot (Kimi)",
        "MOONSHOT_API_KEY",
        MOONSHOT_BASE_URL,
        true,
    ),
    (
        "kimi_coding",
        "Kimi for Code",
        "KIMI_API_KEY",
        KIMI_CODING_BASE_URL,
        true,
    ),
    (
        "qianfan",
        "Baidu Qianfan",
        "QIANFAN_API_KEY",
        QIANFAN_BASE_URL,
        true,
    ),
    (
        "volcengine",
        "Volcano Engine (Doubao)",
        "VOLCENGINE_API_KEY",
        VOLCENGINE_BASE_URL,
        true,
    ),
    (
        "volcengine_coding",
        "Volcano Engine Coding Plan",
        "VOLCENGINE_API_KEY",
        VOLCENGINE_CODING_BASE_URL,
        true,
    ),
    (
        "bedrock",
        "AWS Bedrock",
        "AWS_ACCESS_KEY_ID",
        BEDROCK_BASE_URL,
        true,
    ),
    (
        "azure",
        "Azure OpenAI",
        "AZURE_OPENAI_API_KEY",
        AZURE_OPENAI_BASE_URL,
        true,
    ),
    ("codex", "OpenAI Codex", "", CODEX_BASE_URL, true),
    ("claude-code", "Claude Code", "", "", false),
    ("qwen-code", "Qwen Code", "", "", false),
];

pub(crate) fn builtin_providers() -> Vec<ProviderInfo> {
    BUILTIN_PROVIDER_ROWS
        .iter()
        .copied()
        .map(provider_info)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn provider<'a>(providers: &'a [ProviderInfo], id: &str) -> &'a ProviderInfo {
        providers
            .iter()
            .find(|provider| provider.id == id)
            .unwrap_or_else(|| panic!("missing provider {id}"))
    }

    #[test]
    fn builtin_provider_count_is_stable() {
        assert_eq!(builtin_providers().len(), 41);
    }

    #[test]
    fn builtin_provider_order_is_stable() {
        let providers = builtin_providers();
        let ids: Vec<&str> = providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect();

        assert_eq!(
            &ids[..8],
            [
                "anthropic",
                "openai",
                "gemini",
                "deepseek",
                "groq",
                "openrouter",
                "mistral",
                "together",
            ]
        );
        assert_eq!(&ids[ids.len() - 3..], ["codex", "claude-code", "qwen-code"]);
    }

    #[test]
    fn provider_ids_are_unique() {
        let providers = builtin_providers();
        let mut ids = HashSet::new();
        for provider in providers {
            assert!(ids.insert(provider.id));
        }
    }

    #[test]
    fn local_and_cli_providers_do_not_require_api_keys() {
        let providers = builtin_providers();
        for id in ["ollama", "vllm", "lmstudio", "lemonade"] {
            let provider = provider(&providers, id);
            assert!(!provider.key_required);
            assert_eq!(provider.auth_status, AuthStatus::NotRequired);
            assert!(!provider.base_url.is_empty());
        }

        for id in ["claude-code", "qwen-code"] {
            let provider = provider(&providers, id);
            assert!(!provider.key_required);
            assert_eq!(provider.auth_status, AuthStatus::NotRequired);
            assert!(provider.api_key_env.is_empty());
            assert!(provider.base_url.is_empty());
        }
    }

    #[test]
    fn major_remote_providers_have_explicit_auth_contracts() {
        let providers = builtin_providers();
        let expected = [
            ("anthropic", "ANTHROPIC_API_KEY", ANTHROPIC_BASE_URL),
            ("openai", "OPENAI_API_KEY", OPENAI_BASE_URL),
            ("gemini", "GEMINI_API_KEY", GEMINI_BASE_URL),
            ("qwen", "DASHSCOPE_API_KEY", QWEN_BASE_URL),
            ("bedrock", "AWS_ACCESS_KEY_ID", BEDROCK_BASE_URL),
        ];

        for (id, env, base_url) in expected {
            let provider = provider(&providers, id);
            assert!(provider.key_required);
            assert_eq!(provider.auth_status, AuthStatus::Missing);
            assert_eq!(provider.api_key_env, env);
            assert_eq!(provider.base_url, base_url);
        }
    }

    #[test]
    fn codex_and_azure_keep_special_provider_contracts() {
        let providers = builtin_providers();

        let codex = provider(&providers, "codex");
        assert!(codex.key_required);
        assert_eq!(codex.auth_status, AuthStatus::Missing);
        assert!(codex.api_key_env.is_empty());
        assert_eq!(codex.base_url, CODEX_BASE_URL);

        let azure = provider(&providers, "azure");
        assert!(azure.key_required);
        assert_eq!(azure.api_key_env, "AZURE_OPENAI_API_KEY");
        assert!(azure.base_url.is_empty());
    }
}
