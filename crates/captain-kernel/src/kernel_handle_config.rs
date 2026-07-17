use crate::model_switch::ModelSwitchSessionStrategy;
use captain_types::agent::AgentId;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};
use tracing::{info, warn};

use super::kernel_config_support::{
    rotate_backups_with_prefix, rotate_config_backups, set_secret_file_permissions,
    validate_secret_assignment,
};
use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_config_read(&self, path: &str) -> Result<Option<String>, String> {
        // The config file is the source of truth. Prefer reading it live so
        // memories, boot-time snapshots, and hot-reload gaps cannot shadow a
        // user's persisted config change.
        let config_path = self.config.home_dir.join("config.toml");
        let mut config = if config_path.exists() {
            crate::config::load_config(Some(&config_path))
        } else {
            self.config.clone()
        };
        config.default_model = self.effective_default_model();
        let toml_val = toml::Value::try_from(&config)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = &toml_val;
        for part in &parts {
            match current.get(part) {
                Some(v) => current = v,
                None => return Ok(None),
            }
        }
        Ok(Some(config_value_to_string(current)))
    }

    pub(super) async fn handle_config_write(&self, path: &str, value: &str) -> Result<(), String> {
        let config_path = self.config.home_dir.join("config.toml");
        let (content, mut doc) = read_config_document(&config_path)?;
        let old_size = content.len();
        let old_top_keys = top_level_edit_keys(&doc);
        let backup_path = create_config_backup(&config_path, &self.config.home_dir)?;

        apply_config_write_path(&mut doc, path, value)?;
        validate_top_keys_preserved(&old_top_keys, &doc, &backup_path)?;
        let serialized = doc.to_string();
        validate_serialized_config_size(old_size, serialized.len(), &backup_path)?;
        write_config_and_validate_roundtrip(
            &config_path,
            &serialized,
            &old_top_keys,
            &backup_path,
        )?;

        match self.reload_config() {
            Ok(plan) => {
                info!(
                    path = %path,
                    value = %value,
                    backup = %backup_path.display(),
                    restart_required = plan.restart_required,
                    "Config updated via tool and reload attempted"
                );
            }
            Err(e) => {
                warn!(
                    path = %path,
                    value = %value,
                    backup = %backup_path.display(),
                    error = %e,
                    "Config updated via tool but reload failed"
                );
            }
        }
        Ok(())
    }

    pub(super) async fn handle_update_self_config(
        &self,
        agent_id: &str,
        config_json: &str,
    ) -> Result<String, String> {
        let id: AgentId = agent_id
            .parse()
            .map_err(|_| format!("Invalid agent ID: {agent_id}"))?;

        let patch: serde_json::Value =
            serde_json::from_str(config_json).map_err(|e| format!("Invalid JSON: {e}"))?;

        let mut changes = Vec::new();

        if patch.get("model").is_some() || patch.get("provider").is_some() {
            let model = patch.get("model").and_then(|v| v.as_str()).ok_or_else(|| {
                "Safe model/provider switch requires a 'model'. Call model_switch_plan first."
                    .to_string()
            })?;
            let provider = patch.get("provider").and_then(|v| v.as_str());
            let strategy = patch
                .get("session_strategy")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    "Safe model/provider switch requires 'session_strategy' = 'new_session' or 'compact_session'. Call model_switch_plan first and ask the user which strategy to apply."
                        .to_string()
                })?
                .parse::<ModelSwitchSessionStrategy>()?;
            let result = self
                .apply_model_switch(id, model, provider, strategy)
                .map_err(|e| format!("{e}"))?;
            changes.push(format!(
                "model/provider → {}/{} ({})",
                result.plan.target_provider,
                result.plan.target_model,
                result.session_strategy.as_str()
            ));
        }

        if let Some(fallbacks) = patch.get("fallback_models") {
            let fb: Vec<captain_types::agent::FallbackModel> =
                serde_json::from_value(fallbacks.clone())
                    .map_err(|e| format!("Invalid fallback_models: {e}"))?;
            self.registry
                .update_fallback_models(id, fb)
                .map_err(|e| format!("{e}"))?;
            changes.push("fallback_models updated".to_string());
        }

        if patch.get("routing").is_some() {
            return Err(
                "Automatic per-turn model routing was removed. Configure the model/provider explicitly or create a specialist sub-agent."
                    .to_string(),
            );
        }

        if let Some(desc) = patch.get("description").and_then(|v| v.as_str()) {
            self.registry
                .update_description(id, desc.to_string())
                .map_err(|e| format!("{e}"))?;
            changes.push(format!("description → {}", &desc[..desc.len().min(40)]));
        }

        if let Some(prompt) = patch.get("system_prompt").and_then(|v| v.as_str()) {
            self.registry
                .update_system_prompt(id, prompt.to_string())
                .map_err(|e| format!("{e}"))?;
            changes.push("system_prompt updated".to_string());
        }

        if let Some(entry) = self.registry.get(id) {
            let _ = self.memory.save_agent(&entry);
        }

        if changes.is_empty() {
            Ok("No changes applied.".to_string())
        } else {
            let msg = format!("Config updated: {}", changes.join(", "));
            info!(agent_id = %agent_id, "{msg}");
            Ok(msg)
        }
    }

    pub(super) fn handle_model_switch_plan(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let id: AgentId = agent_id
            .parse()
            .map_err(|_| format!("Invalid agent ID: {agent_id}"))?;
        self.plan_model_switch(id, model, provider)
            .map(|plan| serde_json::json!(plan))
            .map_err(|e| format!("{e}"))
    }

    pub(super) fn handle_model_switch_apply(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
        session_strategy: &str,
    ) -> Result<serde_json::Value, String> {
        let id: AgentId = agent_id
            .parse()
            .map_err(|_| format!("Invalid agent ID: {agent_id}"))?;
        let strategy = session_strategy.parse::<ModelSwitchSessionStrategy>()?;
        self.apply_model_switch(id, model, provider, strategy)
            .map(|result| serde_json::json!(result))
            .map_err(|e| format!("{e}"))
    }

    pub(super) fn handle_secret_read(&self, key: &str) -> Result<Option<String>, String> {
        warn!(key = %key, "Secret accessed via tool (value exposed to agent)");
        let secrets_path = self.config.home_dir.join("secrets.env");
        if !secrets_path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&secrets_path)
            .map_err(|e| format!("Failed to read secrets.env: {e}"))?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Ok(Some(v.trim().to_string()));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn handle_secret_write(&self, key: &str, value: &str) -> Result<(), String> {
        validate_secret_assignment(key, value)?;
        let secrets_path = self.config.home_dir.join("secrets.env");
        let original = if secrets_path.exists() {
            Some(
                std::fs::read_to_string(&secrets_path)
                    .map_err(|e| format!("Failed to read secrets.env: {e}"))?,
            )
        } else {
            None
        };
        let mut lines: Vec<String> = original
            .as_deref()
            .unwrap_or("")
            .lines()
            .map(|l| l.to_string())
            .collect();

        let backup_path = if secrets_path.exists() {
            let backup_dir = self.config.home_dir.join("secrets-backups");
            captain_types::durable_fs::create_dir_all(&backup_dir)
                .map_err(|e| format!("Failed to create secrets-backups dir: {e}"))?;
            let ts = chrono::Utc::now()
                .format("%Y-%m-%dT%H-%M-%S-%3f")
                .to_string();
            let backup_path = backup_dir.join(format!("secrets.env.{ts}"));
            captain_types::durable_fs::atomic_copy(&secrets_path, &backup_path)
                .map_err(|e| format!("Secret pre-write backup failed: {e}"))?;
            rotate_backups_with_prefix(&backup_dir, "secrets.env.", 20);
            Some(backup_path)
        } else {
            None
        };

        upsert_secret_line(&mut lines, key, value);

        let serialized = lines.join("\n") + "\n";
        captain_types::durable_fs::atomic_write(&secrets_path, serialized.as_bytes())
            .map_err(|e| format!("Failed to persist secrets.env: {e}"))?;
        set_secret_file_permissions(&secrets_path)?;

        match self.handle_secret_read(key)? {
            Some(saved) if saved == value => {}
            _ => {
                if let Some(bp) = &backup_path {
                    let _ = captain_types::durable_fs::atomic_copy(bp, &secrets_path);
                }
                return Err(format!(
                    "Secret roundtrip validation failed for '{key}'.{}",
                    backup_path
                        .as_ref()
                        .map(|bp| format!(" Rolled back from {}", bp.display()))
                        .unwrap_or_default()
                ));
            }
        }
        // Mirror into the live process env so callers that resolve via
        // std::env::var (e.g. read_token in start_channel_bridge_with_config)
        // see the new value without a daemon restart. Parity with the CLI
        // path dotenv::save_secret_key.
        std::env::set_var(key, value);
        info!(key = %key, "Secret updated via tool");
        Ok(())
    }
}

