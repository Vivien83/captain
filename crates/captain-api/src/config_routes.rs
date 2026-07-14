use crate::state::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use std::{path::Path, sync::Arc};

type ConfigJsonResponse = (StatusCode, Json<serde_json::Value>);

#[derive(Debug)]
struct ConfigSetRequest {
    path: String,
    value: serde_json::Value,
}

/// GET /api/config - Get kernel configuration with secrets redacted.
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.kernel.config;
    let default_model = state.kernel.effective_default_model();
    Json(serde_json::json!({
        "home_dir": config.home_dir.to_string_lossy(),
        "data_dir": config.data_dir.to_string_lossy(),
        "api_key": if config.api_key.is_empty() { "not set" } else { "***" },
        "default_model": {
            "provider": default_model.provider,
            "model": default_model.model,
            "api_key_env": default_model.api_key_env,
        },
        "memory": {
            "backend": match config.memory.backend {
                captain_types::config::MemoryBackend::Graph => "graph",
                captain_types::config::MemoryBackend::Mempalace => "mempalace",
            },
            "decay_rate": config.memory.decay_rate,
            "consolidation_interval_hours": config.memory.consolidation_interval_hours,
        },
        "web_terminal": {
            "enabled": config.web_terminal.enabled,
            "default_mode": config.web_terminal.default_mode,
            "allow_raw_shell": config.web_terminal.allow_raw_shell,
            "max_sessions": config.web_terminal.max_sessions,
        },
        "deployment": {
            "profile": config.deployment.profile,
            "public_url": config.deployment.public_url,
            "https": config.deployment.https,
            "reverse_proxy": config.deployment.reverse_proxy,
        },
    }))
}

/// POST /api/config/reload - Reload configuration from disk.
pub async fn config_reload(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partial"
            } else if plan.has_changes() {
                "applied"
            } else {
                "no_changes"
            };

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    "restart_required": plan.restart_required,
                    "restart_reasons": plan.restart_reasons,
                    "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                    "noop_changes": plan.noop_changes,
                })),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

/// GET /api/config/raw - Read raw config.toml content.
pub async fn config_raw_get(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config_path = state.kernel.config.home_dir.join("config.toml");
    match std::fs::read_to_string(&config_path) {
        Ok(content) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "content": content,
                "path": config_path.display().to_string(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to read config: {e}")})),
        ),
    }
}

/// PUT /api/config/raw - Write config.toml after validation and snapshot.
pub async fn config_raw_put(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let content = match raw_config_content(&body) {
        Ok(content) => content,
        Err(response) => return response,
    };

    if let Err(e) = validate_raw_config_content(content) {
        return raw_config_error(StatusCode::BAD_REQUEST, e);
    }

    let config_path = state.kernel.config.home_dir.join("config.toml");
    let backup_dir = state.kernel.config.home_dir.join("config-backups");
    let snapshot_path = match create_config_snapshot(&config_path, &backup_dir) {
        Ok(snapshot_path) => snapshot_path,
        Err(error) => return raw_config_error(StatusCode::INTERNAL_SERVER_ERROR, error),
    };

    match write_raw_config_atomically(&config_path, content) {
        Ok(()) => {
            tracing::info!("Config updated via raw editor");
            record_raw_config_audit(&state);
            raw_config_saved_response(&snapshot_path)
        }
        Err(error) => {
            let rolled_back = rollback_config_snapshot(&config_path, &snapshot_path);
            raw_config_failed_response(error, rolled_back, &snapshot_path)
        }
    }
}

fn raw_config_content(body: &serde_json::Value) -> Result<&str, ConfigJsonResponse> {
    body.get("content")
        .and_then(|value| value.as_str())
        .ok_or_else(|| raw_config_error(StatusCode::BAD_REQUEST, "Missing 'content' field"))
}

fn raw_config_error(status: StatusCode, error: impl Into<String>) -> ConfigJsonResponse {
    (status, Json(serde_json::json!({"error": error.into()})))
}

