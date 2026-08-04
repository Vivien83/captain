//! Agent and fleet orchestration handlers.

use crate::core_tools::SUBAGENT_DEFAULT_TOOLS;
use crate::kernel_handle::KernelHandle;
use crate::tools::{require_kernel, AGENT_CALL_DEPTH, MAX_AGENT_CALL_DEPTH};
use captain_types::agent::AgentManifest;
use captain_types::agent_api::{AgentApiSpawnProvisionReport, AgentApiSpawnProvisionRequest};
use captain_types::agent_delegation::{
    AgentDelegationJobRecord, AgentDelegationStatus, AGENT_DELEGATION_MAX_DEPTH,
    AGENT_DELEGATION_MAX_LINEAGE_TOKENS,
};
use captain_types::tool_compat::normalize_tool_name;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= MAX_AGENT_CALL_DEPTH {
        return Err(format!(
            "Inter-agent call depth exceeded (max {}). \
             A->B->C chain is too deep. Use the task queue instead.",
            MAX_AGENT_CALL_DEPTH
        ));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            kh.send_to_agent(agent_id, message).await
        })
        .await
}

pub(crate) async fn tool_agent_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    parent_id: Option<&str>,
    parent_allowed_tools: Option<&[String]>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manifest_toml = input["manifest_toml"]
        .as_str()
        .ok_or("Missing 'manifest_toml' parameter")?;
    let provision_request = parse_agent_api_spawn_provision(input)?;
    validate_child_agent_tool_scope(manifest_toml, parent_allowed_tools)?;
    let (id, name) = kh.spawn_agent(manifest_toml, parent_id).await?;
    let api_report = kh
        .provision_spawned_agent_api(&id, provision_request)
        .await
        .ok();
    Ok(format_agent_spawn_success(&id, &name, api_report.as_ref()))
}

fn parse_agent_api_spawn_provision(
    input: &serde_json::Value,
) -> Result<AgentApiSpawnProvisionRequest, String> {
    match input.get("agent_api") {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone())
            .map_err(|err| format!("Invalid 'agent_api' provisioning object: {err}")),
        _ => Ok(AgentApiSpawnProvisionRequest::default()),
    }
}

fn format_agent_spawn_success(
    id: &str,
    name: &str,
    api_report: Option<&AgentApiSpawnProvisionReport>,
) -> String {
    let mut output = format!("Agent spawned successfully.\n  ID: {id}\n  Name: {name}");
    let Some(report) = api_report else {
        output.push_str(&format!(
            "\n\nAgent API protocol:\n  Status: provisioning_unavailable\n  Next: inspect with captain agent api {id} or GET /api/agents/{id}/api"
        ));
        return output;
    };

    output.push_str(&format!(
        "\n\nAgent API protocol ({})\n  Status: {}\n  Manifest: {}\n  Events: {}",
        report.protocol, report.status, report.manifest_url, report.audit_events_url
    ));
    output.push_str(&format!(
        "\n\nIngress\n  Status: {}\n  URL: {}\n  Auth: {}\n  Token env: {}\n  Rotate: {}",
        report.ingress.status,
        report.ingress.ingress_url,
        report.ingress.auth_scheme,
        report.ingress.token_env,
        report.ingress.token_rotate_url
    ));
    if let Some(token) = report.ingress.token.as_deref() {
        output.push_str(&format!("\n  Token returned once: {token}"));
    }
    if let Some(warning) = report.ingress.warning.as_deref() {
        output.push_str(&format!("\n  Warning: {warning}"));
    }
    output.push_str(&format!(
        "\n\nEgress\n  Status: {}\n  Configure: {}\n  Test: {}\n  Queue: {}\n  Retry: {}",
        report.egress.status,
        report.egress.configure_url,
        report.egress.test_url,
        report.egress.queue_status_url,
        report.egress.retry_url_template
    ));
    if let Some(callback_secret) = report.egress.callback_secret.as_deref() {
        output.push_str(&format!(
            "\n  Callback secret returned once: {callback_secret}"
        ));
    }
    if let Some(issue) = report.egress.issue.as_deref() {
        output.push_str(&format!("\n  Issue: {issue}"));
    }
    if !report.operator_actions.is_empty() {
        output.push_str("\n\nOperator actions");
        for action in &report.operator_actions {
            output.push_str(&format!("\n  - {action}"));
        }
    }
    output
}

