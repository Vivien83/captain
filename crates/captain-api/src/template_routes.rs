//! Agent template route handlers.

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_types::agent::AgentManifest;

/// GET /api/templates - List available agent templates.
pub async fn list_templates() -> impl IntoResponse {
    let agents_dir = captain_kernel::config::captain_home().join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("agent.toml");
            if !manifest_path.exists() {
                continue;
            }

            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let description = std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|content| toml::from_str::<AgentManifest>(&content).ok())
                .map(|manifest| manifest.description)
                .unwrap_or_default();

            templates.push(serde_json::json!({
                "name": name,
                "description": description,
            }));
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}

/// GET /api/templates/:name - Get template details.
pub async fn get_template(Path(name): Path<String>) -> impl IntoResponse {
    let agents_dir = captain_kernel::config::captain_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Template not found"})),
        );
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "provider": manifest.model.provider,
                            "model": manifest.model.model,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(error) => {
                tracing::warn!("Invalid template manifest for '{name}': {error}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": "Invalid template manifest"})),
                )
            }
        },
        Err(error) => {
            tracing::warn!("Failed to read template '{name}': {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to read template"})),
            )
        }
    }
}
