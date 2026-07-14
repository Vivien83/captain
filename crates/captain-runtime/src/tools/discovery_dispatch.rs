//! Docs and capability discovery dispatch.

use std::sync::Arc;

use captain_skills::registry::SkillRegistry;

use crate::kernel_handle::KernelHandle;
use crate::mcp;

use super::{
    tool_capability_search, tool_captain_docs, tool_search, tool_skill_check, tool_skill_search,
    tool_skill_view,
};

pub(crate) async fn dispatch_discovery_tool(
    tool_name: &str,
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "captain_docs" => tool_captain_docs(input).await,
        "capability_search" => {
            tool_capability_search(input, skill_registry, mcp_connections, kernel).await
        }
        "skill_search" => tool_skill_search(input, skill_registry),
        "skill_view" => tool_skill_view(input, skill_registry),
        "skill_check" => tool_skill_check(input, skill_registry),
        "tool_search" => tool_search(input).await,
        other => Err(format!("Unknown discovery tool: {other}")),
    }
}
