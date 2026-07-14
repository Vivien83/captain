//! Provider credential and connectivity route handlers.

use crate::secret_env::{remove_secret_env, write_secret_env};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

type ApiJsonResponse = (StatusCode, Json<serde_json::Value>);

fn provider_env_var(state: &AppState, name: &str) -> String {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    catalog
        .get_provider(name)
        .map(|provider| provider.api_key_env.clone())
        .unwrap_or_else(|| format!("{}_API_KEY", name.to_uppercase().replace('-', "_")))
}

/// POST /api/providers/{name}/key - Save an API key for a provider.
///
/// SECURITY: Writes to `~/.captain/secrets.env`, sets env var in process,
/// and refreshes auth detection. Key is zeroized after use.
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let key = match parse_provider_key(&body) {
        Ok(key) => key,
        Err(response) => return response,
    };

    let env_var = provider_env_var(&state, &name);
    if let Err(error) = persist_provider_key(&state, &env_var, &key) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, error);
    }

    let (current_provider, current_key_env) = current_provider_key_env(&state);
    let suggested_default_model =
        suggested_default_model_for_saved_key(&state, &current_provider, &current_key_env, &name);
    let switched =
        refresh_current_default_key_env_if_needed(&state, &current_provider, &name, &env_var);

    let response = provider_key_saved_response(name, switched, suggested_default_model);
    (StatusCode::OK, Json(response))
}

fn parse_provider_key(body: &serde_json::Value) -> Result<String, ApiJsonResponse> {
    match body["key"].as_str() {
        Some(key) if !key.trim().is_empty() => Ok(key.trim().to_string()),
        _ => Err(json_error(
            StatusCode::BAD_REQUEST,
            "Missing or empty 'key' field".to_string(),
        )),
    }
}

fn json_error(status: StatusCode, error: String) -> ApiJsonResponse {
    (status, Json(serde_json::json!({ "error": error })))
}

fn persist_provider_key(state: &AppState, env_var: &str, key: &str) -> Result<(), String> {
    state.kernel.store_credential(env_var, key);

    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    write_secret_env(&secrets_path, env_var, key)
        .map_err(|error| format!("Failed to write secrets.env: {error}"))?;

    std::env::set_var(env_var, key);
    refresh_provider_auth(state);
    Ok(())
}

fn refresh_provider_auth(state: &AppState) {
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .detect_auth();
}

fn current_provider_key_env(state: &AppState) -> (String, String) {
    let guard = state
        .kernel
        .default_model_override
        .read()
        .unwrap_or_else(|error| error.into_inner());
    match guard.as_ref() {
        Some(default_model) => (
            default_model.provider.clone(),
            default_model.api_key_env.clone(),
        ),
        None => (
            state.kernel.config.default_model.provider.clone(),
            state.kernel.config.default_model.api_key_env.clone(),
        ),
    }
}

fn env_has_key(env_var: &str) -> bool {
    !env_var.is_empty()
        && std::env::var(env_var)
            .ok()
            .filter(|value| !value.is_empty())
            .is_some()
}

fn suggested_default_model_for_saved_key(
    state: &AppState,
    current_provider: &str,
    current_key_env: &str,
    saved_provider: &str,
) -> Option<String> {
    if env_has_key(current_key_env) || current_provider == saved_provider {
        return None;
    }

    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|error| error.into_inner());
    catalog.default_model_for_provider(saved_provider)
}

fn refresh_current_default_key_env_if_needed(
    state: &AppState,
    current_provider: &str,
    saved_provider: &str,
    env_var: &str,
) -> bool {
    if current_provider != saved_provider {
        return false;
    }

    let needs_update = {
        let guard = state
            .kernel
            .default_model_override
            .read()
            .unwrap_or_else(|error| error.into_inner());
        match guard.as_ref() {
            Some(default_model) => default_model.api_key_env != env_var,
            None => state.kernel.config.default_model.api_key_env != env_var,
        }
    };
    if needs_update {
        let mut guard = state
            .kernel
            .default_model_override
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let base = guard
            .clone()
            .unwrap_or_else(|| state.kernel.config.default_model.clone());
        *guard = Some(captain_types::config::DefaultModelConfig {
            api_key_env: env_var.to_string(),
            ..base
        });
    }
    false
}

fn provider_key_saved_response(
    name: String,
    switched: bool,
    suggested_default_model: Option<String>,
) -> serde_json::Value {
    let mut response = serde_json::json!({"status": "saved", "provider": name});
    if switched {
        response["switched_default"] = serde_json::json!(true);
        response["message"] = serde_json::json!(format!(
            "API key saved and default provider switched to '{}'.",
            name
        ));
    } else if let Some(model_id) = suggested_default_model {
        response["default_switch_available"] = serde_json::json!(true);
        response["suggested_default_model"] = serde_json::json!(model_id);
        response["message"] = serde_json::json!(
            "API key saved. Default provider was not changed automatically; use model_switch_plan/model_switch_apply to switch safely."
        );
    }
    response
}

