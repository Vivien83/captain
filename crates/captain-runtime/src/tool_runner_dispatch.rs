use std::path::Path;
use std::sync::Arc;

use captain_skills::registry::SkillRegistry;
use captain_types::config::{DockerSandboxConfig, ExecPolicy};
use captain_types::tool::ToolResult;

use crate::browser::{BrowserManager, BrowserToolResult};
use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::media_understanding::MediaEngine;
use crate::process_manager::ProcessManager;
use crate::tools::{
    dispatch_a2a_tool, dispatch_agent_tool, dispatch_automation_tool, dispatch_browser_tool,
    dispatch_canvas_tool, dispatch_capspec_management_tool, dispatch_channel_tool,
    dispatch_config_tool, dispatch_coordination_tool, dispatch_discovery_tool,
    dispatch_docker_tool, dispatch_document_tool, dispatch_fallback_tool, dispatch_file_tool,
    dispatch_goal_tool, dispatch_hand_tool, dispatch_improvement_tool, dispatch_knowledge_tool,
    dispatch_location_tool, dispatch_media_tool, dispatch_memory_tool, dispatch_package_tool,
    dispatch_peer_tool, dispatch_process_tool, dispatch_project_tool, dispatch_shell_exec,
    dispatch_skill_runtime_tool, dispatch_ssh_tool, dispatch_system_update, dispatch_tool_run_tool,
    dispatch_web_tool, tool_execute_code, tool_screenshot, ShellDispatchOutcome,
    WebDispatchOutcome,
};
use crate::tts::TtsEngine;
use crate::web_search::WebToolsContext;

#[path = "tool_runner_capspec.rs"]
mod capspec;

pub(super) struct ToolDispatchRequest<'a> {
    pub tool_use_id: &'a str,
    pub tool_name: &'a str,
    pub input: &'a serde_json::Value,
    pub kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub allowed_tools: Option<&'a [String]>,
    pub caller_agent_id: Option<&'a str>,
    pub skill_registry: Option<&'a SkillRegistry>,
    pub mcp_connections: Option<&'a tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    pub web_ctx: Option<&'a WebToolsContext>,
    pub browser_ctx: Option<&'a BrowserManager>,
    pub allowed_env_vars: Option<&'a [String]>,
    pub workspace_root: Option<&'a Path>,
    pub media_engine: Option<&'a MediaEngine>,
    pub exec_policy: Option<&'a ExecPolicy>,
    pub tts_engine: Option<&'a TtsEngine>,
    pub docker_config: Option<&'a DockerSandboxConfig>,
    pub process_manager: Option<&'a ProcessManager>,
}

pub(super) enum ToolDispatchOutcome {
    Blocked(ToolResult),
    Dispatched(Result<String, String>),
    Browser(Result<BrowserToolResult, String>),
}

pub(super) async fn dispatch_tool(request: ToolDispatchRequest<'_>) -> ToolDispatchOutcome {
    if let Some(outcome) = dispatch_io_execution_tool(&request).await {
        return outcome;
    }
    if let Some(outcome) = dispatch_network_shell_tool(&request).await {
        return outcome;
    }
    if let Some(outcome) = dispatch_kernel_state_tool(&request).await {
        return outcome;
    }
    if let Some(outcome) = dispatch_runtime_capability_tool(&request).await {
        return outcome;
    }
    if let Some(outcome) = capspec::dispatch_capspec_tool(&request).await {
        return outcome;
    }

    ToolDispatchOutcome::Dispatched(
        dispatch_fallback_tool(
            request.tool_name,
            request.input,
            request.skill_registry,
            request.mcp_connections,
        )
        .await,
    )
}