fn effective_child_tool_policy(manifest: &AgentManifest) -> Option<Vec<String>> {
    let mut tools = explicit_child_tool_policy(manifest)?;
    add_subagent_default_tools(&mut tools);
    Some(tools)
}

fn explicit_child_tool_policy(manifest: &AgentManifest) -> Option<Vec<String>> {
    if !manifest.tool_allowlist.is_empty() {
        if manifest.tool_allowlist.iter().any(|t| t == "*") {
            return None;
        }
        return Some(normalized_tool_list(&manifest.tool_allowlist));
    }

    if !manifest.capabilities.tools.is_empty() {
        if manifest.capabilities.tools.iter().any(|t| t == "*") {
            return None;
        }
        return Some(normalized_tool_list(&manifest.capabilities.tools));
    }

    None
}

fn normalized_tool_list(tools: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tool in tools {
        push_normalized_tool(&mut out, tool);
    }
    out
}

fn push_normalized_tool(tools: &mut Vec<String>, tool: &str) {
    let normalized = normalize_tool_name(tool);
    if !tools
        .iter()
        .any(|existing| normalize_tool_name(existing) == normalized)
    {
        tools.push(normalized.to_string());
    }
}

fn add_subagent_default_tools(tools: &mut Vec<String>) {
    for tool in SUBAGENT_DEFAULT_TOOLS {
        push_normalized_tool(tools, tool);
    }
}

pub(crate) fn validate_child_agent_tool_scope(
    manifest_toml: &str,
    parent_allowed_tools: Option<&[String]>,
) -> Result<(), String> {
    let child_manifest: AgentManifest = toml::from_str(manifest_toml)
        .map_err(|e| captain_types::agent::format_agent_manifest_parse_error(&e, manifest_toml))?;
    let child_tools = effective_child_tool_policy(&child_manifest).ok_or_else(|| {
        "Denied agent_spawn: child manifest must declare an explicit non-wildcard \
         tool_allowlist or capabilities.tools. Sub-agents cannot rely on a profile-only \
         or unrestricted tool surface."
            .to_string()
    })?;

    let Some(parent_allowed_tools) = parent_allowed_tools else {
        return Ok(());
    };
    if parent_allowed_tools.iter().any(|t| t == "*") {
        return Ok(());
    }

    let mut parent_tools: HashSet<String> = parent_allowed_tools
        .iter()
        .map(|t| normalize_tool_name(t).to_string())
        .collect();
    for tool in SUBAGENT_DEFAULT_TOOLS {
        parent_tools.insert(normalize_tool_name(tool).to_string());
    }
    let denied: Vec<String> = child_tools
        .iter()
        .map(|t| normalize_tool_name(t).to_string())
        .filter(|t| !parent_tools.contains(t))
        .collect();

    if denied.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Denied agent_spawn: child requests tools outside parent scope: {}",
            denied.join(", ")
        ))
    }
}

pub(crate) fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = format!("Running agents ({}):\n", agents.len());
    for a in &agents {
        output.push_str(&format!(
            "  - {} (id: {}, state: {}, model: {}:{})\n",
            a.name, a.id, a.state, a.model_provider, a.model_name
        ));
    }
    Ok(output)
}

pub(crate) fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    kh.kill_agent(agent_id)?;
    Ok(format!("Agent {agent_id} killed successfully."))
}

pub(crate) fn tool_agent_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let status = kh.agent_status_info(agent_id)?;
    Ok(serde_json::to_string_pretty(&status).unwrap_or_else(|_| status.to_string()))
}

