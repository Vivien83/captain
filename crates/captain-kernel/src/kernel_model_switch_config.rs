use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{KernelError, KernelResult};
use captain_types::agent::AgentEntry;
use captain_types::config::DefaultModelConfig;
use captain_types::error::CaptainError;
use tracing::info;

use super::kernel_config_support::rotate_config_backups;
use super::{strip_provider_prefix, CaptainKernel, PRINCIPAL_AGENT_NAME};

impl CaptainKernel {
    pub(super) fn is_principal_agent(entry: &AgentEntry) -> bool {
        entry.name.eq_ignore_ascii_case("captain")
    }

    pub(super) fn reconcile_principal_agent_with_default_model(
        entry: &mut AgentEntry,
        default_model: &DefaultModelConfig,
    ) -> bool {
        if !Self::is_principal_agent(entry) {
            return false;
        }

        let mut changed = false;

        if entry.manifest.name.trim().is_empty() || entry.manifest.name == "unnamed" {
            entry.manifest.name = PRINCIPAL_AGENT_NAME.to_string();
            changed = true;
        }
        if entry.manifest.description.trim().is_empty() {
            entry.manifest.description = "Captain — principal agent".to_string();
            changed = true;
        }
        if !default_model.provider.is_empty()
            && entry.manifest.model.provider != default_model.provider
        {
            entry.manifest.model.provider = default_model.provider.clone();
            changed = true;
        }
        if !default_model.model.is_empty() {
            let desired_model =
                strip_provider_prefix(&default_model.model, &default_model.provider);
            if entry.manifest.model.model != desired_model {
                entry.manifest.model.model = desired_model;
                changed = true;
            }
        }

        let desired_api_key_env = if default_model.api_key_env.trim().is_empty() {
            None
        } else {
            Some(default_model.api_key_env.clone())
        };
        if entry.manifest.model.api_key_env != desired_api_key_env {
            entry.manifest.model.api_key_env = desired_api_key_env;
            changed = true;
        }
        if entry.manifest.model.base_url != default_model.base_url {
            entry
                .manifest
                .model
                .base_url
                .clone_from(&default_model.base_url);
            changed = true;
        }

        changed
    }

    fn default_api_key_env_for_switch(&self, provider: &str) -> String {
        // Codex OAuth is intentionally distinct from OpenAI API auth. Leaving
        // api_key_env empty mirrors `captain login codex --with-model`: the
        // Codex driver reads the OAuth session, and only falls back to
        // OPENAI_API_KEY when no session exists.
        if matches!(provider, "codex" | "openai-codex") {
            return String::new();
        }

        self.model_catalog
            .read()
            .ok()
            .and_then(|catalog| {
                catalog
                    .get_provider(provider)
                    .map(|p| p.api_key_env.clone())
            })
            .filter(|env| !env.trim().is_empty())
            .unwrap_or_else(|| self.config.resolve_api_key_env(provider))
    }

    pub(super) fn persist_principal_default_model_switch(
        &self,
        provider: &str,
        model: &str,
    ) -> KernelResult<()> {
        let config_path = self.config.home_dir.join("config.toml");
        let content = read_model_switch_config(&config_path)?;
        let old_size = content.len();
        let mut doc = parse_model_switch_config(&content)?;
        let old_top_keys = top_level_keys(&doc);
        let backup_path = backup_existing_model_switch_config(&config_path, &self.config.home_dir)?;

        let api_key_env = self.default_api_key_env_for_switch(provider);
        upsert_default_model_switch(&mut doc, provider, model, &api_key_env)?;
        ensure_top_level_keys_preserved(&old_top_keys, &doc)?;
        let serialized = serialize_guarded_model_switch_config(&doc, old_size)?;
        write_model_switch_config(&config_path, &serialized)?;
        validate_model_switch_config_roundtrip(&config_path, backup_path.as_deref())?;

        self.set_default_model_override(provider, model, api_key_env);
        info!(
            provider = %provider,
            model = %model,
            config = %config_path.display(),
            "Principal model switch persisted to global default_model"
        );
        Ok(())
    }

    fn set_default_model_override(&self, provider: &str, model: &str, api_key_env: String) {
        let mut guard = self
            .default_model_override
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env,
            base_url: None,
        });
    }
}

fn read_model_switch_config(config_path: &Path) -> KernelResult<String> {
    if !config_path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(config_path).map_err(|e| {
        model_switch_internal(format!("Failed to read config.toml for model switch: {e}"))
    })
}

fn parse_model_switch_config(content: &str) -> KernelResult<toml_edit::DocumentMut> {
    content.parse().map_err(|e| {
        model_switch_internal(format!("Failed to parse config.toml for model switch: {e}"))
    })
}

fn backup_existing_model_switch_config(
    config_path: &Path,
    home_dir: &Path,
) -> KernelResult<Option<PathBuf>> {
    let backup_dir = home_dir.join("config-backups");
    captain_types::durable_fs::create_dir_all(&backup_dir).map_err(|e| {
        model_switch_internal(format!("Failed to create config backup directory: {e}"))
    })?;
    if !config_path.exists() {
        return Ok(None);
    }

    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let path = backup_dir.join(format!("config.toml.{ts}"));
    captain_types::durable_fs::atomic_copy(config_path, &path)
        .map_err(|e| model_switch_internal(format!("Config pre-write backup failed: {e}")))?;
    rotate_config_backups(&backup_dir, 20);
    Ok(Some(path))
}

