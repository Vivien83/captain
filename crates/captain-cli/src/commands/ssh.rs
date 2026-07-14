use std::io::{self, Write};

use zeroize::Zeroizing;

use crate::{captain_home, prompt_input, prompt_secret, ui};

/// Adapter wrapping `CredentialVault` so it implements `SshSecretStore`.
struct VaultStore<'a>(&'a mut captain_extensions::vault::CredentialVault);

impl captain_runtime::ssh_vault::SshSecretStore for VaultStore<'_> {
    fn get(&self, key: &str) -> Option<Zeroizing<String>> {
        self.0.get(key)
    }

    fn set(&mut self, key: String, value: Zeroizing<String>) -> Result<(), String> {
        self.0.set(key, value).map_err(|e| e.to_string())
    }

    fn remove(&mut self, key: &str) -> Result<bool, String> {
        self.0.remove(key).map_err(|e| e.to_string())
    }

    fn list_keys(&self) -> Vec<String> {
        self.0.list_keys().into_iter().map(String::from).collect()
    }
}

fn open_ssh_vault() -> Option<captain_extensions::vault::CredentialVault> {
    let home = captain_home();
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);
    if !vault.exists() {
        ui::error("Vault not initialized. Run: captain vault init");
        return None;
    }
    if let Err(e) = vault.unlock() {
        ui::error(&format!("Could not unlock vault: {e}"));
        return None;
    }
    Some(vault)
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    p.to_string()
}

pub(crate) fn cmd_ssh_add(name: &str) {
    use captain_runtime::ssh_vault as sv;

    let key_path = prompt_input("Path to private key file (e.g. ~/.ssh/id_ed25519): ");
    let expanded = expand_tilde(&key_path);
    let pem = match std::fs::read_to_string(&expanded) {
        Ok(c) => c,
        Err(e) => {
            ui::error(&format!("Cannot read '{expanded}': {e}"));
            std::process::exit(1);
        }
    };
    let passphrase_in = prompt_secret("Passphrase (Enter if none): ");
    let passphrase = if passphrase_in.is_empty() {
        None
    } else {
        Some(passphrase_in.as_str())
    };
    let fingerprint = match sv::fingerprint_of(&pem, passphrase) {
        Ok(fp) => fp,
        Err(e) => {
            ui::error(&format!("Invalid private key: {e}"));
            std::process::exit(1);
        }
    };
    let host = prompt_input("Host: ");
    if host.is_empty() {
        ui::error("Host is required.");
        std::process::exit(1);
    }
    let user = prompt_input("User: ");
    if user.is_empty() {
        ui::error("User is required.");
        std::process::exit(1);
    }
    let port_in = prompt_input("Port [22]: ");
    let port: u16 = if port_in.is_empty() {
        22
    } else {
        port_in.parse().unwrap_or(22)
    };

    let mut vault = match open_ssh_vault() {
        Some(v) => v,
        None => std::process::exit(1),
    };
    let mut store = VaultStore(&mut vault);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let key = sv::SshKey {
        name: name.to_string(),
        host,
        port,
        user,
        private_key: Zeroizing::new(pem),
        passphrase: passphrase.map(|p| Zeroizing::new(p.to_string())),
        fingerprint: fingerprint.clone(),
        added_at: now,
        last_used: None,
    };
    if let Err(e) = sv::save_ssh_key(&mut store, key) {
        ui::error(&format!("Failed to store: {e}"));
        std::process::exit(1);
    }

    let audit_dir = captain_home().join("audit");
    sv::audit_log(&audit_dir, "add", name, &fingerprint, true);
    ui::success(&format!(
        "Added SSH key '{name}' (fingerprint {fingerprint})."
    ));
}

pub(crate) fn cmd_ssh_list() {
    use captain_runtime::ssh_vault as sv;

    let mut vault = match open_ssh_vault() {
        Some(v) => v,
        None => std::process::exit(1),
    };
    let store = VaultStore(&mut vault);
    let names = sv::list_ssh_keys(&store);
    if names.is_empty() {
        println!("No SSH keys stored. Add one with: captain ssh add <name>");
        return;
    }
    let default = sv::get_default_ssh_key(&store);
    println!(
        "{:<16} {:<28} {:<16} {:<6} FINGERPRINT",
        "NAME", "HOST", "USER", "PORT"
    );
    for n in names {
        if let Some(k) = sv::load_ssh_key(&store, &n) {
            let marker = if default.as_deref() == Some(n.as_str()) {
                "★"
            } else {
                " "
            };
            println!(
                "{marker} {:<14} {:<28} {:<16} {:<6} {}",
                k.name, k.host, k.user, k.port, k.fingerprint
            );
        }
    }
}

