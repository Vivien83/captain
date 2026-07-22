use super::CaptainKernel;
use captain_runtime::reflection_job::{LlmDriverCompleter, NoopCompleter, ReflectionCompleter};
use captain_runtime::workflow_learning_proposer::{ActiveModelIdentity, WorkflowDraftProposer};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

impl CaptainKernel {
    fn learning_reflection_provider(&self) -> String {
        self.config
            .learning
            .reflection_provider
            .clone()
            .unwrap_or_else(|| self.config.default_model.provider.clone())
    }

    fn checkpoint_provider(&self) -> String {
        self.config
            .checkpoints
            .provider
            .clone()
            .unwrap_or_else(|| self.config.default_model.provider.clone())
    }

    pub fn resolve_learning_reflection_model(&self) -> String {
        let provider = self.learning_reflection_provider();
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        super::normalize_background_model_for_provider(
            &catalog,
            &provider,
            &self.config.learning.reflection_model,
        )
    }

    pub fn resolve_learning_reflection_fallbacks(&self) -> Vec<String> {
        let provider = self.learning_reflection_provider();
        let primary = self.resolve_learning_reflection_model();
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        super::normalize_background_fallbacks_for_provider(
            &catalog,
            &provider,
            &primary,
            &self.config.learning.fallback_models,
        )
    }

    pub fn resolve_checkpoint_model(&self) -> String {
        let provider = self.checkpoint_provider();
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        super::normalize_background_model_for_provider(
            &catalog,
            &provider,
            &self.config.checkpoints.model,
        )
    }

    /// The V2 workflow proposer follows the effective user-selected model.
    /// It deliberately ignores the legacy `[skills]` proposer override and
    /// fallback list so a draft can never be attributed to another model.
    pub fn workflow_learning_active_model(&self) -> ActiveModelIdentity {
        let configured = self.effective_default_model();
        ActiveModelIdentity {
            provider: configured.provider.clone(),
            model: captain_runtime::agent_loop::strip_provider_prefix(
                &configured.model,
                &configured.provider,
            ),
        }
    }

    /// Build a strict workflow-learning proposer bound to that exact model.
    /// Driver initialization errors are returned; no no-op or fallback model
    /// is substituted.
    pub fn build_workflow_learning_proposer(&self) -> Result<WorkflowDraftProposer, String> {
        let configured = self.effective_default_model();
        let identity = self.workflow_learning_active_model();
        let api_key = if configured.api_key_env.is_empty() {
            self.resolve_credential(&self.config.resolve_api_key_env(&configured.provider))
        } else {
            self.resolve_credential(&configured.api_key_env)
        };
        let driver_config = captain_runtime::llm_driver::DriverConfig {
            provider: configured.provider.clone(),
            api_key,
            base_url: configured
                .base_url
                .clone()
                .or_else(|| self.lookup_provider_url(&configured.provider)),
            skip_permissions: true,
        };
        let driver = captain_runtime::drivers::create_driver_with_quota_observer(
            &driver_config,
            self.quota_observer_for(&configured.provider),
        )
        .map_err(|error| {
            format!(
                "active workflow proposer driver {}:{} unavailable: {error}",
                identity.provider, identity.model
            )
        })?;
        let completer: Arc<dyn ReflectionCompleter> = Arc::new(LlmDriverCompleter {
            driver,
            max_tokens: 32_768,
            temperature: 0.1,
        });
        info!(
            provider = %identity.provider,
            model = %identity.model,
            "workflow learning V2 proposer bound to effective active model without fallback"
        );
        Ok(WorkflowDraftProposer::new(
            completer,
            identity,
            Duration::from_secs(self.config.skills.reflection_timeout_secs.max(1)),
            self.config.language.clone(),
        ))
    }

