//! Agent feedback route handlers.

use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use captain_types::agent::AgentId;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

/// POST /api/agents/{id}/feedback - Submit feedback on an agent response.
pub async fn submit_feedback(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let workspace = match agent_workspace(&state, &id) {
        Ok(workspace) => workspace,
        Err(response) => return response,
    };
    let feedback_path = workspace.join("FEEDBACK.jsonl");
    let line = serde_json::to_string(&body).unwrap_or_default();

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&feedback_path)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "{line}");
            json_response(StatusCode::OK, serde_json::json!({"status": "ok"}))
        }
        Err(error) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error": format!("{error}")}),
        ),
    }
}

/// GET /api/agents/{id}/feedback - Get feedback entries.
pub async fn get_feedback(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let workspace = match agent_workspace(&state, &id) {
        Ok(workspace) => workspace,
        Err(response) => return response,
    };
    let feedback_path = workspace.join("FEEDBACK.jsonl");
    let content = std::fs::read_to_string(&feedback_path).unwrap_or_default();
    let entries = parse_feedback_entries(&content);

    json_response(
        StatusCode::OK,
        serde_json::json!({"feedback": entries, "count": entries.len()}),
    )
}

#[allow(clippy::result_large_err)]
fn agent_workspace(state: &AppState, id: &str) -> Result<PathBuf, Response> {
    let agent_id = parse_agent_id(id)?;
    let entry = state.kernel.registry.get(agent_id).ok_or_else(|| {
        json_response(
            StatusCode::NOT_FOUND,
            serde_json::json!({"error": "Agent not found"}),
        )
    })?;

    entry.manifest.workspace.clone().ok_or_else(|| {
        json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "Agent has no workspace"}),
        )
    })
}

#[allow(clippy::result_large_err)]
fn parse_agent_id(id: &str) -> Result<AgentId, Response> {
    id.parse().map_err(|_| {
        json_response(
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "Invalid agent ID"}),
        )
    })
}

fn parse_feedback_entries(content: &str) -> Vec<serde_json::Value> {
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_id_rejects_invalid_id() {
        let response = parse_agent_id("bad-id").unwrap_err();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_feedback_entries_skips_invalid_lines() {
        let content = "{\"score\":1}\nnot-json\n{\"score\":2}";

        let entries = parse_feedback_entries(content);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["score"], 1);
        assert_eq!(entries[1]["score"], 2);
    }
}