fn create_config_snapshot(
    config_path: &Path,
    backup_dir: &Path,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(backup_dir)
        .map_err(|error| format!("Failed to create config backup dir: {error}"))?;
    let ts = chrono::Utc::now()
        .format("%Y-%m-%dT%H-%M-%S-%3f")
        .to_string();
    let snapshot_path = backup_dir.join(format!("config.toml.raw-editor.{ts}"));
    if config_path.exists() {
        std::fs::copy(config_path, &snapshot_path)
            .map_err(|error| format!("Failed to create config snapshot: {error}"))?;
        tracing::info!("Config snapshot saved to {}", snapshot_path.display());
    }
    Ok(snapshot_path)
}

fn write_raw_config_atomically(config_path: &Path, content: &str) -> Result<(), String> {
    let tmp_path = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, content)
        .map_err(|error| format!("Failed to write config temp file: {error}"))
        .and_then(|_| set_config_file_permissions(&tmp_path))
        .and_then(|_| {
            std::fs::rename(&tmp_path, config_path)
                .map_err(|error| format!("Failed to replace config.toml: {error}"))
        })
        .and_then(|_| set_config_file_permissions(config_path))
        .and_then(|_| {
            let written = std::fs::read_to_string(config_path)
                .map_err(|error| format!("Failed to re-read config.toml: {error}"))?;
            validate_raw_config_content(&written)
        })
}

fn rollback_config_snapshot(config_path: &Path, snapshot_path: &Path) -> bool {
    if !snapshot_path.exists() {
        return false;
    }
    let _ = std::fs::copy(snapshot_path, config_path);
    let _ = set_config_file_permissions(config_path);
    true
}

fn record_raw_config_audit(state: &AppState) {
    state.kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::ConfigChange,
        "config.toml updated via raw editor",
        "ok",
    );
}

fn raw_config_saved_response(snapshot_path: &Path) -> ConfigJsonResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "saved",
            "snapshot": snapshot_path.display().to_string(),
        })),
    )
}

fn raw_config_failed_response(
    error: String,
    rolled_back: bool,
    snapshot_path: &Path,
) -> ConfigJsonResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": error,
            "rolled_back": rolled_back,
            "snapshot": snapshot_path.display().to_string(),
        })),
    )
}

/// POST /api/config/validate - Validate raw config.toml content.
pub async fn config_validate(Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let content = match body.get("content").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "Missing 'content' field"})),
            );
        }
    };

    match validate_raw_config_content(content) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "message": "config.toml is valid",
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

/// GET /api/config/template - Return the default config.toml template.
pub async fn config_template_get() -> impl IntoResponse {
    match captain_types::config_template::render_default_toml_with_header() {
        Ok(content) => (
            StatusCode::OK,
            Json(serde_json::json!({ "content": content })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to render config template: {e}")})),
        ),
    }
}

/// GET /api/config/schema - Return the active config editor schema.
pub async fn config_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(config_schema_document(config_schema_options(&state)))
}

struct ConfigSchemaOptions {
    provider_options: Vec<String>,
    model_options: Vec<serde_json::Value>,
}

fn config_schema_options(state: &AppState) -> ConfigSchemaOptions {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_options: Vec<String> = catalog
        .list_providers()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let model_options: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .map(|m| serde_json::json!({"id": m.id, "name": m.display_name, "provider": m.provider}))
        .collect();
    ConfigSchemaOptions {
        provider_options,
        model_options,
    }
}

fn config_schema_document(options: ConfigSchemaOptions) -> serde_json::Value {
    serde_json::json!({
        "sections": {
            "general": general_config_schema(),
            "default_model": default_model_config_schema(options),
            "memory": simple_fields_schema(&[
                ("decay_rate", "number"),
                ("vector_dims", "number"),
            ]),
            "web": simple_fields_schema(&[
                ("provider", "string"),
                ("timeout_secs", "number"),
                ("max_results", "number"),
            ]),
            "browser": simple_fields_schema(&[
                ("headless", "boolean"),
                ("timeout_secs", "number"),
                ("executable_path", "string"),
            ]),
            "network": network_config_schema(),
            "extensions": simple_fields_schema(&[
                ("auto_connect", "boolean"),
                ("health_check_interval_secs", "number"),
            ]),
            "vault": simple_fields_schema(&[("path", "string")]),
            "a2a": simple_fields_schema(&[
                ("enabled", "boolean"),
                ("name", "string"),
                ("description", "string"),
                ("url", "string"),
            ]),
            "channels": channels_config_schema(),
        }
    })
}

