//! Runtime handlers for capability and tool discovery.

use std::sync::Arc;

use captain_skills::registry::SkillRegistry;

use crate::core_tools::is_core_tool;
use crate::kernel_handle::KernelHandle;
use crate::mcp;

use super::{
    builtin_tool_definitions, check_skill, search_capabilities, search_deferred_builtin_tools,
    search_skills, view_skill,
};

pub(crate) async fn tool_capability_search(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    search_capabilities(
        input,
        skill_registry,
        mcp_connections,
        kernel,
        builtin_tool_definitions(),
        is_core_tool,
    )
    .await
}

pub(crate) fn tool_skill_search(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    search_skills(input, skill_registry)
}

pub(crate) fn tool_skill_view(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    view_skill(input, skill_registry)
}

pub(crate) fn tool_skill_check(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    check_skill(input, skill_registry)
}

pub(crate) async fn tool_search(input: &serde_json::Value) -> Result<String, String> {
    search_deferred_builtin_tools(input, builtin_tool_definitions(), is_core_tool)
}
