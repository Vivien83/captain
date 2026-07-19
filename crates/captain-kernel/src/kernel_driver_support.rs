use crate::error::{KernelError, KernelResult};

use super::CaptainKernel;
use captain_runtime::agent_loop::strip_provider_prefix;
use captain_runtime::drivers;
use captain_runtime::llm_cache::{global_cache as llm_global_cache, CachedLlmDriver};
use captain_runtime::llm_driver::{DriverConfig, LlmDriver};
use captain_types::agent::{AgentManifest, FallbackModel};
use captain_types::config::DefaultModelConfig;
use std::sync::Arc;
use tracing::{debug, warn};

type DriverTarget = (Arc<dyn LlmDriver>, String, String);

impl CaptainKernel {
    pub(crate) fn lookup_provider_url(&self, provider: &str) -> Option<String> {
        if let Some(url) = self.config.provider_urls.get(provider) {
            return Some(url.clone());
        }
        if let Ok(catalog) = self.model_catalog.read() {
            if let Some(provider_entry) = catalog.get_provider(provider) {
                if !provider_entry.base_url.is_empty() {
                    return Some(provider_entry.base_url.clone());
                }
            }
        }
        None
    }

    /// Effective system default model, including hot-reload/model-switch
    /// updates applied after boot.
    pub fn effective_default_model(&self) -> DefaultModelConfig {
        self.default_model_override
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone())
    }

    /// Runtime readiness of the currently effective default LLM provider.
    pub fn default_llm_driver_status(&self) -> (bool, Option<String>) {
        let default_model = self.effective_default_model();
        let api_key = if !default_model.api_key_env.is_empty() {
            self.resolve_credential(&default_model.api_key_env)
        } else {
            let env_var = self.config.resolve_api_key_env(&default_model.provider);
            self.resolve_credential(&env_var)
        };
        let driver_config = DriverConfig {
            provider: default_model.provider.clone(),
            api_key,
            base_url: default_model
                .base_url
                .clone()
                .or_else(|| self.lookup_provider_url(&default_model.provider)),
            skip_permissions: true,
        };

        match drivers::create_driver(&driver_config) {
            Ok(_) => (true, None),
            Err(e) => {
                let use_boot_error = !self.llm_driver_ready
                    && default_model.provider == self.config.default_model.provider;
                let error = if use_boot_error {
                    self.llm_driver_error
                        .clone()
                        .unwrap_or_else(|| e.to_string())
                } else {
                    e.to_string()
                };
                (false, Some(error))
            }
        }
    }

    pub(crate) fn resolve_driver(
        &self,
        manifest: &AgentManifest,
    ) -> KernelResult<Arc<dyn LlmDriver>> {
        let effective_default = self.effective_default_model();
        let primary = self.resolve_primary_driver(manifest, &effective_default)?;
        Ok(wrap_with_cache(
            self.driver_with_configured_fallbacks(manifest, primary),
        ))
    }

    fn resolve_primary_driver(
        &self,
        manifest: &AgentManifest,
        effective_default: &DefaultModelConfig,
    ) -> KernelResult<Arc<dyn LlmDriver>> {
        let agent_provider = &manifest.model.provider;
        let driver_config = DriverConfig {
            provider: agent_provider.clone(),
            api_key: self.primary_api_key(manifest, effective_default),
            base_url: self.primary_base_url(manifest, effective_default),
            skip_permissions: true,
        };

        match drivers::create_driver_with_quota_observer(
            &driver_config,
            self.quota_observer_for(agent_provider),
        ) {
            Ok(driver) => Ok(driver),
            Err(e) => {
                if should_use_boot_default_driver(manifest, effective_default) {
                    debug!(
                        provider = %agent_provider,
                        error = %e,
                        "Fresh driver creation failed, falling back to boot-time default"
                    );
                    Ok(Arc::clone(&self.default_driver))
                } else {
                    Err(KernelError::BootFailed(format!(
                        "Agent LLM driver init failed: {e}"
                    )))
                }
            }
        }
    }

    fn primary_api_key(
        &self,
        manifest: &AgentManifest,
        effective_default: &DefaultModelConfig,
    ) -> Option<String> {
        let agent_provider = &manifest.model.provider;
        if let Some(env) = manifest.model.api_key_env.as_ref() {
            return self.resolve_credential(env);
        }
        if agent_provider == &effective_default.provider
            && !effective_default.api_key_env.is_empty()
        {
            return self.resolve_credential(&effective_default.api_key_env);
        }
        let env_var = self.config.resolve_api_key_env(agent_provider);
        self.resolve_credential(&env_var)
    }

    fn primary_base_url(
        &self,
        manifest: &AgentManifest,
        effective_default: &DefaultModelConfig,
    ) -> Option<String> {
        let agent_provider = &manifest.model.provider;
        if manifest.model.base_url.is_some() {
            return manifest.model.base_url.clone();
        }
        if agent_provider == &effective_default.provider {
            return effective_default
                .base_url
                .clone()
                .or_else(|| self.lookup_provider_url(agent_provider));
        }
        self.lookup_provider_url(agent_provider)
    }

    fn driver_with_configured_fallbacks(
        &self,
        manifest: &AgentManifest,
        primary: Arc<dyn LlmDriver>,
    ) -> Arc<dyn LlmDriver> {
        if !manifest.fallback_models.is_empty() {
            let mut chain: Vec<DriverTarget> =
                vec![primary_driver_target(primary.clone(), manifest)];
            for fallback_model in &manifest.fallback_models {
                if let Some(target) = self.fallback_driver_target(fallback_model) {
                    chain.push(target);
                }
            }
            if chain.len() > 1 {
                let notice_template =
                    captain_runtime::drivers::fallback::notice_template_for(&self.config.language);
                let fallback: Arc<dyn LlmDriver> = Arc::new(
                    captain_runtime::drivers::fallback::FallbackDriver::with_targets(chain)
                        .with_notice_template(notice_template),
                );
                return fallback;
            }
        }
        primary
    }

    fn fallback_driver_target(&self, fallback_model: &FallbackModel) -> Option<DriverTarget> {
        let config = DriverConfig {
            provider: fallback_model.provider.clone(),
            api_key: fallback_api_key(&self.config, fallback_model),
            base_url: fallback_model
                .base_url
                .clone()
                .or_else(|| self.lookup_provider_url(&fallback_model.provider)),
            skip_permissions: true,
        };
        match drivers::create_driver_with_quota_observer(
            &config,
            self.quota_observer_for(&fallback_model.provider),
        ) {
            Ok(driver) => Some((
                driver,
                fallback_runtime_model(fallback_model),
                provider_model_label(&fallback_model.provider, &fallback_model.model),
            )),
            Err(e) => {
                warn!(
                    "Fallback driver '{}' failed to init: {e}",
                    fallback_model.provider
                );
                None
            }
        }
    }

    fn quota_observer_for(
        &self,
        provider: &str,
    ) -> Option<captain_runtime::provider_quota::ProviderQuotaObserver> {
        matches!(provider, "codex" | "openai-codex").then(|| {
            crate::provider_quota_monitor::provider_quota_observer(
                self.memory.provider_quotas().clone(),
            )
        })
    }
}

