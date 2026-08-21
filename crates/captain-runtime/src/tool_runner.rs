//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

use crate::execution_routing::{
    current_turn_execution_context, RemoteToolExecutionRequest, ResolvedExecutionTarget,
};
use crate::kernel_handle::KernelHandle;
use crate::mcp;
pub(crate) use crate::tools::emit_tool_chunk;
pub(crate) use crate::tools::ensure_no_secret_literal;
#[cfg(test)]
use crate::tools::extract_pdf_text_from_bytes;
#[cfg(test)]
use crate::tools::screenshot_command;
#[cfg(test)]
use crate::tools::validate_child_agent_tool_scope;
use crate::tools::{
    cached_tool_result, finalize_dispatch_result, finalize_remote_dispatch_result,
    run_pre_dispatch_checks, DispatchFinalizeContext,
};
#[cfg(test)]
use crate::tools::{
    compact_memory_context_result, compact_mempalace_search_result, detect_image_format,
    ensure_cron_webhook_url_is_public, extract_image_dimensions, format_file_size,
    hash_web_password, memory_context_tokens, memory_recall_part, parse_schedule_to_cron,
    render_error_with_suggestion, sanitize_canvas_html, tool_apply_patch, tool_canvas_present,
    tool_capability_search, tool_edit_file, tool_execute_code, tool_file_inspect_batch,
    tool_file_write, tool_glob, tool_grep, tool_learning_review_decide, tool_learning_review_list,
    tool_multi_edit, tool_pkg_wrapper, tool_search, tool_self_improvement_review,
    tool_skill_refinement_decide, tool_skill_refinement_list, tool_skill_refinement_propose,
    tool_skill_refinement_restore, tool_skill_refinement_update, tool_skill_search,
    tool_ssh_download, tool_ssh_exec, tool_ssh_upload, tool_system_bug_list,
    tool_system_bug_report, tool_system_bug_update, tool_workflow_learning_list,
    write_web_credentials_config, CARGO_SUBCOMMANDS, DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
    NPM_SUBCOMMANDS, PIP_SUBCOMMANDS, SKILL_REFINEMENTS_KEY, SYSTEM_BUGS_KEY,
};
pub use crate::tools::{
    current_agent_depth, current_agent_lineage_depth, current_origin_channel, progress_sink,
    with_agent_lineage_depth, with_origin_channel, with_progress_sink, ProgressThrottle,
    ToolProgressEvent, ToolStreamCtx, CANVAS_MAX_BYTES, TOOL_STREAM,
};
#[cfg(test)]
use crate::tools::{ensure_extension_for_mime, sanitize_download_filename};
#[cfg(test)]
use crate::tools::{find_python_interpreter, validate_pip_allowlist};
#[cfg(test)]
use crate::tools::{AGENT_CALL_DEPTH, MAX_AGENT_CALL_DEPTH};
use crate::web_search::WebToolsContext;
use crate::workflow_learning_runtime::{record_tool_finished, record_tool_started};
use captain_skills::registry::SkillRegistry;
use captain_types::tool::{ToolDefinition, ToolResult};
use captain_types::tool_compat::normalize_tool_name;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;

#[path = "tool_runner_dispatch.rs"]
mod tool_runner_dispatch;

use self::tool_runner_dispatch::{dispatch_tool, ToolDispatchOutcome, ToolDispatchRequest};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ToolCacheMode {
    #[default]
    Global,
    #[allow(dead_code)] // retained by the cfg(test) legacy parity oracle
    Disabled,
}

