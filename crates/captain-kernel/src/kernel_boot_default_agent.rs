use super::kernel_model_support::build_configured_fallbacks;
use super::kernel_workspace_security::PRINCIPAL_AGENT_NAME;
use super::{CaptainKernel, CAPTAIN_SYSTEM_PROMPT};
use captain_types::agent::{AgentManifest, ModelConfig};
use captain_types::config::{DefaultModelConfig, FallbackProviderConfig};
use tracing::{info, warn};

pub(super) fn ensure_default_captain(kernel: &CaptainKernel) {
    if kernel
        .registry
        .list()
        .iter()
        .any(|entry| is_boot_principal_name(&entry.name))
    {
        return;
    }

    info!("No agents found — spawning Captain");
    let manifest = build_default_captain_manifest(
        &kernel.config.default_model,
        &kernel.config.fallback_providers,
    );
    match kernel.spawn_agent(manifest) {
        Ok(id) => info!(id = %id, "Default assistant spawned"),
        Err(e) => warn!("Failed to spawn default assistant: {e}"),
    }
}

fn build_default_captain_manifest(
    default_model: &DefaultModelConfig,
    fallback_providers: &[FallbackProviderConfig],
) -> AgentManifest {
    AgentManifest {
        name: PRINCIPAL_AGENT_NAME.to_string(),
        description: "Captain — principal agent".to_string(),
        model: ModelConfig {
            provider: default_model.provider.clone(),
            model: default_model.model.clone(),
            system_prompt: CAPTAIN_SYSTEM_PROMPT.to_string(),
            api_key_env: non_empty_api_key_env(default_model),
            base_url: default_model.base_url.clone(),
            ..Default::default()
        },
        fallback_models: build_configured_fallbacks(fallback_providers),
        ..Default::default()
    }
}

fn non_empty_api_key_env(default_model: &DefaultModelConfig) -> Option<String> {
    if default_model.api_key_env.is_empty() {
        None
    } else {
        Some(default_model.api_key_env.clone())
    }
}

fn is_boot_principal_name(name: &str) -> bool {
    name == PRINCIPAL_AGENT_NAME || name == "assistant"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_principal_name_accepts_legacy_assistant_alias() {
        assert!(is_boot_principal_name("captain"));
        assert!(is_boot_principal_name("assistant"));
        assert!(!is_boot_principal_name("researcher"));
        assert!(!is_boot_principal_name("Captain"));
    }

    #[test]
    fn default_captain_manifest_uses_global_default_model() {
        let default_model = DefaultModelConfig {
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            api_key_env: String::new(),
            base_url: None,
        };
        let manifest = build_default_captain_manifest(&default_model, &[]);

        assert_eq!(manifest.name, "captain");
        assert_eq!(manifest.description, "Captain — principal agent");
        assert_eq!(manifest.model.provider, "codex");
        assert_eq!(manifest.model.model, "gpt-5.5");
        assert_eq!(manifest.model.api_key_env, None);
        assert_eq!(manifest.model.base_url, None);
        assert_eq!(manifest.model.system_prompt, CAPTAIN_SYSTEM_PROMPT);
        assert!(manifest.fallback_models.is_empty());
        assert_eq!(
            manifest.orchestration_mode,
            captain_types::agent::OrchestrationMode::Direct
        );
    }

    #[test]
    fn default_captain_manifest_preserves_explicit_auth_and_fallbacks() {
        let default_model = DefaultModelConfig {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            base_url: Some("https://api.example.invalid".to_string()),
        };
        let fallback_providers = vec![FallbackProviderConfig {
            provider: "ollama".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key_env: String::new(),
            base_url: Some("http://localhost:11434".to_string()),
        }];

        let manifest = build_default_captain_manifest(&default_model, &fallback_providers);

        assert_eq!(
            manifest.model.api_key_env.as_deref(),
            Some("ANTHROPIC_API_KEY")
        );
        assert_eq!(
            manifest.model.base_url.as_deref(),
            Some("https://api.example.invalid")
        );
        assert_eq!(manifest.fallback_models.len(), 1);
        assert_eq!(manifest.fallback_models[0].provider, "ollama");
        assert_eq!(manifest.fallback_models[0].api_key_env, None);
        assert_eq!(
            manifest.fallback_models[0].base_url.as_deref(),
            Some("http://localhost:11434")
        );
    }
}