async fn dispatch_io_execution_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "file_inspect_batch" | "file_read" | "file_write" | "file_list" | "apply_patch"
        | "edit_file" | "multi_edit" | "grep" | "glob" => {
            dispatch_file_tool(
                request.tool_name,
                request.input,
                request.workspace_root,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        "ssh_health_check" | "ssh_exec" | "ssh_upload" | "ssh_download" => {
            dispatch_ssh_tool(request.tool_name, request.input, request.exec_policy).await
        }
        "cargo" | "npm" | "pip" => {
            dispatch_package_tool(
                request.tool_name,
                request.input,
                request.allowed_env_vars,
                request.workspace_root,
                request.exec_policy,
            )
            .await
        }
        "screenshot" => tool_screenshot(request.input, request.workspace_root).await,
        "execute_code" => tool_execute_code(request.input, request.workspace_root).await,
        "document_pipeline" | "document_create" | "document_extract" => {
            dispatch_document_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.workspace_root,
                request.caller_agent_id,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_network_shell_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    match request.tool_name {
        "web_research_batch" | "web_download" | "web_fetch" | "web_search" => Some(
            match dispatch_web_tool(
                request.tool_use_id,
                request.tool_name,
                request.input,
                request.web_ctx,
                request.workspace_root,
                request.kernel,
                request.caller_agent_id,
            )
            .await
            {
                WebDispatchOutcome::Blocked(result) => ToolDispatchOutcome::Blocked(result),
                WebDispatchOutcome::Result(result) => ToolDispatchOutcome::Dispatched(result),
            },
        ),
        "shell_exec" => Some(
            match dispatch_shell_exec(
                request.tool_use_id,
                request.input,
                request.kernel,
                request.caller_agent_id,
                request.allowed_env_vars,
                request.workspace_root,
                request.exec_policy,
            )
            .await
            {
                ShellDispatchOutcome::Blocked(result) => ToolDispatchOutcome::Blocked(result),
                ShellDispatchOutcome::Result(result) => ToolDispatchOutcome::Dispatched(result),
            },
        ),
        _ => None,
    }
}

async fn dispatch_kernel_state_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    if let Some(outcome) = dispatch_agent_memory_config_tool(request).await {
        return Some(outcome);
    }
    if let Some(outcome) = dispatch_config_state_tool(request).await {
        return Some(outcome);
    }
    if let Some(outcome) = dispatch_operator_channel_tool(request).await {
        return Some(outcome);
    }
    if let Some(outcome) = dispatch_coordination_project_tool(request).await {
        return Some(outcome);
    }
    dispatch_project_state_tool(request).await
}

async fn dispatch_agent_memory_config_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "agent_send"
        | "agent_spawn"
        | "agent_list"
        | "agent_kill"
        | "agent_status"
        | "agent_caps"
        | "agent_watch"
        | "agent_delegate"
        | "agent_correct"
        | "fleet_create_manager"
        | "fleet_list_managers"
        | "fleet_close_manager"
        | "fleet_set_mission"
        | "fleet_configure_autoscale"
        | "fleet_metrics" => {
            dispatch_agent_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
                request.allowed_tools,
            )
            .await
        }
        "memory_store"
        | "memory_recall"
        | "memory_context_batch"
        | "memory_save"
        | "workspace_add"
        | "memory_forget"
        | "session_recall"
        | "session_tool_call_summary" => {
            dispatch_memory_tool(
                request.tool_name,
                request.input,
                request.mcp_connections,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_config_state_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "config_read"
        | "config_write"
        | "self_configure"
        | "model_switch_plan"
        | "model_switch_apply"
        | "codex_auth_status"
        | "codex_tool_probe"
        | "codex_login_start"
        | "codex_login_poll"
        | "secret_read"
        | "secret_write"
        | "web_credentials_update"
        | "config_setup"
        | "mcp_catalog_search"
        | "mcp_integration_install"
        | "mcp_status"
        | "config_schema" => {
            dispatch_config_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_operator_channel_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "goal_create"
        | "goal_list"
        | "goal_pause"
        | "goal_resume"
        | "goal_status"
        | "goal_delete"
        | "goal_list_suggestions"
        | "goal_apply_suggestion"
        | "goal_reject_suggestion" => {
            dispatch_goal_tool(request.tool_name, request.input, request.kernel)
        }
        "peer_list" => dispatch_peer_tool(request.tool_name, request.kernel),
        "channel_reconfigure"
        | "channel_delivery_batch"
        | "channel_send"
        | "telegram_set_topic"
        | "telegram_get_topic" => {
            dispatch_channel_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.workspace_root,
                request.caller_agent_id,
            )
            .await
        }
        "captain_docs" | "capability_search" | "skill_search" | "skill_view" | "skill_check"
        | "tool_search" => {
            dispatch_discovery_tool(
                request.tool_name,
                request.input,
                request.skill_registry,
                request.mcp_connections,
                request.kernel,
                request.workspace_root,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_coordination_project_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "agent_find" | "task_post" | "task_claim" | "task_complete" | "task_list"
        | "event_publish" => {
            dispatch_coordination_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        "schedule_create" | "schedule_list" | "schedule_delete" => {
            dispatch_automation_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        "knowledge_add_entity" | "knowledge_add_relation" | "knowledge_query" => {
            dispatch_knowledge_tool(request.tool_name, request.input, request.kernel).await
        }
        "cron_create"
        | "cron_list"
        | "cron_update"
        | "cron_cancel"
        | "reminder_set"
        | "file_trigger_register"
        | "file_trigger_list"
        | "file_trigger_set_enabled"
        | "file_trigger_remove"
        | "todo_create"
        | "todo_list"
        | "todo_complete"
        | "todo_reopen"
        | "todo_delete" => {
            dispatch_automation_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_project_state_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "project_create"
        | "project_list"
        | "project_get"
        | "project_archive"
        | "project_resume"
        | "project_task_create"
        | "project_task_list"
        | "project_task_update"
        | "milestone_create"
        | "milestone_list"
        | "milestone_complete"
        | "milestone_progress"
        | "checkpoint_save" => dispatch_project_tool(
            request.tool_name,
            request.input,
            request.kernel,
            request.caller_agent_id,
        ),
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_runtime_capability_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    if let Some(result) = dispatch_capspec_management_tool(
        request.tool_name,
        request.input,
        request.kernel,
        request.workspace_root,
        request.caller_agent_id,
    ) {
        return Some(ToolDispatchOutcome::Dispatched(result));
    }
    if let Some(outcome) = dispatch_tool_run_supervision_tool(request).await {
        return Some(outcome);
    }
    if let Some(outcome) = dispatch_media_environment_tool(request).await {
        return Some(outcome);
    }
    if let Some(outcome) = dispatch_process_extension_tool(request).await {
        return Some(outcome);
    }
    let browser_canvas_dispatch: std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<ToolDispatchOutcome>> + Send + '_>,
    > = Box::pin(dispatch_browser_canvas_tool(request));
    browser_canvas_dispatch.await
}

async fn dispatch_tool_run_supervision_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "tool_run_start" | "tool_run_status" | "tool_run_result" | "tool_run_cancel"
        | "tool_run_list" => {
            dispatch_tool_run_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.allowed_tools,
                request.caller_agent_id,
                request.allowed_env_vars,
                request.workspace_root,
                request.exec_policy,
            )
            .await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_media_environment_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "image_analyze" | "media_pipeline" | "media_describe" | "media_transcribe"
        | "video_analyze" | "image_generate" | "text_to_speech" | "speech_to_text" => {
            dispatch_media_tool(
                request.tool_name,
                request.input,
                request.media_engine,
                request.tts_engine,
                request.workspace_root,
                request.tool_use_id,
            )
            .await
        }
        "docker_exec" => {
            dispatch_docker_tool(
                request.tool_name,
                request.input,
                request.docker_config,
                request.workspace_root,
                request.caller_agent_id,
            )
            .await
        }
        "location_get" | "system_time" => dispatch_location_tool(request.tool_name).await,
        "system_update" => {
            dispatch_system_update(request.input, request.kernel, request.caller_agent_id).await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_process_extension_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    let result = match request.tool_name {
        "self_improvement_review"
        | "system_bug_report"
        | "system_bug_list"
        | "system_bug_update"
        | "learning_review_list"
        | "learning_review_decide"
        | "workflow_learning_list"
        | "skill_refinement_propose"
        | "skill_refinement_list"
        | "skill_refinement_decide"
        | "skill_refinement_update"
        | "skill_refinement_restore" => {
            dispatch_improvement_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.caller_agent_id,
                request.skill_registry,
            )
            .await
        }
        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            dispatch_process_tool(
                request.tool_name,
                request.input,
                request.process_manager,
                request.caller_agent_id,
            )
            .await
        }
        "hand_list" | "hand_activate" | "hand_status" | "hand_deactivate" | "scaffold_hand" => {
            dispatch_hand_tool(request.tool_name, request.input, request.kernel).await
        }
        "skill_execute" | "scaffold_skill" => {
            dispatch_skill_runtime_tool(
                request.tool_name,
                request.input,
                request.kernel,
                request.workspace_root,
            )
            .await
        }
        "a2a_discover" | "a2a_send" => {
            dispatch_a2a_tool(request.tool_name, request.input, request.kernel).await
        }
        _ => return None,
    };

    Some(ToolDispatchOutcome::Dispatched(result))
}

async fn dispatch_browser_canvas_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    match request.tool_name {
        "browser_batch"
        | "browser_navigate"
        | "browser_click"
        | "browser_type"
        | "browser_keys"
        | "browser_select"
        | "browser_hover"
        | "browser_screenshot"
        | "browser_read_page"
        | "browser_close"
        | "browser_scroll"
        | "browser_wait"
        | "browser_run_js"
        | "browser_back"
        | "browser_status"
        | "browser_network_log"
        | "browser_observe"
        | "browser_diagnostics" => Some(ToolDispatchOutcome::Browser(
            dispatch_browser_tool(
                request.tool_name,
                request.input,
                request.browser_ctx,
                request.caller_agent_id,
            )
            .await,
        )),
        "canvas_present" => Some(ToolDispatchOutcome::Dispatched(
            dispatch_canvas_tool(request.tool_name, request.input, request.workspace_root).await,
        )),
        _ => None,
    }
}
