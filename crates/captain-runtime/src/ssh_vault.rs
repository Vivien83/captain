//! SSH credential storage typed on top of the existing AES-256-GCM vault.
//!
//! Each saved entry is a `SshKey` serialized as JSON and stored under the
//! key `ssh:{name}`. The vault itself (master key in OS keyring or
//! `CAPTAIN_VAULT_KEY` env) handles the actual at-rest encryption.
//!
//! Fingerprints are computed via the `ssh-key` crate (SHA-256 over the
//! public key portion) so they match `ssh-keygen -lf <key>.pub` output.

use serde::{Deserialize, Serialize};
use ssh_key::PrivateKey;
use zeroize::Zeroizing;

/// A single SSH credential entry stored in the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKey {
    /// User-friendly alias, e.g. "prod-server".
    pub name: String,
    /// Hostname or IP of the remote.
    pub host: String,
    /// SSH port. Defaults to 22.
    pub port: u16,
    /// Remote login user.
    pub user: String,
    /// Private key content in PEM/OpenSSH format. Wrapped in `Zeroizing`
    /// so the in-memory copy is wiped on drop.
    #[serde(serialize_with = "ser_zeroizing", deserialize_with = "de_zeroizing")]
    pub private_key: Zeroizing<String>,
    /// Optional passphrase for the private key (also zeroized).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_opt_zeroizing",
        deserialize_with = "de_opt_zeroizing"
    )]
    pub passphrase: Option<Zeroizing<String>>,
    /// SHA-256 public-key fingerprint (matches `ssh-keygen -lf`).
    pub fingerprint: String,
    /// Unix epoch seconds when this entry was created.
    pub added_at: i64,
    /// Unix epoch seconds of last successful use (test/exec/upload).
    /// Updated by the various ssh tools (Q.7+).
    #[serde(default)]
    pub last_used: Option<i64>,
}

fn ser_zeroizing<S: serde::Serializer>(v: &Zeroizing<String>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(v.as_str())
}

fn de_zeroizing<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Zeroizing<String>, D::Error> {
    Ok(Zeroizing::new(String::deserialize(d)?))
}

fn ser_opt_zeroizing<S: serde::Serializer>(
    v: &Option<Zeroizing<String>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match v {
        Some(z) => s.serialize_some(z.as_str()),
        None => s.serialize_none(),
    }
}

fn de_opt_zeroizing<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Zeroizing<String>>, D::Error> {
    let opt: Option<String> = Option::deserialize(d)?;
    Ok(opt.map(Zeroizing::new))
}

/// Compute the SHA-256 public-key fingerprint for an OpenSSH private key.
/// Returns the standard `SHA256:base64nopad` form, matching `ssh-keygen -lf`.
pub fn fingerprint_of(pem: &str, passphrase: Option<&str>) -> Result<String, String> {
    let private = PrivateKey::from_openssh(pem)
        .map_err(|e| format!("Failed to parse OpenSSH private key: {e}"))?;
    // If the key is encrypted, decrypt it first (otherwise we can't extract
    // the public half).
    let pubkey = if private.is_encrypted() {
        let pp = passphrase
            .ok_or_else(|| "Private key is encrypted but no passphrase was supplied".to_string())?;
        private
            .decrypt(pp.as_bytes())
            .map_err(|e| format!("Failed to decrypt private key: {e}"))?
            .public_key()
            .clone()
    } else {
        private.public_key().clone()
    };
    let fp = pubkey.fingerprint(ssh_key::HashAlg::Sha256);
    Ok(fp.to_string())
}

/// Vault key prefix for SSH entries.
pub const SSH_KEY_PREFIX: &str = "ssh:";
/// Vault key for the "default SSH key" alias pointer.
pub const SSH_DEFAULT_KEY: &str = "ssh:_default";

/// Convenience trait abstracting the underlying `CredentialVault` so this
/// module can be unit-tested without a real on-disk vault.
pub trait SshSecretStore {
    fn get(&self, key: &str) -> Option<Zeroizing<String>>;
    fn set(&mut self, key: String, value: Zeroizing<String>) -> Result<(), String>;
    fn remove(&mut self, key: &str) -> Result<bool, String>;
    fn list_keys(&self) -> Vec<String>;
}

