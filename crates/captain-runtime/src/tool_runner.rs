//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

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
    cached_tool_result, finalize_dispatch_result, run_pre_dispatch_checks, DispatchFinalizeContext,
};
#[cfg(test)]
use crate::tools::{
    compact_memory_context_result, compact_mempalace_search_result, detect_image_format,
    ensure_cron_webhook_url_is_public, extract_image_dimensions, format_file_size,
    hash_web_password, memory_context_tokens, memory_recall_part, parse_schedule_to_cron,
    render_error_with_suggestion, sanitize_canvas_html, tool_apply_patch, tool_canvas_present,
    tool_capability_search, tool_edit_file, tool_execute_code, tool_file_inspect_batch,
    tool_file_write, tool_glob, tool_grep, tool_learning_review_decide, tool_learning_review_list,
    tool_multi_edit, tool_pkg_wrapper, tool_search, tool_self_improvement_review, tool_shell_exec,
    tool_skill_proposal_decide, tool_skill_proposal_list, tool_skill_refinement_decide,
    tool_skill_refinement_list, tool_skill_refinement_propose, tool_skill_refinement_restore,
    tool_skill_refinement_update, tool_skill_search, tool_ssh_download, tool_ssh_exec,
    tool_ssh_upload, tool_system_bug_list, tool_system_bug_report, tool_system_bug_update,
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
use captain_skills::registry::SkillRegistry;
use captain_types::tool::{ToolDefinition, ToolResult};
use captain_types::tool_compat::normalize_tool_name;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;

#[path = "tool_runner_dispatch.rs"]
mod tool_runner_dispatch;

use self::tool_runner_dispatch::{dispatch_tool, ToolDispatchOutcome, ToolDispatchRequest};

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
    // Normalize the tool name through compat mappings so LLM-hallucinated aliases
    // (e.g. "fs-write" → "file_write") resolve to the canonical Captain name.
    let tool_name = normalize_tool_name(tool_name);

    // v3.12b — wall-clock for the LearningSignal emission at the end.
    let dispatch_start = std::time::Instant::now();

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
        return blocked;
    }

    // v3.10f — cache lookup before dispatch.
    let tool_cache = crate::tool_cache::global_cache();
    if let Some(cached) = cached_tool_result(tool_use_id, tool_name, input, &tool_cache).await {
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
        ToolDispatchOutcome::Blocked(result) => return result,
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
    mod schedule_parse;
    mod schema_guidance;
    mod security_execute;
    mod session_workspace;
    mod skill_view_runtime;
    mod ssh_package;
    mod tool_search_runtime;
    use memory_save_runtime::MemSaveStubKernel;
}
