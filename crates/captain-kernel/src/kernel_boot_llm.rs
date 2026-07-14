use captain_runtime::agent_loop::strip_provider_prefix;
use captain_runtime::drivers;
use captain_runtime::llm_driver::{
    CompletionRequest, CompletionResponse, DriverConfig, LlmDriver, LlmError,
};
use captain_types::config::{FallbackProviderConfig, KernelConfig};
use std::sync::Arc;
use tracing::{info, warn};

pub(super) struct BootLlmDriver {
    pub(super) driver: Arc<dyn LlmDriver>,
    pub(super) ready: bool,
    pub(super) error: Option<String>,
}

pub(super) fn build_boot_llm_driver(
    config: &mut KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
) -> BootLlmDriver {
    let mut targets = BootDriverTargets::default();
    add_primary_or_autodetected_driver(&mut targets, config, credential_resolver);
    add_fallback_drivers(&mut targets, config, credential_resolver);
    finalize_boot_llm_driver(targets, &config.language)
}

#[derive(Default)]
struct BootDriverTargets {
    drivers: Vec<Arc<dyn LlmDriver>>,
    model_chain: Vec<(Arc<dyn LlmDriver>, String, String)>,
    init_errors: Vec<String>,
}

impl BootDriverTargets {
    fn push_primary(&mut self, driver: Arc<dyn LlmDriver>, label: String) {
        self.drivers.push(driver.clone());
        self.model_chain.push((driver, String::new(), label));
    }

    fn push_fallback(&mut self, driver: Arc<dyn LlmDriver>, fb: &FallbackProviderConfig) {
        self.drivers.push(driver.clone());
        self.model_chain.push((
            driver,
            strip_provider_prefix(&fb.model, &fb.provider),
            format!("{}/{}", fb.provider, fb.model),
        ));
    }

    fn push_error(&mut self, error: String) {
        self.init_errors.push(error);
    }
}

fn add_primary_or_autodetected_driver(
    targets: &mut BootDriverTargets,
    config: &mut KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
) {
    let driver_config = default_driver_config(config, credential_resolver);
    match drivers::create_driver(&driver_config) {
        Ok(driver) => targets.push_primary(driver, default_model_label(config)),
        Err(e) => {
            targets.push_error(format!("{}: {e}", config.default_model.provider));
            warn!(
                provider = %config.default_model.provider,
                error = %e,
                "Primary LLM driver init failed — trying auto-detect"
            );
            try_autodetected_driver(targets, config, credential_resolver);
        }
    }
}

fn try_autodetected_driver(
    targets: &mut BootDriverTargets,
    config: &mut KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
) {
    let Some((provider, model, env_var)) = drivers::detect_available_provider() else {
        return;
    };
    let auto_config = DriverConfig {
        provider: provider.to_string(),
        api_key: credential_resolver
            .resolve(env_var)
            .map(|z: zeroize::Zeroizing<String>| z.to_string()),
        base_url: config.provider_urls.get(provider).cloned(),
        skip_permissions: true,
    };
    match drivers::create_driver(&auto_config) {
        Ok(driver) => {
            info!(
                provider = %provider,
                model = %model,
                "Auto-detected provider from {} — using as default",
                env_var
            );
            config.default_model.provider = provider.to_string();
            config.default_model.model = model.to_string();
            config.default_model.api_key_env = env_var.to_string();
            targets.push_primary(driver, default_model_label(config));
        }
        Err(e) => {
            targets.push_error(format!("{provider}: {e}"));
            warn!(provider = %provider, error = %e, "Auto-detected provider also failed");
        }
    }
}

fn add_fallback_drivers(
    targets: &mut BootDriverTargets,
    config: &KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
) {
    for fb in &config.fallback_providers {
        let fb_config = fallback_driver_config(config, credential_resolver, fb);
        match drivers::create_driver(&fb_config) {
            Ok(driver) => {
                info!(
                    provider = %fb.provider,
                    model = %fb.model,
                    "Fallback provider configured"
                );
                targets.push_fallback(driver, fb);
            }
            Err(e) => {
                targets.push_error(format!("fallback {}: {e}", fb.provider));
                warn!(
                    provider = %fb.provider,
                    error = %e,
                    "Fallback provider init failed — skipped"
                );
            }
        }
    }
}

fn finalize_boot_llm_driver(targets: BootDriverTargets, language: &str) -> BootLlmDriver {
    let BootDriverTargets {
        drivers,
        model_chain,
        init_errors,
    } = targets;

    if drivers.len() > 1 {
        let notice_template = captain_runtime::drivers::fallback::notice_template_for(language);
        return BootLlmDriver {
            driver: Arc::new(
                captain_runtime::drivers::fallback::FallbackDriver::with_targets(model_chain)
                    .with_notice_template(notice_template),
            ),
            ready: true,
            error: None,
        };
    }

    if let Some(single) = drivers.into_iter().next() {
        return BootLlmDriver {
            driver: single,
            ready: true,
            error: None,
        };
    }

    let error = no_llm_driver_error(&init_errors);
    warn!(error = %error, "No LLM drivers available — agents will return errors until a provider is configured");
    BootLlmDriver {
        driver: Arc::new(StubDriver) as Arc<dyn LlmDriver>,
        ready: false,
        error: Some(error),
    }
}

