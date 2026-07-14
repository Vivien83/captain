use crate::kernel_handle::KernelHandle;
use crate::tools::{ensure_no_secret_literal, require_kernel};
use std::sync::Arc;

pub(crate) fn tool_config_read(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    match kh.config_read(path)? {
        Some(val) => Ok(val),
        None => Ok(format!("Config path '{}' not found or empty.", path)),
    }
}

pub(crate) async fn tool_config_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let value = input["value"].as_str().ok_or("Missing 'value' parameter")?;
    if matches!(path, "default_model.provider" | "default_model.model") {
        return Err(
            "Direct default_model provider/model writes are refused. Use model_switch_plan first, then model_switch_apply with explicit session_strategy (new_session or compact_session)."
                .to_string(),
        );
    }
    ensure_no_secret_literal("config_write", "value", value)?;

    // Closed-set fields reject invalid values outright — force does not
    // bypass this (an invalid value is always an error, live case:
    // stt_model = "openai" was accepted then ignored by the runtime).
    if let Some(allowed) = allowed_values_for(path) {
        if !allowed.contains(&value) {
            return Err(format!(
                "Invalid value '{value}' for '{path}'. Allowed values: {}.",
                allowed.join(", ")
            ));
        }
    }

    // Validate the key against the runtime schema at write time, so a
    // single call either succeeds or explains itself. Without this, an
    // invented key (e.g. 'stt.provider') was silently written and ignored
    // by the runtime — which trained agents into defensive bursts of
    // config_schema/config_read/captain_docs before every write.
    let force = input["force"].as_bool().unwrap_or(false);
    if !force && kh.config_read(path).ok().flatten().is_none() {
        let schema = kh.config_schema()?;
        let schema: toml::Value = toml::from_str(&schema)
            .map_err(|e| format!("config schema unavailable for validation: {e}"))?;
        if let PathClass::Unknown {
            deepest_known,
            valid_keys,
        } = classify_config_path(&schema, path)
        {
            let keys = if valid_keys.is_empty() {
                "(none)".to_string()
            } else {
                valid_keys.join(", ")
            };
            return Err(format!(
                "Unknown config key '{path}': the runtime would ignore it. \
                 Valid keys under '{deepest_known}': {keys}. \
                 If this is a documented optional field absent from the schema \
                 template, retry with force:true. If you meant a user \
                 preference rather than runtime config, use memory_save."
            ));
        }
    }

    kh.config_write(path, value).await?;
    Ok(format!("Config '{}' set to '{}'.", path, value))
}

/// Closed value sets for config fields that only accept specific values.
/// Kept small on purpose: only fields whose invalid values were observed
/// (or would be) silently ignored by the runtime.
fn allowed_values_for(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "stt_model" => Some(captain_types::config::ALLOWED_STT_MODELS),
        "log_level" => Some(&["trace", "debug", "info", "warn", "error"]),
        _ => None,
    }
}

/// Outcome of resolving a dotted path against the schema template.
enum PathClass {
    /// Every segment resolves (or the parent is a dynamic map).
    Known,
    /// A segment diverges from a known, non-empty table.
    Unknown {
        deepest_known: String,
        valid_keys: Vec<String>,
    },
}

/// Resolve `path` segment by segment inside the schema TOML.
/// An empty table in the schema is treated as a dynamic map (user-keyed
/// sections), which accepts any sub-key.
fn classify_config_path(schema: &toml::Value, path: &str) -> PathClass {
    let mut current = schema;
    let mut resolved: Vec<&str> = Vec::new();
    for segment in path.split('.') {
        let Some(table) = current.as_table() else {
            // Descending into a scalar: parent is known, path is deeper
            // than the schema models. Accept (arrays/inline structures).
            return PathClass::Known;
        };
        if table.is_empty() {
            return PathClass::Known;
        }
        match table.get(segment) {
            Some(next) => {
                current = next;
                resolved.push(segment);
            }
            None => {
                return PathClass::Unknown {
                    deepest_known: if resolved.is_empty() {
                        "(top level)".to_string()
                    } else {
                        resolved.join(".")
                    },
                    valid_keys: table.keys().cloned().collect(),
                };
            }
        }
    }
    PathClass::Known
}

