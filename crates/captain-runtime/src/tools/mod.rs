//! Modular tool runtime surface.

pub mod a2a_definitions;
pub mod a2a_dispatch;
pub mod a2a_ops;
pub mod agent_definitions;
pub mod agent_dispatch;
pub mod agent_ops;
pub mod automation_dispatch;
pub mod browser_definitions;
pub mod browser_dispatch;
pub mod builtin_definitions;
pub mod canvas_dispatch;
pub mod canvas_ops;
pub mod capability_search;
pub mod capspec_definitions;
pub mod capspec_dispatch;
pub mod capspec_ops;
pub mod channel_definitions;
pub mod channel_dispatch;
pub mod channel_ops;
pub(crate) mod channel_policy;
pub mod code_execution;
pub mod codex_auth_ops;
pub mod config_definitions;
pub mod config_dispatch;
pub mod config_ops;
pub mod coordination_definitions;
pub mod coordination_dispatch;
pub mod coordination_ops;
pub mod discovery;
pub mod discovery_dispatch;
pub mod discovery_ops;
pub mod dispatch_fallback;
pub mod dispatch_finalize;
pub mod dispatch_guard;
pub mod docker_dispatch;
pub mod docker_ops;
pub mod docs_ops;
pub mod document_definitions;
pub mod document_dispatch;
pub mod document_extract;
pub mod document_ops;
pub mod errors;
pub mod execution_context;
pub mod file_definitions;
pub mod file_dispatch;
pub mod file_ops;
pub mod file_paths;
pub mod file_search;
pub mod fleet_definitions;
pub mod goal_definitions;
pub mod goal_dispatch;
pub mod goal_ops;
pub mod hand_definitions;
pub mod hand_dispatch;
pub mod hand_ops;
pub mod image_ops;
mod improvement_common;
pub mod improvement_definitions;
pub mod improvement_dispatch;
pub mod improvement_ops;
pub mod input;
pub mod kernel_access;
pub mod knowledge_definitions;
pub mod knowledge_dispatch;
pub mod knowledge_ops;
pub mod location_definitions;
pub mod location_dispatch;
pub mod location_ops;
pub mod mcp_definitions;
pub mod mcp_ops;
pub mod media_dispatch;
pub mod media_ops;
pub mod memory_commit;
pub mod memory_context;
pub mod memory_context_batch;
pub mod memory_definitions;
pub mod memory_dispatch;
pub mod memory_ops;
pub mod meta_definitions;
pub mod multimedia_definitions;
pub mod output;
pub mod package_definitions;
pub mod package_dispatch;
pub mod package_ops;
pub mod patch_ops;
pub mod peer_definitions;
pub mod peer_dispatch;
pub mod peer_ops;
pub mod process_dispatch;
pub mod process_ops;
pub mod progress;
pub mod project_definitions;
pub mod project_dispatch;
pub(crate) mod project_input;
pub mod project_ops;
pub mod registry;
pub mod schedule_definitions;
pub mod schedule_ops;
pub mod screenshot;
pub mod search;
pub mod security;
pub mod session_workspace_definitions;
pub mod session_workspace_ops;
pub mod shell_definitions;
pub mod shell_dispatch;
pub mod shell_ops;
pub mod skill_check;
pub mod skill_definitions;
pub(crate) mod skill_linked_files;
pub mod skill_refinement_ops;
pub(crate) mod skill_refinement_output;
pub(crate) mod skill_refinement_snapshots;
pub mod skill_runtime_dispatch;
pub mod skill_runtime_ops;
pub mod skill_search;
pub mod skill_view;
pub(crate) mod skill_view_validation;
pub mod ssh_definitions;
pub mod ssh_dispatch;
pub mod ssh_ops;
pub mod streaming;
pub mod system_bug_ops;
pub mod tool_run_definitions;
pub mod tool_run_dispatch;
pub mod tool_run_ops;
pub mod update_definitions;
pub mod update_dispatch;
pub mod video_ops;
pub mod voice_ops;
pub mod web_credentials_ops;
pub mod web_definitions;
pub mod web_dispatch;
pub mod web_download;
pub mod web_ops;

