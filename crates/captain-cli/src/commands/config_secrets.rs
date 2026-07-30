use std::io::{self, Write};

use colored::Colorize;

use crate::{
    captain_home, dotenv, production_credential_resolver_at, prompt_secret, test_api_key_value, ui,
};

pub(crate) fn cmd_config_set_key(provider: &str) {
    let env_var = provider_to_env_var(provider);
    let mut resolver = production_credential_resolver();
    if resolver.is_externally_managed(&env_var) {
        ui::error(&format!(
            "{env_var} is managed by secret-sources.toml; rotate the external file instead."
        ));
        std::process::exit(2);
    }

    let key = prompt_secret(&format!("  Paste your {provider} API key: "));
    if key.is_empty() {
        ui::error("No key provided. Cancelled.");
        return;
    }

    match resolver.store_credential(&env_var, &key) {
        Ok(()) => {
            ui::success(&format!("Saved {env_var} to ~/.captain/secrets.env"));
            print!("  Testing key... ");
            let _ = io::stdout().flush();
            if test_api_key_value(provider, &key) {
                println!("{}", "OK".bright_green());
            } else {
                println!("{}", "could not verify (may still work)".bright_yellow());
            }
        }
        Err(e) => {
            ui::error(&format!("Failed to save key: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_delete_key(provider: &str) {
    let env_var = provider_to_env_var(provider);
    let resolver = production_credential_resolver();
    if resolver.is_externally_managed(&env_var) {
        ui::error(&format!(
            "{env_var} is managed by secret-sources.toml; remove that mapping to stop using it."
        ));
        std::process::exit(2);
    }
    remove_from_vault_best_effort(&env_var);

    let secrets_result = dotenv::remove_secret_key(&env_var);
    let env_result = dotenv::remove_env_key(&env_var);

    match (secrets_result, env_result) {
        (Ok(()), Ok(())) => ui::success(&format!(
            "Removed {env_var} from ~/.captain/secrets.env and .env"
        )),
        (Ok(()), Err(e)) => ui::warn_with_fix(
            &format!("Removed from secrets.env, but .env cleanup failed: {e}"),
            "Remove the legacy .env entry manually if it exists",
        ),
        (Err(e), Ok(())) => ui::warn_with_fix(
            &format!("Removed from .env, but secrets.env cleanup failed: {e}"),
            "Remove the secrets.env entry manually if it exists",
        ),
        (Err(secrets_e), Err(env_e)) => {
            ui::error(&format!(
                "Failed to remove key: secrets.env={secrets_e}; .env={env_e}"
            ));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_test_key(provider: &str) {
    let env_var = provider_to_env_var(provider);
    let resolver = production_credential_resolver();
    let Some(key) = resolver.resolve(&env_var) else {
        ui::error(&format!("{env_var} not set"));
        ui::hint(&format!("Set it: captain config set-key {provider}"));
        std::process::exit(1);
    };

    print!("  Testing {provider} ({env_var})... ");
    let _ = io::stdout().flush();
    if test_api_key_value(provider, &key) {
        println!("{}", "OK".bright_green());
    } else {
        println!("{}", "FAILED (401/403)".bright_red());
        ui::hint(&format!("Update key: captain config set-key {provider}"));
        std::process::exit(1);
    }
}

fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase().replace('-', "_")),
    }
}

fn production_credential_resolver() -> captain_extensions::credentials::CredentialResolver {
    let home = captain_home();
    production_credential_resolver_at(&home).unwrap_or_else(|error| {
        ui::error(&format!("Secret sources unavailable: {error}"));
        std::process::exit(1);
    })
}

fn remove_from_vault_best_effort(env_var: &str) {
    let home = captain_home();
    let vault_path = home.join("vault.enc");
    if !vault_path.exists() {
        return;
    }
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);
    if vault.unlock().is_ok() {
        let _ = vault.remove(env_var);
    }
}
