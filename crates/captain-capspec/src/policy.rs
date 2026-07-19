use crate::model::{Effect, PermissionSet, CAPABILITY_TOOL_PREFIX};

pub(crate) fn validate_permissions(permissions: &PermissionSet) -> Result<(), String> {
    if permissions.tools.is_empty() {
        return Err("permissions.tools must list every callable tool".to_string());
    }
    for tool in &permissions.tools {
        validate_tool_name(tool)?;
        if tool.starts_with(CAPABILITY_TOOL_PREFIX) {
            return Err("nested CapSpec tools are not supported in format 1".to_string());
        }
    }
    for secret in &permissions.secrets {
        if secret.contains('=') || secret.chars().any(char::is_whitespace) {
            return Err(format!("invalid secret identifier '{secret}'"));
        }
    }
    Ok(())
}

pub(crate) fn validate_tool_name(tool: &str) -> Result<(), String> {
    if tool.is_empty()
        || tool.len() > 128
        || tool
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
    {
        return Err(format!("invalid tool name '{tool}'"));
    }
    Ok(())
}

pub(crate) fn validate_scoped_permission(
    step_id: &str,
    tool: &str,
    permissions: &PermissionSet,
) -> Result<(), String> {
    let missing = if matches!(
        tool,
        "file_read" | "file_list" | "file_inspect_batch" | "grep" | "glob"
    ) && permissions.read_paths.is_empty()
    {
        Some("permissions.read_paths")
    } else if matches!(
        tool,
        "file_write" | "apply_patch" | "edit_file" | "multi_edit"
    ) && permissions.write_paths.is_empty()
    {
        Some("permissions.write_paths")
    } else if matches!(
        tool,
        "web_fetch" | "web_search" | "web_download" | "browser_navigate"
    ) && permissions.network_hosts.is_empty()
    {
        Some("permissions.network_hosts")
    } else if tool == "web_download" && permissions.write_paths.is_empty() {
        Some("permissions.write_paths")
    } else if tool.starts_with("ssh_") && permissions.ssh_hosts.is_empty() {
        Some("permissions.ssh_hosts")
    } else if matches!(
        tool,
        "shell_exec" | "cargo" | "npm" | "pip" | "execute_code"
    ) && permissions.shell_commands.is_empty()
    {
        Some("permissions.shell_commands")
    } else if matches!(
        tool,
        "memory_recall" | "memory_context_batch" | "session_recall"
    ) && permissions.memory_read.is_empty()
    {
        Some("permissions.memory_read")
    } else if matches!(tool, "memory_save" | "memory_store" | "memory_forget")
        && permissions.memory_write.is_empty()
    {
        Some("permissions.memory_write")
    } else if tool == "secret_read" && permissions.secrets.is_empty() {
        Some("permissions.secrets")
    } else {
        None
    };
    match missing {
        Some(permission) => Err(format!("step '{step_id}' requires {permission}")),
        None => Ok(()),
    }
}

pub fn reviewed_effect(tool: &str) -> Effect {
    if matches!(
        tool,
        "file_read"
            | "file_list"
            | "file_inspect_batch"
            | "grep"
            | "glob"
            | "web_fetch"
            | "web_search"
            | "document_extract"
            | "memory_recall"
            | "memory_context_batch"
            | "session_recall"
            | "project_get"
            | "project_list"
            | "agent_list"
            | "system_time"
            | "tool_run_status"
            | "tool_run_result"
            | "tool_run_list"
    ) {
        Effect::Read
    } else if matches!(
        tool,
        "file_write"
            | "apply_patch"
            | "edit_file"
            | "multi_edit"
            | "memory_save"
            | "memory_store"
            | "document_create"
            | "document_pipeline"
            | "checkpoint_save"
    ) {
        Effect::Write
    } else if matches!(
        tool,
        "shell_exec"
            | "execute_code"
            | "cargo"
            | "npm"
            | "pip"
            | "ssh_exec"
            | "ssh_upload"
            | "ssh_download"
            | "memory_forget"
            | "secret_write"
            | "system_update"
            | "agent_kill"
    ) {
        Effect::Destructive
    } else {
        Effect::External
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tools_fail_closed_as_external() {
        assert_eq!(reviewed_effect("new_unreviewed_tool"), Effect::External);
    }

    #[test]
    fn known_mutators_cannot_be_marked_read() {
        assert_eq!(reviewed_effect("file_write"), Effect::Write);
        assert_eq!(reviewed_effect("shell_exec"), Effect::Destructive);
    }
}