pub(crate) fn tool_agent_caps(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let report = kh.agent_capability_report(agent_id)?;
    Ok(serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string()))
}

pub(crate) async fn tool_agent_watch(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let limit = input["limit"].as_u64().unwrap_or(10) as usize;
    let events = kh.agent_events(agent_id, limit).await?;
    if events.is_empty() {
        return Ok(format!("No recent events for agent {agent_id}."));
    }
    Ok(serde_json::to_string_pretty(&events).unwrap_or_else(|_| format!("{events:?}")))
}

pub(crate) async fn tool_agent_delegate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    tool_use_id: &str,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller_agent_id = caller_agent_id.ok_or("Agent ID required for agent_delegate")?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let task = input["task"].as_str().ok_or("Missing 'task'")?;
    let max_tokens = input["max_tokens"].as_u64().unwrap_or(5000);
    let depends_on = parse_agent_job_dependencies(input)?;
    let title = input["title"]
        .as_str()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| delegation_title(task));
    let idempotency_key = delegation_idempotency_key(caller_agent_id, tool_use_id);
    let job = kh.start_agent_delegation(
        caller_agent_id,
        agent_id,
        &title,
        task,
        max_tokens,
        &depends_on,
        &idempotency_key,
    )?;
    if !input["wait_for_result"].as_bool().unwrap_or(false) {
        return Ok(render_agent_job(&job, false, false));
    }

    let timeout = input["timeout_seconds"]
        .as_u64()
        .unwrap_or(120)
        .clamp(1, 600);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        let current = kh
            .agent_delegation_status(caller_agent_id, &job.id)?
            .ok_or_else(|| format!("Delegation job vanished: {}", job.id))?;
        if current.status.is_terminal() {
            return Ok(render_agent_job(&current, true, false));
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(render_agent_job(&current, false, true));
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

pub(crate) fn tool_agent_job_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller = caller_agent_id.ok_or("Agent ID required for agent_job_status")?;
    let job_id = input["job_id"].as_str().ok_or("Missing 'job_id'")?;
    let job = kh
        .agent_delegation_status(caller, job_id)?
        .ok_or_else(|| format!("Delegation job not found: {job_id}"))?;
    Ok(render_agent_job(&job, false, false))
}

pub(crate) fn tool_agent_job_result(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller = caller_agent_id.ok_or("Agent ID required for agent_job_result")?;
    let job_id = input["job_id"].as_str().ok_or("Missing 'job_id'")?;
    let job = kh
        .agent_delegation_status(caller, job_id)?
        .ok_or_else(|| format!("Delegation job not found: {job_id}"))?;
    Ok(render_agent_job(&job, job.status.is_terminal(), false))
}

pub(crate) fn tool_agent_job_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller = caller_agent_id.ok_or("Agent ID required for agent_job_list")?;
    let status = input["status"]
        .as_str()
        .map(|status| {
            AgentDelegationStatus::parse(status).ok_or_else(|| {
                format!(
                    "Unknown delegation status '{status}'. Use blocked, queued, running, \
                     cancel_requested, succeeded, failed, cancelled, uncertain, or dependency_failed."
                )
            })
        })
        .transpose()?;
    let jobs = kh.list_agent_delegations(
        caller,
        status,
        input["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize,
    )?;
    let jobs = jobs
        .iter()
        .map(|job| agent_job_value(job, false, false))
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "count": jobs.len(),
        "jobs": jobs,
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn tool_agent_job_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller = caller_agent_id.ok_or("Agent ID required for agent_job_cancel")?;
    let job_id = input["job_id"].as_str().ok_or("Missing 'job_id'")?;
    let job = kh.cancel_agent_delegation(caller, job_id)?;
    Ok(render_agent_job(&job, false, false))
}