    /// Build the reflection completer for v3.12d. Uses
    /// `learning.reflection_provider` when set, otherwise falls back to
    /// `default_model.provider`. On driver init failure returns
    /// NoopCompleter so the rest of the pipeline stays observable.
    ///
    /// R.2.2 — exposed publicly so the API server can wire the goal
    /// reflection cron at boot using the same provider/model the
    /// existing memory-reflection pipeline uses.
    pub fn build_reflection_completer(&self) -> Arc<dyn ReflectionCompleter> {
        let provider = self.learning_reflection_provider();
        let env_var = self
            .config
            .learning
            .reflection_api_key_env
            .clone()
            .unwrap_or_else(|| self.config.resolve_api_key_env(&provider));
        let api_key = std::env::var(&env_var).ok();
        let resolved_model = self.resolve_learning_reflection_model();
        let driver_config = captain_runtime::llm_driver::DriverConfig {
            provider: provider.clone(),
            api_key,
            base_url: self.lookup_provider_url(&provider),
            skip_permissions: true,
        };
        match captain_runtime::drivers::create_driver(&driver_config) {
            Ok(driver) => {
                info!(
                    provider = %provider,
                    configured_model = %self.config.learning.reflection_model,
                    model = %resolved_model,
                    env_var = %env_var,
                    "v3.12d reflection completer: LlmDriverCompleter active"
                );
                Arc::new(LlmDriverCompleter::new(driver))
            }
            Err(e) => {
                warn!(
                    provider = %provider,
                    error = %e,
                    "v3.12d reflection completer: driver init failed — falling back to NoopCompleter"
                );
                Arc::new(NoopCompleter)
            }
        }
    }

    pub(crate) fn build_checkpoint_completer(&self) -> Arc<dyn ReflectionCompleter> {
        let provider = self.checkpoint_provider();
        let env_var = self
            .config
            .checkpoints
            .api_key_env
            .clone()
            .unwrap_or_else(|| self.config.resolve_api_key_env(&provider));
        let api_key = std::env::var(&env_var).ok();
        let resolved_model = self.resolve_checkpoint_model();
        let driver_config = captain_runtime::llm_driver::DriverConfig {
            provider: provider.clone(),
            api_key,
            base_url: self.lookup_provider_url(&provider),
            skip_permissions: true,
        };
        match captain_runtime::drivers::create_driver(&driver_config) {
            Ok(driver) => {
                info!(
                    provider = %provider,
                    configured_model = %self.config.checkpoints.model,
                    model = %resolved_model,
                    "session checkpoint summarizer: LlmDriverCompleter active"
                );
                Arc::new(LlmDriverCompleter::new(driver))
            }
            Err(e) => {
                warn!(
                    provider = %provider,
                    error = %e,
                    "session checkpoint summarizer: driver init failed — falling back to NoopCompleter"
                );
                Arc::new(NoopCompleter)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;

    #[test]
    fn learning_reflection_provider_override_remaps_legacy_codex_background_models() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-reflection-model-test");
        let mut config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        config.default_model.provider = "anthropic".to_string();
        config.learning.reflection_provider = Some("codex".to_string());
        config.learning.reflection_model = "gpt-5.3-codex-spark".to_string();
        config.learning.fallback_models = vec![
            "claude-sonnet-4-6".to_string(),
            "codex/gpt-5.3-codex".to_string(),
        ];
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        assert_eq!(kernel.resolve_learning_reflection_model(), "gpt-5.5");
        assert!(kernel.resolve_learning_reflection_fallbacks().is_empty());
        kernel.shutdown();
    }

    #[test]
    fn workflow_learning_uses_effective_default_not_legacy_proposer_override() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-workflow-proposer-model-test");
        let mut config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        config.default_model.provider = "codex".to_string();
        config.default_model.model = "codex/gpt-5.6-sol".to_string();
        config.skills = toml::from_str(
            r#"
            reflection_provider = "anthropic"
            proposer_model = "claude-haiku-4.5"
            fallback_models = ["claude-sonnet-4.6"]
            "#,
        )
        .expect("legacy skills keys remain harmless during deserialization");
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        assert_eq!(
            kernel.workflow_learning_active_model(),
            ActiveModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            }
        );
        kernel.shutdown();
    }
}
