//! Procedural skill runtime dispatch.

use std::path::Path;
use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{tool_scaffold_skill, tool_skill_md_execute};

pub(crate) async fn dispatch_skill_runtime_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    match tool_name {
        "skill_execute" => tool_skill_md_execute(input, kernel, workspace_root).await,
        "scaffold_skill" => tool_scaffold_skill(input, workspace_root).await,
        other => Err(format!("Unknown skill runtime tool: {other}")),
    }
}