fn read_config_document(config_path: &Path) -> Result<(String, DocumentMut), String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config.toml: {e}"))?;
    let doc = content
        .parse()
        .map_err(|e| format!("Failed to parse config.toml: {e}"))?;
    Ok((content, doc))
}

fn create_config_backup(config_path: &Path, home_dir: &Path) -> Result<PathBuf, String> {
    let backup_dir = home_dir.join("config-backups");
    if let Err(e) = captain_types::durable_fs::create_dir_all(&backup_dir) {
        warn!(error = %e, "Could not create config-backups dir");
    }
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let backup_path = backup_dir.join(format!("config.toml.{ts}"));
    if let Err(e) = captain_types::durable_fs::atomic_copy(config_path, &backup_path) {
        warn!(error = %e, "Config pre-write backup failed — aborting write for safety");
        return Err(format!("Pre-write backup failed: {e}"));
    }
    rotate_config_backups(&backup_dir, 20);
    Ok(backup_path)
}

fn apply_config_write_path(doc: &mut DocumentMut, path: &str, value: &str) -> Result<(), String> {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(format!("Invalid config path: '{path}'"));
    }
    let leaf_key = parts[parts.len() - 1];
    let parent_parts = &parts[..parts.len() - 1];

    let mut current: &mut Item = doc.as_item_mut();
    for (i, part) in parent_parts.iter().enumerate() {
        let path_so_far = parent_parts[..=i].join(".");
        if current.is_none() {
            *current = Item::Table(toml_edit::Table::new());
        }
        let tbl = current
            .as_table_mut()
            .ok_or_else(|| format!("Config path '{path_so_far}' exists but is not a table"))?;
        if !tbl.contains_key(part) {
            let mut t = toml_edit::Table::new();
            t.set_implicit(false);
            tbl.insert(part, Item::Table(t));
        }
        current = tbl
            .get_mut(part)
            .ok_or_else(|| format!("Failed to descend into '{path_so_far}'"))?;
    }

    let leaf_tbl = current
        .as_table_mut()
        .ok_or_else(|| format!("Parent path '{}' is not a table", parent_parts.join(".")))?;
    leaf_tbl.insert(leaf_key, config_item_from_text_value(value));
    Ok(())
}

