use std::io::{self, Write};
use std::path::Path;

use colored::Colorize;

use super::auth::cmd_login_codex;
use super::setup_support::{setup_config_string, setup_env_or_answer_any, setup_read_config_value};
use crate::{
    check_ollama_available, codex_auth_available, detect_best_provider, dotenv, prompt_input,
    prompt_secret, provider_list, test_api_key, ui, write_default_model_config,
};

#[derive(Clone, Copy)]
pub(crate) struct SetupProvider {
    pub(crate) id: &'static str,
    pub(crate) display: &'static str,
    /// `None` when no env-key is needed (Codex OAuth, claude-code, ollama).
    pub(crate) env_var: Option<&'static str>,
    pub(crate) default_model: &'static str,
    pub(crate) hint: &'static str,
    /// `true` triggers the OAuth device-code flow instead of an API-key prompt.
    pub(crate) oauth: bool,
}

const SETUP_PROVIDERS: &[SetupProvider] = &[
    SetupProvider {
        id: "codex",
        display: "Codex (ChatGPT Plus/Pro abonnement)",
        env_var: None,
        default_model: "gpt-5.5",
        hint: "OAuth, pas de clé API",
        oauth: true,
    },
    SetupProvider {
        id: "anthropic",
        display: "Anthropic (Claude)",
        env_var: Some("ANTHROPIC_API_KEY"),
        default_model: "claude-sonnet-4-20250514",
        hint: "qualité de référence",
        oauth: false,
    },
    SetupProvider {
        id: "groq",
        display: "Groq",
        env_var: Some("GROQ_API_KEY"),
        default_model: "llama-3.3-70b-versatile",
        hint: "free tier généreux",
        oauth: false,
    },
    SetupProvider {
        id: "openai",
        display: "OpenAI",
        env_var: Some("OPENAI_API_KEY"),
        default_model: "gpt-4o",
        hint: "API officielle",
        oauth: false,
    },
    SetupProvider {
        id: "openrouter",
        display: "OpenRouter",
        env_var: Some("OPENROUTER_API_KEY"),
        default_model: "openrouter/google/gemini-2.5-flash",
        hint: "tous les modèles, 1 clé",
        oauth: false,
    },
    SetupProvider {
        id: "ollama",
        display: "Ollama (local)",
        env_var: None,
        default_model: "llama3.2",
        hint: "100% local, pas de clé",
        oauth: false,
    },
    SetupProvider {
        id: "claude-code",
        display: "Claude Code (CLI)",
        env_var: None,
        default_model: "claude-sonnet-4-20250514",
        hint: "utilise le CLI Claude Code installé",
        oauth: false,
    },
];

pub(crate) struct SetupBaseModel {
    pub(crate) provider: &'static SetupProvider,
    pub(crate) model: String,
    pub(crate) ready: bool,
}

pub(crate) struct NonInteractiveBaseModel {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) ready: bool,
}

pub(crate) fn setup_configure_base_model(captain_dir: &Path) -> SetupBaseModel {
    ui::section("Configuration de base");
    println!("  Objectif : Captain doit être prêt à répondre avant les préférences.");
    let existing_config = setup_read_config_value(captain_dir);
    let existing_provider = setup_config_string(existing_config.as_ref(), "default_model.provider");
    let existing_model = setup_config_string(existing_config.as_ref(), "default_model.model");

    loop {
        ui::blank();
        let provider = setup_pick_provider(existing_provider.as_deref());
        let env_key_label = provider.env_var.unwrap_or("");
        let selected_model = setup_select_model_for_provider(
            existing_provider.as_deref(),
            existing_model.as_deref(),
            provider,
        );

        let result = if provider.oauth {
            setup_configure_oauth_provider(captain_dir, provider, &selected_model, env_key_label)
        } else if let Some(env_var) = provider.env_var {
            setup_configure_api_key_provider(
                captain_dir,
                provider,
                &selected_model,
                env_key_label,
                env_var,
            )
        } else {
            setup_configure_local_provider(captain_dir, provider, &selected_model, env_key_label)
        };

        if let Some(base_model) = result {
            return base_model;
        }
    }
}