fn should_use_boot_default_driver(
    manifest: &AgentManifest,
    effective_default: &DefaultModelConfig,
) -> bool {
    manifest.model.provider == effective_default.provider
        && manifest.model.api_key_env.is_none()
        && manifest.model.base_url.is_none()
}

fn primary_driver_target(primary: Arc<dyn LlmDriver>, manifest: &AgentManifest) -> DriverTarget {
    (
        primary,
        String::new(),
        provider_model_label(&manifest.model.provider, &manifest.model.model),
    )
}

fn fallback_api_key(
    config: &captain_types::config::KernelConfig,
    fallback_model: &FallbackModel,
) -> Option<String> {
    if let Some(env) = &fallback_model.api_key_env {
        std::env::var(env).ok()
    } else {
        let env_var = config.resolve_api_key_env(&fallback_model.provider);
        std::env::var(&env_var).ok()
    }
}

fn fallback_runtime_model(fallback_model: &FallbackModel) -> String {
    strip_provider_prefix(&fallback_model.model, &fallback_model.provider)
}

fn provider_model_label(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

pub(crate) fn resolve_daemon_api_key(home_dir: &std::path::Path) -> Option<(&'static str, String)> {
    let secrets_path = home_dir.join("secrets.env");
    let dotenv_path = home_dir.join(".env");
    let resolver = captain_extensions::credentials::CredentialResolver::new_with_secrets(
        None,
        Some(&secrets_path),
        Some(&dotenv_path),
    );
    for key in ["CAPTAIN_DAEMON_API_KEY", "CAPTAIN_API_KEY"] {
        if let Some(value) = resolver.resolve(key) {
            let value = value.trim().to_string();
            if !value.is_empty() {
                return Some((key, value));
            }
        }
    }
    None
}

fn wrap_with_cache(driver: Arc<dyn LlmDriver>) -> Arc<dyn LlmDriver> {
    match llm_global_cache() {
        Some(cache) => Arc::new(CachedLlmDriver::new(driver, cache)),
        None => driver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_api_key_resolves_from_secrets_env() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("secrets.env"),
            "CAPTAIN_DAEMON_API_KEY=captain_api_secret\n",
        )
        .unwrap();
        let resolved = resolve_daemon_api_key(tmp.path());
        assert_eq!(
            resolved,
            Some(("CAPTAIN_DAEMON_API_KEY", "captain_api_secret".to_string()))
        );
    }

    #[test]
    fn boot_default_driver_fallback_only_applies_without_agent_overrides() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "codex".to_string();
        let default_model = DefaultModelConfig::default();

        assert!(should_use_boot_default_driver(&manifest, &default_model));

        manifest.model.api_key_env = Some("CUSTOM_KEY".to_string());
        assert!(!should_use_boot_default_driver(&manifest, &default_model));

        manifest.model.api_key_env = None;
        manifest.model.base_url = Some("https://example.invalid".to_string());
        assert!(!should_use_boot_default_driver(&manifest, &default_model));

        manifest.model.base_url = None;
        manifest.model.provider = "anthropic".to_string();
        assert!(!should_use_boot_default_driver(&manifest, &default_model));
    }

    #[test]
    fn fallback_target_metadata_strips_provider_prefix_and_keeps_label() {
        let fallback_model = FallbackModel {
            provider: "openai".to_string(),
            model: "openai/gpt-5.1".to_string(),
            api_key_env: None,
            base_url: None,
        };

        assert_eq!(fallback_runtime_model(&fallback_model), "gpt-5.1");
        assert_eq!(
            provider_model_label(&fallback_model.provider, &fallback_model.model),
            "openai/openai/gpt-5.1"
        );
    }
}
