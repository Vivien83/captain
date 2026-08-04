use zeroize::Zeroizing;

use crate::{captain_home, prompt_secret, ui};

pub(crate) fn cmd_vault_init() {
    let home = captain_home();
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);

    match vault.init() {
        Ok(()) => ui::success("Credential vault initialized."),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_vault_set(key: &str) {
    let home = captain_home();
    if let Err(error) = ensure_vault_key_is_local(&home, key) {
        ui::error(&error);
        std::process::exit(2);
    }
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error("Vault not initialized. Run: captain vault init");
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    let value = prompt_secret(&format!("Enter value for {key}: "));
    if value.is_empty() {
        ui::error("Empty value — not stored.");
        std::process::exit(1);
    }

    match vault.set(key.to_string(), Zeroizing::new(value)) {
        Ok(()) => ui::success(&format!("Stored '{key}' in vault.")),
        Err(e) => {
            ui::error(&format!("Failed to store: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_vault_sources(json: bool) {
    let home = captain_home();
    let config_path =
        home.join(captain_extensions::external_secret_sources::SECRET_SOURCES_FILENAME);
    let sources = load_external_sources(&home);
    let statuses = sources.statuses();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "count": statuses.len(),
                "sources": statuses,
            }))
            .unwrap_or_default()
        );
        return;
    }
    if statuses.is_empty() {
        println!("No external secret sources configured.");
        ui::hint(&format!(
            "Create {} to reference mounted secret files.",
            config_path.display()
        ));
        return;
    }

    println!("{:<32} {:<8} {:<10} DETAIL", "KEY", "TYPE", "STATUS");
    println!("{}", "-".repeat(76));
    for status in statuses {
        let state = if status.ready { "ready" } else { "error" };
        let detail = status
            .error_code
            .or(status.warning_code)
            .unwrap_or_else(|| "live rotation".to_string());
        println!(
            "{:<32} {:<8} {:<10} {}",
            status.key, status.source_type, state, detail
        );
    }
}

pub(crate) fn cmd_vault_list() {
    let home = captain_home();
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        println!("Vault not initialized. Run: captain vault init");
        return;
    }
    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    let keys = vault.list_keys();
    if keys.is_empty() {
        println!("Vault is empty.");
    } else {
        println!("Stored credentials ({}):", keys.len());
        for key in keys {
            println!("  {key}");
        }
    }
}

pub(crate) fn cmd_vault_remove(key: &str) {
    let home = captain_home();
    if let Err(error) = ensure_vault_key_is_local(&home, key) {
        ui::error(&error);
        std::process::exit(2);
    }
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error("Vault not initialized.");
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        std::process::exit(1);
    }

    match vault.remove(key) {
        Ok(true) => ui::success(&format!("Removed '{key}' from vault.")),
        Ok(false) => println!("Key '{key}' not found in vault."),
        Err(e) => {
            ui::error(&format!("Failed to remove: {e}"));
            std::process::exit(1);
        }
    }
}

fn ensure_vault_key_is_local(home: &std::path::Path, key: &str) -> Result<(), String> {
    if captain_kernel::gmail_persistence::is_managed_gmail_vault_key(key) {
        Err(format!(
            "'{key}' is managed by `captain email`; use email connect or disconnect instead."
        ))
    } else if load_external_sources(home).is_configured(key) {
        Err(format!(
            "'{key}' is managed by secret-sources.toml; change the external mapping or file instead."
        ))
    } else {
        Ok(())
    }
}

fn load_external_sources(
    home: &std::path::Path,
) -> captain_extensions::external_secret_sources::ExternalSecretSources {
    let path = home.join(captain_extensions::external_secret_sources::SECRET_SOURCES_FILENAME);
    captain_extensions::external_secret_sources::ExternalSecretSources::load(&path).unwrap_or_else(
        |error| {
            ui::error(&format!("Secret sources unavailable: {error}"));
            std::process::exit(1);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::ensure_vault_key_is_local;

    #[test]
    fn vault_mutations_refuse_externally_managed_keys() {
        let home = tempfile::tempdir().unwrap();
        let mounted = home.path().join("mounted-secret");
        std::fs::write(&mounted, "value\n").unwrap();
        std::fs::write(
            home.path().join("secret-sources.toml"),
            format!(
                "version = 1\n[sources.TEST_EXTERNAL_VAULT]\ntype = \"file\"\npath = {:?}\n",
                mounted.display().to_string()
            ),
        )
        .unwrap();

        let error = ensure_vault_key_is_local(home.path(), "TEST_EXTERNAL_VAULT").unwrap_err();
        assert!(error.contains("managed by secret-sources.toml"));
        assert!(ensure_vault_key_is_local(home.path(), "LOCAL_VAULT_KEY").is_ok());
        assert!(ensure_vault_key_is_local(home.path(), "CAPTAIN_GMAIL_TOKEN_DEADBEEF").is_err());
        assert!(ensure_vault_key_is_local(home.path(), "CAPTAIN_GMAIL_CLIENT_DEADBEEF").is_err());
    }
}