pub(crate) async fn tool_self_configure(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Cannot self-configure without an agent context")?;
    let config_json =
        serde_json::to_string(input).map_err(|e| format!("Failed to serialize config: {e}"))?;
    ensure_no_secret_literal("self_configure", "input", &config_json)?;
    kh.update_self_config(agent_id, &config_json).await
}

pub(crate) fn tool_model_switch_plan(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Cannot plan a model switch without an agent context")?;
    let model = input["model"].as_str().ok_or("Missing 'model' parameter")?;
    let provider = input.get("provider").and_then(|v| v.as_str());
    let mut plan = kh.model_switch_plan(agent_id, model, provider)?;

    let can_apply = plan
        .get("can_apply")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let target_provider = plan
        .get("target_provider")
        .and_then(|v| v.as_str())
        .unwrap_or(provider.unwrap_or(""))
        .to_string();
    let target_model = plan
        .get("target_model")
        .and_then(|v| v.as_str())
        .unwrap_or(model)
        .to_string();

    if let Some(obj) = plan.as_object_mut() {
        obj.insert(
            "next_action".to_string(),
            serde_json::json!({
                "if_user_already_chose": "Call model_switch_apply immediately with the matching session_strategy. Do not inspect docs, do not run shell commands, and do not write default_model directly.",
                "if_choice_missing": "Use ask_user with options Nouvelle session and Resume compact, then call model_switch_apply with new_session or compact_session.",
                "valid_choice_mapping": {
                    "Nouvelle": "new_session",
                    "Nouvelle session": "new_session",
                    "Resume compact": "compact_session",
                    "Résumé compact": "compact_session"
                }
            }),
        );
        obj.insert(
            "pending_choice_contract".to_string(),
            serde_json::json!({
                "status": if can_apply { "awaiting_user_choice" } else { "not_created" },
                "model": target_model.as_str(),
                "provider": target_provider.as_str(),
                "ttl_seconds": 900
            }),
        );
    }

    if can_apply {
        let key = crate::model_switch_pending::pending_model_switch_key(agent_id);
        let expires_at_unix = chrono::Utc::now().timestamp() + 900;
        if let Err(e) = kh.memory_kv_store(
            &key,
            serde_json::json!({
                "status": "pending",
                "agent_id": agent_id,
                "provider": target_provider.as_str(),
                "model": target_model.as_str(),
                "created_at": chrono::Utc::now().to_rfc3339(),
                "expires_at_unix": expires_at_unix,
                "source": "model_switch_plan"
            }),
        ) {
            if let Some(obj) = plan.as_object_mut() {
                obj.insert("pending_choice_warning".to_string(), serde_json::json!(e));
            }
        }
    }

    serde_json::to_string_pretty(&plan).map_err(|e| format!("Failed to serialize plan: {e}"))
}

pub(crate) fn tool_model_switch_apply(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Cannot apply a model switch without an agent context")?;
    let model = input["model"].as_str().ok_or("Missing 'model' parameter")?;
    let strategy = input["session_strategy"]
        .as_str()
        .ok_or("Missing 'session_strategy' parameter")?;
    let provider = input.get("provider").and_then(|v| v.as_str());
    let result = kh.model_switch_apply(agent_id, model, provider, strategy)?;
    let _ = kh.memory_kv_store(
        &crate::model_switch_pending::pending_model_switch_key(agent_id),
        serde_json::json!({
            "status": "consumed",
            "updated_at": chrono::Utc::now().to_rfc3339(),
            "source": "model_switch_apply"
        }),
    );
    serde_json::to_string_pretty(&result).map_err(|e| format!("Failed to serialize result: {e}"))
}

pub(crate) fn tool_secret_read(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    match kh.secret_read(key)? {
        Some(val) => {
            let masked = if val.len() > 8 {
                format!(
                    "{}...{} ({} chars)",
                    &val[..4],
                    &val[val.len() - 4..],
                    val.len()
                )
            } else {
                format!("****** ({} chars)", val.len())
            };
            Ok(format!("Secret '{}': {}", key, masked))
        }
        None => Ok(format!("Secret '{}' not found.", key)),
    }
}