pub(crate) fn tool_agent_job_resume(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller = caller_agent_id.ok_or("Agent ID required for agent_job_resume")?;
    let job_id = input["job_id"].as_str().ok_or("Missing 'job_id'")?;
    let job = kh.resume_agent_delegation(caller, job_id)?;
    Ok(render_agent_job(&job, false, false))
}

fn parse_agent_job_dependencies(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let Some(value) = input.get("depends_on") else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or("'depends_on' must be an array of delegation job ids")?;
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .ok_or_else(|| "'depends_on' contains an invalid job id".to_string())
        })
        .collect()
}

fn delegation_idempotency_key(caller_agent_id: &str, tool_use_id: &str) -> String {
    format!(
        "agent-delegation:{}",
        blake3::hash(format!("{caller_agent_id}\0{tool_use_id}").as_bytes()).to_hex()
    )
}

fn delegation_title(task: &str) -> String {
    let first_line = task
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Delegated task");
    captain_types::truncate_str(first_line, 160).to_string()
}

fn render_agent_job(
    job: &AgentDelegationJobRecord,
    include_result: bool,
    wait_timed_out: bool,
) -> String {
    serde_json::to_string_pretty(&agent_job_value(job, include_result, wait_timed_out))
        .unwrap_or_else(|_| format!("Delegation job {}: {}", job.id, job.status.as_str()))
}

fn agent_job_value(
    job: &AgentDelegationJobRecord,
    include_result: bool,
    wait_timed_out: bool,
) -> serde_json::Value {
    let terminal = job.status.is_terminal();
    let cancellable = matches!(
        job.status,
        AgentDelegationStatus::Blocked
            | AgentDelegationStatus::Queued
            | AgentDelegationStatus::Running
            | AgentDelegationStatus::CancelRequested
    );
    let resumable = matches!(
        job.status,
        AgentDelegationStatus::Failed
            | AgentDelegationStatus::Uncertain
            | AgentDelegationStatus::DependencyFailed
    );
    serde_json::json!({
        "job_id": job.id,
        "root_job_id": job.root_job_id,
        "parent_job_id": job.parent_job_id,
        "depth": job.depth,
        "max_depth": AGENT_DELEGATION_MAX_DEPTH,
        "title": job.title,
        "target_agent_id": job.target_agent_id,
        "status": job.status.as_str(),
        "detached": true,
        "state_version": job.state_version,
        "attempt_count": job.attempt_count,
        "depends_on": job.depends_on,
        "budget_tokens": job.max_tokens,
        "lineage_reserved_tokens": job.lineage_reserved_tokens,
        "lineage_budget_tokens": AGENT_DELEGATION_MAX_LINEAGE_TOKENS,
        "lineage_budget_remaining_tokens":
            AGENT_DELEGATION_MAX_LINEAGE_TOKENS.saturating_sub(job.lineage_reserved_tokens),
        "used_tokens": job.used_tokens,
        "budget_exceeded": job.used_tokens.is_some_and(|used| used > job.max_tokens),
        "result_available": terminal && job.result.is_some(),
        "result_truncated": job.result_truncated,
        "result": include_result.then(|| job.result.clone()).flatten(),
        "error_code": job.error_code,
        "error_message": job.error_message,
        "cancellable": cancellable,
        "resumable": resumable,
        "wait_timed_out": wait_timed_out,
        "replay_requires_explicit_resume": job.status == AgentDelegationStatus::Uncertain,
        "next_actions": if terminal {
            if resumable {
                vec![
                    "Inspect this record, then use agent_job_resume only if replay is intentional.",
                    "Use agent_job_list to inspect sibling and dependent jobs.",
                ]
            } else {
                vec!["Use agent_job_result to read the bounded final result."]
            }
        } else {
            vec![
                "Continue other independent work; do not block on this job.",
                "Use agent_job_status or agent_job_list to return to it later.",
                "Use agent_job_cancel if it should stop.",
            ]
        },
    })
}

