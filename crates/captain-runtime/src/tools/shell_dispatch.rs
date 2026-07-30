//! Guarded shell dispatch with existing early-block behavior preserved.

use std::path::Path;
use std::sync::Arc;

use captain_types::config::ExecPolicy;
use captain_types::tool::ToolResult;

use crate::kernel_handle::KernelHandle;

use super::{shell_exec_approval_preview, tool_shell_exec};

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
    let permit = match crate::guarded_exec::review_shell(
        crate::guarded_exec::ExecSurface::ShellTool,
        command,
        exec_policy,
        true,
    ) {
        Ok(crate::guarded_exec::ReviewDecision::Proceed(permit)) => permit,
        Ok(crate::guarded_exec::ReviewDecision::ApprovalRequired { pattern }) => {
            if let Some(blocked) =
                ask_for_critical_pattern(tool_use_id, command, kernel, caller_agent_id, pattern)
                    .await
            {
                return ShellDispatchOutcome::Blocked(blocked);
            }
            crate::guarded_exec::permit_after_operator_approval(
                crate::guarded_exec::ExecSurface::ShellTool,
                command,
            )
        }
        Err(reason) => {
            return ShellDispatchOutcome::Blocked(blocked_result(tool_use_id, reason));
        }
    };
    ShellDispatchOutcome::Result(
        tool_shell_exec(
            input,
            allowed_env_vars.unwrap_or(&[]),
            workspace_root,
            exec_policy,
            permit,
        )
        .await,
    )
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
    let action_digest =
        captain_types::approval::approval_action_digest("shell_exec_critical", command.as_bytes());
    let approved = match kernel {
        Some(kh) => match kh
            .request_approval(
                agent_id_str,
                "shell_exec_critical",
                &summary,
                &action_digest,
            )
            .await
        {
            Ok(outcome) => {
                if !outcome.is_approved() {
                    let reason = outcome
                        .reason
                        .as_deref()
                        .map(|reason| format!(" Operator reason: {reason}"))
                        .unwrap_or_default();
                    return Some(blocked_result(
                        tool_use_id,
                        format!(
                            "shell_exec blocked: hyper-critical pattern `{pat}` was refused by the user.{reason} Do not retry the same command unchanged."
                        ),
                    ));
                }
                true
            }
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
