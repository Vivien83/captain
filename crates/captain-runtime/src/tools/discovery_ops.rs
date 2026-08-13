//! Runtime handlers for capability and tool discovery.

use std::path::Path;
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
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let client_origin = crate::client_authority::is_paired_client_origin(
        super::current_origin_channel().as_deref(),
    );
    let mut bounded_input = input.clone();
    if client_origin {
        bounded_input["sources"] = serde_json::json!(["builtin"]);
    }
    let definitions = crate::client_authority::filter_tool_definitions_for_origin(
        builtin_tool_definitions(),
        super::current_origin_channel().as_deref(),
    );
    search_capabilities(
        &bounded_input,
        (!client_origin).then_some(skill_registry).flatten(),
        (!client_origin).then_some(mcp_connections).flatten(),
        (!client_origin).then_some(kernel).flatten(),
        (!client_origin).then_some(workspace_root).flatten(),
        definitions,
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
    let origin = super::current_origin_channel();
    let definitions = crate::client_authority::filter_tool_definitions_for_origin(
        builtin_tool_definitions(),
        origin.as_deref(),
    );
    search_deferred_builtin_tools(input, definitions, is_core_tool)
}
