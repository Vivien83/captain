//! Route handlers for the Captain API.

pub use crate::a2a_routes::{
    a2a_agent_card, a2a_cancel_task, a2a_discover_external, a2a_external_task_status, a2a_get_task,
    a2a_list_agents, a2a_list_external_agents, a2a_send_external, a2a_send_task,
};
pub use crate::agent_api_egress_config_routes::{
    configure_agent_api_egress, test_agent_api_egress,
};
pub use crate::agent_api_egress_routes::agent_api_egress_retry;
pub use crate::agent_api_manifest_routes::agent_api_manifest;
pub use crate::agent_api_routes::{
    agent_api_egress_status, agent_api_events, agent_api_ingress, agent_api_status,
};
pub use crate::agent_api_token_routes::rotate_agent_api_token;
pub use crate::agent_config_routes::{clone_agent, patch_agent_config, update_agent_identity};
pub use crate::agent_control_routes::{interrupt_agent, stop_agent};
pub use crate::agent_delivery_routes::get_agent_deliveries;
pub use crate::agent_file_routes::{
    get_agent_file, list_agent_files, set_agent_file, KNOWN_IDENTITY_FILES,
};
pub use crate::agent_lifecycle_routes::{
    fleet_metrics, get_agent, kill_agent, list_agents, list_fleets, restart_agent,
};
pub use crate::agent_message_routes::{
    answer_message, inject_attachments_into_session, send_message, send_message_stream,
};
pub use crate::agent_runtime_config_routes::{
    get_agent_mcp_servers, get_agent_skills, get_agent_tools, model_switch_apply,
    model_switch_plan, set_agent_mcp_servers, set_agent_skills, set_agent_tools, set_model,
    ModelSwitchApplyRequest, ModelSwitchPlanRequest,
};
pub use crate::agent_session_view_routes::get_agent_session;
pub use crate::agent_spawn_routes::spawn_agent;
pub use crate::agent_update_routes::{patch_agent, update_agent};
pub use crate::approval_routes::{
    approve_always_request, approve_request, approve_session_request, clear_session_approvals,
    create_approval, list_approvals, reject_request, CreateApprovalRequest,
};
pub use crate::audit_routes::{audit_recent, audit_repair, audit_verify, logs_stream};
pub use crate::binding_routes::{add_binding, list_bindings, remove_binding};
pub use crate::channel_routes::{
    clear_inbound_dead_letters, configure_channel, list_channels, reload_channels, remove_channel,
    test_channel,
};
pub use crate::clawhub_routes::{
    clawhub_browse, clawhub_install, clawhub_search, clawhub_skill_code, clawhub_skill_detail,
};
pub use crate::command_routes::list_commands;
pub use crate::comms_routes::{
    comms_events, comms_events_stream, comms_send, comms_task, comms_topology,
};
pub use crate::config_routes::{
    config_raw_get, config_raw_put, config_reload, config_schema, config_set, config_template_get,
    config_validate, get_config,
};
pub use crate::consciousness_routes::{
    get_consciousness_mood, get_consciousness_neuromodulators, get_consciousness_user_state,
    graph_consciousness, graph_consciousness_digest_preview, graph_consciousness_digest_send,
    graph_delete_entity, graph_dream_cycle, graph_entities, graph_entity_detail, graph_facts,
    graph_invalidate_fact, graph_search, graph_stats,
};
pub use crate::cron_routes::{
    create_cron_job, cron_job_status, delete_cron_job, list_cron_jobs, run_cron_job,
    toggle_cron_job, update_cron_job,
};
pub use crate::feedback_routes::{get_feedback, submit_feedback};
pub use crate::hand_install_routes::{install_hand, install_hand_deps, upsert_hand};
pub use crate::hand_instance_routes::{
    activate_hand, deactivate_hand, get_hand_settings, hand_instance_browser, hand_stats,
    pause_hand, resume_hand, update_hand_settings,
};
pub use crate::hand_routes::{check_hand_deps, get_hand, list_active_hands, list_hands};
pub use crate::health_routes::{health, health_detail, prometheus_metrics};
pub use crate::integration_routes::{
    add_integration, integrations_health, list_available_integrations, list_integrations,
    reconnect_integration, reload_integrations, remove_integration,
};
pub use crate::kv_routes::{delete_agent_kv_key, get_agent_kv, get_agent_kv_key, set_agent_kv_key};
pub use crate::mcp_routes::{list_mcp_servers, mcp_http};
pub use crate::memory_event_routes::memory_events_stream;
pub use crate::memory_migration_routes::memory_migrate_to_mempalace;
pub use crate::model_routes::{
    add_custom_model, get_model, list_aliases, list_models, list_providers, remove_custom_model,
    update_model_pricing,
};
pub use crate::model_update_routes::{decide_model_update, list_model_updates};
pub use crate::pairing_routes::{
    pairing_complete, pairing_devices, pairing_notify, pairing_remove_device, pairing_request,
};
pub use crate::peer_routes::{list_peers, network_status};
pub use crate::process_routes::kill_process;
pub use crate::profile_routes::{list_profiles, set_agent_mode};
pub use crate::provider_oauth_routes::{copilot_oauth_poll, copilot_oauth_start};
pub use crate::provider_routes::{
    delete_provider_key, set_provider_key, set_provider_url, test_provider,
};
pub use crate::schedule_routes::{
    create_schedule, delete_schedule, list_schedules, run_schedule, update_schedule,
};
pub use crate::security_routes::security_status;
pub use crate::session_routes::{
    clear_agent_history, compact_session, create_agent_session, delete_session,
    find_session_by_label, get_session, list_agent_sessions, list_session_events, list_sessions,
    reset_session, restore_session, set_session_label, switch_agent_session,
};
pub use crate::skill_routes::{
    create_skill, install_skill, list_skills, marketplace_search, uninstall_skill,
};
pub use crate::state::AppState;
pub use crate::status_routes::status;
pub use crate::system_routes::{add_workspace_path, shutdown, version};
pub use crate::telegram_topic_routes::{
    delete_telegram_topic, list_telegram_topics, set_telegram_topic,
};
pub use crate::template_routes::{get_template, list_templates};
pub use crate::tool_routes::list_tools;
pub use crate::trigger_routes::{
    create_file_trigger, create_trigger, delete_file_trigger, delete_trigger, list_file_triggers,
    list_triggers, update_file_trigger, update_trigger,
};
pub use crate::upload_routes::{register_upload, resolve_attachments, serve_upload, upload_file};
pub use crate::usage_budget_routes::{
    agent_budget_ranking, agent_budget_status, budget_status, update_agent_budget, update_budget,
    usage_by_model, usage_daily, usage_stats, usage_summary,
};
pub use crate::voice_routes::{get_stt, update_stt};
pub use crate::web_auth_routes::{auth_check, auth_login, auth_logout};
pub use crate::webhook_routes::{webhook_agent, webhook_wake};
pub use crate::whatsapp_routes::{whatsapp_qr_start, whatsapp_qr_status};
pub use crate::workflow_routes::{
    create_workflow, delete_workflow, get_workflow, list_workflow_runs, list_workflows,
    run_workflow, update_workflow,
};