/// Persist a new SSH key entry. Overwrites any existing entry with the same
/// name (caller is responsible for warning the user beforehand).
pub fn save_ssh_key<S: SshSecretStore>(store: &mut S, key: SshKey) -> Result<(), String> {
    let json = serde_json::to_string(&key).map_err(|e| format!("serialize: {e}"))?;
    let storage_key = format!("{SSH_KEY_PREFIX}{}", key.name);
    store.set(storage_key, Zeroizing::new(json))
}

/// Load a single SSH key by alias.
pub fn load_ssh_key<S: SshSecretStore>(store: &S, name: &str) -> Option<SshKey> {
    let raw = store.get(&format!("{SSH_KEY_PREFIX}{name}"))?;
    serde_json::from_str(&raw).ok()
}

/// How a user-provided SSH alias was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshAliasResolution {
    /// The requested alias exists exactly in the vault.
    Exact,
    /// A single stored alias matched the user's shorthand.
    UniqueAlias,
    /// The request was generic and the configured default SSH key was used.
    Default,
    /// The request was generic and there was only one SSH key in the vault.
    OnlyAlias,
}

/// Result of resolving a user-provided SSH alias.
#[derive(Debug, Clone)]
pub struct ResolvedSshKey {
    pub requested: String,
    pub resolved: String,
    pub key: SshKey,
    pub resolution: SshAliasResolution,
}

/// List all stored SSH key aliases.
pub fn list_ssh_keys<S: SshSecretStore>(store: &S) -> Vec<String> {
    store
        .list_keys()
        .into_iter()
        .filter_map(|k| {
            k.strip_prefix(SSH_KEY_PREFIX)
                .filter(|n| !n.starts_with('_'))
                .map(|n| n.to_string())
        })
        .collect()
}

/// Resolve an SSH key alias in the way an autonomous agent needs:
/// exact alias first, then one unambiguous shorthand, then explicit default
/// for generic requests such as "server" or "remote".
pub fn resolve_ssh_key<S: SshSecretStore>(
    store: &S,
    requested: &str,
) -> Result<ResolvedSshKey, String> {
    let requested = requested.trim();
    if requested.is_empty() {
        return Err("Missing SSH alias. Retry with `key_name` set to a vault alias.".to_string());
    }

    if let Some(key) = load_ssh_key(store, requested) {
        return Ok(ResolvedSshKey {
            requested: requested.to_string(),
            resolved: requested.to_string(),
            key,
            resolution: SshAliasResolution::Exact,
        });
    }

    let mut aliases = list_ssh_keys(store);
    aliases.sort();
    aliases.dedup();

    if is_plain_generic_ssh_alias_request(requested) {
        if let Some(default_alias) = get_default_ssh_key(store) {
            if aliases.iter().any(|alias| alias == &default_alias) {
                return resolved_alias(
                    store,
                    requested,
                    &default_alias,
                    SshAliasResolution::Default,
                );
            }
        }

        if aliases.len() == 1 {
            return resolved_alias(store, requested, &aliases[0], SshAliasResolution::OnlyAlias);
        }
    }

    let mut matches: Vec<String> = aliases
        .iter()
        .filter(|alias| alias_matches_request(alias, requested))
        .cloned()
        .collect();
    matches.sort();
    matches.dedup();

    if matches.len() == 1 {
        return resolved_alias(
            store,
            requested,
            &matches[0],
            SshAliasResolution::UniqueAlias,
        );
    }

    if matches.len() > 1 {
        return Err(format!(
            "SSH alias '{requested}' is ambiguous. Matching aliases: {}. \
             Retry ssh_exec with the exact `key_name`; if unsure, call \
             captain_docs({{\"family\":\"ssh\",\"query\":\"alias resolution\"}}).",
            matches.join(", ")
        ));
    }

    if contains_generic_ssh_alias_token(requested) {
        if let Some(default_alias) = get_default_ssh_key(store) {
            if aliases.iter().any(|alias| alias == &default_alias) {
                return resolved_alias(
                    store,
                    requested,
                    &default_alias,
                    SshAliasResolution::Default,
                );
            }
        }

        if aliases.len() == 1 {
            return resolved_alias(store, requested, &aliases[0], SshAliasResolution::OnlyAlias);
        }
    }

    Err(format!(
        "No SSH key named '{requested}'. Known aliases: {}. \
         Retry with an exact alias, or call captain_docs({{\"family\":\"ssh\",\"query\":\"alias not found recovery\"}}) \
         before asking the user. Do not diagnose Captain's SSH vault through shell_exec.",
        known_aliases_hint(&aliases)
    ))
}

