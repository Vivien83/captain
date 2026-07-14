//! Fallback dispatch for MCP tools and skill-provided tools.

use captain_skills::registry::SkillRegistry;
use tracing::debug;

use crate::mcp;

pub(crate) async fn dispatch_fallback_tool(
    tool_name: &str,
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    if mcp::is_mcp_tool(tool_name) {
        return dispatch_mcp_tool(tool_name, input, mcp_connections).await;
    }
    dispatch_skill_tool(tool_name, input, skill_registry).await
}

async fn dispatch_mcp_tool(
    tool_name: &str,
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let Some(mcp_conns) = mcp_connections else {
        return Err(format!("MCP not available for tool: {tool_name}"));
    };
    let mut conns = mcp_conns.lock().await;
    let known_names: Vec<String> = conns.iter().map(|c| c.name().to_string()).collect();
    let known_refs: Vec<&str> = known_names.iter().map(|s| s.as_str()).collect();
    let Some(server_name) = mcp::extract_mcp_server_from_known(tool_name, &known_refs) else {
        return Err(format!("Invalid MCP tool name: {tool_name}"));
    };
    let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) else {
        return Err(format!("MCP server '{server_name}' not connected"));
    };
    debug!(
        tool = tool_name,
        server = server_name,
        "Dispatching to MCP server"
    );
    conn.call_tool(tool_name, input)
        .await
        .map_err(|e| format!("MCP tool call failed: {e}"))
}

async fn dispatch_skill_tool(
    tool_name: &str,
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let Some(registry) = skill_registry else {
        return Err(format!("Unknown tool: {tool_name}"));
    };
    let Some(skill) = registry.find_tool_provider(tool_name) else {
        return Err(format!("Unknown tool: {tool_name}"));
    };
    debug!(
        tool = tool_name,
        skill = %skill.manifest.skill.name,
        "Dispatching to skill"
    );
    match captain_skills::loader::execute_skill_tool(&skill.manifest, &skill.path, tool_name, input)
        .await
    {
        Ok(skill_result) => {
            let content = serde_json::to_string(&skill_result.output)
                .unwrap_or_else(|_| skill_result.output.to_string());
            if skill_result.is_error {
                Err(content)
            } else {
                Ok(content)
            }
        }
        Err(e) => Err(format!("Skill execution failed: {e}")),
    }
}
