use serde::{Deserialize, Serialize};

/// Fallback provider chain, tried in order if the primary provider fails.
///
/// Configurable in `config.toml` under `[[fallback_providers]]`:
/// ```toml
/// [[fallback_providers]]
/// provider = "ollama"
/// model = "llama3.2:latest"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FallbackProviderConfig {
    /// Provider name (e.g., "ollama", "groq").
    pub provider: String,
    /// Model to use from this provider.
    pub model: String,
    /// Environment variable for API key (empty for local providers).
    #[serde(default)]
    pub api_key_env: String,
    /// Base URL override (uses catalog default if None).
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Default LLM model configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultModelConfig {
    /// Provider name (e.g., "codex", "anthropic", "openai").
    pub provider: String,
    /// Model identifier.
    pub model: String,
    /// Environment variable name for the API key.
    pub api_key_env: String,
    /// Optional base URL override.
    pub base_url: Option<String>,
}

impl Default for DefaultModelConfig {
    fn default() -> Self {
        Self {
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            api_key_env: String::new(),
            base_url: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultModelConfig, FallbackProviderConfig};
    use crate::config::KernelConfig;

    #[test]
    fn default_model_config_defaults_stay_codex_first() {
        let config = DefaultModelConfig::default();

        assert_eq!(config.provider, "codex");
        assert_eq!(config.model, "gpt-5.5");
        assert!(config.api_key_env.is_empty());
        assert!(config.base_url.is_none());
    }

    #[test]
    fn default_model_config_deserializes_partial_toml_with_defaults() {
        let config: DefaultModelConfig = toml::from_str(
            r#"
            provider = "openai"
            model = "gpt-4.1"
            "#,
        )
        .unwrap();

        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-4.1");
        assert!(config.api_key_env.is_empty());
        assert!(config.base_url.is_none());
    }

    #[test]
    fn default_model_config_roundtrips_custom_provider_fields() {
        let config = DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: Some("http://127.0.0.1:11434/v1".to_string()),
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: DefaultModelConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(decoded.provider, "ollama");
        assert_eq!(decoded.model, "llama3.2:latest");
        assert_eq!(decoded.api_key_env, "OLLAMA_API_KEY");
        assert_eq!(
            decoded.base_url.as_deref(),
            Some("http://127.0.0.1:11434/v1")
        );
    }

    #[test]
    fn fallback_config_serde_roundtrips_local_provider() {
        let fallback = FallbackProviderConfig {
            provider: "ollama".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key_env: String::new(),
            base_url: None,
        };

        let json = serde_json::to_string(&fallback).unwrap();
        let decoded: FallbackProviderConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.provider, "ollama");
        assert_eq!(decoded.model, "llama3.2:latest");
        assert!(decoded.api_key_env.is_empty());
        assert!(decoded.base_url.is_none());
    }

    #[test]
    fn fallback_config_defaults_to_empty_chain_in_kernel_config() {
        let config = KernelConfig::default();

        assert!(config.fallback_providers.is_empty());
    }

    #[test]
    fn fallback_config_deserializes_kernel_toml_chain() {
        let config: KernelConfig = toml::from_str(
            r#"
            [[fallback_providers]]
            provider = "ollama"
            model = "llama3.2:latest"

            [[fallback_providers]]
            provider = "groq"
            model = "llama-3.3-70b-versatile"
            api_key_env = "GROQ_API_KEY"
            "#,
        )
        .unwrap();

        assert_eq!(config.fallback_providers.len(), 2);
        assert_eq!(config.fallback_providers[0].provider, "ollama");
        assert_eq!(config.fallback_providers[0].model, "llama3.2:latest");
        assert_eq!(config.fallback_providers[1].provider, "groq");
        assert_eq!(config.fallback_providers[1].api_key_env, "GROQ_API_KEY");
    }
}
