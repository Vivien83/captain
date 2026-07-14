//! Config, secrets, Codex auth, and MCP setup dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_codex_auth_status, tool_codex_login_poll, tool_codex_login_start, tool_codex_tool_probe,
    tool_config_read, tool_config_schema, tool_config_setup, tool_config_write,
    tool_mcp_catalog_search, tool_mcp_integration_install, tool_mcp_status,
    tool_model_switch_apply, tool_model_switch_plan, tool_secret_read, tool_secret_write,
    tool_self_configure, tool_web_credentials_update,
};

pub(crate) async fn dispatch_config_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "config_read" => tool_config_read(input, kernel),
        "config_write" => tool_config_write(input, kernel).await,
        "self_configure" => tool_self_configure(input, kernel, caller_agent_id).await,
        "model_switch_plan" => tool_model_switch_plan(input, kernel, caller_agent_id),
        "model_switch_apply" => tool_model_switch_apply(input, kernel, caller_agent_id),
        "codex_auth_status" => tool_codex_auth_status(),
        "codex_tool_probe" => tool_codex_tool_probe(input).await,
        "codex_login_start" => tool_codex_login_start(kernel).await,
        "codex_login_poll" => tool_codex_login_poll(input, kernel, caller_agent_id).await,
        "secret_read" => tool_secret_read(input, kernel),
        "secret_write" => tool_secret_write(input, kernel),
        "web_credentials_update" => tool_web_credentials_update(input, kernel).await,
        "config_setup" => tool_config_setup(input, kernel).await,
        "mcp_catalog_search" => tool_mcp_catalog_search(input, kernel).await,
        "mcp_integration_install" => tool_mcp_integration_install(input, kernel).await,
        "mcp_status" => tool_mcp_status(kernel).await,
        "config_schema" => tool_config_schema(kernel),
        other => Err(format!("Unknown config/auth tool: {other}")),
    }
}
