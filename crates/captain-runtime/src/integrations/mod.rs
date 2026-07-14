//! R.3.1 — Generic auto-install integrations (foundation).
//!
//! Reusable primitive on top of which `R.3.2 telegram`, `R.3.3 tts/stt`
//! and any future channel/skill integration is wired.
//!
//! ```text
//! agent ──tool config_setup──▶ setup_integration() ──┐
//!                                                    │
//!                                          ┌─────────┴──────────┐
//!                                          ▼                    ▼
//!                                  validate(creds)       (on success)
//!                                          │             ┌────┴────────────────┐
//!                                          │             ▼                     ▼
//!                                          │       backup config         vault.set()
//!                                          │       toml_edit patch       per key
//!                                          │       atomic write
//!                                          ▼
//!                                  ApplyOutcome { backup_path, vault_keys, ... }
//! ```
//!
//! The trait `IntegrationSetup` is the single point of extension: each
//! new integration only needs to implement it and register itself in
//! [`get_integration`]. The orchestrator [`setup_integration`] then takes
//! care of credential vaulting, config patching, and atomic backup.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod stt_whisper;
pub mod telegram;
pub mod tts_elevenlabs;
pub mod tts_openai;

/// Patch applied on top of `~/.captain/config.toml` while preserving
/// existing comments and formatting via `toml_edit`.
///
/// `path` is the dotted TOML table path (e.g. `["channels", "telegram"]`),
/// `key` the leaf key (e.g. `"default_chat_id"`), `value` an already-built
/// `toml_edit::Item` (string, integer, array, …).
#[derive(Debug, Clone)]
pub struct ConfigPatch {
    pub path: Vec<String>,
    pub key: String,
    pub value: toml_edit::Item,
}

/// Contract every auto-installable integration must implement.
///
/// Implementors live in `integrations/<name>.rs` and are registered in
/// [`get_integration`]. Keep impls **pure**: no I/O during `validate` or
/// `vault_keys`/`config_patch` — those are called transactionally by the
/// orchestrator. Side-effects belong in `test()`.
#[async_trait::async_trait]
pub trait IntegrationSetup: Send + Sync {
    /// Stable canonical name (lowercase ASCII, snake_case allowed).
    fn name(&self) -> &str;

    /// Human-readable description shown in CLI / agent feedback.
    fn description(&self) -> &str;

    /// Validate the credentials JSON object. Must reject malformed input
    /// before any side-effect happens.
    fn validate(&self, creds: &serde_json::Value) -> Result<(), String>;

    /// Pairs `(vault_key, secret_value)` to encrypt-and-store. The
    /// orchestrator will prefix every key with `integration:<name>:` so
    /// implementors only return the suffix (e.g. `"bot_token"`).
    ///
    /// Returns an empty vec for integrations with no secret material.
    fn vault_keys(&self, creds: &serde_json::Value) -> Vec<(String, String)>;

    /// R.3.3 — pairs `(env_var_name, value)` that must be exported into
    /// the running process so engines reading via `std::env::var()` (TTS
    /// and STT today) see the new credentials immediately, without a
    /// daemon restart. Persistence to `secrets.env` happens via the same
    /// `vault_set` callback the orchestrator already uses.
    ///
    /// Default: empty (most integrations rely solely on namespaced vault
    /// keys).
    fn env_exports(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let _ = creds;
        Vec::new()
    }

    /// Patches to apply on top of `config.toml`, preserving comments.
    fn config_patch(&self, creds: &serde_json::Value) -> Vec<ConfigPatch>;

    /// Live test: ping the remote to confirm credentials work end-to-end.
    /// Should be skippable (orchestrator may pass `--no-test`).
    async fn test(&self, creds: &serde_json::Value) -> Result<String, String>;
}

/// Resolve an integration by canonical name. Returns `None` if unknown.
pub fn get_integration(name: &str) -> Option<Box<dyn IntegrationSetup>> {
    match name {
        "telegram" => Some(Box::new(telegram::Telegram)),
        "tts_elevenlabs" => Some(Box::new(tts_elevenlabs::TtsElevenLabs)),
        "tts_openai" => Some(Box::new(tts_openai::TtsOpenAi)),
        "stt_whisper" => Some(Box::new(stt_whisper::SttWhisper)),
        _ => None,
    }
}

/// List of every integration name registered. Useful for CLI completion
/// and agent introspection.
pub fn list_integrations() -> Vec<&'static str> {
    vec!["telegram", "tts_elevenlabs", "tts_openai", "stt_whisper"]
}

/// Outcome of a successful [`setup_integration`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub integration: String,
    pub backup_path: Option<PathBuf>,
    pub vault_keys: Vec<String>,
    pub patched_paths: Vec<String>,
    pub test_message: Option<String>,
    /// R.3.3 — env vars exported into the current process (also persisted
    /// via the `vault_set` callback so they survive a restart).
    #[serde(default)]
    pub env_exports: Vec<String>,
}

