//! Agent and fleet tool dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_agent_caps, tool_agent_correct, tool_agent_delegate, tool_agent_job_cancel,
    tool_agent_job_list, tool_agent_job_result, tool_agent_job_resume, tool_agent_job_status,
    tool_agent_kill, tool_agent_list, tool_agent_send, tool_agent_spawn, tool_agent_status,
    tool_agent_watch, tool_fleet_close_manager, tool_fleet_configure_autoscale,
    tool_fleet_create_manager, tool_fleet_list_managers, tool_fleet_metrics,
    tool_fleet_set_mission,
};

pub(crate) async fn dispatch_agent_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    allowed_tools: Option<&[String]>,
    tool_use_id: &str,
) -> Result<String, String> {
    match tool_name {
        "agent_send" => tool_agent_send(input, kernel).await,
        "agent_spawn" => tool_agent_spawn(input, kernel, caller_agent_id, allowed_tools).await,
        "agent_list" => tool_agent_list(kernel),
        "agent_kill" => tool_agent_kill(input, kernel),
        "agent_status" => tool_agent_status(input, kernel),
        "agent_caps" => tool_agent_caps(input, kernel),
        "agent_watch" => tool_agent_watch(input, kernel).await,
        "agent_delegate" => tool_agent_delegate(input, kernel, caller_agent_id, tool_use_id).await,
        "agent_job_status" => tool_agent_job_status(input, kernel, caller_agent_id),
        "agent_job_result" => tool_agent_job_result(input, kernel, caller_agent_id),
        "agent_job_list" => tool_agent_job_list(input, kernel, caller_agent_id),
        "agent_job_cancel" => tool_agent_job_cancel(input, kernel, caller_agent_id),
        "agent_job_resume" => tool_agent_job_resume(input, kernel, caller_agent_id),
        "agent_correct" => tool_agent_correct(input, kernel).await,
        "fleet_create_manager" => tool_fleet_create_manager(input, kernel).await,
        "fleet_list_managers" => tool_fleet_list_managers(kernel),
        "fleet_close_manager" => tool_fleet_close_manager(input, kernel).await,
        "fleet_set_mission" => tool_fleet_set_mission(input, kernel),
        "fleet_configure_autoscale" => tool_fleet_configure_autoscale(input, kernel),
        "fleet_metrics" => tool_fleet_metrics(input, kernel),
        other => Err(format!("Unknown agent/fleet tool: {other}")),
    }
}