/// Execute a tool by name with the given input, returning a ToolResult.
///
/// The optional `kernel` handle enables inter-agent tools. If `None`,
/// agent tools will return an error indicating the kernel is not available.
///
/// `allowed_tools` enforces capability-based security: if provided, only
/// tools in the list may execute. This prevents an LLM from hallucinating
/// tool names outside the agent's capability grants.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
) -> ToolResult {
    execute_tool_with_cache_mode(
        tool_use_id,
        tool_name,
        input,
        kernel,
        allowed_tools,
        caller_agent_id,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        media_engine,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
        ToolCacheMode::Global,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_tool_with_cache_mode(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&captain_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    cache_mode: ToolCacheMode,
) -> ToolResult {
    // Normalize the tool name through compat mappings so LLM-hallucinated aliases
    // (e.g. "fs-write" → "file_write") resolve to the canonical Captain name.
    let tool_name = normalize_tool_name(tool_name);
    let global_exec_policy = kernel.map(|handle| handle.global_exec_policy());
    let effective_exec_policy =
        intersect_execution_policy(exec_policy, global_exec_policy.as_ref());
    let exec_policy = effective_exec_policy.as_ref();

    // v3.12b — wall-clock for the LearningSignal emission at the end.
    let dispatch_start = std::time::Instant::now();
    record_tool_started(tool_use_id, tool_name, input);

    // Grouped tool dispatch removed — tools are now flat with proper schemas.
    // tool_groups::resolve_grouped_tool is no longer called here.

    if let Some(blocked) = run_pre_dispatch_checks(
        tool_use_id,
        tool_name,
        input,
        kernel,
        allowed_tools,
        caller_agent_id,
        workspace_root,
    )
    .await
    {
        record_tool_finished(
            tool_use_id,
            tool_name,
            blocked.is_error,
            0,
            "pre_dispatch_blocked",
        );
        return blocked;
    }

    if let Some(context) = current_turn_execution_context() {
        if let ResolvedExecutionTarget::Node {
            device_id,
            workspace_id,
        } = context.target
        {
            if crate::node_tool_runtime::local_node_tool_family(tool_name).is_some() {
                let result = execute_remote_node_tool(
                    RemoteToolExecutionRequest {
                        scope_id: context.scope_id,
                        tool_use_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        input: input.clone(),
                        caller_agent_id: caller_agent_id.unwrap_or("unknown").to_string(),
                        device_id,
                        workspace_id,
                    },
                    kernel,
                )
                .await;
                let no_cache = None;
                return finalize_remote_dispatch_result(
                    DispatchFinalizeContext {
                        tool_use_id,
                        tool_name,
                        input,
                        kernel,
                        mcp_connections,
                        caller_agent_id,
                        tool_cache: &no_cache,
                        dispatch_start,
                    },
                    result,
                );
            }
        }
    }

    // v3.10f — cache lookup before dispatch.
    let tool_cache = match cache_mode {
        ToolCacheMode::Global => crate::tool_cache::global_cache(),
        ToolCacheMode::Disabled => None,
    };
    if let Some(cached) = cached_tool_result(tool_use_id, tool_name, input, &tool_cache).await {
        record_tool_finished(tool_use_id, tool_name, cached.is_error, 0, "cache_hit");
        return cached;
    }

    debug!(tool_name, "Executing tool");
    let dispatch = dispatch_tool(ToolDispatchRequest {
        tool_use_id,
        tool_name,
        input,
        kernel,
        allowed_tools,
        caller_agent_id,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        media_engine,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
    })
    .await;
    let (result, transient_content) = match dispatch {
        ToolDispatchOutcome::Blocked(result) => {
            record_tool_finished(
                tool_use_id,
                tool_name,
                result.is_error,
                0,
                "dispatch_blocked",
            );
            return result;
        }
        ToolDispatchOutcome::Dispatched(result) => (result, Vec::new()),
        ToolDispatchOutcome::Browser(result) => match result {
            Ok(output) => (Ok(output.content), output.transient_content),
            Err(error) => (Err(error), Vec::new()),
        },
    };

    let mut tool_result = finalize_dispatch_result(
        DispatchFinalizeContext {
            tool_use_id,
            tool_name,
            input,
            kernel,
            mcp_connections,
            caller_agent_id,
            tool_cache: &tool_cache,
            dispatch_start,
        },
        result,
    )
    .await;
    if !tool_result.is_error {
        tool_result.transient_content = transient_content;
    }
    tool_result
}

async fn execute_remote_node_tool(
    request: RemoteToolExecutionRequest,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> ToolResult {
    let tool_use_id = request.tool_use_id.clone();
    let Some(kernel) = kernel else {
        return remote_node_error(
            &tool_use_id,
            "the Kernel execution rail is unavailable; no local fallback was attempted",
        );
    };
    match kernel.execute_remote_tool(request).await {
        Ok(result) => result,
        Err(error) => remote_node_error(
            &tool_use_id,
            &format!("{error}; no local fallback was attempted"),
        ),
    }
}

fn remote_node_error(tool_use_id: &str, message: &str) -> ToolResult {
    ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content: format!("Remote Node execution failed: {message}"),
        is_error: true,
        transient_content: Vec::new(),
    }
}

fn intersect_execution_policy(
    agent_policy: Option<&captain_types::config::ExecPolicy>,
    global_policy: Option<&captain_types::config::ExecPolicy>,
) -> Option<captain_types::config::ExecPolicy> {
    match (agent_policy, global_policy) {
        (None, None) => None,
        (Some(policy), None) | (None, Some(policy)) => Some(policy.clone()),
        (Some(agent), Some(global)) => Some(agent.intersect(global)),
    }
}

/// Get definitions for all built-in tools.
pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    crate::tools::builtin_tool_definitions()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        tool_channel_reconfigure, tool_memory_forget, tool_memory_save, tool_session_recall,
        tool_workspace_add,
    };

    mod agent_scope;
    mod canvas_runtime;
    mod capability_search_runtime;
    mod capspec_runtime;
    mod channel_reconfigure_runtime;
    mod depth_schedule;
    mod dispatch_contracts;
    mod document_web;
    mod error_recovery;
    mod file_edit;
    mod file_search;
    mod image_runtime;
    mod improvement_bounds;
    mod improvement_output_safety;
    mod improvement_runtime;
    mod improvement_safety;
    mod memory_forget_context;
    mod memory_save_runtime;
    mod project_runtime;
    mod registry_config;
    mod remote_routing;
    mod schedule_parse;
    mod schema_guidance;
    mod security_execute;
    mod session_workspace;
    mod skill_view_runtime;
    mod ssh_package;
    mod tool_search_runtime;
    use memory_save_runtime::MemSaveStubKernel;

    #[test]
    fn global_execution_profile_is_a_non_bypassable_floor() {
        use captain_types::config::{ExecPolicy, ExecSecurityMode, ExecutionProfile};

        let agent = ExecPolicy {
            profile: ExecutionProfile::PersonalWorkstation,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let global = ExecPolicy {
            profile: ExecutionProfile::UntrustedExecution,
            mode: ExecSecurityMode::Allowlist,
            ..ExecPolicy::default()
        };
        let effective = intersect_execution_policy(Some(&agent), Some(&global))
            .expect("effective execution policy");

        assert_eq!(effective.profile, ExecutionProfile::UntrustedExecution);
        assert_eq!(effective.mode, ExecSecurityMode::Allowlist);
        assert_eq!(effective.effective_mode(), ExecSecurityMode::Deny);
    }

    #[test]
    fn global_execution_mode_is_a_non_bypassable_floor() {
        use captain_types::config::{ExecPolicy, ExecSecurityMode};

        let agent = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let global = ExecPolicy {
            mode: ExecSecurityMode::Deny,
            ..ExecPolicy::default()
        };
        let effective = intersect_execution_policy(Some(&agent), Some(&global))
            .expect("effective execution policy");

        assert_eq!(effective.effective_mode(), ExecSecurityMode::Deny);
    }
}