pub(crate) async fn tool_agent_correct(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let message = input["message"].as_str().ok_or("Missing 'message'")?;
    kh.inject_system_message(agent_id, message).await?;
    Ok(format!("Correction sent to agent {agent_id}."))
}

pub(crate) async fn tool_fleet_create_manager(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = input["name"].as_str().ok_or("Missing 'name'")?;
    let domain = input["domain"].as_str().ok_or("Missing 'domain'")?;
    let model = input["model"].as_str();
    let budget = input["budget_tokens"].as_u64().unwrap_or(10000);
    let (id, spawned_name) = kh.create_manager(name, domain, model, budget).await?;
    Ok(format!(
        "Manager '{spawned_name}' created (id: {id}, budget: {budget} tokens/h)."
    ))
}

pub(crate) fn tool_fleet_list_managers(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let managers = kh.list_managers();
    if managers.is_empty() {
        return Ok("No active managers.".to_string());
    }
    Ok(serde_json::to_string_pretty(&managers).unwrap_or_else(|_| format!("{managers:?}")))
}

pub(crate) async fn tool_fleet_close_manager(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let killed = kh.close_manager(manager_id).await?;
    Ok(format!("Manager closed. {killed} agent(s) terminated."))
}

pub(crate) fn tool_fleet_set_mission(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let mission = input["mission"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    kh.set_manager_mission(manager_id, mission)?;
    Ok(match mission {
        Some(m) => format!("Mission set: {m}"),
        None => "Mission cleared.".to_string(),
    })
}

pub(crate) fn tool_fleet_configure_autoscale(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let cfg = captain_types::agent::AutoScaleConfig {
        enabled: input["enabled"].as_bool().unwrap_or(true),
        min_workers: input["min_workers"].as_u64().unwrap_or(0) as u32,
        max_workers: input["max_workers"].as_u64().unwrap_or(3) as u32,
        spawn_threshold: input["spawn_threshold"].as_u64().unwrap_or(2) as u32,
        kill_threshold: input["kill_threshold"].as_u64().unwrap_or(0) as u32,
        cooldown_secs: input["cooldown_secs"].as_u64().unwrap_or(60),
        worker_template: input["worker_template"].as_str().map(String::from),
    };
    kh.configure_autoscale(manager_id, cfg.clone())?;
    Ok(format!(
        "Autoscale configured: min={} max={} spawn>={} kill<={} cooldown={}s",
        cfg.min_workers,
        cfg.max_workers,
        cfg.spawn_threshold,
        cfg.kill_threshold,
        cfg.cooldown_secs
    ))
}

pub(crate) fn tool_fleet_metrics(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let metrics = kh.fleet_metrics(manager_id)?;
    Ok(serde_json::to_string_pretty(&metrics).unwrap_or_else(|_| format!("{metrics:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::{
        agent::AgentId,
        agent_api::{pending_egress_report, ready_ingress_report, AgentApiSpawnProvisionReport},
    };

    #[test]
    fn agent_spawn_success_includes_api_protocol_and_actions() {
        let agent_id: AgentId = "99999999-9999-9999-9999-999999999999".parse().unwrap();
        let report = AgentApiSpawnProvisionReport::new(
            &agent_id,
            ready_ingress_report(&agent_id, "cap_at_test-token-value".to_string()),
            pending_egress_report(&agent_id),
            Vec::new(),
        );

        let output =
            format_agent_spawn_success(&agent_id.to_string(), "veille-tech", Some(&report));

        assert!(output.contains("Agent spawned successfully."));
        assert!(output.contains("ID: 99999999-9999-9999-9999-999999999999"));
        assert!(output.contains("Name: veille-tech"));
        assert!(output.contains("Agent API protocol (agent-as-service.v1)"));
        assert!(output.contains("Status: ingress_ready"));
        assert!(output.contains("/hooks/agents/99999999-9999-9999-9999-999999999999/ingress"));
        assert!(output.contains("Token returned once: cap_at_test-token-value"));
        assert!(output
            .contains("/api/agents/99999999-9999-9999-9999-999999999999/api/egress/configure"));
        assert!(output.contains("cannot infer the external callback URL"));
        assert!(output.contains("Operator actions"));
    }

    #[test]
    fn agent_api_spawn_provision_defaults_to_ingress_token() {
        let parsed = parse_agent_api_spawn_provision(&serde_json::json!({})).unwrap();

        assert!(parsed.provision_ingress_token);
        assert!(parsed.generate_callback_secret);
        assert!(parsed.egress_callback_url.is_none());
    }

    #[test]
    fn delegation_idempotency_is_stable_and_scoped_to_caller_and_tool_call() {
        let first = delegation_idempotency_key("captain", "tool-42");
        assert_eq!(first, delegation_idempotency_key("captain", "tool-42"));
        assert_ne!(first, delegation_idempotency_key("captain", "tool-43"));
        assert_ne!(first, delegation_idempotency_key("other-agent", "tool-42"));
        assert!(first.starts_with("agent-delegation:"));
    }

    #[test]
    fn job_projection_hides_result_until_explicit_result_read() {
        let mut job = delegation_record(AgentDelegationStatus::Succeeded);
        job.result = Some("bounded private result".to_string());
        let status = agent_job_value(&job, false, false);
        assert!(status["result"].is_null());
        assert_eq!(status["result_available"], true);
        assert_eq!(status["root_job_id"], "job-root");
        assert_eq!(status["parent_job_id"], "job-parent");
        assert_eq!(status["depth"], 3);
        assert_eq!(status["max_depth"], AGENT_DELEGATION_MAX_DEPTH);
        assert_eq!(status["lineage_reserved_tokens"], 15_000);
        assert_eq!(
            status["lineage_budget_tokens"],
            AGENT_DELEGATION_MAX_LINEAGE_TOKENS
        );
        assert_eq!(
            status["lineage_budget_remaining_tokens"],
            AGENT_DELEGATION_MAX_LINEAGE_TOKENS - 15_000
        );
        assert!(status.get("task").is_none());

        let result = agent_job_value(&job, true, false);
        assert_eq!(result["result"], "bounded private result");
    }

    #[test]
    fn uncertain_projection_requires_an_explicit_replay_decision() {
        let job = delegation_record(AgentDelegationStatus::Uncertain);
        let value = agent_job_value(&job, false, false);
        assert_eq!(value["resumable"], true);
        assert_eq!(value["replay_requires_explicit_resume"], true);
        assert!(value["next_actions"][0]
            .as_str()
            .unwrap()
            .contains("replay is intentional"));
    }

    fn delegation_record(status: AgentDelegationStatus) -> AgentDelegationJobRecord {
        AgentDelegationJobRecord {
            id: "job-42".to_string(),
            idempotency_key: "idem-42".to_string(),
            root_job_id: "job-root".to_string(),
            parent_job_id: Some("job-parent".to_string()),
            depth: 3,
            lineage_reserved_tokens: 15_000,
            caller_agent_id: "captain".to_string(),
            target_agent_id: "reviewer".to_string(),
            title: "Review".to_string(),
            task: "Private task".to_string(),
            max_tokens: 5_000,
            depends_on: Vec::new(),
            status,
            state_version: 1,
            attempt_count: 1,
            lease_owner: None,
            lease_expires_at_unix_ms: None,
            effect_state: captain_types::agent_delegation::AgentDelegationEffectState::Completed,
            result: None,
            result_truncated: false,
            used_tokens: Some(100),
            error_code: None,
            error_message: None,
            cancel_requested_at_unix_ms: None,
            started_at_unix_ms: Some(1),
            completed_at_unix_ms: Some(2),
            created_at_unix_ms: 1,
            updated_at_unix_ms: 2,
        }
    }
}
