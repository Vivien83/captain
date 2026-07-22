//! Post-dispatch retry, error shaping, cache update, and learning signals.

use std::sync::Arc;
use std::time::Instant;

use captain_types::tool::ToolResult;
use tracing::{info, warn};

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tool_cache::{CachedToolResult, ToolKind, ToolResultCache};
use crate::workflow_learning_runtime::record_tool_finished;

use super::{
    is_retryable_tool, is_write_tool_that_must_not_be_masked, render_error_with_suggestion,
    tool_config_read, tool_cron_cancel, tool_cron_create, tool_cron_list, tool_cron_update,
    tool_file_trigger_list, tool_file_trigger_register, tool_file_trigger_remove,
    tool_file_trigger_set_enabled, tool_knowledge_add_entity, tool_knowledge_add_relation,
    tool_knowledge_query, tool_memory_recall, tool_memory_recall_mempalace, tool_memory_save,
    tool_memory_store, tool_memory_store_mempalace, tool_secret_read, tool_todo_complete,
    tool_todo_create, tool_todo_delete, tool_todo_list, tool_todo_reopen,
};

pub(crate) struct DispatchFinalizeContext<'a> {
    pub tool_use_id: &'a str,
    pub tool_name: &'a str,
    pub input: &'a serde_json::Value,
    pub kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub mcp_connections: Option<&'a tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    pub caller_agent_id: Option<&'a str>,
    pub tool_cache: &'a Option<Arc<ToolResultCache>>,
    pub dispatch_start: Instant,
}

pub(crate) async fn finalize_dispatch_result(
    ctx: DispatchFinalizeContext<'_>,
    result: Result<String, String>,
) -> ToolResult {
    let (result, retry_count) = retry_if_transient(&ctx, result).await;
    let recovered_after_retry = retry_count > 0 && result.is_ok();
    let dispatch_failed = result.is_err();
    let tool_result = result_to_tool_result(&ctx, result, recovered_after_retry);
    update_cache(&ctx, &tool_result).await;
    emit_learning_signal(&ctx, &tool_result, recovered_after_retry, dispatch_failed);
    let (learning_is_error, output_class) =
        learning_outcome(dispatch_failed, tool_result.is_error, recovered_after_retry);
    record_tool_finished(
        ctx.tool_use_id,
        ctx.tool_name,
        learning_is_error,
        retry_count,
        output_class,
    );
    tool_result
}

fn learning_outcome(
    dispatch_failed: bool,
    visible_tool_error: bool,
    recovered_after_retry: bool,
) -> (bool, &'static str) {
    if recovered_after_retry {
        (false, "retry_success")
    } else if dispatch_failed && !visible_tool_error {
        (true, "transient_unavailable")
    } else if visible_tool_error {
        (true, "tool_error")
    } else {
        (false, "tool_success")
    }
}

async fn retry_if_transient(
    ctx: &DispatchFinalizeContext<'_>,
    result: Result<String, String>,
) -> (Result<String, String>, u32) {
    if result.is_ok() || !is_retryable_tool(ctx.tool_name) {
        return (result, 0);
    }
    warn!(
        tool_name = ctx.tool_name,
        "Tool failed, retrying once after 500ms"
    );
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mem_backend = ctx.kernel.map(|kh| kh.memory_backend()).unwrap_or_default();
    let retry = match ctx.tool_name {
        "memory_save" => tool_memory_save(ctx.input, ctx.mcp_connections, ctx.kernel).await,
        "memory_store" => match mem_backend {
            captain_types::config::MemoryBackend::Mempalace => {
                tool_memory_store_mempalace(ctx.input, ctx.mcp_connections, ctx.kernel).await
            }
            captain_types::config::MemoryBackend::Graph => tool_memory_store(ctx.input, ctx.kernel),
        },
        "memory_recall" => match mem_backend {
            captain_types::config::MemoryBackend::Mempalace => {
                tool_memory_recall_mempalace(ctx.input, ctx.mcp_connections, ctx.kernel).await
            }
            captain_types::config::MemoryBackend::Graph => {
                tool_memory_recall(ctx.input, ctx.kernel)
            }
        },
        "cron_create" => tool_cron_create(ctx.input, ctx.kernel, ctx.caller_agent_id).await,
        "cron_list" => tool_cron_list(ctx.kernel, ctx.caller_agent_id).await,
        "cron_update" => tool_cron_update(ctx.input, ctx.kernel, ctx.caller_agent_id).await,
        "cron_cancel" => tool_cron_cancel(ctx.input, ctx.kernel).await,
        "file_trigger_register" => {
            tool_file_trigger_register(ctx.input, ctx.kernel, ctx.caller_agent_id).await
        }
        "file_trigger_list" => {
            tool_file_trigger_list(ctx.input, ctx.kernel, ctx.caller_agent_id).await
        }
        "file_trigger_set_enabled" => tool_file_trigger_set_enabled(ctx.input, ctx.kernel).await,
        "file_trigger_remove" => tool_file_trigger_remove(ctx.input, ctx.kernel).await,
        "todo_create" => tool_todo_create(ctx.input, ctx.kernel),
        "todo_list" => tool_todo_list(ctx.input, ctx.kernel),
        "todo_complete" => tool_todo_complete(ctx.input, ctx.kernel),
        "todo_reopen" => tool_todo_reopen(ctx.input, ctx.kernel),
        "todo_delete" => tool_todo_delete(ctx.input, ctx.kernel),
        "knowledge_query" => tool_knowledge_query(ctx.input, ctx.kernel).await,
        "knowledge_add_entity" => tool_knowledge_add_entity(ctx.input, ctx.kernel).await,
        "knowledge_add_relation" => tool_knowledge_add_relation(ctx.input, ctx.kernel).await,
        "config_read" => tool_config_read(ctx.input, ctx.kernel),
        "secret_read" => tool_secret_read(ctx.input, ctx.kernel),
        _ => result,
    };
    if retry.is_ok() {
        info!(tool_name = ctx.tool_name, "Retry succeeded");
        (retry, 1)
    } else {
        (retry, 1)
    }
}