fn upsert_default_model_switch(
    doc: &mut toml_edit::DocumentMut,
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> KernelResult<()> {
    let root = doc.as_table_mut();
    if !root.contains_key("default_model") {
        let mut table = toml_edit::Table::new();
        table.set_implicit(false);
        root.insert("default_model", toml_edit::Item::Table(table));
    }
    let default_model = root
        .get_mut("default_model")
        .and_then(|item| item.as_table_mut())
        .ok_or_else(|| {
            model_switch_internal("Config path 'default_model' exists but is not a table")
        })?;
    default_model.insert("provider", toml_edit::value(provider));
    default_model.insert("model", toml_edit::value(model));
    default_model.insert("api_key_env", toml_edit::value(api_key_env));
    default_model.remove("base_url");
    Ok(())
}

fn ensure_top_level_keys_preserved(
    old_top_keys: &BTreeSet<String>,
    doc: &toml_edit::DocumentMut,
) -> KernelResult<()> {
    let new_top_keys = top_level_keys(doc);
    let lost: Vec<&String> = old_top_keys.difference(&new_top_keys).collect();
    if lost.is_empty() {
        return Ok(());
    }
    Err(model_switch_internal(format!(
        "Refusing to write config.toml: top-level keys would be lost: {lost:?}"
    )))
}

fn serialize_guarded_model_switch_config(
    doc: &toml_edit::DocumentMut,
    old_size: usize,
) -> KernelResult<String> {
    let serialized = doc.to_string();
    if old_size > 1_000 && serialized.len() < (old_size * 7 / 10) {
        return Err(model_switch_internal(format!(
            "Refusing to write config.toml: suspicious shrinkage ({} -> {} bytes)",
            old_size,
            serialized.len()
        )));
    }
    Ok(serialized)
}

fn write_model_switch_config(config_path: &Path, serialized: &str) -> KernelResult<()> {
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes()).map_err(|e| {
        model_switch_internal(format!(
            "Failed to persist config.toml for model switch: {e}"
        ))
    })
}

fn validate_model_switch_config_roundtrip(
    config_path: &Path,
    backup_path: Option<&Path>,
) -> KernelResult<()> {
    if let Err(e) = std::fs::read_to_string(config_path)
        .map_err(|e| e.to_string())
        .and_then(|s| {
            s.parse::<toml::Value>()
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    {
        if let Some(path) = backup_path {
            let _ = captain_types::durable_fs::atomic_copy(path, config_path);
        }
        return Err(model_switch_internal(format!(
            "Roundtrip validation failed after model switch ({e}); config rollback attempted"
        )));
    }
    Ok(())
}

fn top_level_keys(doc: &toml_edit::DocumentMut) -> BTreeSet<String> {
    doc.as_table().iter().map(|(k, _)| k.to_string()).collect()
}

fn model_switch_internal(message: impl Into<String>) -> KernelError {
    KernelError::Captain(CaptainError::Internal(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_default_model_switch_preserves_sections_and_clears_base_url() {
        let mut doc: toml_edit::DocumentMut = r#"[default_model]
provider = "anthropic"
model = "claude-sonnet-4-6"
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://old.example.invalid"

[workspace]
extra_paths = []
"#
        .parse()
        .expect("parse config");

        let old_top_keys = top_level_keys(&doc);
        upsert_default_model_switch(&mut doc, "codex", "gpt-5.5", "")
            .expect("upsert default_model");
        ensure_top_level_keys_preserved(&old_top_keys, &doc).expect("keys should be preserved");

        let parsed: toml::Value = doc.to_string().parse().expect("valid toml");
        let default_model = parsed
            .get("default_model")
            .and_then(|v| v.as_table())
            .expect("default_model table");
        assert_eq!(
            default_model.get("provider").and_then(|v| v.as_str()),
            Some("codex")
        );
        assert_eq!(
            default_model.get("model").and_then(|v| v.as_str()),
            Some("gpt-5.5")
        );
        assert_eq!(
            default_model.get("api_key_env").and_then(|v| v.as_str()),
            Some("")
        );
        assert!(!default_model.contains_key("base_url"));
        assert!(parsed.get("workspace").is_some());
    }

    #[test]
    fn top_level_key_guard_rejects_lost_sections() {
        let original: toml_edit::DocumentMut = r#"[default_model]
provider = "anthropic"

[workspace]
extra_paths = []
"#
        .parse()
        .expect("parse original");
        let shrunk: toml_edit::DocumentMut = r#"[default_model]
provider = "codex"
"#
        .parse()
        .expect("parse shrunk");

        let err = ensure_top_level_keys_preserved(&top_level_keys(&original), &shrunk)
            .expect_err("lost workspace key must be rejected");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("top-level keys would be lost"));
        assert!(rendered.contains("workspace"));
    }

    #[test]
    fn serialize_guard_rejects_suspicious_shrinkage() {
        let doc: toml_edit::DocumentMut = r#"[default_model]
provider = "codex"
"#
        .parse()
        .expect("parse config");

        let err = serialize_guarded_model_switch_config(&doc, 2_000)
            .expect_err("large config shrinking by more than thirty percent is suspicious");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("suspicious shrinkage"));
    }
}