fn setup_select_model_for_provider(
    existing_provider: Option<&str>,
    existing_model: Option<&str>,
    provider: &SetupProvider,
) -> String {
    if existing_provider == Some(provider.id) {
        return existing_model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(provider.default_model)
            .to_string();
    }
    provider.default_model.to_string()
}

fn setup_configure_oauth_provider(
    captain_dir: &Path,
    provider: &'static SetupProvider,
    selected_model: &str,
    env_key_label: &str,
) -> Option<SetupBaseModel> {
    if codex_auth_available() {
        ui::success("Codex OAuth déjà configuré");
    } else {
        let answer = prompt_input("  Connecter Codex maintenant ? [Y/n] ");
        if answer.is_empty() || answer.starts_with(['y', 'Y']) {
            cmd_login_codex(false);
        } else if setup_choose_another_provider(provider.display) {
            return None;
        } else {
            return Some(setup_write_base_model(
                captain_dir,
                provider,
                selected_model,
                env_key_label,
                false,
            ));
        }
    }

    if codex_auth_available() {
        return Some(setup_write_base_model(
            captain_dir,
            provider,
            selected_model,
            env_key_label,
            true,
        ));
    }

    ui::check_fail("Codex n'est pas encore authentifié");
    if setup_choose_another_provider(provider.display) {
        None
    } else {
        Some(setup_write_base_model(
            captain_dir,
            provider,
            selected_model,
            env_key_label,
            false,
        ))
    }
}

fn setup_configure_api_key_provider(
    captain_dir: &Path,
    provider: &'static SetupProvider,
    selected_model: &str,
    env_key_label: &str,
    env_var: &str,
) -> Option<SetupBaseModel> {
    if setup_env_key_present(env_var) {
        ui::success(&format!("{env_var} déjà présent"));
    } else if !setup_prompt_and_save_api_key(env_var) {
        if setup_choose_another_provider(provider.display) {
            return None;
        }
        return Some(setup_write_base_model(
            captain_dir,
            provider,
            selected_model,
            env_key_label,
            false,
        ));
    }

    print!("  Vérification du provider... ");
    let _ = io::stdout().flush();
    if test_api_key(provider.id, env_var) {
        println!("{}", "OK".bright_green());
        return Some(setup_write_base_model(
            captain_dir,
            provider,
            selected_model,
            env_key_label,
            true,
        ));
    }

    println!("{}", "échec".bright_red());
    ui::check_fail("La clé semble refusée par le provider");
    if setup_choose_another_provider(provider.display) {
        None
    } else {
        Some(setup_write_base_model(
            captain_dir,
            provider,
            selected_model,
            env_key_label,
            false,
        ))
    }
}

fn setup_env_key_present(env_var: &str) -> bool {
    std::env::var(env_var)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .is_some()
}

fn setup_prompt_and_save_api_key(env_var: &str) -> bool {
    ui::blank();
    println!("  Colle ta clé {} :", env_var.bold());
    let key = prompt_secret("  > ");
    if key.is_empty() {
        ui::check_fail(&format!("{env_var} est requis pour rendre Captain prêt"));
        return false;
    }

    match dotenv::save_secret_key(env_var, &key) {
        Ok(()) => {
            ui::success(&format!(
                "{env_var} sauvegardée dans ~/.captain/secrets.env"
            ));
            true
        }
        Err(e) => {
            ui::error_with_fix(
                &format!("Sauvegarde secrets.env : {e}"),
                &format!("Définis manuellement : export {env_var}=..."),
            );
            std::process::exit(1);
        }
    }
}

fn setup_configure_local_provider(
    captain_dir: &Path,
    provider: &'static SetupProvider,
    selected_model: &str,
    env_key_label: &str,
) -> Option<SetupBaseModel> {
    let ready = setup_local_provider_ready(provider);
    if !ready && setup_choose_another_provider(provider.display) {
        return None;
    }
    Some(setup_write_base_model(
        captain_dir,
        provider,
        selected_model,
        env_key_label,
        ready,
    ))
}

