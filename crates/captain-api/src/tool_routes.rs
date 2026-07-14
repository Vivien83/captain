//! Tool catalog route handlers.

use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use captain_runtime::tool_runner::builtin_tool_definitions;
use std::sync::Arc;

/// GET /api/tools - List all tool definitions.
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tools: Vec<serde_json::Value> = builtin_tool_definitions()
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();

    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        for tool in mcp_tools.iter() {
            tools.push(serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
                "source": "mcp",
            }));
        }
    }

    Json(serde_json::json!({"tools": tools, "total": tools.len()}))
}
