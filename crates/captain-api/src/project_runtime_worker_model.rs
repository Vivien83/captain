use crate::project_runtime_prompt_context::runtime_worker_system_prompt_for_tools;
use captain_types::agent::ModelConfig;
use captain_types::config::DefaultModelConfig;

pub(crate) fn runtime_worker_model_config<F>(
    default_model: DefaultModelConfig,
    phase: &str,
    authorized_tools: &[String],
    model_exists: F,
) -> ModelConfig
where
    F: Fn(&str, &str) -> bool,
{
    let model_name = select_runtime_worker_model(
        &default_model.provider,
        &default_model.model,
        phase,
        model_exists,
    );
    ModelConfig {
        provider: default_model.provider,
        model: model_name,
        max_tokens: if matches!(phase, "build" | "verify") {
            8192
        } else {
            4096
        },
        temperature: if phase == "think" { 0.35 } else { 0.2 },
        system_prompt: runtime_worker_system_prompt_for_tools(phase, authorized_tools),
        api_key_env: if default_model.api_key_env.trim().is_empty() {
            None
        } else {
            Some(default_model.api_key_env)
        },
        base_url: default_model.base_url,
    }
}

pub(crate) fn select_runtime_worker_model<F>(
    provider: &str,
    default_model: &str,
    phase: &str,
    model_exists: F,
) -> String
where
    F: Fn(&str, &str) -> bool,
{
    if !matches!(phase, "observe" | "think" | "learn") {
        return default_model.to_string();
    }
    for candidate in runtime_worker_model_candidates(provider) {
        if model_exists(provider, candidate) {
            return (*candidate).to_string();
        }
    }
    default_model.to_string()
}

fn runtime_worker_model_candidates(provider: &str) -> &'static [&'static str] {
    match provider.to_ascii_lowercase().as_str() {
        "codex" => &["gpt-5.4-mini", "gpt-5.3-codex-spark"],
        "openai" => &["gpt-5.1-mini", "gpt-4.1-mini"],
        "anthropic" => &["claude-haiku-4.5", "claude-3-5-haiku-latest"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_model(provider: &str, model: &str) -> DefaultModelConfig {
        DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: " ".to_string(),
            base_url: Some("https://models.example.test".to_string()),
        }
    }

    #[test]
    fn selector_uses_first_available_light_model_for_read_phases() {
        let selected = select_runtime_worker_model("Codex", "gpt-5.4", "observe", |_, model| {
            model == "gpt-5.3-codex-spark"
        });

        assert_eq!(selected, "gpt-5.3-codex-spark");
    }

    #[test]
    fn selector_keeps_default_for_heavy_phases() {
        let selected = select_runtime_worker_model("openai", "gpt-5.1", "build", |_, _| true);

        assert_eq!(selected, "gpt-5.1");
    }

    #[test]
    fn selector_falls_back_when_provider_or_candidates_missing() {
        assert_eq!(
            select_runtime_worker_model("local", "llama-prod", "think", |_, _| true),
            "llama-prod"
        );
        assert_eq!(
            select_runtime_worker_model("anthropic", "claude-sonnet", "learn", |_, _| false),
            "claude-sonnet"
        );
    }

    #[test]
    fn model_config_sets_phase_budget_temperature_and_prompt() {
        let config = runtime_worker_model_config(
            default_model("openai", "gpt-5.1"),
            "verify",
            &["cargo_test".to_string()],
            |_, _| false,
        );

        assert_eq!(config.provider, "openai");
        assert_eq!(config.model, "gpt-5.1");
        assert_eq!(config.max_tokens, 8192);
        assert_eq!(config.temperature, 0.2);
        assert!(config.system_prompt.contains("verify phase"));
        assert!(config.system_prompt.contains("cargo_test"));
        assert!(config.api_key_env.is_none());
        assert_eq!(
            config.base_url.as_deref(),
            Some("https://models.example.test")
        );
    }

    #[test]
    fn model_config_uses_think_temperature_and_non_blank_key_env() {
        let mut default = default_model("anthropic", "claude-sonnet");
        default.api_key_env = "ANTHROPIC_API_KEY".to_string();

        let config = runtime_worker_model_config(default, "think", &[], |provider, model| {
            provider.eq_ignore_ascii_case("anthropic") && model == "claude-haiku-4.5"
        });

        assert_eq!(config.model, "claude-haiku-4.5");
        assert_eq!(config.max_tokens, 4096);
        assert_eq!(config.temperature, 0.35);
        assert_eq!(config.api_key_env.as_deref(), Some("ANTHROPIC_API_KEY"));
    }
}