pub(crate) fn cmd_ssh_test(name: &str) {
    use captain_runtime::ssh_vault as sv;
    use std::net::ToSocketAddrs;

    let mut vault = match open_ssh_vault() {
        Some(v) => v,
        None => std::process::exit(1),
    };
    let store = VaultStore(&mut vault);
    let key = match sv::load_ssh_key(&store, name) {
        Some(k) => k,
        None => {
            ui::error(&format!(
                "No SSH key named '{name}'. List with: captain ssh list"
            ));
            std::process::exit(1);
        }
    };

    if let Err(e) = sv::fingerprint_of(
        &key.private_key,
        key.passphrase.as_deref().map(|s| s.as_str()),
    ) {
        ui::error(&format!("Stored key fails to parse: {e}"));
        std::process::exit(1);
    }

    let addr = format!("{}:{}", key.host, key.port);
    print!("Connecting to {addr}…");
    io::stdout().flush().ok();
    let result = std::net::TcpStream::connect_timeout(
        &match addr.to_socket_addrs() {
            Ok(mut iter) => match iter.next() {
                Some(a) => a,
                None => {
                    println!(" FAIL (no address resolved)");
                    let audit_dir = captain_home().join("audit");
                    sv::audit_log(&audit_dir, "test", name, "no address resolved", false);
                    std::process::exit(1);
                }
            },
            Err(e) => {
                println!(" FAIL ({e})");
                let audit_dir = captain_home().join("audit");
                sv::audit_log(&audit_dir, "test", name, &e.to_string(), false);
                std::process::exit(1);
            }
        },
        std::time::Duration::from_secs(5),
    );
    let audit_dir = captain_home().join("audit");
    match result {
        Ok(_) => {
            println!(" OK");
            ui::success(
                "TCP reachable. Full SSH handshake test will arrive with Q.7 (tool_ssh_exec).",
            );
            sv::audit_log(&audit_dir, "test", name, "tcp ok", true);
        }
        Err(e) => {
            println!(" FAIL ({e})");
            sv::audit_log(&audit_dir, "test", name, &e.to_string(), false);
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_ssh_remove(name: &str) {
    use captain_runtime::ssh_vault as sv;

    let confirm = prompt_input(&format!("Remove SSH key '{name}'? [y/N]: "));
    if !matches!(confirm.to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Aborted.");
        return;
    }
    let mut vault = match open_ssh_vault() {
        Some(v) => v,
        None => std::process::exit(1),
    };
    let mut store = VaultStore(&mut vault);
    match sv::delete_ssh_key(&mut store, name) {
        Ok(true) => {
            let audit_dir = captain_home().join("audit");
            sv::audit_log(&audit_dir, "remove", name, "", true);
            ui::success(&format!("Removed SSH key '{name}'."));
        }
        Ok(false) => println!("No SSH key named '{name}'."),
        Err(e) => {
            ui::error(&format!("Failed to remove: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_ssh_use(name: &str) {
    use captain_runtime::ssh_vault as sv;

    let mut vault = match open_ssh_vault() {
        Some(v) => v,
        None => std::process::exit(1),
    };
    let mut store = VaultStore(&mut vault);
    match sv::set_default_ssh_key(&mut store, name) {
        Ok(()) => {
            let audit_dir = captain_home().join("audit");
            sv::audit_log(&audit_dir, "use", name, "set as default", true);
            ui::success(&format!("'{name}' is now the default SSH key."));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_ssh_kh_list() {
    use captain_runtime::ssh_known_hosts as kh;

    let path = kh::default_known_hosts_path();
    if !path.exists() {
        println!("(empty — file does not exist yet: {})", path.display());
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            println!("# {} ({} entries)", path.display(), lines.len());
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            ui::error(&format!("Cannot read {}: {e}", path.display()));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_ssh_kh_clear() {
    use captain_runtime::ssh_known_hosts as kh;

    let path = kh::default_known_hosts_path();
    if !path.exists() {
        println!("Already empty.");
        return;
    }
    let confirm = prompt_input(&format!(
        "Clear all known_hosts entries in {} ? [y/N]: ",
        path.display()
    ));
    if !matches!(confirm.to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("Aborted.");
        return;
    }
    let bak = path.with_extension(format!(
        "bak.{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    if let Err(e) = std::fs::copy(&path, &bak) {
        ui::error(&format!("Backup failed: {e}"));
        std::process::exit(1);
    }
    if let Err(e) = std::fs::write(&path, "") {
        ui::error(&format!("Clear failed: {e}"));
        std::process::exit(1);
    }
    ui::success(&format!(
        "Cleared {}. Backup: {}",
        path.display(),
        bak.display()
    ));
}

pub(crate) fn cmd_ssh_kh_mode(arg: Option<&str>) {
    use captain_runtime::ssh_known_hosts as kh;

    match arg {
        None => {
            let m = kh::current_mode();
            println!("Current mode: {}", m.as_str());
            println!(
                "  (env CAPTAIN_SSH_KH_MODE > sidecar {} > default `tofu_learn`)",
                kh::mode_path().display()
            );
            println!("  Available: strict | tofu_learn | insecure");
        }
        Some(s) => match s.parse::<kh::KhVerificationMode>() {
            Ok(m) => match kh::set_mode(m) {
                Ok(()) => ui::success(&format!(
                    "known_hosts mode set to '{}' (persisted to {}).",
                    m.as_str(),
                    kh::mode_path().display()
                )),
                Err(e) => {
                    ui::error(&e);
                    std::process::exit(1);
                }
            },
            Err(_) => {
                ui::error(&format!(
                    "Unknown mode '{s}'. Use one of: strict | tofu_learn | insecure"
                ));
                std::process::exit(1);
            }
        },
    }
}
