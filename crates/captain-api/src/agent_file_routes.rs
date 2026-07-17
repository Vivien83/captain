//! Agent workspace identity file route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_types::agent::AgentId;
use std::sync::Arc;

/// Whitelisted workspace prompt files that can be read/written via API.
///
/// Retired memory/profile files are intentionally absent: the user profile is
/// global `~/.captain/USER.md`, and durable memory is MemPalace-backed.
pub const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "TOOLS.md",
    "AGENTS.md",
    "STYLE.md",
    "HEARTBEAT.md",
];

#[derive(serde::Deserialize)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// GET /api/agents/{id}/files - List workspace identity files.
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workspace = match agent_workspace(&state, &id) {
        Ok(workspace) => workspace,
        Err(response) => return response,
    };

    let files: Vec<_> = KNOWN_IDENTITY_FILES
        .iter()
        .map(|name| {
            let path = workspace.join(name);
            let size_bytes = if path.exists() {
                std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0)
            } else {
                0
            };
            serde_json::json!({
                "name": name,
                "exists": path.exists(),
                "size_bytes": size_bytes,
            })
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!({ "files": files }))).into_response()
}

/// GET /api/agents/{id}/files/{filename} - Read a workspace identity file.
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return error(StatusCode::BAD_REQUEST, "File not in whitelist");
    }

    let workspace = match agent_workspace(&state, &id) {
        Ok(workspace) => workspace,
        Err(response) => return response,
    };

    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(path) => path,
        Err(_) => return error(StatusCode::NOT_FOUND, "File not found"),
    };
    if let Err(response) = ensure_inside_workspace(&workspace, &canonical) {
        return response;
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(content) => content,
        Err(_) => return error(StatusCode::NOT_FOUND, "File not found"),
    };
    let size_bytes = content.len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
        .into_response()
}

/// PUT /api/agents/{id}/files/{filename} - Write a workspace identity file.
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return error(StatusCode::BAD_REQUEST, "File not in whitelist");
    }

    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "File content too large (max 32KB)",
        );
    }

    let workspace = match agent_workspace(&state, &id) {
        Ok(workspace) => workspace,
        Err(response) => return response,
    };

    let file_path = workspace.join(&filename);
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        file_path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .map(|parent| parent.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if let Err(response) = ensure_inside_workspace(&workspace, &check_path) {
        return response;
    }

    if let Err(e) = captain_types::durable_fs::atomic_write(&file_path, req.content.as_bytes()) {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Persist failed: {e}"),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": req.content.len(),
        })),
    )
        .into_response()
}

#[allow(clippy::result_large_err)]
fn agent_workspace(
    state: &AppState,
    id: &str,
) -> Result<std::path::PathBuf, axum::response::Response> {
    let agent_id: AgentId = id
        .parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "Invalid agent ID"))?;

    let entry = state
        .kernel
        .registry
        .get(agent_id)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Agent not found"))?;

    entry
        .manifest
        .workspace
        .clone()
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Agent has no workspace"))
}

#[allow(clippy::result_large_err)]
fn ensure_inside_workspace(
    workspace: &std::path::Path,
    path: &std::path::Path,
) -> Result<(), axum::response::Response> {
    let workspace = workspace
        .canonicalize()
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Workspace path error"))?;
    if path.starts_with(workspace) {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "Path traversal denied"))
    }
}

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}
