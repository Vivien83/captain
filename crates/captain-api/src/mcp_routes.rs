use crate::state::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use captain_runtime::tool_runner::builtin_tool_definitions;
use std::sync::Arc;

/// GET /api/mcp/servers - List configured MCP servers and their tools.
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config_servers: Vec<serde_json::Value> = state
        .kernel
        .config
        .mcp_servers
        .iter()
        .map(|server| {
            let transport = match &server.transport {
                captain_types::config::McpTransportEntry::Stdio { command, args } => {
                    serde_json::json!({
                        "type": "stdio",
                        "command": command,
                        "args": args,
                    })
                }
                captain_types::config::McpTransportEntry::Sse { url } => {
                    serde_json::json!({
                        "type": "sse",
                        "url": url,
                    })
                }
            };
            serde_json::json!({
                "name": server.name,
                "transport": transport,
                "timeout_secs": server.timeout_secs,
                "env": server.env,
            })
        })
        .collect();

    let connections = state.kernel.mcp_connections.lock().await;
    let connected: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let tools: Vec<serde_json::Value> = conn
                .tools()
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                    })
                })
                .collect();
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": true,
            })
        })
        .collect();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": connected.len(),
    }))
}

/// POST /mcp - Handle MCP JSON-RPC requests over HTTP.
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut tools = builtin_tool_definitions();
    {
        let registry = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for skill_tool in registry.all_tool_definitions() {
            tools.push(captain_types::tool::ToolDefinition {
                name: skill_tool.name.clone(),
                description: skill_tool.description.clone(),
                input_schema: skill_tool.input_schema.clone(),
            });
        }
    }
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        tools.extend(mcp_tools.iter().cloned());
    }

    let method = request["method"].as_str().unwrap_or("");
    if method == "tools/call" {
        let tool_name = request["params"]["name"].as_str().unwrap_or("");
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        if !tools.iter().any(|tool| tool.name == tool_name) {
            return Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned(),
                "error": {"code": -32602, "message": format!("Unknown tool: {tool_name}")}
            }));
        }

        let skill_snapshot = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        let kernel_handle: Arc<dyn captain_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn captain_runtime::kernel_handle::KernelHandle>;
        let result = captain_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            None,
            None,
            Some(&skill_snapshot),
            Some(&state.kernel.mcp_connections),
            Some(&state.kernel.web_ctx),
            Some(&state.kernel.browser_ctx),
            None,
            None,
            Some(&state.kernel.media_engine),
            None,
            if state.kernel.config.tts.enabled {
                Some(&state.kernel.tts_engine)
            } else {
                None
            },
            if state.kernel.config.docker.enabled {
                Some(&state.kernel.config.docker)
            } else {
                None
            },
            Some(&*state.kernel.process_manager),
        )
        .await;

        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned(),
            "result": {
                "content": [{"type": "text", "text": result.content}],
                "isError": result.is_error,
            }
        }));
    }

    let response = captain_runtime::mcp_server::handle_mcp_request(&request, &tools).await;
    Json(response)
}
