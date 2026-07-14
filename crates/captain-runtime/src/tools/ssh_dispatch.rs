//! SSH and SFTP dispatch.

use captain_types::config::ExecPolicy;

use super::{tool_ssh_download, tool_ssh_exec, tool_ssh_health_check, tool_ssh_upload};

pub(crate) async fn dispatch_ssh_tool(
    tool_name: &str,
    input: &serde_json::Value,
    exec_policy: Option<&ExecPolicy>,
) -> Result<String, String> {
    match tool_name {
        "ssh_health_check" => tool_ssh_health_check(input, exec_policy).await,
        "ssh_exec" => tool_ssh_exec(input, exec_policy).await,
        "ssh_upload" => tool_ssh_upload(input).await,
        "ssh_download" => tool_ssh_download(input).await,
        other => Err(format!("Unknown ssh tool: {other}")),
    }
}