fn top_level_edit_keys(doc: &DocumentMut) -> BTreeSet<String> {
    doc.as_table().iter().map(|(k, _)| k.to_string()).collect()
}

fn validate_top_keys_preserved(
    old_top_keys: &BTreeSet<String>,
    doc: &DocumentMut,
    backup_path: &Path,
) -> Result<(), String> {
    let new_top_keys = top_level_edit_keys(doc);
    let lost: Vec<&String> = old_top_keys.difference(&new_top_keys).collect();
    if !lost.is_empty() {
        return Err(format!(
            "Refusing to write: top-level keys would be lost: {lost:?}. Backup at {}",
            backup_path.display()
        ));
    }
    Ok(())
}

fn validate_serialized_config_size(
    old_size: usize,
    serialized_size: usize,
    backup_path: &Path,
) -> Result<(), String> {
    if old_size > 100 && serialized_size < (old_size * 7 / 10) {
        return Err(format!(
            "Refusing to write: serialized config shrank suspiciously ({} → {} bytes). \
             Backup at {}",
            old_size,
            serialized_size,
            backup_path.display()
        ));
    }
    Ok(())
}

fn write_config_and_validate_roundtrip(
    config_path: &Path,
    serialized: &str,
    old_top_keys: &BTreeSet<String>,
    backup_path: &Path,
) -> Result<(), String> {
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())
        .map_err(|e| format!("Failed to persist config.toml: {e}"))?;

    let reparsed = std::fs::read_to_string(config_path)
        .map_err(|e| e.to_string())
        .and_then(|s| s.parse::<toml::Value>().map_err(|e| e.to_string()));
    match reparsed {
        Ok(value) => validate_roundtrip_top_keys(config_path, old_top_keys, backup_path, &value),
        Err(e) => {
            let _ = captain_types::durable_fs::atomic_copy(backup_path, config_path);
            Err(format!(
                "Roundtrip re-parse failed ({e}). Rolled back from {}",
                backup_path.display()
            ))
        }
    }
}