fn result_to_tool_result(
    ctx: &DispatchFinalizeContext<'_>,
    result: Result<String, String>,
    recovered_after_retry: bool,
) -> ToolResult {
    match result {
        Ok(content) => ToolResult {
            tool_use_id: ctx.tool_use_id.to_string(),
            content,
            is_error: false,
            transient_content: Vec::new(),
        },
        Err(err) => error_to_tool_result(ctx, err, recovered_after_retry),
    }
}

fn error_to_tool_result(
    ctx: &DispatchFinalizeContext<'_>,
    err: String,
    _recovered_after_retry: bool,
) -> ToolResult {
    let is_transient = is_retryable_tool(ctx.tool_name)
        && !is_write_tool_that_must_not_be_masked(ctx.tool_name)
        && !err.contains("missing field")
        && !err.contains("Missing")
        && !err.contains("Invalid")
        && !err.contains("invalid")
        && !err.contains("required");
    if is_transient {
        warn!(
            tool_name = ctx.tool_name,
            error = %err,
            "Internal tool failed after retry — masking transient error"
        );
        return ToolResult {
            tool_use_id: ctx.tool_use_id.to_string(),
            content: "Temporarily unavailable. Continue without this result.".to_string(),
            is_error: false,
            transient_content: Vec::new(),
        };
    }

    let (command_hint, tool_for_ctx): (Option<String>, &str) = match ctx.tool_name {
        "shell_exec" | "execute_code" | "docker_exec" | "docker_run" => (
            ctx.input
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            ctx.tool_name,
        ),
        _ => (None, ctx.tool_name),
    };
    let error_ctx = crate::retry_transformer::ErrorContext {
        tool_name: tool_for_ctx,
        command: command_hint.as_deref(),
        host_os: host_os_label(),
    };
    let suggestion = crate::retry_transformer::analyze(&err, &error_ctx);
    ToolResult {
        tool_use_id: ctx.tool_use_id.to_string(),
        content: render_error_with_suggestion(ctx.tool_name, &err, &suggestion),
        is_error: true,
        transient_content: Vec::new(),
    }
}

async fn update_cache(ctx: &DispatchFinalizeContext<'_>, tool_result: &ToolResult) {
    let Some(cache) = ctx.tool_cache.as_ref() else {
        return;
    };
    let cached = CachedToolResult {
        output: tool_result.content.clone(),
        is_error: tool_result.is_error,
    };
    let _ = cache.store(ctx.tool_name, ctx.input, &cached).await;
    if cache
        .policy(ctx.tool_name)
        .is_some_and(|policy| policy.kind == ToolKind::Write)
    {
        let _ = cache.invalidate_after_write(ctx.tool_name, ctx.input).await;
    }
}

fn emit_learning_signal(
    ctx: &DispatchFinalizeContext<'_>,
    tool_result: &ToolResult,
    recovered_after_retry: bool,
    dispatch_failed: bool,
) {
    let duration_ms = ctx.dispatch_start.elapsed().as_millis() as u64;
    let agent_id = ctx.caller_agent_id.unwrap_or("unknown").to_string();
    let signal = if dispatch_failed || tool_result.is_error {
        crate::learning_bus::LearningSignal::ToolFailure {
            agent_id: agent_id.clone(),
            tool: ctx.tool_name.to_string(),
            error: tool_result.content.clone(),
            source: "tool_runner".to_string(),
        }
    } else if recovered_after_retry {
        crate::learning_bus::LearningSignal::RetrySuccess {
            agent_id: agent_id.clone(),
            tool: ctx.tool_name.to_string(),
            prior_errors: 1,
            source: "tool_runner".to_string(),
        }
    } else {
        crate::learning_bus::LearningSignal::ToolSuccess {
            agent_id: agent_id.clone(),
            tool: ctx.tool_name.to_string(),
            duration_ms,
            source: "tool_runner".to_string(),
        }
    };
    let _ = crate::learning_bus::emit(signal);
}

fn host_os_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::learning_outcome;

    #[test]
    fn learning_outcome_never_counts_a_masked_dispatch_failure_as_success() {
        assert_eq!(
            learning_outcome(true, false, false),
            (true, "transient_unavailable")
        );
        assert_eq!(learning_outcome(true, true, false), (true, "tool_error"));
        assert_eq!(
            learning_outcome(false, false, true),
            (false, "retry_success")
        );
        assert_eq!(
            learning_outcome(false, false, false),
            (false, "tool_success")
        );
    }
}
