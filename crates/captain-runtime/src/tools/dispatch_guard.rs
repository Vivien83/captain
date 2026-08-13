//! Pre-dispatch policy checks for builtin tool execution.

use std::path::Path;
use std::sync::Arc;

use captain_types::tool::ToolResult;
use tracing::{debug, warn};

use crate::kernel_handle::KernelHandle;
use crate::tool_cache::ToolResultCache;

use super::{
    current_agent_lineage_depth, render_error_with_suggestion, resolve_file_path_for_caller,
};

pub(crate) async fn run_pre_dispatch_checks(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> Option<ToolResult> {
    if let Some(result) = enforce_lineage_depth(tool_use_id, tool_name) {
        return Some(result);
    }
    if let Some(result) = enforce_paired_client_authority(tool_use_id, tool_name, input) {
        return Some(result);
    }
    if let Some(result) =
        enforce_kernel_tool_blocklist(tool_use_id, tool_name, kernel, caller_agent_id)
    {
        return Some(result);
    }
    if let Some(result) = enforce_allowed_tools(tool_use_id, tool_name, allowed_tools) {
        return Some(result);
    }
    request_approval_if_needed(
        tool_use_id,
        tool_name,
        input,
        kernel,
        caller_agent_id,
        workspace_root,
    )
    .await
}

fn enforce_paired_client_authority(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<ToolResult> {
    let origin = super::current_origin_channel();
    if !crate::client_authority::is_paired_client_origin(origin.as_deref())
        || crate::client_authority::paired_client_tool_is_allowed(tool_name, input)
    {
        return None;
    }
    warn!(
        tool = %tool_name,
        authority_version = crate::client_authority::PAIRED_CLIENT_AUTHORITY_VERSION,
        "Tool denied by paired Client authority"
    );
    Some(denied_tool_result(
        tool_use_id,
        tool_name,
        &format!(
            "Permission denied: tool '{tool_name}' is outside paired Client authority v{}. The operation was not performed and must not be retried through another local tool.",
            crate::client_authority::PAIRED_CLIENT_AUTHORITY_VERSION
        ),
    ))
}

fn enforce_kernel_tool_blocklist(
    tool_use_id: &str,
    tool_name: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Option<ToolResult> {
    let kernel = kernel?;
    if !kernel.tool_is_blocked_for_agent(caller_agent_id, tool_name) {
        return None;
    }
    warn!(
        tool = %tool_name,
        caller_agent_id,
        "Tool denied by the agent's hard blocklist"
    );
    Some(denied_tool_result(
        tool_use_id,
        tool_name,
        &format!("Permission denied: tool '{tool_name}' is in this agent's tool_blocklist"),
    ))
}

pub(crate) async fn cached_tool_result(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    cache: &Option<Arc<ToolResultCache>>,
) -> Option<ToolResult> {
    let Some(cache) = cache else {
        return None;
    };
    match cache.get(tool_name, input).await {
        Ok(Some(cached)) => {
            debug!(tool_name, "tool cache hit");
            Some(ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: cached.output,
                is_error: cached.is_error,
                transient_content: Vec::new(),
            })
        }
        _ => None,
    }
}

fn enforce_lineage_depth(tool_use_id: &str, tool_name: &str) -> Option<ToolResult> {
    let lineage_depth = current_agent_lineage_depth();
    if lineage_depth == 0 {
        return None;
    }
    let policy = crate::tool_policy::ToolPolicy::default();
    let canonical = tool_name.to_string();
    let filtered = crate::tool_policy::filter_tools_by_depth(
        std::slice::from_ref(&canonical),
        lineage_depth,
        policy.subagent_max_depth,
    );
    if !filtered.is_empty() {
        return None;
    }
    warn!(
        tool = %canonical,
        lineage_depth,
        max_depth = policy.subagent_max_depth,
        "Tool denied by sub-agent depth policy"
    );
    Some(denied_tool_result(
        tool_use_id,
        &canonical,
        &format!(
            "Permission denied: tool '{canonical}' is not allowed for sub-agent depth {lineage_depth}"
        ),
    ))
}

fn enforce_allowed_tools(
    tool_use_id: &str,
    tool_name: &str,
    allowed_tools: Option<&[String]>,
) -> Option<ToolResult> {
    let allowed = allowed_tools?;
    let canonical = tool_name.to_string();
    if allowed.iter().any(|t| t.as_str() == canonical.as_str()) {
        return None;
    }
    warn!(
        tool = %canonical,
        allowed_count = allowed.len(),
        "Tool not in allowed_tools policy — denying"
    );
    Some(denied_tool_result(
        tool_use_id,
        &canonical,
        &format!("Permission denied: tool '{canonical}' is not in this agent's allowed_tools list"),
    ))
}

async fn request_approval_if_needed(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> Option<ToolResult> {
    if matches!(
        crate::execution_routing::current_turn_execution_context().map(|context| context.target),
        Some(crate::execution_routing::ResolvedExecutionTarget::Node { .. })
    ) && crate::node_tool_runtime::local_node_tool_family(tool_name).is_some()
    {
        // The Node reviews the exact workspace-local action and raises one
        // digest-bound approval through the Hub. Prompting here as well would
        // create two competing approvals for the same operation.
        return None;
    }
    let kernel = kernel?;
    if !kernel.requires_approval(tool_name) {
        return None;
    }
    let agent_id_str = caller_agent_id.unwrap_or("unknown");
    let summary =
        approval_preview_summary(tool_name, input, kernel, caller_agent_id, workspace_root);
    let action_input = serde_json::to_vec(input).unwrap_or_else(|_| input.to_string().into_bytes());
    let action_digest = captain_types::approval::approval_action_digest(tool_name, &action_input);
    match kernel
        .request_approval(agent_id_str, tool_name, &summary, &action_digest)
        .await
    {
        Ok(outcome) if outcome.is_approved() => {
            debug!(tool_name, "Approval granted — proceeding with execution");
            None
        }
        Ok(outcome) => {
            warn!(tool_name, decision = ?outcome.decision, reason = ?outcome.reason, "Approval denied — blocking tool execution");
            let reason = outcome
                .reason
                .as_deref()
                .map(|reason| format!(" Operator reason: {reason}"))
                .unwrap_or_default();
            let rule = outcome
                .rule_id
                .map(|id| format!(" Durable rule: {id}."))
                .unwrap_or_default();
            Some(denied_tool_result(
                tool_use_id,
                tool_name,
                &format!(
                    "Execution denied: '{tool_name}' requires human approval and was denied or timed out. The operation was not performed.{reason}{rule} Do not retry the same action unchanged."
                ),
            ))
        }
        Err(e) => {
            warn!(tool_name, error = %e, "Approval system error");
            Some(denied_tool_result(
                tool_use_id,
                tool_name,
                &format!("Approval system error: {e}"),
            ))
        }
    }
}

fn denied_tool_result(tool_use_id: &str, tool_name: &str, message: &str) -> ToolResult {
    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: render_error_with_suggestion(
            tool_name,
            message,
            &crate::retry_transformer::RetryTransform::None,
        ),
        is_error: true,
        transient_content: Vec::new(),
    }
}

fn approval_preview_summary(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: &Arc<dyn KernelHandle>,
    caller_agent_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> String {
    match tool_name {
        "shell_exec" => shell_exec_approval_preview(input),
        "file_write" => file_write_approval_preview(input, kernel, caller_agent_id, workspace_root),
        "email_compose"
        | "email_reply"
        | "email_update"
        | "email_attachment_save"
        | "email_automation_rule_save"
        | "email_automation_rule_set_enabled"
        | "email_automation_rule_remove"
        | "email_automation_delivery_requeue" => email_approval_preview(tool_name, input),
        _ => generic_approval_preview(tool_name, input),
    }
}

pub(crate) fn shell_exec_approval_preview(input: &serde_json::Value) -> String {
    let command = input["command"].as_str().unwrap_or("<missing command>");
    format!(
        "Approval preview (no command executed yet).\n\
         Tool: shell_exec\n\
         Command list before run:\n\
         1. {}\n\
         Requires explicit confirmation before execution.",
        captain_types::truncate_str(command, 240)
    )
}

fn email_approval_preview(tool_name: &str, input: &serde_json::Value) -> String {
    let account = input["account"].as_str().unwrap_or("<default>");
    let detail = match tool_name {
        "email_compose" => format!(
            "delivery={}, to={}, cc={}, bcc={}, subject={}, attachments={}",
            input["delivery"].as_str().unwrap_or("draft"),
            preview_string_array(&input["to"]),
            preview_string_array(&input["cc"]),
            preview_string_array(&input["bcc"]),
            captain_types::truncate_str(input["subject"].as_str().unwrap_or("<empty>"), 120),
            input["attachments"].as_array().map_or(0, Vec::len),
        ),
        "email_reply" => format!(
            "delivery={}, message_id={}, attachments={}",
            input["delivery"].as_str().unwrap_or("draft"),
            captain_types::truncate_str(input["message_id"].as_str().unwrap_or("<missing>"), 80),
            input["attachments"].as_array().map_or(0, Vec::len),
        ),
        "email_update" => format!(
            "action={}, message_id={}, labels={}",
            input["action"].as_str().unwrap_or("<missing>"),
            captain_types::truncate_str(input["message_id"].as_str().unwrap_or("<missing>"), 80),
            input["label_ids"].as_array().map_or(0, Vec::len),
        ),
        "email_attachment_save" => format!(
            "message_id={}, attachment_id={}, path={}, overwrite={}",
            captain_types::truncate_str(input["message_id"].as_str().unwrap_or("<missing>"), 80),
            captain_types::truncate_str(input["attachment_id"].as_str().unwrap_or("<missing>"), 80),
            captain_types::truncate_str(input["path"].as_str().unwrap_or("<missing>"), 160),
            input["overwrite"].as_bool().unwrap_or(false),
        ),
        "email_automation_rule_save" => format!(
            "rule_id={}, expected_version={}, account={}, name={}, target_agent={}, enabled={}, include_body={}",
            captain_types::truncate_str(input["id"].as_str().unwrap_or("<derived>"), 96),
            input["expected_version"].as_u64().map_or_else(|| "<new>".to_string(), |value| value.to_string()),
            captain_types::truncate_str(input["account"].as_str().unwrap_or("<default>"), 48),
            captain_types::truncate_str(input["name"].as_str().unwrap_or("<missing>"), 120),
            captain_types::truncate_str(input["target_agent"].as_str().unwrap_or("captain"), 128),
            input["enabled"].as_bool().unwrap_or(true),
            input["include_body"].as_bool().unwrap_or(false),
        ),
        "email_automation_rule_set_enabled" => format!(
            "rule_id={}, expected_version={}, enabled={}",
            captain_types::truncate_str(input["rule_id"].as_str().unwrap_or("<missing>"), 96),
            input["expected_version"].as_u64().unwrap_or_default(),
            input["enabled"].as_bool().unwrap_or(false),
        ),
        "email_automation_rule_remove" => format!(
            "rule_id={}, expected_version={}",
            captain_types::truncate_str(input["rule_id"].as_str().unwrap_or("<missing>"), 96),
            input["expected_version"].as_u64().unwrap_or_default(),
        ),
        "email_automation_delivery_requeue" => format!(
            "delivery_id={}, expected_status={}",
            captain_types::truncate_str(input["delivery_id"].as_str().unwrap_or("<missing>"), 96),
            input["expected_status"].as_str().unwrap_or("<missing>"),
        ),
        _ => "unsupported email operation".to_string(),
    };
    format!(
        "Approval preview (no email or file side effect executed yet).\n\
         Tool: {tool_name}\n\
         Account: {}\n\
         Planned operation: {}\n\
         Message bodies and attachment contents are intentionally omitted.",
        captain_types::truncate_str(account, 48),
        captain_types::truncate_str(&detail, 480),
    )
}

fn preview_string_array(value: &serde_json::Value) -> String {
    let joined = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if joined.is_empty() {
        "<none>".to_string()
    } else {
        captain_types::truncate_str(&joined, 160).to_string()
    }
}

fn file_write_approval_preview(
    input: &serde_json::Value,
    kernel: &Arc<dyn KernelHandle>,
    caller_agent_id: Option<&str>,
    workspace_root: Option<&Path>,
) -> String {
    let raw_path = input["path"].as_str().unwrap_or("<missing path>");
    let new_content = input["content"].as_str().unwrap_or("");
    let diff_preview =
        match resolve_file_path_for_caller(raw_path, workspace_root, Some(kernel), caller_agent_id)
        {
            Ok(path) => file_write_diff_preview(&path, new_content),
            Err(err) => format!("diff preview unavailable before write: {err}"),
        };
    format!(
        "Approval preview (no file written yet).\n\
         Tool: file_write\n\
         Planned write: replace entire file at `{}` ({} bytes).\n\
         Diff before write: {}\n\
         Requires explicit confirmation before mutation.",
        captain_types::truncate_str(raw_path, 160),
        new_content.len(),
        diff_preview
    )
}

fn generic_approval_preview(tool_name: &str, input: &serde_json::Value) -> String {
    let input_str = input.to_string();
    format!(
        "Approval preview (no side effects executed yet).\nTool: {tool_name}\nInput: {}",
        captain_types::truncate_str(&input_str, 240)
    )
}

fn file_write_diff_preview(path: &Path, new_content: &str) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= 64 * 1024 => {
            match std::fs::read_to_string(path) {
                Ok(old_content) => first_line_diff_preview(&old_content, new_content),
                Err(_) => format!(
                    "existing non-text or unreadable file, {} -> {} bytes",
                    metadata.len(),
                    new_content.len()
                ),
            }
        }
        Ok(metadata) if metadata.is_file() => format!(
            "existing file too large for inline diff, {} -> {} bytes",
            metadata.len(),
            new_content.len()
        ),
        Ok(_) => "target exists but is not a regular file".to_string(),
        Err(_) => format!("new file, +{} bytes", new_content.len()),
    }
}

fn first_line_diff_preview(old_content: &str, new_content: &str) -> String {
    if old_content == new_content {
        return "no textual change".to_string();
    }
    let mut old_lines = old_content.lines();
    let mut new_lines = new_content.lines();
    let mut line_no = 1usize;
    loop {
        match (old_lines.next(), new_lines.next()) {
            (Some(old), Some(new)) if old == new => line_no += 1,
            (old, new) => {
                return format!(
                    "line {line_no}: -{} +{} ({} -> {} bytes)",
                    captain_types::truncate_str(old.unwrap_or("<missing>"), 80),
                    captain_types::truncate_str(new.unwrap_or("<missing>"), 80),
                    old_content.len(),
                    new_content.len()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn paired_client_origin_denies_administrative_tools_before_dispatch() {
        let result = super::super::with_origin_channel(
            Some(crate::client_authority::paired_client_origin("test")),
            run_pre_dispatch_checks(
                "tool-call-1",
                "config_read",
                &serde_json::json!({}),
                None,
                None,
                None,
                None,
            ),
        )
        .await
        .expect("paired Client guard must return a denial");

        assert!(result.is_error);
        assert!(result.content.contains("outside paired Client authority"));
    }

    #[tokio::test]
    async fn paired_client_origin_keeps_explicit_memory_authority() {
        let result = super::super::with_origin_channel(
            Some(crate::client_authority::paired_client_origin("test")),
            run_pre_dispatch_checks(
                "tool-call-2",
                "memory_recall",
                &serde_json::json!({"query": "preferences"}),
                None,
                None,
                None,
                None,
            ),
        )
        .await;

        assert!(result.is_none());
    }

    struct PreviewKernel;

    #[async_trait::async_trait]
    impl KernelHandle for PreviewKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("not implemented".to_string())
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            Err("not implemented".to_string())
        }

        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        fn find_agents(&self, _query: &str) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            Err("not implemented".to_string())
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn shell_approval_preview_lists_command_before_run() {
        let preview = shell_exec_approval_preview(&serde_json::json!({
            "command": "rm -rf target/tmp"
        }));

        assert!(preview.contains("no command executed yet"));
        assert!(preview.contains("Command list before run"));
        assert!(preview.contains("rm -rf target/tmp"));
        assert!(preview.contains("confirmation before execution"));
    }

    #[test]
    fn file_write_approval_preview_includes_diff_before_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let kernel: Arc<dyn KernelHandle> = Arc::new(PreviewKernel);

        let preview = file_write_approval_preview(
            &serde_json::json!({
                "path": "note.txt",
                "content": "alpha\ngamma\n"
            }),
            &kernel,
            Some("captain"),
            Some(dir.path()),
        );

        assert!(preview.contains("no file written yet"));
        assert!(preview.contains("Diff before write"));
        assert!(preview.contains("line 2"));
        assert!(preview.contains("-beta"));
        assert!(preview.contains("+gamma"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "alpha\nbeta\n");
    }

    #[test]
    fn email_approval_preview_never_exposes_message_body() {
        let preview = email_approval_preview(
            "email_compose",
            &serde_json::json!({
                "account": "work",
                "to": ["recipient@example.com"],
                "subject": "Status report",
                "text_body": "PRIVATE BODY MUST NOT LEAK",
                "html_body": "<p>PRIVATE HTML MUST NOT LEAK</p>",
                "delivery": "send",
                "attachments": [{"path": "private.txt"}]
            }),
        );

        assert!(preview.contains("recipient@example.com"));
        assert!(preview.contains("Status report"));
        assert!(preview.contains("delivery=send"));
        assert!(!preview.contains("PRIVATE BODY"));
        assert!(!preview.contains("PRIVATE HTML"));
        assert!(!preview.contains("private.txt"));
    }
}
