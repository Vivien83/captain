use captain_types::config::ExecPolicy;
use std::path::Path;

pub(crate) struct ToolResult {
    pub(crate) content: String,
    pub(crate) is_error: bool,
    pub(crate) transient_content: Vec<String>,
}

pub(crate) async fn execute_tool(
    _tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    workspace_root: &Path,
    exec_policy: &ExecPolicy,
) -> ToolResult {
    let result = match tool_name {
        "file_read" => crate::tools::tool_file_read(input, Some(workspace_root), None, None).await,
        "file_write" => {
            crate::tools::tool_file_write(input, Some(workspace_root), None, None).await
        }
        "file_list" => crate::tools::tool_file_list(input, Some(workspace_root), None, None).await,
        "glob" => crate::tools::tool_glob(input, Some(workspace_root), None, None).await,
        "grep" => crate::tools::tool_grep(input, Some(workspace_root), None, None).await,
        "edit_file" => crate::tools::tool_edit_file(input, Some(workspace_root), None, None).await,
        "multi_edit" => {
            crate::tools::tool_multi_edit(input, Some(workspace_root), None, None).await
        }
        "apply_patch" => {
            crate::tools::tool_apply_patch(input, Some(workspace_root), None, None).await
        }
        "file_inspect_batch" => {
            crate::tools::tool_file_inspect_batch(input, Some(workspace_root), None, None).await
        }
        "shell_exec" => crate::shell_exec::execute(input, workspace_root, exec_policy).await,
        _ => Err("Unsupported local Node tool".to_string()),
    };
    match result {
        Ok(content) => ToolResult {
            content,
            is_error: false,
            transient_content: Vec::new(),
        },
        Err(content) => ToolResult {
            content,
            is_error: true,
            transient_content: Vec::new(),
        },
    }
}
