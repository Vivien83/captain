use super::CaptainKernel;
use captain_runtime::reflection_job::{LlmDriverCompleter, NoopCompleter, ReflectionCompleter};
use captain_runtime::skill_diff::SkillDiffConfig;
use std::sync::Arc;
use tracing::{info, warn};

impl CaptainKernel {
    /// Resolved skills generated directory. `generated_dir` is
    /// relative to `home_dir` unless absolute.
    pub(crate) fn skills_generated_root(&self) -> std::path::PathBuf {
        let gd = std::path::Path::new(&self.config.skills.generated_dir);
        if gd.is_absolute() {
            gd.to_path_buf()
        } else {
            self.config.home_dir.join(gd)
        }
    }

    pub(crate) fn skill_diff_config(&self) -> SkillDiffConfig {
        let mut roots = Vec::new();
        let user_skills = self.config.home_dir.join("skills");
        roots.push(user_skills);
        let generated = self.skills_generated_root();
        if !roots.contains(&generated) {
            roots.push(generated);
        }
        SkillDiffConfig::new(roots)
    }

    fn skills_reflection_provider(&self) -> String {
        self.config
            .skills
            .reflection_provider
            .clone()
            .unwrap_or_else(|| self.config.default_model.provider.clone())
    }

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

    pub(crate) fn resolve_skills_proposer_model(&self) -> String {
        let provider = self.skills_reflection_provider();
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        super::normalize_background_model_for_provider(
            &catalog,
            &provider,
            &self.config.skills.proposer_model,
        )
    }

    pub(crate) fn resolve_skills_proposer_fallbacks(&self) -> Vec<String> {
        let provider = self.skills_reflection_provider();
        let primary = self.resolve_skills_proposer_model();
        let catalog = self.model_catalog.read().unwrap_or_else(|e| e.into_inner());
        super::normalize_background_fallbacks_for_provider(
            &catalog,
            &provider,
            &primary,
            &self.config.skills.fallback_models,
        )
    }

    /// Build the proposer completer for v3.13b. Same shape as the
    /// reflection completer: fall back to NoopCompleter on init
    /// failure so the detector + policy stages stay observable.
    pub(crate) fn build_proposer_completer(&self) -> Arc<dyn ReflectionCompleter> {
        let provider = self.skills_reflection_provider();
        let env_var = self
            .config
            .skills
            .reflection_api_key_env
            .clone()
            .unwrap_or_else(|| self.config.resolve_api_key_env(&provider));
        let api_key = std::env::var(&env_var).ok();
        let resolved_model = self.resolve_skills_proposer_model();
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
                    configured_model = %self.config.skills.proposer_model,
                    model = %resolved_model,
                    "v3.13b skill proposer: LlmDriverCompleter active"
                );
                Arc::new(LlmDriverCompleter::new(driver))
            }
            Err(e) => {
                warn!(
                    provider = %provider,
                    error = %e,
                    "v3.13b skill proposer: driver init failed — falling back to Noop"
                );
                Arc::new(NoopCompleter)
            }
        }
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
    fn skill_diff_config_deduplicates_generated_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-skill-diff-root-test");
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            skills: captain_types::config::SkillsConfig {
                generated_dir: "skills".to_string(),
                ..Default::default()
            },
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");

        let diff_config = kernel.skill_diff_config();

        assert_eq!(diff_config.roots, vec![home_dir.join("skills")]);
        kernel.shutdown();
    }

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
}