fn general_config_schema() -> serde_json::Value {
    serde_json::json!({
        "root_level": true,
        "fields": {
            "api_listen": "string",
            "api_key": direct_secret_schema(
                &["CAPTAIN_DAEMON_API_KEY", "CAPTAIN_API_KEY"],
                "Set the daemon API key with captain setup access or secrets.env; direct config writes are rejected."
            ),
            "log_level": "string"
        }
    })
}

fn default_model_config_schema(options: ConfigSchemaOptions) -> serde_json::Value {
    serde_json::json!({
        "hot_reloadable": true,
        "fields": {
            "provider": { "type": "select", "options": options.provider_options },
            "model": { "type": "select", "options": options.model_options },
            "api_key_env": "string",
            "base_url": "string"
        }
    })
}

fn simple_fields_schema(fields: &[(&str, &str)]) -> serde_json::Value {
    let fields = fields
        .iter()
        .map(|(name, field_type)| ((*name).to_string(), serde_json::json!(field_type)))
        .collect::<serde_json::Map<_, _>>();
    serde_json::json!({ "fields": fields })
}

fn network_config_schema() -> serde_json::Value {
    serde_json::json!({
        "fields": {
            "enabled": "boolean",
            "listen_addr": "string",
            "shared_secret": direct_secret_schema(
                &[],
                "Direct network shared_secret writes are rejected from config.toml."
            )
        }
    })
}

fn channels_config_schema() -> serde_json::Value {
    simple_fields_schema(&[
        ("telegram", "object"),
        ("discord", "object"),
        ("signal", "object"),
        ("email", "object"),
    ])
}

/// POST /api/config/set - Set one config value and persist to config.toml.
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let request = match parse_config_set_request(&body) {
        Ok(request) => request,
        Err(response) => return response,
    };

    let config_path = state.kernel.config.home_dir.join("config.toml");
    let mut table = read_config_table(&config_path);
    if let Err(response) = apply_config_set_value(&mut table, &request.path, &request.value) {
        return response;
    }
    if let Err(response) = validate_and_write_config_table(&config_path, &table) {
        return response;
    }

    let reload_status = config_reload_status(&state);
    record_config_set_audit(&state, &request.path);

    config_set_saved_response(reload_status, request.path)
}

fn parse_config_set_request(
    body: &serde_json::Value,
) -> Result<ConfigSetRequest, ConfigJsonResponse> {
    let path = body
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| config_set_error(StatusCode::BAD_REQUEST, "missing 'path' field"))?;
    let value = body
        .get("value")
        .cloned()
        .ok_or_else(|| config_set_error(StatusCode::BAD_REQUEST, "missing 'value' field"))?;
    Ok(ConfigSetRequest { path, value })
}

fn config_set_error(status: StatusCode, error: impl Into<String>) -> ConfigJsonResponse {
    (
        status,
        Json(serde_json::json!({"status": "error", "error": error.into()})),
    )
}

fn read_config_table(config_path: &Path) -> toml::value::Table {
    if !config_path.exists() {
        return toml::value::Table::new();
    }
    std::fs::read_to_string(config_path)
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

fn apply_config_set_value(
    table: &mut toml::value::Table,
    path: &str,
    value: &serde_json::Value,
) -> Result<(), ConfigJsonResponse> {
    let toml_val = json_to_toml_value(value);
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            table.insert(parts[0].to_string(), toml_val);
            Ok(())
        }
        2 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut nested) = section {
                nested.insert(parts[1].to_string(), toml_val);
            }
            Ok(())
        }
        3 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut nested) = section {
                let sub = nested
                    .entry(parts[1].to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut leaf) = sub {
                    leaf.insert(parts[2].to_string(), toml_val);
                }
            }
            Ok(())
        }
        _ => Err(config_set_error(
            StatusCode::BAD_REQUEST,
            "path too deep (max 3 levels)",
        )),
    }
}