pub use a2a_definitions::a2a_tool_definitions;
pub(crate) use a2a_dispatch::dispatch_a2a_tool;
pub(crate) use a2a_ops::{tool_a2a_discover, tool_a2a_send};
pub use agent_definitions::agent_tool_definitions;
pub(crate) use agent_dispatch::dispatch_agent_tool;
#[cfg(test)]
pub(crate) use agent_ops::validate_child_agent_tool_scope;
pub(crate) use agent_ops::{
    tool_agent_caps, tool_agent_correct, tool_agent_delegate, tool_agent_kill, tool_agent_list,
    tool_agent_send, tool_agent_spawn, tool_agent_status, tool_agent_watch,
    tool_fleet_close_manager, tool_fleet_configure_autoscale, tool_fleet_create_manager,
    tool_fleet_list_managers, tool_fleet_metrics, tool_fleet_set_mission,
};
pub(crate) use automation_dispatch::dispatch_automation_tool;
pub use browser_definitions::browser_tool_definitions;
pub(crate) use browser_dispatch::dispatch_browser_tool;
pub use builtin_definitions::builtin_tool_definitions;
pub(crate) use canvas_dispatch::dispatch_canvas_tool;
#[cfg(test)]
pub(crate) use canvas_ops::sanitize_canvas_html;
pub(crate) use canvas_ops::tool_canvas_present;
pub use capability_search::search_capabilities;
pub use capspec_definitions::capspec_management_tool_definitions;
pub(crate) use capspec_dispatch::dispatch_capspec_management_tool;
pub(crate) use capspec_ops::tool_capability_forge;
pub use channel_definitions::channel_tool_definitions;
pub(crate) use channel_dispatch::dispatch_channel_tool;
pub(crate) use channel_ops::{
    tool_channel_delivery_batch, tool_channel_reconfigure, tool_channel_send,
    tool_telegram_get_topic, tool_telegram_set_topic,
};
pub(crate) use code_execution::tool_execute_code;
#[cfg(test)]
pub(crate) use code_execution::{find_python_interpreter, validate_pip_allowlist};
pub(crate) use codex_auth_ops::{
    tool_codex_auth_status, tool_codex_login_poll, tool_codex_login_start, tool_codex_tool_probe,
};
pub use config_definitions::config_tool_definitions;
pub(crate) use config_dispatch::dispatch_config_tool;
pub(crate) use config_ops::{
    tool_config_read, tool_config_schema, tool_config_setup, tool_config_write,
    tool_model_switch_apply, tool_model_switch_plan, tool_secret_read, tool_secret_write,
    tool_self_configure,
};
pub use coordination_definitions::coordination_tool_definitions;
pub(crate) use coordination_dispatch::dispatch_coordination_tool;
pub(crate) use coordination_ops::{
    tool_agent_find, tool_event_publish, tool_task_claim, tool_task_complete, tool_task_list,
    tool_task_post,
};
pub(crate) use discovery::lexical_tool_score;
pub use discovery::{discovery_tool_definitions, search_deferred_builtin_tools};
pub(crate) use discovery_dispatch::dispatch_discovery_tool;
pub(crate) use discovery_ops::{
    tool_capability_search, tool_search, tool_skill_check, tool_skill_search, tool_skill_view,
};
pub(crate) use dispatch_fallback::dispatch_fallback_tool;
pub(crate) use dispatch_finalize::{finalize_dispatch_result, DispatchFinalizeContext};
pub(crate) use dispatch_guard::{
    cached_tool_result, run_pre_dispatch_checks, shell_exec_approval_preview,
};
pub(crate) use docker_dispatch::dispatch_docker_tool;
pub(crate) use docker_ops::tool_docker_exec;
pub(crate) use docs_ops::tool_captain_docs;
pub use document_definitions::document_tool_definitions;
pub(crate) use document_dispatch::dispatch_document_tool;
#[cfg(test)]
pub(crate) use document_extract::extract_pdf_text_from_bytes;
pub(crate) use document_extract::tool_document_extract;
pub(crate) use document_ops::tool_document_pipeline;
pub(crate) use errors::{
    is_retryable_tool, is_write_tool_that_must_not_be_masked, render_error_with_suggestion,
};
pub use execution_context::current_agent_lineage_depth;
pub use execution_context::{current_agent_depth, with_agent_lineage_depth, CANVAS_MAX_BYTES};
pub(crate) use execution_context::{AGENT_CALL_DEPTH, MAX_AGENT_CALL_DEPTH};
pub use file_definitions::file_tool_definitions;
pub(crate) use file_dispatch::dispatch_file_tool;
pub(crate) use file_ops::{
    tool_edit_file, tool_file_list, tool_file_read, tool_file_write, tool_multi_edit,
};
pub(crate) use file_paths::{resolve_file_path, resolve_file_path_for_caller, validate_path};
pub(crate) use file_search::{tool_file_inspect_batch, tool_glob, tool_grep};
pub use fleet_definitions::fleet_tool_definitions;
pub use goal_definitions::goal_tool_definitions;
pub(crate) use goal_dispatch::dispatch_goal_tool;
pub(crate) use goal_ops::{
    tool_goal_apply_suggestion, tool_goal_create, tool_goal_delete, tool_goal_list,
    tool_goal_list_suggestions, tool_goal_pause, tool_goal_reject_suggestion, tool_goal_resume,
    tool_goal_status,
};
pub use hand_definitions::hand_tool_definitions;
pub(crate) use hand_dispatch::dispatch_hand_tool;
pub(crate) use hand_ops::{
    tool_hand_activate, tool_hand_deactivate, tool_hand_list, tool_hand_status, tool_scaffold_hand,
};
#[cfg(test)]
pub(crate) use image_ops::{detect_image_format, extract_image_dimensions, format_file_size};
pub(crate) use image_ops::{tool_image_analyze, tool_image_generate};
pub use improvement_definitions::improvement_tool_definitions;
pub(crate) use improvement_dispatch::dispatch_improvement_tool;
pub(crate) use improvement_ops::{
    tool_learning_review_decide, tool_learning_review_list, tool_self_improvement_review,
    tool_workflow_learning_list,
};
pub(crate) use input::{collect_string_list, hex_nibble};
pub(crate) use kernel_access::require_kernel;
pub use knowledge_definitions::knowledge_tool_definitions;
pub(crate) use knowledge_dispatch::dispatch_knowledge_tool;
pub(crate) use knowledge_ops::{
    tool_knowledge_add_entity, tool_knowledge_add_relation, tool_knowledge_query,
};
pub use location_definitions::location_tool_definitions;
pub(crate) use location_dispatch::dispatch_location_tool;
pub(crate) use location_ops::{tool_location_get, tool_system_time};
pub use mcp_definitions::mcp_tool_definitions;
pub(crate) use mcp_ops::{tool_mcp_catalog_search, tool_mcp_integration_install, tool_mcp_status};
pub(crate) use media_dispatch::dispatch_media_tool;
pub(crate) use media_ops::{tool_media_describe, tool_media_pipeline, tool_media_transcribe};
pub(crate) use memory_commit::{tool_memory_forget, tool_memory_save};
pub(crate) use memory_context::{
    compact_memory_context_result, DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
};
#[cfg(test)]
pub(crate) use memory_context::{compact_mempalace_search_result, memory_context_tokens};
pub(crate) use memory_context_batch::tool_memory_context_batch;
pub use memory_definitions::memory_tool_definitions;
pub(crate) use memory_dispatch::dispatch_memory_tool;
#[cfg(test)]
pub(crate) use memory_ops::memory_recall_part;
pub(crate) use memory_ops::{
    call_mempalace_tool, tool_memory_recall, tool_memory_recall_mempalace, tool_memory_store,
    tool_memory_store_mempalace,
};
pub use meta_definitions::meta_tool_definitions;
pub use multimedia_definitions::multimedia_tool_definitions;
pub(crate) use output::truncate_owned;
pub use package_definitions::package_tool_definitions;
pub(crate) use package_dispatch::dispatch_package_tool;
pub(crate) use package_ops::{
    tool_pkg_wrapper, CARGO_SUBCOMMANDS, NPM_SUBCOMMANDS, PIP_SUBCOMMANDS,
};
pub(crate) use patch_ops::tool_apply_patch;
pub use peer_definitions::peer_tool_definitions;
pub(crate) use peer_dispatch::dispatch_peer_tool;
pub(crate) use peer_ops::tool_peer_list;
pub(crate) use process_dispatch::dispatch_process_tool;
pub(crate) use process_ops::{
    tool_process_kill, tool_process_list, tool_process_poll, tool_process_start, tool_process_write,
};
pub(crate) use progress::emit_progress;
pub use progress::{
    current_origin_channel, progress_sink, with_origin_channel, with_progress_sink,
    ProgressThrottle, ToolProgressEvent,
};
pub use project_definitions::project_tool_definitions;
pub(crate) use project_dispatch::dispatch_project_tool;
pub(crate) use project_ops::{
    tool_checkpoint_save, tool_milestone_complete, tool_milestone_create, tool_milestone_list,
    tool_milestone_progress, tool_project_archive, tool_project_create, tool_project_delete,
    tool_project_get, tool_project_list, tool_project_resume, tool_project_task_create,
    tool_project_task_list, tool_project_task_update,
};
pub use registry::ToolRegistry;
pub use schedule_definitions::schedule_tool_definitions;
#[cfg(test)]
pub(crate) use schedule_ops::{ensure_cron_webhook_url_is_public, parse_schedule_to_cron};
pub(crate) use schedule_ops::{
    tool_cron_cancel, tool_cron_create, tool_cron_list, tool_cron_update, tool_file_trigger_list,
    tool_file_trigger_register, tool_file_trigger_remove, tool_file_trigger_set_enabled,
    tool_reminder_set, tool_schedule_create, tool_schedule_delete, tool_schedule_list,
    tool_todo_complete, tool_todo_create, tool_todo_delete, tool_todo_list, tool_todo_reopen,
};
#[cfg(test)]
pub(crate) use screenshot::screenshot_command;
pub(crate) use screenshot::tool_screenshot;
pub(crate) use search::{
    lexical_weighted_score, query_tokens, result_name, result_score, result_source,
    snippet_for_tokens,
};
pub(crate) use security::{
    check_taint_browser_batch, check_taint_net_fetch, check_taint_shell_exec,
    ensure_no_secret_literal,
};
pub use session_workspace_definitions::session_workspace_tool_definitions;
pub(crate) use session_workspace_ops::{
    tool_session_recall, tool_session_tool_call_summary, tool_workspace_add,
};
pub use shell_definitions::shell_tool_definitions;
pub(crate) use shell_dispatch::{dispatch_shell_exec, ShellDispatchOutcome};
pub(crate) use shell_ops::tool_shell_exec;
pub use skill_check::check_skill;
pub use skill_definitions::skill_tool_definitions;
#[cfg(test)]
pub(crate) use skill_refinement_ops::SKILL_REFINEMENTS_KEY;
pub(crate) use skill_refinement_ops::{
    tool_skill_refinement_decide, tool_skill_refinement_list, tool_skill_refinement_propose,
    tool_skill_refinement_restore, tool_skill_refinement_update,
};
pub(crate) use skill_runtime_dispatch::dispatch_skill_runtime_tool;
pub(crate) use skill_runtime_ops::{tool_scaffold_skill, tool_skill_md_execute};
pub use skill_search::search_skills;
pub use skill_view::view_skill;
pub use ssh_definitions::ssh_tool_definitions;
pub(crate) use ssh_dispatch::dispatch_ssh_tool;
pub(crate) use ssh_ops::{
    tool_ssh_download, tool_ssh_exec, tool_ssh_health_check, tool_ssh_upload,
};
pub(crate) use streaming::emit_tool_chunk;
pub use streaming::{ToolStreamCtx, TOOL_STREAM};
#[cfg(test)]
pub(crate) use system_bug_ops::SYSTEM_BUGS_KEY;
pub(crate) use system_bug_ops::{
    tool_system_bug_list, tool_system_bug_report, tool_system_bug_update,
};
pub use tool_run_definitions::tool_run_tool_definitions;
pub(crate) use tool_run_dispatch::dispatch_tool_run_tool;
pub(crate) use tool_run_ops::{
    tool_run_cancel, tool_run_list, tool_run_result, tool_run_start, tool_run_status,
    ToolRunStartContext,
};
pub use update_definitions::update_tool_definitions;
pub(crate) use update_dispatch::dispatch_system_update;
pub(crate) use video_ops::tool_video_analyze;
pub(crate) use voice_ops::{tool_speech_to_text, tool_text_to_speech};
pub(crate) use web_credentials_ops::tool_web_credentials_update;
#[cfg(test)]
pub(crate) use web_credentials_ops::{hash_web_password, write_web_credentials_config};
pub use web_definitions::web_tool_definitions;
pub(crate) use web_dispatch::{dispatch_web_tool, WebDispatchOutcome};
pub(crate) use web_download::tool_web_download;
#[cfg(test)]
pub(crate) use web_download::{ensure_extension_for_mime, sanitize_download_filename};
pub(crate) use web_ops::{tool_web_fetch_legacy, tool_web_research_batch, tool_web_search_legacy};
