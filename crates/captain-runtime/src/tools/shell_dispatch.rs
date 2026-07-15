//! Guarded shell dispatch with existing early-block behavior preserved.

use std::path::Path;
use std::sync::Arc;

use captain_types::config::{ExecPolicy, ExecSecurityMode};
use captain_types::tool::ToolResult;
use tracing::debug;

use crate::kernel_handle::KernelHandle;

use super::{
    check_taint_shell_exec, ensure_no_secret_literal, shell_exec_approval_preview, tool_shell_exec,
};

pub(crate) enum ShellDispatchOutcome {
    Blocked(ToolResult),
    Result(Result<String, String>),
}

pub(crate) async fn dispatch_shell_exec(
    tool_use_id: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    exec_policy: Option<&ExecPolicy>,
) -> ShellDispatchOutcome {
    let command = input["command"].as_str().unwrap_or("");
    if let Err(reason) = ensure_no_secret_literal("shell_exec", "command", command) {
        return ShellDispatchOutcome::Blocked(blocked_result(tool_use_id, reason));
    }
    if let Some(blocked) =
        enforce_critical_pattern(tool_use_id, command, kernel, caller_agent_id, exec_policy).await
    {
        return ShellDispatchOutcome::Blocked(blocked);
    }
    if let Some(blocked) = enforce_shell_policy(tool_use_id, command, exec_policy) {
        return ShellDispatchOutcome::Blocked(blocked);
    }
    ShellDispatchOutcome::Result(
        tool_shell_exec(
            input,
            allowed_env_vars.unwrap_or(&[]),
            workspace_root,
            exec_policy,
        )
        .await,
    )
}

async fn enforce_critical_pattern(
    tool_use_id: &str,
    command: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    exec_policy: Option<&ExecPolicy>,
) -> Option<ToolResult> {
    let critical_mode = exec_policy.map(|p| p.critical_mode).unwrap_or_default();
    match crate::critical_patterns::decide(command, critical_mode) {
        crate::critical_patterns::CriticalDecision::Proceed => None,
        crate::critical_patterns::CriticalDecision::Block(pat) => Some(blocked_result(
            tool_use_id,
            format!(
                "shell_exec blocked: hyper-critical pattern `{pat}` detected. \
                 Current security.critical_mode = '{:?}'. \
                 Switch to 'open' to enable one-shot user approval, \
                 or remove the pattern from the command.",
                critical_mode
            ),
        )),
        crate::critical_patterns::CriticalDecision::AskUser(pat) => {
            ask_for_critical_pattern(tool_use_id, command, kernel, caller_agent_id, pat).await
        }
    }
}

async fn ask_for_critical_pattern(
    tool_use_id: &str,
    command: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    pat: &str,
) -> Option<ToolResult> {
    let agent_id_str = caller_agent_id.unwrap_or("unknown");
    let summary = format!(
        "shell_exec critical pattern `{pat}` detected.\n{}",
        shell_exec_approval_preview(&serde_json::json!({ "command": command }))
    );
    let approved = match kernel {
        Some(kh) => match kh
            .request_approval(agent_id_str, "shell_exec_critical", &summary)
            .await
        {
            Ok(approved) => approved,
            Err(e) => {
                return Some(blocked_result(
                    tool_use_id,
                    format!(
                        "shell_exec blocked: critical pattern `{pat}` and \
                         approval flow failed: {e}"
                    ),
                ));
            }
        },
        None => false,
    };
    if !approved {
        return Some(blocked_result(
            tool_use_id,
            format!(
                "shell_exec blocked: hyper-critical pattern `{pat}` \
                 was refused by the user (or no UI available)."
            ),
        ));
    }
    debug!(pattern = pat, "Q.9 critical command approved by user");
    None
}

fn enforce_shell_policy(
    tool_use_id: &str,
    command: &str,
    exec_policy: Option<&ExecPolicy>,
) -> Option<ToolResult> {
    let is_full_mode = exec_policy
        .map(|p| p.mode == ExecSecurityMode::Full)
        .unwrap_or(true);
    if !is_full_mode {
        if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
            return Some(blocked_result(
                tool_use_id,
                format!(
                    "shell_exec blocked: command contains {reason}. \
                     Shell metacharacters are not allowed in {:?} mode.",
                    exec_policy.map(|p| p.mode).unwrap_or_default()
                ),
            ));
        }
    }

    if let Some(policy) = exec_policy {
        if let Err(reason) = crate::subprocess_sandbox::validate_command_allowlist(command, policy)
        {
            return Some(blocked_result(
                tool_use_id,
                format!(
                    "shell_exec blocked: {reason}. Current exec_policy.mode = '{:?}'. \
                     To allow shell commands, set exec_policy.mode = 'full' in the agent manifest or config.toml.",
                    policy.mode
                ),
            ));
        }
    }

    let is_full_exec = exec_policy.is_some_and(|p| p.mode == ExecSecurityMode::Full);
    if !is_full_exec {
        if let Some(violation) = check_taint_shell_exec(command) {
            return Some(blocked_result(
                tool_use_id,
                format!("Taint violation: {violation}"),
            ));
        }
    }
    None
}

fn blocked_result(tool_use_id: &str, content: String) -> ToolResult {
    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content,
        is_error: true,
        transient_content: Vec::new(),
    }
}