fn default_driver_config(
    config: &KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
) -> DriverConfig {
    DriverConfig {
        provider: config.default_model.provider.clone(),
        api_key: provider_api_key(
            config,
            credential_resolver,
            &config.default_model.provider,
            &config.default_model.api_key_env,
        ),
        base_url: provider_base_url(
            config,
            &config.default_model.provider,
            &config.default_model.base_url,
        ),
        skip_permissions: true,
    }
}

fn fallback_driver_config(
    config: &KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
    fb: &FallbackProviderConfig,
) -> DriverConfig {
    DriverConfig {
        provider: fb.provider.clone(),
        api_key: provider_api_key(config, credential_resolver, &fb.provider, &fb.api_key_env),
        base_url: provider_base_url(config, &fb.provider, &fb.base_url),
        skip_permissions: true,
    }
}

fn provider_api_key(
    config: &KernelConfig,
    credential_resolver: &captain_extensions::credentials::CredentialResolver,
    provider: &str,
    explicit_env: &str,
) -> Option<String> {
    credential_resolver
        .resolve(&api_key_env_for_provider(config, provider, explicit_env))
        .map(|z: zeroize::Zeroizing<String>| z.to_string())
}

fn api_key_env_for_provider(config: &KernelConfig, provider: &str, explicit_env: &str) -> String {
    if explicit_env.is_empty() {
        config.resolve_api_key_env(provider)
    } else {
        explicit_env.to_string()
    }
}

fn provider_base_url(
    config: &KernelConfig,
    provider: &str,
    explicit_base_url: &Option<String>,
) -> Option<String> {
    explicit_base_url
        .clone()
        .or_else(|| config.provider_urls.get(provider).cloned())
}

fn default_model_label(config: &KernelConfig) -> String {
    format!(
        "{}/{}",
        config.default_model.provider, config.default_model.model
    )
}

fn no_llm_driver_error(driver_init_errors: &[String]) -> String {
    if driver_init_errors.is_empty() {
        "No LLM provider could be initialized.".to_string()
    } else {
        driver_init_errors.join("; ")
    }
}

struct StubDriver;

#[async_trait::async_trait]
impl LlmDriver for StubDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::MissingApiKey(
            "No LLM provider configured. Set an API key (e.g. GROQ_API_KEY) and restart, \
             configure a provider via the dashboard, \
             or use Ollama for local models (no API key needed)."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_llm_driver_error_defaults_when_no_provider_was_attempted() {
        assert_eq!(
            no_llm_driver_error(&[]),
            "No LLM provider could be initialized."
        );
    }

    #[test]
    fn no_llm_driver_error_joins_attempted_provider_errors() {
        let errors = vec![
            "openai: missing key".to_string(),
            "fallback groq: invalid key".to_string(),
        ];

        assert_eq!(
            no_llm_driver_error(&errors),
            "openai: missing key; fallback groq: invalid key"
        );
    }

    #[test]
    fn api_key_env_prefers_explicit_then_config_mapping_then_convention() {
        let mut config = KernelConfig::default();
        config
            .provider_api_keys
            .insert("openai".to_string(), "OPENAI_CUSTOM_KEY".to_string());

        assert_eq!(
            api_key_env_for_provider(&config, "openai", "OPENAI_OVERRIDE"),
            "OPENAI_OVERRIDE"
        );
        assert_eq!(
            api_key_env_for_provider(&config, "openai", ""),
            "OPENAI_CUSTOM_KEY"
        );
        assert_eq!(
            api_key_env_for_provider(&config, "my-provider", ""),
            "MY_PROVIDER_API_KEY"
        );
    }

    #[test]
    fn provider_base_url_prefers_explicit_then_provider_override() {
        let mut config = KernelConfig::default();
        config
            .provider_urls
            .insert("ollama".to_string(), "http://shared:11434/v1".to_string());
        let explicit = Some("http://explicit:11434/v1".to_string());

        assert_eq!(
            provider_base_url(&config, "ollama", &explicit).as_deref(),
            Some("http://explicit:11434/v1")
        );
        assert_eq!(
            provider_base_url(&config, "ollama", &None).as_deref(),
            Some("http://shared:11434/v1")
        );
        assert_eq!(provider_base_url(&config, "groq", &None), None);
    }

    #[test]
    fn default_model_label_uses_running_default_model() {
        let mut config = KernelConfig::default();
        config.default_model.provider = "groq".to_string();
        config.default_model.model = "llama-3.3-70b-versatile".to_string();

        assert_eq!(default_model_label(&config), "groq/llama-3.3-70b-versatile");
    }
}