fn validate_and_write_config_table(
    config_path: &Path,
    table: &toml::value::Table,
) -> Result<(), ConfigJsonResponse> {
    validate_config_value(toml::Value::Table(table.clone()))
        .map_err(|error| config_set_error(StatusCode::BAD_REQUEST, error))?;
    let toml_string = toml::to_string_pretty(table).map_err(|error| {
        config_set_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize failed: {error}"),
        )
    })?;
    std::fs::write(config_path, &toml_string).map_err(|error| {
        config_set_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write failed: {error}"),
        )
    })
}

fn config_reload_status(state: &AppState) -> &'static str {
    match state.kernel.reload_config() {
        Ok(plan) if plan.restart_required => "applied_partial",
        Ok(_) => "applied",
        Err(_) => "saved_reload_failed",
    }
}

fn record_config_set_audit(state: &AppState, path: &str) {
    state.kernel.audit_log.record(
        "system",
        captain_runtime::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
    );
}

fn config_set_saved_response(status: &'static str, path: String) -> ConfigJsonResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": status, "path": path})),
    )
}

fn direct_secret_schema(env_keys: &[&str], message: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "secret",
        "writable": false,
        "storage": "secrets.env",
        "env_keys": env_keys,
        "message": message,
    })
}

fn validate_raw_config_content(content: &str) -> Result<(), String> {
    let root: toml::Value = content.parse().map_err(|e| format!("Invalid TOML: {e}"))?;
    validate_config_value(root)
}

fn validate_config_value(mut root: toml::Value) -> Result<(), String> {
    reject_direct_secret_assignments(&root)?;
    if let toml::Value::Table(ref mut table) = root {
        if let Some(toml::Value::Table(api_section)) = table.get("api").cloned() {
            for key in &["api_key", "api_listen", "log_level"] {
                if !table.contains_key(*key) {
                    if let Some(value) = api_section.get(*key) {
                        table.insert(key.to_string(), value.clone());
                    }
                }
            }
        }
    }
    root.try_into::<captain_types::config::KernelConfig>()
        .map(|_| ())
        .map_err(|e| format!("Config schema validation failed: {e}"))
}

fn reject_direct_secret_assignments(root: &toml::Value) -> Result<(), String> {
    let fields = captain_types::config::find_direct_secret_assignments_in_value(root);
    if fields.is_empty() {
        return Ok(());
    }

    Err(format!(
        "Direct secret fields are not allowed in config.toml: {}. Store secrets in secrets.env or environment variables and reference *_env fields instead.",
        fields.join(", ")
    ))
}