fn validate_roundtrip_top_keys(
    config_path: &Path,
    old_top_keys: &BTreeSet<String>,
    backup_path: &Path,
    reparsed: &toml::Value,
) -> Result<(), String> {
    let rt_keys: BTreeSet<String> = reparsed
        .as_table()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();
    let lost: Vec<&String> = old_top_keys.difference(&rt_keys).collect();
    if !lost.is_empty() {
        let _ = captain_types::durable_fs::atomic_copy(backup_path, config_path);
        return Err(format!(
            "Roundtrip validation failed, keys lost after write: {lost:?}. \
             Rolled back from {}",
            backup_path.display()
        ));
    }
    Ok(())
}

fn config_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn config_item_from_text_value(value: &str) -> Item {
    if value == "true" {
        toml_edit::value(true)
    } else if value == "false" {
        toml_edit::value(false)
    } else if let Ok(n) = value.parse::<i64>() {
        toml_edit::value(n)
    } else if let Ok(f) = value.parse::<f64>() {
        toml_edit::value(f)
    } else {
        toml_edit::value(value.to_string())
    }
}

fn upsert_secret_line(lines: &mut Vec<String>, key: &str, value: &str) {
    for line in lines.iter_mut() {
        if let Some((k, _)) = line.split_once('=') {
            if k.trim() == key {
                *line = format!("{}={}", key, value);
                return;
            }
        }
    }
    lines.push(format!("{}={}", key, value));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_item_from_text_value_preserves_typed_config_writes() {
        assert_eq!(config_item_from_text_value("true").to_string(), "true");
        assert_eq!(config_item_from_text_value("42").to_string(), "42");
        assert_eq!(config_item_from_text_value("3.5").to_string(), "3.5");
        assert_eq!(
            config_item_from_text_value("gpt-5").to_string(),
            "\"gpt-5\""
        );
    }

    #[test]
    fn config_value_to_string_matches_tool_contract() {
        assert_eq!(
            config_value_to_string(&toml::Value::String("codex".into())),
            "codex"
        );
        assert_eq!(config_value_to_string(&toml::Value::Integer(7)), "7");
        assert_eq!(
            config_value_to_string(&toml::Value::Boolean(false)),
            "false"
        );
    }

    #[test]
    fn apply_config_write_path_creates_nested_tables_with_typed_value() {
        let mut doc: DocumentMut = "default_model = { provider = \"codex\" }\n"
            .parse()
            .unwrap();

        apply_config_write_path(&mut doc, "channels.telegram.enabled", "true").unwrap();

        assert_eq!(doc["channels"]["telegram"]["enabled"].as_bool(), Some(true));
        assert!(top_level_edit_keys(&doc).contains("default_model"));
    }

    #[test]
    fn apply_config_write_path_rejects_scalar_parent() {
        let mut doc: DocumentMut = "channels = \"disabled\"\n".parse().unwrap();
        let err = apply_config_write_path(&mut doc, "channels.telegram.enabled", "true")
            .expect_err("scalar parent should be rejected");

        assert_eq!(
            err,
            "Config path 'channels.telegram' exists but is not a table"
        );
    }

    #[test]
    fn upsert_secret_line_updates_existing_or_appends() {
        let mut lines = vec![
            "# comment".to_string(),
            "OPENAI_API_KEY=old".to_string(),
            "OTHER=1".to_string(),
        ];
        upsert_secret_line(&mut lines, "OPENAI_API_KEY", "new");
        upsert_secret_line(&mut lines, "GROQ_API_KEY", "fresh");

        assert_eq!(lines[1], "OPENAI_API_KEY=new");
        assert_eq!(lines[3], "GROQ_API_KEY=fresh");
    }
}