fn setup_local_provider_ready(provider: &SetupProvider) -> bool {
    match provider.id {
        "ollama" => {
            let ok = check_ollama_available();
            if !ok {
                ui::check_fail("Ollama ne répond pas sur localhost:11434");
                ui::hint("Installe-le puis lance : ollama pull llama3.2");
            }
            ok
        }
        "claude-code" => {
            let ok = captain_runtime::drivers::claude_code::claude_code_available();
            if !ok {
                ui::check_fail("Claude Code CLI non disponible");
                ui::hint("Installe et connecte Claude Code, ou choisis un autre provider");
            }
            ok
        }
        _ => true,
    }
}

fn setup_write_base_model(
    captain_dir: &Path,
    provider: &'static SetupProvider,
    selected_model: &str,
    env_key_label: &str,
    ready: bool,
) -> SetupBaseModel {
    setup_write_default_model_or_exit(captain_dir, provider.id, selected_model, env_key_label);
    SetupBaseModel {
        provider,
        model: selected_model.to_string(),
        ready,
    }
}

pub(crate) fn setup_configure_base_model_non_interactive(
    captain_dir: &Path,
    answers: Option<&toml::Value>,
) -> NonInteractiveBaseModel {
    let detected = detect_best_provider();
    let provider = setup_noninteractive_provider(answers, detected.0);
    let (default_env, default_model) = setup_provider_defaults(&provider);
    let model =
        setup_noninteractive_model(answers, &provider, detected.0, detected.2, default_model);
    let api_key_env =
        setup_noninteractive_api_key_env(answers, &provider, detected.0, detected.1, default_env);

    setup_persist_noninteractive_api_key(&api_key_env, answers);
    setup_write_noninteractive_default_model_or_exit(captain_dir, &provider, &model, &api_key_env);

    let ready = setup_noninteractive_provider_ready(&provider, &api_key_env);

    NonInteractiveBaseModel {
        provider,
        model,
        ready,
    }
}

fn setup_noninteractive_provider(answers: Option<&toml::Value>, detected_provider: &str) -> String {
    setup_env_or_answer_any(
        "CAPTAIN_PROVIDER",
        answers,
        &["provider", "default_model.provider"],
    )
    .unwrap_or_else(|| detected_provider.to_string())
}

fn setup_noninteractive_model(
    answers: Option<&toml::Value>,
    provider: &str,
    detected_provider: &str,
    detected_model: &str,
    default_model: &str,
) -> String {
    setup_env_or_answer_any("CAPTAIN_MODEL", answers, &["model", "default_model.model"])
        .unwrap_or_else(|| {
            setup_detected_or_default(provider, detected_provider, detected_model, default_model)
        })
}

fn setup_noninteractive_api_key_env(
    answers: Option<&toml::Value>,
    provider: &str,
    detected_provider: &str,
    detected_env: &str,
    default_env: &str,
) -> String {
    setup_env_or_answer_any(
        "CAPTAIN_API_KEY_ENV",
        answers,
        &["api_key_env", "default_model.api_key_env"],
    )
    .unwrap_or_else(|| {
        setup_detected_or_default(provider, detected_provider, detected_env, default_env)
    })
}

fn setup_detected_or_default(
    provider: &str,
    detected_provider: &str,
    detected_value: &str,
    default_value: &str,
) -> String {
    if detected_provider == provider {
        detected_value.to_string()
    } else {
        default_value.to_string()
    }
}

fn setup_persist_noninteractive_api_key(api_key_env: &str, answers: Option<&toml::Value>) {
    if api_key_env.is_empty() {
        return;
    }

    if let Some(api_key) = setup_env_or_answer_any(
        "CAPTAIN_API_KEY",
        answers,
        &["api_key", "default_model.api_key"],
    ) {
        if let Err(e) = dotenv::save_secret_key(api_key_env, &api_key) {
            ui::warn_with_fix(
                &format!("Failed to persist {api_key_env}: {e}"),
                "Set the env var manually before starting Captain",
            );
        }
    } else if let Ok(api_key) = std::env::var(api_key_env) {
        if !api_key.trim().is_empty() {
            let _ = dotenv::save_secret_key(api_key_env, &api_key);
        }
    }
}