/// DELETE /api/providers/{name}/key - Remove an API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let env_var = provider_env_var(&state, &name);

    if env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider does not require an API key"})),
        );
    }

    state.kernel.remove_credential(&env_var);

    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update secrets.env: {e}")})),
        );
    }

    std::env::remove_var(&env_var);
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "provider": name})),
    )
}

/// POST /api/providers/{name}/test - Test a provider's connectivity.
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let current_default = state.kernel.effective_default_model();
    let (env_var, base_url, key_required, default_model) = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(provider) => {
                let mut model_id = if current_default.provider == name {
                    current_default.model.clone()
                } else {
                    catalog
                        .default_model_for_provider(&name)
                        .unwrap_or_default()
                };
                if name == "codex" {
                    model_id = model_id
                        .strip_prefix("codex/")
                        .unwrap_or(&model_id)
                        .to_string();
                }
                (
                    provider.api_key_env.clone(),
                    provider.base_url.clone(),
                    provider.key_required,
                    model_id,
                )
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": format!("Unknown provider '{}'", name)})),
                );
            }
        }
    };

    let api_key = std::env::var(&env_var).ok();
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider API key not configured"})),
        );
    }

    let start = std::time::Instant::now();
    let driver_config = captain_runtime::llm_driver::DriverConfig {
        provider: name.clone(),
        api_key,
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
        skip_permissions: true,
    };

    match captain_runtime::drivers::create_driver(&driver_config) {
        Ok(driver) => {
            let test_req = captain_runtime::llm_driver::CompletionRequest {
                model: default_model.clone(),
                messages: vec![captain_types::message::Message::user("Hi")],
                tools: vec![],
                max_tokens: 1,
                temperature: 0.0,
                system: None,
                thinking: None,
                tool_choice: None,
                cache_hints: captain_runtime::llm_driver::CacheHints::default(),
            };
            match driver.complete(test_req).await {
                Ok(_) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "provider": name,
                        "latency_ms": start.elapsed().as_millis(),
                    })),
                ),
                Err(e) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "provider": name,
                        "error": format!("{e}"),
                    })),
                ),
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Failed to create driver: {e}"),
            })),
        ),
    }
}

/// PUT /api/providers/{name}/url - Set a custom base URL for a provider.
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let base_url = match body["base_url"].as_str() {
        Some(url) if !url.trim().is_empty() => url.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'base_url' field"})),
            );
        }
    };

    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "base_url must start with http:// or https://"})),
        );
    }

    {
        let mut catalog = state
            .kernel
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
    }

    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        );
    }

    let probe = captain_runtime::provider_health::probe_provider(&name, &base_url).await;

    if !probe.discovered_models.is_empty() {
        if let Ok(mut catalog) = state.kernel.model_catalog.write() {
            catalog.merge_discovered_models(&name, &probe.discovered_models);
        }
    }

    let mut response = serde_json::json!({
        "status": "saved",
        "provider": name,
        "base_url": base_url,
        "reachable": probe.reachable,
        "latency_ms": probe.latency_ms,
    });
    if !probe.discovered_models.is_empty() {
        response["discovered_models"] = serde_json::json!(probe.discovered_models);
    }

    (StatusCode::OK, Json(response))
}

fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_urls") {
        root.insert(
            "provider_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let urls_table = root
        .get_mut("provider_urls")
        .and_then(|value| value.as_table_mut())
        .ok_or("provider_urls is not a table")?;

    urls_table.insert(provider.to_string(), toml::Value::String(url.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(config_path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_provider_key, provider_key_saved_response};
    use axum::http::StatusCode;

    #[test]
    fn parse_provider_key_trims_and_rejects_empty_values() {
        let parsed = parse_provider_key(&serde_json::json!({"key": "  sk-test  "}))
            .expect("valid key should parse");
        assert_eq!(parsed, "sk-test");

        let err = parse_provider_key(&serde_json::json!({"key": "   "}))
            .expect_err("empty key should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(err.1 .0["error"], "Missing or empty 'key' field");
    }

    #[test]
    fn provider_key_response_preserves_safe_switch_guidance() {
        let response = provider_key_saved_response(
            "groq".to_string(),
            false,
            Some("groq/llama-3.3-70b-versatile".to_string()),
        );

        assert_eq!(response["status"], "saved");
        assert_eq!(response["provider"], "groq");
        assert_eq!(response["default_switch_available"], true);
        assert_eq!(
            response["suggested_default_model"],
            "groq/llama-3.3-70b-versatile"
        );
        assert!(response["message"]
            .as_str()
            .unwrap()
            .contains("Default provider was not changed automatically"));
    }
}
