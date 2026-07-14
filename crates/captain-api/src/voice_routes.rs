//! Voice and STT configuration route handlers.

use crate::state::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_types::config::ALLOWED_STT_MODELS;
use std::path::Path;
use std::sync::Arc;

/// GET /api/stt - Return the currently configured STT model.
pub async fn get_stt(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "model": state.kernel.config.stt_model
    }))
}

/// PUT /api/stt - Update the STT model and persist to config.toml.
///
/// Accepts JSON `{ "model": "whisper-small" }`.
pub async fn update_stt(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let model = match parse_stt_model(&body) {
        Ok(model) => model,
        Err(response) => return response,
    };

    let reload_status = match persist_stt_model(&state, &model) {
        Ok(status) => status,
        Err(response) => return response,
    };

    json_response(
        StatusCode::OK,
        serde_json::json!({
            "status": "ok",
            "model": model,
            "reload": reload_status,
        }),
    )
}

#[allow(clippy::result_large_err)]
fn parse_stt_model(body: &serde_json::Value) -> Result<String, Response> {
    let Some(model) = body.get("model").and_then(|value| value.as_str()) else {
        return Err(json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"status": "error", "error": "missing 'model' field"}),
        ));
    };

    if !ALLOWED_STT_MODELS.contains(&model) {
        return Err(json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "status": "error",
                "error": format!(
                    "unknown model '{}'. Allowed: {}",
                    model,
                    ALLOWED_STT_MODELS.join(", ")
                ),
            }),
        ));
    }

    Ok(model.to_string())
}

#[allow(clippy::result_large_err)]
fn persist_stt_model(state: &AppState, model: &str) -> Result<&'static str, Response> {
    let config_path = state.kernel.config.home_dir.join("config.toml");
    let mut table = load_config_table(&config_path);
    table.insert(
        "stt_model".to_string(),
        toml::Value::String(model.to_string()),
    );

    let toml_string = toml::to_string_pretty(&table).map_err(|error| {
        json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "status": "error",
                "error": format!("serialize failed: {error}"),
            }),
        )
    })?;

    std::fs::write(&config_path, toml_string).map_err(|error| {
        json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "status": "error",
                "error": format!("write failed: {error}"),
            }),
        )
    })?;

    Ok(match state.kernel.reload_config() {
        Ok(plan) if plan.restart_required => "applied_partial",
        Ok(_) => "applied",
        Err(_) => "written_no_reload",
    })
}

fn load_config_table(config_path: &Path) -> toml::value::Table {
    if !config_path.exists() {
        return toml::value::Table::new();
    }

    match std::fs::read_to_string(config_path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => toml::value::Table::new(),
    }
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stt_model_accepts_allowed_model() {
        let body = serde_json::json!({"model": "whisper-small"});

        assert_eq!(parse_stt_model(&body).unwrap(), "whisper-small");
    }

    #[test]
    fn parse_stt_model_rejects_missing_model() {
        let body = serde_json::json!({});

        let response = parse_stt_model(&body).unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_stt_model_rejects_unknown_model() {
        let body = serde_json::json!({"model": "unknown"});

        let response = parse_stt_model(&body).unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