fn set_config_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Failed to set config file permissions: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                toml::Value::Integer(i as i64)
            } else if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        _ => toml::Value::String(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_config_validation_rejects_direct_secret_fields() {
        let err = validate_raw_config_content(
            r#"
api_key = "captain-api-secret"

[network]
shared_secret = "ofp-secret"
"#,
        )
        .expect_err("direct secrets must not validate");

        assert!(err.contains("Direct secret fields"));
        assert!(err.contains("api_key"));
        assert!(err.contains("network.shared_secret"));
        assert!(err.contains("secrets.env"));
    }

    #[test]
    fn raw_config_validation_rejects_legacy_api_secret_field() {
        let err = validate_raw_config_content(
            r#"
[api]
api_key = "legacy-secret"
"#,
        )
        .expect_err("legacy direct API secret must not validate");

        assert!(err.contains("api.api_key"));
        assert!(
            !err.contains("api_key, api.api_key"),
            "legacy migration must not duplicate the same secret report: {err}"
        );
    }

    #[test]
    fn raw_config_validation_allows_env_references_and_empty_legacy_secret() {
        validate_raw_config_content(
            r#"
[api]
api_key = ""

[default_model]
api_key_env = "OPENAI_API_KEY"

[auth]
password_hash = ""
"#,
        )
        .expect("empty direct fields and env references must stay valid");
    }

    #[test]
    fn config_set_validation_rejects_nested_direct_secret_value() {
        let mut network = toml::value::Table::new();
        network.insert(
            "shared_secret".to_string(),
            toml::Value::String("ofp-secret".to_string()),
        );

        let mut table = toml::value::Table::new();
        table.insert("network".to_string(), toml::Value::Table(network));

        let err = validate_config_value(toml::Value::Table(table))
            .expect_err("config_set must not persist direct secret values");
        assert!(err.contains("network.shared_secret"));
    }

    #[test]
    fn config_set_request_requires_path_and_value() {
        let missing_path = parse_config_set_request(&serde_json::json!({"value": true}))
            .expect_err("missing path must fail");
        assert_eq!(missing_path.0, StatusCode::BAD_REQUEST);
        assert_eq!(missing_path.1 .0["error"], "missing 'path' field");

        let missing_value =
            parse_config_set_request(&serde_json::json!({"path": "memory.backend"}))
                .expect_err("missing value must fail");
        assert_eq!(missing_value.0, StatusCode::BAD_REQUEST);
        assert_eq!(missing_value.1 .0["error"], "missing 'value' field");

        let request = parse_config_set_request(
            &serde_json::json!({"path": "memory.backend", "value": "graph"}),
        )
        .expect("valid config set request should parse");
        assert_eq!(request.path, "memory.backend");
        assert_eq!(request.value, "graph");
    }

    #[test]
    fn apply_config_set_value_supports_three_levels_and_rejects_deeper_paths() {
        let mut table = toml::value::Table::new();
        apply_config_set_value(
            &mut table,
            "channels.telegram.enabled",
            &serde_json::json!(true),
        )
        .expect("three level paths must be accepted");

        let telegram = table
            .get("channels")
            .and_then(|value| value.get("telegram"))
            .and_then(|value| value.get("enabled"));
        assert_eq!(telegram, Some(&toml::Value::Boolean(true)));

        let err = apply_config_set_value(
            &mut table,
            "channels.telegram.webhook.url",
            &serde_json::json!("https://example.test"),
        )
        .expect_err("deeper paths must be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(err.1 .0["error"], "path too deep (max 3 levels)");
    }

    #[test]
    fn raw_config_content_requires_content_field() {
        let missing = raw_config_content(&serde_json::json!({})).expect_err("content is required");
        assert_eq!(missing.0, StatusCode::BAD_REQUEST);
        assert_eq!(missing.1 .0["error"], "Missing 'content' field");

        let valid = serde_json::json!({"content": "api_listen = \"127.0.0.1:0\""});
        let content = raw_config_content(&valid).expect("content string should parse");
        assert_eq!(content, "api_listen = \"127.0.0.1:0\"");
    }

    #[test]
    fn raw_config_failed_response_reports_rollback_and_snapshot() {
        let response = raw_config_failed_response(
            "boom".to_string(),
            true,
            Path::new("/tmp/config.toml.raw-editor.test"),
        );

        assert_eq!(response.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.1 .0["error"], "boom");
        assert_eq!(response.1 .0["rolled_back"], true);
        assert_eq!(
            response.1 .0["snapshot"],
            "/tmp/config.toml.raw-editor.test"
        );
    }

    #[test]
    fn config_schema_document_keeps_secret_guards_and_core_channels() {
        let schema = config_schema_document(ConfigSchemaOptions {
            provider_options: vec!["openai".to_string()],
            model_options: vec![
                serde_json::json!({"id": "gpt-5", "name": "GPT-5", "provider": "openai"}),
            ],
        });

        assert_eq!(
            schema["sections"]["default_model"]["fields"]["provider"]["options"][0],
            "openai"
        );
        assert_eq!(
            schema["sections"]["general"]["fields"]["api_key"]["writable"],
            false
        );
        assert_eq!(
            schema["sections"]["network"]["fields"]["shared_secret"]["writable"],
            false
        );
        assert_eq!(
            schema["sections"]["channels"]["fields"]["telegram"],
            "object"
        );
        assert_eq!(schema["sections"]["channels"]["fields"]["signal"], "object");
        assert_eq!(schema["sections"]["channels"]["fields"]["email"], "object");
        assert!(schema["sections"]["channels"]["fields"]["slack"].is_null());
        assert!(schema["sections"]["channels"]["fields"]["whatsapp"].is_null());
    }

    #[test]
    fn direct_secret_schema_marks_field_non_writable() {
        let schema = direct_secret_schema(&["CAPTAIN_API_KEY"], "use secrets.env");

        assert_eq!(schema["type"], "secret");
        assert_eq!(schema["writable"], false);
        assert_eq!(schema["storage"], "secrets.env");
        assert_eq!(schema["env_keys"][0], "CAPTAIN_API_KEY");
    }
}