/// Orchestrator. Executes the full transactional workflow for a given
/// integration:
///
/// 1. resolve via [`get_integration`]
/// 2. `validate(creds)` — reject early if malformed
/// 3. backup `config.toml` to `<config>.bak.<unix_ts>`
/// 4. write each vault key (`integration:<name>:<suffix>`)
/// 5. patch the TOML document (preserving comments)
/// 6. atomic write back
/// 7. (optional) live `test()`
/// 8. (optional) `notify(name)` callback — used by R.3.2 to publish
///    `SystemEvent::IntegrationConfigured` on the event bus so channel
///    managers can hot-reload the affected adapter.
///
/// On any error in step 1-3 nothing is written. On error in step 4-6 the
/// orchestrator best-effort restores the backup before propagating.
/// `notify` is only invoked on full success (never on partial failure or
/// rollback) so a listener can rely on it as a "definitely applied" signal.
pub async fn setup_integration(
    name: &str,
    creds: &serde_json::Value,
    config_path: &Path,
    mut vault_set: impl FnMut(&str, &str) -> Result<(), String>,
    run_live_test: bool,
    notify: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<ApplyOutcome, String> {
    let integration =
        get_integration(name).ok_or_else(|| format!("unknown integration: {name}"))?;

    integration.validate(creds)?;

    let backup_path = backup_config(config_path)?;

    let vault_pairs = integration.vault_keys(creds);
    let mut written_keys = Vec::with_capacity(vault_pairs.len());
    for (suffix, value) in &vault_pairs {
        let full_key = format!("integration:{name}:{suffix}");
        if let Err(e) = vault_set(&full_key, value) {
            // Best-effort rollback: restore backup if we created one.
            if let Some(ref bp) = backup_path {
                let _ = std::fs::copy(bp, config_path);
            }
            return Err(format!("vault write failed for {full_key}: {e}"));
        }
        written_keys.push(full_key);
    }

    // R.3.3 — also persist env-var-exported credentials and inject them
    // into the current process so engines reading via std::env::var (TTS,
    // STT) see them immediately. Persistence reuses the same vault_set
    // callback (caller decides where it lands: secrets.env, vault.enc, …).
    let env_pairs = integration.env_exports(creds);
    let mut exported_env_names = Vec::with_capacity(env_pairs.len());
    for (env_name, env_value) in &env_pairs {
        if let Err(e) = vault_set(env_name, env_value) {
            if let Some(ref bp) = backup_path {
                let _ = std::fs::copy(bp, config_path);
            }
            return Err(format!("env-export persistence failed for {env_name}: {e}"));
        }
        // Edition 2021: still safe (no unsafe required). Same pattern as
        // dotenv::save_env_key and the existing kernel MCP env injection.
        std::env::set_var(env_name, env_value);
        exported_env_names.push(env_name.clone());
    }

    let patches = integration.config_patch(creds);
    let patched_paths: Vec<String> = patches
        .iter()
        .map(|p| {
            let mut full = p.path.clone();
            full.push(p.key.clone());
            full.join(".")
        })
        .collect();

    if let Err(e) = apply_config_patch(config_path, &patches) {
        if let Some(ref bp) = backup_path {
            let _ = std::fs::copy(bp, config_path);
        }
        return Err(format!("config patch failed: {e}"));
    }

    let test_message = if run_live_test {
        match integration.test(creds).await {
            Ok(msg) => Some(msg),
            Err(e) => return Err(format!("live test failed: {e}")),
        }
    } else {
        None
    };

    if let Some(cb) = notify {
        cb(name);
    }

    Ok(ApplyOutcome {
        integration: name.to_string(),
        backup_path,
        vault_keys: written_keys,
        patched_paths,
        test_message,
        env_exports: exported_env_names,
    })
}

/// Copy the existing config to `<path>.bak.<unix_ts>`. Returns `None` if
/// the source file does not exist (first-time install — nothing to back
/// up).
pub fn backup_config(config_path: &Path) -> Result<Option<PathBuf>, String> {
    if !config_path.exists() {
        return Ok(None);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut bak = config_path.as_os_str().to_owned();
    bak.push(format!(".bak.{ts}"));
    let bak_path = PathBuf::from(bak);
    std::fs::copy(config_path, &bak_path).map_err(|e| format!("backup copy failed: {e}"))?;
    Ok(Some(bak_path))
}

/// Apply `patches` on top of `config_path` using `toml_edit` so existing
/// comments and key ordering are preserved. If the file does not exist it
/// is created from an empty document.
pub fn apply_config_patch(config_path: &Path, patches: &[ConfigPatch]) -> Result<(), String> {
    let raw = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .map_err(|e| format!("read {}: {e}", config_path.display()))?
    } else {
        String::new()
    };

    let mut doc: toml_edit::DocumentMut = raw.parse().map_err(|e| format!("parse TOML: {e}"))?;

    for patch in patches {
        // Navigate (or create) the nested table path.
        let mut cursor: &mut toml_edit::Item = doc.as_item_mut();
        for segment in &patch.path {
            // Promote bare values to a table on first descent if needed.
            if !cursor.is_table_like() {
                *cursor = toml_edit::Item::Table(toml_edit::Table::new());
            }
            let table = cursor
                .as_table_mut()
                .ok_or_else(|| format!("path segment '{segment}' is not a table"))?;
            if !table.contains_key(segment) {
                let mut new_t = toml_edit::Table::new();
                new_t.set_implicit(false);
                table.insert(segment, toml_edit::Item::Table(new_t));
            }
            cursor = &mut table[segment];
        }
        let leaf_table = cursor
            .as_table_mut()
            .ok_or_else(|| format!("leaf for '{}' is not a table", patch.key))?;
        leaf_table.insert(&patch.key, patch.value.clone());
    }

    // Atomic write: write to sibling tmp + rename.
    let tmp = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, config_path).map_err(|e| format!("rename tmp -> config: {e}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
