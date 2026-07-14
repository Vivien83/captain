//! Docker sandbox dispatch.

use std::path::Path;

use captain_types::config::DockerSandboxConfig;

use super::tool_docker_exec;

pub(crate) async fn dispatch_docker_tool(
    tool_name: &str,
    input: &serde_json::Value,
    docker_config: Option<&DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "docker_exec" => {
            tool_docker_exec(input, docker_config, workspace_root, caller_agent_id).await
        }
        other => Err(format!("Unknown docker tool: {other}")),
    }
}