fn setup_write_noninteractive_default_model_or_exit(
    captain_dir: &Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    if let Err(e) = write_default_model_config(captain_dir, provider, model, api_key_env) {
        ui::error_with_fix(
            &format!("Écriture config provider : {e}"),
            "Vérifie les permissions de ~/.captain/config.toml",
        );
        std::process::exit(1);
    }
}

fn setup_noninteractive_provider_ready(provider: &str, api_key_env: &str) -> bool {
    match provider {
        "codex" => codex_auth_available(),
        "ollama" => check_ollama_available(),
        "claude-code" => captain_runtime::drivers::claude_code::claude_code_available(),
        _ if !api_key_env.is_empty() => setup_env_key_present(api_key_env),
        _ => true,
    }
}

fn setup_pick_provider(default_provider: Option<&str>) -> &'static SetupProvider {
    ui::section("Provider LLM");
    let default_idx = default_provider
        .and_then(|id| SETUP_PROVIDERS.iter().position(|p| p.id == id))
        .unwrap_or(0);
    for (i, p) in SETUP_PROVIDERS.iter().enumerate() {
        let hint = if p.hint.is_empty() {
            String::new()
        } else {
            format!(" — {}", p.hint)
        };
        println!("    {}. {:<40}{}", i + 1, p.display, hint.dimmed());
    }
    let answer = prompt_input(&format!("  Choix [{}] : ", default_idx + 1));
    let idx = answer
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=SETUP_PROVIDERS.len()).contains(n))
        .map(|n| n - 1)
        .unwrap_or(default_idx);
    &SETUP_PROVIDERS[idx]
}

fn setup_provider_defaults(provider: &str) -> (&'static str, &'static str) {
    if let Some(def) = SETUP_PROVIDERS.iter().find(|p| p.id == provider) {
        return (def.env_var.unwrap_or(""), def.default_model);
    }
    if let Some((_, env, model, _)) = provider_list()
        .into_iter()
        .find(|(id, _, _, _)| *id == provider)
    {
        return (env, model);
    }
    ("", "gpt-5.5")
}

fn setup_choose_another_provider(display: &str) -> bool {
    let answer = prompt_input(&format!(
        "  {display} n'est pas prêt. Choisir un autre provider ? [Y/n] "
    ));
    answer.is_empty() || answer.starts_with(['y', 'Y'])
}

fn setup_write_default_model_or_exit(
    captain_dir: &Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    if let Err(e) = write_default_model_config(captain_dir, provider, model, api_key_env) {
        ui::error_with_fix(
            &format!("Écriture config provider : {e}"),
            "Vérifie les permissions de ~/.captain/config.toml",
        );
        std::process::exit(1);
    }
    ui::success(&format!("Provider par défaut : {provider}/{model}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str) -> &'static SetupProvider {
        SETUP_PROVIDERS
            .iter()
            .find(|provider| provider.id == id)
            .expect("setup provider exists")
    }

    #[test]
    fn setup_select_model_reuses_existing_model_for_same_provider() {
        let selected =
            setup_select_model_for_provider(Some("codex"), Some("gpt-custom"), provider("codex"));

        assert_eq!(selected, "gpt-custom");
    }

    #[test]
    fn setup_select_model_uses_default_for_empty_existing_model() {
        let selected =
            setup_select_model_for_provider(Some("codex"), Some("  "), provider("codex"));

        assert_eq!(selected, provider("codex").default_model);
    }

    #[test]
    fn setup_select_model_uses_provider_default_when_provider_changes() {
        let selected = setup_select_model_for_provider(
            Some("anthropic"),
            Some("claude-custom"),
            provider("openai"),
        );

        assert_eq!(selected, provider("openai").default_model);
    }

    #[test]
    fn setup_detected_or_default_keeps_detected_value_for_detected_provider() {
        let selected = setup_detected_or_default("codex", "codex", "gpt-detected", "gpt-default");

        assert_eq!(selected, "gpt-detected");
    }

    #[test]
    fn setup_detected_or_default_uses_default_when_provider_differs() {
        let selected = setup_detected_or_default("openai", "codex", "gpt-detected", "gpt-default");

        assert_eq!(selected, "gpt-default");
    }
}
