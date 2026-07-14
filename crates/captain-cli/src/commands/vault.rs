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