fn resolved_alias<S: SshSecretStore>(
    store: &S,
    requested: &str,
    resolved: &str,
    resolution: SshAliasResolution,
) -> Result<ResolvedSshKey, String> {
    let key = load_ssh_key(store, resolved)
        .ok_or_else(|| format!("Resolved SSH alias '{resolved}' no longer exists in the vault"))?;
    Ok(ResolvedSshKey {
        requested: requested.to_string(),
        resolved: resolved.to_string(),
        key,
        resolution,
    })
}

fn alias_matches_request(alias: &str, requested: &str) -> bool {
    let alias = normalize_alias(alias);
    let requested = normalize_alias(requested);
    if alias.is_empty() || requested.len() < 3 {
        return false;
    }
    if alias == requested {
        return true;
    }

    alias.starts_with(&format!("{requested}-"))
        || alias.starts_with(&format!("{requested}_"))
        || alias.starts_with(&format!("{requested}."))
        || alias.ends_with(&format!("-{requested}"))
        || alias.ends_with(&format!("_{requested}"))
        || alias.ends_with(&format!(".{requested}"))
        || alias
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| part == requested)
        || alias_tokens(&requested).into_iter().any(|requested_part| {
            alias_tokens(&alias)
                .iter()
                .any(|alias_part| alias_part == &requested_part)
        })
}

fn normalize_alias(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn is_plain_generic_ssh_alias_request(requested: &str) -> bool {
    let requested = normalize_alias(requested);
    matches!(
        requested.as_str(),
        "default"
            | "server"
            | "serveur"
            | "host"
            | "machine"
            | "remote"
            | "distant"
            | "ssh"
            | "vps"
            | "prod"
            | "production"
    ) || requested.contains("serveur")
        || requested.contains("server")
}

fn contains_generic_ssh_alias_token(requested: &str) -> bool {
    alias_tokens(requested).into_iter().any(|token| {
        matches!(
            token.as_str(),
            "default"
                | "server"
                | "serveur"
                | "host"
                | "machine"
                | "remote"
                | "distant"
                | "ssh"
                | "vps"
                | "prod"
                | "production"
        )
    })
}

fn alias_tokens(value: &str) -> Vec<String> {
    normalize_alias(value)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| part.len() >= 3 && !is_alias_stopword(part))
        .map(ToString::to_string)
        .collect()
}

fn is_alias_stopword(part: &str) -> bool {
    matches!(
        part,
        "mon"
            | "mes"
            | "ton"
            | "tes"
            | "son"
            | "ses"
            | "the"
            | "and"
            | "avec"
            | "pour"
            | "sur"
            | "dans"
    )
}

fn known_aliases_hint(aliases: &[String]) -> String {
    if aliases.is_empty() {
        "none registered".to_string()
    } else {
        aliases.join(", ")
    }
}

/// Delete an SSH key entry. Returns `true` if it existed.
pub fn delete_ssh_key<S: SshSecretStore>(store: &mut S, name: &str) -> Result<bool, String> {
    store.remove(&format!("{SSH_KEY_PREFIX}{name}"))
}

/// Set the named key as the default for `tool_ssh_*` tools.
pub fn set_default_ssh_key<S: SshSecretStore>(store: &mut S, name: &str) -> Result<(), String> {
    if load_ssh_key(store, name).is_none() {
        return Err(format!(
            "Cannot set default to '{name}': no such SSH key in vault"
        ));
    }
    store.set(
        SSH_DEFAULT_KEY.to_string(),
        Zeroizing::new(name.to_string()),
    )
}

/// Read the current default SSH key alias, if any.
pub fn get_default_ssh_key<S: SshSecretStore>(store: &S) -> Option<String> {
    store.get(SSH_DEFAULT_KEY).map(|z| z.to_string())
}

/// Append an audit-log line for an SSH operation.
///
/// One JSON object per line (jsonl). Best-effort: errors are logged but
/// never propagated — auditing must never break a working operation.
pub fn audit_log(
    audit_dir: &std::path::Path,
    op: &str,
    key_name: &str,
    detail: &str,
    success: bool,
) {
    if std::fs::create_dir_all(audit_dir).is_err() {
        return;
    }
    let log_path = audit_dir.join("ssh.log");
    let line = serde_json::json!({
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "op": op,
        "key": key_name,
        "ok": success,
        "detail": detail,
    });
    let mut buf = line.to_string();
    buf.push('\n');
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = f.write_all(buf.as_bytes());
    }
}

#[cfg(test)]
#[path = "ssh_vault_tests.rs"]
mod tests;