pub(crate) fn tool_secret_write(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input["value"].as_str().ok_or("Missing 'value' parameter")?;
    kh.secret_write(key, value)?;
    Ok(format!("Secret '{}' stored successfully.", key))
}

pub(crate) async fn tool_config_setup(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let integration = input
        .get("integration")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'integration' (string) parameter")?;
    let creds = match input.get("credentials") {
        Some(value) => value.clone(),
        None => {
            let mut legacy = serde_json::Map::new();
            if let Some(obj) = input.as_object() {
                for (key, value) in obj {
                    if !matches!(key.as_str(), "integration" | "run_test" | "test") {
                        legacy.insert(key.clone(), value.clone());
                    }
                }
            }
            if legacy.is_empty() {
                return Err("Missing 'credentials' object. Example: {\"integration\":\"tts_elevenlabs\",\"credentials\":{\"api_key\":\"...\"},\"run_test\":true}".into());
            }
            serde_json::Value::Object(legacy)
        }
    };
    if !creds.is_object() {
        return Err("'credentials' must be a JSON object".into());
    }
    let run_test = input
        .get("run_test")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let home = kh
        .home_dir()
        .ok_or("Kernel did not expose a home_dir; cannot locate config.toml")?;
    let config_path = home.join("config.toml");
    let kh_for_vault = kh.clone();
    let vault_set = move |key: &str, value: &str| -> Result<(), String> {
        kh_for_vault.secret_write(key, value)
    };
    let kh_for_notify = kh.clone();
    let notify = move |n: &str| {
        kh_for_notify.publish_integration_configured(n);
    };
    let outcome = crate::integrations::setup_integration(
        integration,
        &creds,
        &config_path,
        vault_set,
        run_test,
        Some(&notify),
    )
    .await?;

    serde_json::to_string_pretty(&outcome).map_err(|e| format!("serialize outcome: {e}"))
}

pub(crate) fn tool_config_schema(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    kh.config_schema()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> toml::Value {
        toml::from_str(&captain_types::config_template::render_default_toml().unwrap()).unwrap()
    }

    #[test]
    fn known_paths_classify_as_known() {
        let s = schema();
        assert!(matches!(
            classify_config_path(&s, "data_dir"),
            PathClass::Known
        ));
        assert!(matches!(
            classify_config_path(&s, "default_model.provider"),
            PathClass::Known
        ));
    }

    /// The live failure mode: an invented section ('stt.provider') was
    /// silently written and ignored, so agents pre-checked defensively.
    #[test]
    fn invented_section_is_rejected_with_top_level_keys() {
        let s = schema();
        let PathClass::Unknown {
            deepest_known,
            valid_keys,
        } = classify_config_path(&s, "stt.provider")
        else {
            panic!("invented section must be Unknown");
        };
        assert_eq!(deepest_known, "(top level)");
        assert!(valid_keys.iter().any(|k| k == "default_model"));
    }

    #[test]
    fn unknown_subkey_reports_its_known_parent() {
        let s = schema();
        let PathClass::Unknown { deepest_known, .. } =
            classify_config_path(&s, "default_model.banana")
        else {
            panic!("unknown subkey must be Unknown");
        };
        assert_eq!(deepest_known, "default_model");
    }

    /// Live case: stt_model = "openai" (a provider name, not a model) was
    /// accepted then silently ignored by the runtime.
    #[test]
    fn closed_set_fields_reject_invalid_values() {
        let allowed = allowed_values_for("stt_model").expect("stt_model is closed-set");
        assert!(!allowed.contains(&"openai"));
        assert!(allowed.contains(&"whisper-small"));
        assert!(allowed_values_for("data_dir").is_none());
    }

    /// An empty table in the schema is a dynamic user-keyed map: any
    /// sub-key is accepted.
    #[test]
    fn empty_schema_table_accepts_any_subkey() {
        let s: toml::Value = toml::from_str("[dynamic]\n").unwrap();
        assert!(matches!(
            classify_config_path(&s, "dynamic.anything.goes"),
            PathClass::Known
        ));
    }
}
